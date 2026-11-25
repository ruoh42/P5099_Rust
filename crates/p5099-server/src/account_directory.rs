use std::{
    collections::{HashMap, VecDeque},
    fs, io,
    net::{IpAddr, SocketAddr},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex, MutexGuard,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use p5099_core::{
    account_protocol::{
        ACCOUNT_NONCE_BYTES, AccountFailure, AccountKey, AccountRequest, AccountResponse,
        LAUNCH_TICKET_BYTES, MAX_ACCOUNT_LINE_BYTES, PlayerPresence, RoomPresenceInfo,
        constant_time_eq, from_hex, login_proof, password_key, to_hex, validate_password,
    },
    nickname::{canonical_nickname_key, normalize_nickname},
};
use serde::{Deserialize, Serialize};

use crate::{WorldHandle, connection_limit::ConnectionLimit};
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    net::{TcpListener, TcpStream},
    sync::watch,
};

pub(crate) const ACCOUNT_DIRECTORY_FILE_NAME: &str = "Accounts.json";
const TICKET_LIFETIME: Duration = Duration::from_hours(24);
const FAILED_LOGIN_WINDOW: Duration = Duration::from_mins(10);
const MAX_FAILED_LOGINS: usize = 10;
const MAX_FAILED_LOGINS_PER_ACCOUNT: usize = 30;
const MAX_THROTTLED_KEYS: usize = 4_096;
const REGISTRATION_WINDOW: Duration = Duration::from_hours(1);
const MAX_REGISTRATIONS: usize = 5;
const CONNECTION_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_REQUESTS_PER_CONNECTION: usize = 8;
const MAX_CONNECTIONS: usize = 128;
const MAX_CONNECTIONS_PER_ADDRESS: usize = 8;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct AccountBook {
    #[serde(default)]
    accounts: Vec<StoredAccount>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    banned_addresses: Vec<AddressBan>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub(crate) struct AddressBan {
    pub(crate) address: IpAddr,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) reason: Option<String>,
    pub(crate) created: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct StoredAccount {
    nickname: String,
    key: String,
    created: u64,
    #[serde(default)]
    admin: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    banned: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    ban_reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    banned_until: Option<u64>,
}

impl StoredAccount {
    fn suspended(&self, now: u64) -> bool {
        self.banned && self.banned_until.is_none_or(|until| now < until)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct AccountSummary {
    pub(crate) nickname: String,
    pub(crate) created: u64,
    pub(crate) admin: bool,
    pub(crate) banned: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) ban_reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) banned_until: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct AccountDetail {
    #[serde(flatten)]
    pub(crate) summary: AccountSummary,
    pub(crate) ticket_active: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) last_login: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) last_login_address: Option<IpAddr>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct ThrottleSummary {
    pub(crate) address: IpAddr,
    pub(crate) failed_logins: usize,
    pub(crate) registrations: usize,
    pub(crate) limited: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum AccountAdminError {
    NotFound,
    Exists,
    InvalidNickname,
    InvalidPassword,
    Unavailable,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AccountGrant {
    pub(crate) nickname: String,
    pub(crate) admin: bool,
}

#[derive(Debug)]
struct IssuedTicket {
    ticket: String,
    expires: Instant,
    issued: u64,
    peer: IpAddr,
}

#[derive(Debug, Default)]
struct Throttle {
    failed_logins: HashMap<IpAddr, VecDeque<Instant>>,
    registrations: HashMap<IpAddr, VecDeque<Instant>>,
    failed_accounts: HashMap<String, VecDeque<Instant>>,
}

fn prune<K>(events: &mut HashMap<K, VecDeque<Instant>>, window: Duration, now: Instant) {
    if events.len() > MAX_THROTTLED_KEYS {
        events.retain(|_, events| recent(events, window, now) > 0);
    }
}

fn recent(events: &mut VecDeque<Instant>, window: Duration, now: Instant) -> usize {
    while events
        .front()
        .is_some_and(|event| now.duration_since(*event) > window)
    {
        events.pop_front();
    }
    events.len()
}

#[derive(Debug)]
pub(crate) struct AccountDirectory {
    path: Option<PathBuf>,
    book: Mutex<AccountBook>,
    tickets: Mutex<HashMap<String, IssuedTicket>>,
    throttle: Mutex<Throttle>,
    registration_open: AtomicBool,
    maintenance: AtomicBool,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

pub(crate) fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs())
}

impl AccountDirectory {
    pub(crate) fn in_memory() -> Self {
        Self {
            path: None,
            book: Mutex::new(AccountBook::default()),
            tickets: Mutex::new(HashMap::new()),
            throttle: Mutex::new(Throttle::default()),
            registration_open: AtomicBool::new(true),
            maintenance: AtomicBool::new(false),
        }
    }

    pub(crate) fn open(profile_root: &Path, registration_open: bool) -> io::Result<Self> {
        let path = profile_root.join(ACCOUNT_DIRECTORY_FILE_NAME);
        let book = match fs::read(&path) {
            Ok(bytes) => serde_json::from_slice(&bytes).map_err(|error| {
                io::Error::other(format!("{} is unreadable: {error}", path.display()))
            })?,
            Err(error) if error.kind() == io::ErrorKind::NotFound => AccountBook::default(),
            Err(error) => return Err(error),
        };
        Ok(Self {
            path: Some(path),
            book: Mutex::new(book),
            tickets: Mutex::new(HashMap::new()),
            throttle: Mutex::new(Throttle::default()),
            registration_open: AtomicBool::new(registration_open),
            maintenance: AtomicBool::new(false),
        })
    }

    fn persist(&self, book: &AccountBook) -> io::Result<()> {
        let Some(path) = &self.path else {
            return Ok(());
        };
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let bytes = serde_json::to_vec_pretty(book).map_err(io::Error::other)?;
        let temporary = path.with_extension("json.tmp");
        fs::write(&temporary, bytes)?;
        fs::rename(&temporary, path)
    }

    fn find<'a>(book: &'a AccountBook, nickname: &str) -> Option<&'a StoredAccount> {
        let key = canonical_nickname_key(nickname);
        book.accounts
            .iter()
            .find(|account| canonical_nickname_key(&account.nickname) == key)
    }

    fn stored_key(&self, nickname: &str) -> Option<(String, AccountKey)> {
        let book = lock(&self.book);
        let account = Self::find(&book, nickname)?;
        let key = from_hex(&account.key).ok()?;
        Some((account.nickname.clone(), key))
    }

    pub(crate) fn register(
        &self,
        nickname: &str,
        key: AccountKey,
        peer: IpAddr,
    ) -> Result<String, AccountFailure> {
        if !self.registration_open() {
            return Err(AccountFailure::RegistrationClosed);
        }
        if self.address_banned(peer) {
            return Err(AccountFailure::Banned);
        }
        let nickname = normalize_nickname(nickname).map_err(|_| AccountFailure::InvalidNickname)?;
        {
            let mut throttle = lock(&self.throttle);
            let events = throttle.registrations.entry(peer).or_default();
            if recent(events, REGISTRATION_WINDOW, Instant::now()) >= MAX_REGISTRATIONS {
                return Err(AccountFailure::RateLimited);
            }
        }
        let mut book = lock(&self.book);
        if Self::find(&book, &nickname).is_some() {
            return Err(AccountFailure::Exists);
        }
        let mut updated = book.clone();
        updated.accounts.push(StoredAccount {
            nickname: nickname.clone(),
            key: to_hex(&key),
            created: unix_now(),
            admin: false,
            banned: false,
            ban_reason: None,
            banned_until: None,
        });
        self.persist(&updated).map_err(|error| {
            tracing::error!(%error, "failed to save the account directory");
            AccountFailure::Unavailable
        })?;
        *book = updated;
        drop(book);
        let now = Instant::now();
        let mut throttle = lock(&self.throttle);
        throttle
            .registrations
            .entry(peer)
            .or_default()
            .push_back(now);
        prune(&mut throttle.registrations, REGISTRATION_WINDOW, now);
        Ok(nickname)
    }

    fn check_rate(&self, peer: IpAddr, nickname: &str) -> Result<(), AccountFailure> {
        let key = canonical_nickname_key(nickname);
        let trusted = lock(&self.tickets)
            .get(&key)
            .is_some_and(|ticket| ticket.peer.to_canonical() == peer.to_canonical());
        let now = Instant::now();
        let mut throttle = lock(&self.throttle);
        let by_address = throttle.failed_logins.entry(peer).or_default();
        if recent(by_address, FAILED_LOGIN_WINDOW, now) >= MAX_FAILED_LOGINS {
            return Err(AccountFailure::RateLimited);
        }
        let by_account = throttle
            .failed_accounts
            .get_mut(&key)
            .map_or(0, |events| recent(events, FAILED_LOGIN_WINDOW, now));
        if by_account >= MAX_FAILED_LOGINS_PER_ACCOUNT && !trusted {
            return Err(AccountFailure::RateLimited);
        }
        Ok(())
    }

    fn record_failure(&self, peer: IpAddr, nickname: &str) {
        let known = Self::find(&lock(&self.book), nickname).is_some();
        let now = Instant::now();
        let mut throttle = lock(&self.throttle);
        throttle
            .failed_logins
            .entry(peer)
            .or_default()
            .push_back(now);
        prune(&mut throttle.failed_logins, FAILED_LOGIN_WINDOW, now);
        if known {
            throttle
                .failed_accounts
                .entry(canonical_nickname_key(nickname))
                .or_default()
                .push_back(now);
            prune(&mut throttle.failed_accounts, FAILED_LOGIN_WINDOW, now);
        }
    }

    fn verify(
        &self,
        nickname: &str,
        nonce: &[u8],
        proof: &[u8; 32],
        peer: IpAddr,
    ) -> Result<String, AccountFailure> {
        self.check_rate(peer, nickname)?;
        if self.address_banned(peer) {
            return Err(AccountFailure::Banned);
        }
        let verified = self
            .stored_key(nickname)
            .filter(|(_, key)| constant_time_eq(&login_proof(key, nonce), proof));
        let nickname = verified.map(|(nickname, _)| nickname).ok_or_else(|| {
            self.record_failure(peer, nickname);
            AccountFailure::BadCredentials
        })?;
        if Self::find(&lock(&self.book), &nickname)
            .is_some_and(|account| account.suspended(unix_now()))
        {
            return Err(AccountFailure::Banned);
        }
        Ok(nickname)
    }

    pub(crate) fn login(
        &self,
        nickname: &str,
        nonce: &[u8],
        proof: &[u8; 32],
        peer: IpAddr,
    ) -> Result<(String, String), AccountFailure> {
        let nickname = self.verify(nickname, nonce, proof, peer)?;
        if self.maintenance() && !self.is_admin(&nickname) {
            return Err(AccountFailure::Maintenance);
        }
        let ticket = to_hex(&rand::random::<[u8; LAUNCH_TICKET_BYTES]>());
        lock(&self.tickets).insert(
            canonical_nickname_key(&nickname),
            IssuedTicket {
                ticket: ticket.clone(),
                expires: Instant::now() + TICKET_LIFETIME,
                issued: unix_now(),
                peer,
            },
        );
        Ok((nickname, ticket))
    }

    pub(crate) fn change_password(
        &self,
        nickname: &str,
        nonce: &[u8],
        proof: &[u8; 32],
        new_key: AccountKey,
        peer: IpAddr,
    ) -> Result<(), AccountFailure> {
        let nickname = self.verify(nickname, nonce, proof, peer)?;
        let mut book = lock(&self.book);
        let mut updated = book.clone();
        let key = canonical_nickname_key(&nickname);
        for account in &mut updated.accounts {
            if canonical_nickname_key(&account.nickname) == key {
                account.key = to_hex(&new_key);
            }
        }
        self.persist(&updated).map_err(|error| {
            tracing::error!(%error, "failed to save the account directory");
            AccountFailure::Unavailable
        })?;
        *book = updated;
        drop(book);
        lock(&self.tickets).remove(&key);
        Ok(())
    }

    pub(crate) fn redeem(&self, nickname: &str, ticket: &str) -> Option<AccountGrant> {
        let key = canonical_nickname_key(nickname);
        {
            let mut tickets = lock(&self.tickets);
            let issued = tickets.get(&key)?;
            if Instant::now() > issued.expires {
                tickets.remove(&key);
                return None;
            }
            if !constant_time_eq(issued.ticket.as_bytes(), ticket.trim().as_bytes()) {
                return None;
            }
        }
        let book = lock(&self.book);
        let account =
            Self::find(&book, nickname).filter(|account| !account.suspended(unix_now()))?;
        Some(AccountGrant {
            nickname: account.nickname.clone(),
            admin: account.admin,
        })
    }

    pub(crate) fn is_admin(&self, nickname: &str) -> bool {
        Self::find(&lock(&self.book), nickname).is_some_and(|account| account.admin)
    }

    pub(crate) fn address_banned(&self, address: IpAddr) -> bool {
        let address = address.to_canonical();
        lock(&self.book)
            .banned_addresses
            .iter()
            .any(|ban| ban.address == address)
    }

    pub(crate) fn maintenance(&self) -> bool {
        self.maintenance.load(Ordering::Relaxed)
    }
}

impl AccountDirectory {
    pub(crate) fn registration_open(&self) -> bool {
        self.registration_open.load(Ordering::Relaxed)
    }

    pub(crate) fn set_registration_open(&self, open: bool) {
        self.registration_open.store(open, Ordering::Relaxed);
    }

    pub(crate) fn set_maintenance(&self, on: bool) {
        self.maintenance.store(on, Ordering::Relaxed);
    }

    fn summary(account: &StoredAccount, now: u64) -> AccountSummary {
        let banned = account.suspended(now);
        AccountSummary {
            nickname: account.nickname.clone(),
            created: account.created,
            admin: account.admin,
            banned,
            ban_reason: account.ban_reason.clone().filter(|_| banned),
            banned_until: account.banned_until.filter(|_| banned),
        }
    }

    pub(crate) fn summaries(&self) -> Vec<AccountSummary> {
        let now = unix_now();
        lock(&self.book)
            .accounts
            .iter()
            .map(|account| Self::summary(account, now))
            .collect()
    }

    pub(crate) fn detail(&self, nickname: &str) -> Option<AccountDetail> {
        let summary = Self::summary(Self::find(&lock(&self.book), nickname)?, unix_now());
        let tickets = lock(&self.tickets);
        let ticket = tickets.get(&canonical_nickname_key(&summary.nickname));
        Some(AccountDetail {
            ticket_active: ticket.is_some_and(|ticket| Instant::now() <= ticket.expires),
            last_login: ticket.map(|ticket| ticket.issued),
            last_login_address: ticket.map(|ticket| ticket.peer),
            summary,
        })
    }

    pub(crate) fn create(
        &self,
        nickname: &str,
        password: &str,
        admin: bool,
    ) -> Result<String, AccountAdminError> {
        let nickname =
            normalize_nickname(nickname).map_err(|_| AccountAdminError::InvalidNickname)?;
        validate_password(password).map_err(|_| AccountAdminError::InvalidPassword)?;
        if Self::find(&lock(&self.book), &nickname).is_some() {
            return Err(AccountAdminError::Exists);
        }
        let key = to_hex(&password_key(&nickname, password));
        let mut book = lock(&self.book);
        if Self::find(&book, &nickname).is_some() {
            return Err(AccountAdminError::Exists);
        }
        let mut updated = book.clone();
        updated.accounts.push(StoredAccount {
            nickname: nickname.clone(),
            key,
            created: unix_now(),
            admin,
            banned: false,
            ban_reason: None,
            banned_until: None,
        });
        self.save(&mut book, updated)?;
        Ok(nickname)
    }

    pub(crate) fn delete(&self, nickname: &str) -> Result<String, AccountAdminError> {
        let key = canonical_nickname_key(nickname);
        let mut book = lock(&self.book);
        let mut updated = book.clone();
        let index = updated
            .accounts
            .iter()
            .position(|account| canonical_nickname_key(&account.nickname) == key)
            .ok_or(AccountAdminError::NotFound)?;
        let removed = updated.accounts.remove(index);
        self.save(&mut book, updated)?;
        drop(book);
        lock(&self.tickets).remove(&key);
        Ok(removed.nickname)
    }

    fn save(
        &self,
        book: &mut MutexGuard<'_, AccountBook>,
        updated: AccountBook,
    ) -> Result<(), AccountAdminError> {
        self.persist(&updated).map_err(|error| {
            tracing::error!(%error, "failed to save the account directory");
            AccountAdminError::Unavailable
        })?;
        **book = updated;
        Ok(())
    }

    fn modify(
        &self,
        nickname: &str,
        change: impl FnOnce(&mut StoredAccount),
    ) -> Result<String, AccountAdminError> {
        let key = canonical_nickname_key(nickname);
        let mut book = lock(&self.book);
        let mut updated = book.clone();
        let account = updated
            .accounts
            .iter_mut()
            .find(|account| canonical_nickname_key(&account.nickname) == key)
            .ok_or(AccountAdminError::NotFound)?;
        change(account);
        let registered = account.nickname.clone();
        self.save(&mut book, updated)?;
        Ok(registered)
    }

    pub(crate) fn set_admin(
        &self,
        nickname: &str,
        admin: bool,
    ) -> Result<String, AccountAdminError> {
        self.modify(nickname, |account| account.admin = admin)
    }

    pub(crate) fn set_banned(
        &self,
        nickname: &str,
        banned: bool,
        reason: Option<String>,
        until: Option<u64>,
    ) -> Result<String, AccountAdminError> {
        let registered = self.modify(nickname, |account| {
            account.banned = banned;
            account.ban_reason = if banned { reason } else { None };
            account.banned_until = if banned { until } else { None };
        })?;
        if banned {
            lock(&self.tickets).remove(&canonical_nickname_key(&registered));
        }
        Ok(registered)
    }

    pub(crate) fn reset_password(
        &self,
        nickname: &str,
        password: &str,
    ) -> Result<String, AccountAdminError> {
        validate_password(password).map_err(|_| AccountAdminError::InvalidPassword)?;
        let key = to_hex(&password_key(nickname, password));
        let registered = self.modify(nickname, |account| account.key = key)?;
        lock(&self.tickets).remove(&canonical_nickname_key(&registered));
        Ok(registered)
    }

    pub(crate) fn address_bans(&self) -> Vec<AddressBan> {
        lock(&self.book).banned_addresses.clone()
    }

    pub(crate) fn set_address_banned(
        &self,
        address: IpAddr,
        banned: bool,
        reason: Option<String>,
    ) -> Result<bool, AccountAdminError> {
        let address = address.to_canonical();
        let mut book = lock(&self.book);
        let mut updated = book.clone();
        let existing = updated
            .banned_addresses
            .iter()
            .position(|ban| ban.address == address);
        match (banned, existing) {
            (true, Some(index)) => updated.banned_addresses[index].reason = reason,
            (true, None) => updated.banned_addresses.push(AddressBan {
                address,
                reason,
                created: unix_now(),
            }),
            (false, Some(index)) => {
                updated.banned_addresses.remove(index);
            }
            (false, None) => return Ok(false),
        }
        let changed = updated != *book;
        self.save(&mut book, updated)?;
        drop(book);
        if banned {
            lock(&self.tickets).retain(|_, ticket| ticket.peer.to_canonical() != address);
        }
        Ok(changed)
    }

    pub(crate) fn throttles(&self) -> Vec<ThrottleSummary> {
        let now = Instant::now();
        let mut throttle = lock(&self.throttle);
        let Throttle {
            failed_logins,
            registrations,
            ..
        } = &mut *throttle;
        let mut addresses: Vec<IpAddr> = failed_logins
            .keys()
            .chain(registrations.keys())
            .copied()
            .collect();
        addresses.sort_unstable();
        addresses.dedup();
        addresses
            .into_iter()
            .filter_map(|address| {
                let failed = failed_logins
                    .get_mut(&address)
                    .map_or(0, |events| recent(events, FAILED_LOGIN_WINDOW, now));
                let registered = registrations
                    .get_mut(&address)
                    .map_or(0, |events| recent(events, REGISTRATION_WINDOW, now));
                (failed > 0 || registered > 0).then_some(ThrottleSummary {
                    address,
                    failed_logins: failed,
                    registrations: registered,
                    limited: failed >= MAX_FAILED_LOGINS || registered >= MAX_REGISTRATIONS,
                })
            })
            .collect()
    }

    pub(crate) fn clear_throttles(&self, address: Option<IpAddr>) -> usize {
        let mut throttle = lock(&self.throttle);
        if let Some(address) = address {
            let failed = throttle.failed_logins.remove(&address).is_some();
            let registered = throttle.registrations.remove(&address).is_some();
            usize::from(failed || registered)
        } else {
            let mut addresses: Vec<IpAddr> = throttle
                .failed_logins
                .keys()
                .chain(throttle.registrations.keys())
                .copied()
                .collect();
            addresses.sort_unstable();
            addresses.dedup();
            *throttle = Throttle::default();
            addresses.len()
        }
    }
}

pub(crate) fn spawn_account_service(
    address: SocketAddr,
    accounts: Arc<AccountDirectory>,
    world: Option<WorldHandle>,
    shutdown: watch::Receiver<bool>,
) {
    tokio::spawn(async move {
        let listener = match TcpListener::bind(address).await {
            Ok(listener) => listener,
            Err(error) => {
                tracing::error!(%address, %error, "account service could not bind");
                return;
            }
        };
        tracing::info!(%address, "account service listening");
        accept_loop(listener, accounts, world, shutdown).await;
    });
}

async fn accept_loop(
    listener: TcpListener,
    accounts: Arc<AccountDirectory>,
    world: Option<WorldHandle>,
    mut shutdown: watch::Receiver<bool>,
) {
    let limit = ConnectionLimit::new(MAX_CONNECTIONS, MAX_CONNECTIONS_PER_ADDRESS);
    loop {
        tokio::select! {
            changed = shutdown.changed() => {
                if changed.is_err() || *shutdown.borrow() {
                    return;
                }
            }
            accepted = listener.accept() => match accepted {
                Ok((stream, peer)) => {
                    let Some(permit) = limit.try_admit(peer.ip()) else {
                        tracing::debug!(%peer, "account connection refused: too many connections");
                        continue;
                    };
                    let accounts = Arc::clone(&accounts);
                    let world = world.clone();
                    tokio::spawn(async move {
                        let _permit = permit;
                        let served = tokio::time::timeout(
                            CONNECTION_TIMEOUT,
                            serve_connection(stream, peer, &accounts, world.as_ref()),
                        )
                        .await;
                        if let Ok(Err(error)) = served {
                            tracing::debug!(%peer, %error, "account connection ended");
                        }
                    });
                }
                Err(error) => tracing::warn!(%error, "account service accept failed"),
            },
        }
    }
}

async fn serve_connection(
    stream: TcpStream,
    peer: SocketAddr,
    accounts: &AccountDirectory,
    world: Option<&WorldHandle>,
) -> io::Result<()> {
    let (reader, mut writer) = stream.into_split();
    let mut reader = BufReader::new(reader);
    let mut nonce: Option<(String, [u8; ACCOUNT_NONCE_BYTES])> = None;
    for _ in 0..MAX_REQUESTS_PER_CONNECTION {
        let mut line = Vec::new();
        let read = (&mut reader)
            .take(u64::try_from(MAX_ACCOUNT_LINE_BYTES + 1).unwrap_or(u64::MAX))
            .read_until(b'\n', &mut line)
            .await?;
        if read == 0 {
            return Ok(());
        }
        if line.last() != Some(&b'\n') {
            return Err(io::Error::other("account line too long or unterminated"));
        }
        let line = String::from_utf8_lossy(&line);
        let line = line.trim_end_matches(['\r', '\n']);
        let response = match AccountRequest::parse(line) {
            Ok(AccountRequest::Presence { nickname, ticket }) => {
                presence(accounts, world, &nickname, &ticket).await
            }
            _ => respond(accounts, peer.ip(), &mut nonce, line),
        };
        let mut reply = response.to_line();
        reply.push('\n');
        writer.write_all(reply.as_bytes()).await?;
    }
    Ok(())
}

async fn presence(
    accounts: &AccountDirectory,
    world: Option<&WorldHandle>,
    nickname: &str,
    ticket: &str,
) -> AccountResponse {
    let Some(grant) = accounts.redeem(nickname, ticket) else {
        return AccountResponse::Failed(AccountFailure::BadCredentials);
    };
    let Some(world) = world else {
        return AccountResponse::Presence(PlayerPresence::Offline);
    };
    match world.presence(&grant.nickname).await {
        Ok(None) => AccountResponse::Presence(PlayerPresence::Offline),
        Ok(Some(presence)) => {
            AccountResponse::Presence(presence.room.map_or(PlayerPresence::Lobby, |room| {
                PlayerPresence::Room(RoomPresenceInfo {
                    name: room.name,
                    game_type: room.game_type,
                    speed_type: room.speed_type,
                    racing: room.racing,
                    players: room.players,
                    capacity: room.capacity,
                })
            }))
        }
        Err(_) => AccountResponse::Failed(AccountFailure::Unavailable),
    }
}

fn respond(
    accounts: &AccountDirectory,
    peer: IpAddr,
    nonce: &mut Option<(String, [u8; ACCOUNT_NONCE_BYTES])>,
    line: &str,
) -> AccountResponse {
    let Ok(request) = AccountRequest::parse(line) else {
        return AccountResponse::Failed(AccountFailure::BadRequest);
    };
    let mut take_nonce = |nickname: &str| {
        nonce
            .take()
            .filter(|(owner, _)| canonical_nickname_key(owner) == canonical_nickname_key(nickname))
            .map(|(_, value)| value)
            .ok_or(AccountFailure::BadRequest)
    };
    let result = match request {
        AccountRequest::Register { nickname, key } => {
            accounts.register(&nickname, key, peer).map(|nickname| {
                tracing::info!(%peer, nickname, "registered rider account");
                AccountResponse::Registered { nickname }
            })
        }
        AccountRequest::Challenge { nickname } => {
            let value = rand::random::<[u8; ACCOUNT_NONCE_BYTES]>();
            *nonce = Some((nickname, value));
            Ok(AccountResponse::Nonce(value))
        }
        AccountRequest::Login { nickname, proof } => take_nonce(&nickname)
            .and_then(|value| accounts.login(&nickname, &value, &proof, peer))
            .map(|(nickname, ticket)| {
                tracing::info!(%peer, nickname, "issued launch ticket");
                AccountResponse::Ticket { nickname, ticket }
            }),
        AccountRequest::ChangePassword {
            nickname,
            proof,
            new_key,
        } => take_nonce(&nickname)
            .and_then(|value| accounts.change_password(&nickname, &value, &proof, new_key, peer))
            .map(|()| {
                tracing::info!(%peer, nickname, "changed rider password");
                AccountResponse::PasswordChanged
            }),
        AccountRequest::Presence { .. } => Err(AccountFailure::BadRequest),
    };
    result.unwrap_or_else(|failure| {
        tracing::debug!(%peer, failure = failure.code(), "account request refused");
        AccountResponse::Failed(failure)
    })
}

#[cfg(test)]
mod tests {
    use std::net::Ipv4Addr;

    use p5099_core::account_protocol::password_key;
    use tempfile::tempdir;

    use super::*;

    const PEER: IpAddr = IpAddr::V4(Ipv4Addr::new(203, 0, 113, 5));

    fn login_with(
        accounts: &AccountDirectory,
        nickname: &str,
        key: &AccountKey,
    ) -> Result<(String, String), AccountFailure> {
        let nonce = [5_u8; 32];
        accounts.login(nickname, &nonce, &login_proof(key, &nonce), PEER)
    }

    fn login(
        accounts: &AccountDirectory,
        nickname: &str,
        password: &str,
    ) -> Result<(String, String), AccountFailure> {
        login_with(accounts, nickname, &password_key(nickname, password))
    }

    #[test]
    fn registered_rider_logs_in_and_redeems_the_latest_ticket() {
        let accounts = AccountDirectory::in_memory();
        assert_eq!(
            accounts.register(" Rider ", password_key("Rider", "secret"), PEER),
            Ok("Rider".to_owned())
        );
        assert_eq!(
            accounts.register("rider", password_key("rider", "x"), PEER),
            Err(AccountFailure::Exists)
        );
        assert_eq!(
            login(&accounts, "Rider", "wrong"),
            Err(AccountFailure::BadCredentials)
        );
        let (nickname, first) = login(&accounts, "RIDER", "secret").unwrap();
        assert_eq!(nickname, "Rider");
        assert!(accounts.redeem("rider", &first).is_some());
        let (_, second) = login(&accounts, "Rider", "secret").unwrap();
        assert!(accounts.redeem("Rider", &first).is_none());
        assert_eq!(
            accounts.redeem("Rider", &second),
            Some(AccountGrant {
                nickname: "Rider".to_owned(),
                admin: false
            })
        );
        assert!(accounts.redeem("Other", &second).is_none());
    }

    #[test]
    fn guessing_one_account_from_many_addresses_is_limited_but_its_owner_is_not() {
        let accounts = AccountDirectory::in_memory();
        accounts
            .register("Rider", password_key("Rider", "correct-horse"), PEER)
            .unwrap();
        let nonce = [5_u8; 32];
        let right = login_proof(&password_key("Rider", "correct-horse"), &nonce);
        let wrong = login_proof(&password_key("Rider", "guess"), &nonce);
        accounts.login("Rider", &nonce, &right, PEER).unwrap();
        for attacker in 1..=3_u8 {
            let address = IpAddr::V4(Ipv4Addr::new(198, 51, 100, attacker));
            for _ in 0..MAX_FAILED_LOGINS {
                assert_eq!(
                    accounts.login("Rider", &nonce, &wrong, address),
                    Err(AccountFailure::BadCredentials)
                );
            }
        }
        let fresh = IpAddr::V4(Ipv4Addr::new(198, 51, 100, 9));
        assert_eq!(
            accounts.login("Rider", &nonce, &right, fresh),
            Err(AccountFailure::RateLimited)
        );
        assert!(accounts.login("Rider", &nonce, &right, PEER).is_ok());
    }

    #[test]
    fn banned_rider_cannot_log_in_or_redeem_a_ticket() {
        let accounts = AccountDirectory::in_memory();
        accounts
            .register("Rider", password_key("Rider", "secret"), PEER)
            .unwrap();
        let (_, ticket) = login(&accounts, "Rider", "secret").unwrap();
        assert_eq!(
            accounts.set_banned("rider", true, Some("abuse".to_owned()), None),
            Ok("Rider".to_owned())
        );
        assert!(accounts.redeem("Rider", &ticket).is_none());
        assert_eq!(
            login(&accounts, "Rider", "secret"),
            Err(AccountFailure::Banned)
        );
        assert_eq!(
            login(&accounts, "Rider", "guess"),
            Err(AccountFailure::BadCredentials)
        );

        accounts.set_banned("Rider", false, None, None).unwrap();
        assert_eq!(accounts.summaries()[0].ban_reason, None);
        let (_, ticket) = login(&accounts, "Rider", "secret").unwrap();
        assert!(accounts.redeem("Rider", &ticket).is_some());
    }

    #[test]
    fn temporary_bans_expire() {
        let accounts = AccountDirectory::in_memory();
        accounts
            .register("Rider", password_key("Rider", "secret"), PEER)
            .unwrap();
        accounts
            .set_banned(
                "Rider",
                true,
                Some("cooldown".to_owned()),
                Some(unix_now() + 600),
            )
            .unwrap();
        assert_eq!(
            login(&accounts, "Rider", "secret"),
            Err(AccountFailure::Banned)
        );
        accounts
            .set_banned("Rider", true, Some("over".to_owned()), Some(unix_now() - 1))
            .unwrap();
        let summary = &accounts.summaries()[0];
        assert!(!summary.banned);
        assert_eq!(summary.ban_reason, None);
        assert!(login(&accounts, "Rider", "secret").is_ok());
    }

    #[test]
    fn maintenance_admits_only_administrators() {
        let accounts = AccountDirectory::in_memory();
        accounts
            .register("Rider", password_key("Rider", "secret"), PEER)
            .unwrap();
        accounts.create("Operator", "operator-pass", true).unwrap();
        accounts.set_maintenance(true);
        assert_eq!(
            login(&accounts, "Rider", "secret"),
            Err(AccountFailure::Maintenance)
        );
        assert_eq!(
            login(&accounts, "Rider", "guess"),
            Err(AccountFailure::BadCredentials)
        );
        let (_, ticket) = login(&accounts, "operator", "operator-pass").unwrap();
        let detail = accounts.detail("OPERATOR").unwrap();
        assert!(detail.ticket_active);
        assert_eq!(detail.last_login_address, Some(PEER));
        assert!(accounts.redeem("Operator", &ticket).is_some());
        accounts.set_maintenance(false);
        assert!(login(&accounts, "Rider", "secret").is_ok());
    }

    #[test]
    fn operators_create_and_delete_accounts() {
        let accounts = AccountDirectory::in_memory();
        accounts.set_registration_open(false);
        assert_eq!(
            accounts.create(" Rider ", "secret-pass", false),
            Ok("Rider".to_owned())
        );
        assert_eq!(
            accounts.create("rider", "secret-pass", false),
            Err(AccountAdminError::Exists)
        );
        assert_eq!(
            accounts.create("Other", "x", false),
            Err(AccountAdminError::InvalidPassword)
        );
        let (_, ticket) = login(&accounts, "Rider", "secret-pass").unwrap();
        assert_eq!(accounts.delete("RIDER"), Ok("Rider".to_owned()));
        assert!(accounts.redeem("Rider", &ticket).is_none());
        assert_eq!(accounts.delete("Rider"), Err(AccountAdminError::NotFound));
    }

    #[test]
    fn banned_addresses_are_refused_and_persist() {
        let root = tempdir().unwrap();
        let accounts = AccountDirectory::open(root.path(), true).unwrap();
        accounts
            .register("Rider", password_key("Rider", "secret"), PEER)
            .unwrap();
        let (_, ticket) = login(&accounts, "Rider", "secret").unwrap();
        assert_eq!(
            accounts.set_address_banned(PEER, true, Some("flood".to_owned())),
            Ok(true)
        );
        assert!(accounts.redeem("Rider", &ticket).is_none());
        assert_eq!(
            login(&accounts, "Rider", "secret"),
            Err(AccountFailure::Banned)
        );
        assert_eq!(
            accounts.register("New", password_key("New", "pass"), PEER),
            Err(AccountFailure::Banned)
        );
        let mapped = IpAddr::V6(Ipv4Addr::new(203, 0, 113, 5).to_ipv6_mapped());
        assert!(accounts.address_banned(mapped));

        let reopened = AccountDirectory::open(root.path(), true).unwrap();
        assert_eq!(reopened.address_bans()[0].reason.as_deref(), Some("flood"));
        assert_eq!(reopened.set_address_banned(PEER, false, None), Ok(true));
        assert_eq!(reopened.set_address_banned(PEER, false, None), Ok(false));
        assert!(login(&reopened, "Rider", "secret").is_ok());
    }

    #[test]
    fn operators_see_and_clear_throttles() {
        let accounts = AccountDirectory::in_memory();
        accounts
            .register("Rider", password_key("Rider", "secret"), PEER)
            .unwrap();
        let bad = password_key("Rider", "bad");
        for _ in 0..MAX_FAILED_LOGINS {
            let _ = login_with(&accounts, "Rider", &bad);
        }
        let throttles = accounts.throttles();
        assert_eq!(throttles.len(), 1);
        assert_eq!(throttles[0].failed_logins, MAX_FAILED_LOGINS);
        assert_eq!(throttles[0].registrations, 1);
        assert!(throttles[0].limited);
        assert_eq!(accounts.clear_throttles(Some(PEER)), 1);
        assert!(accounts.throttles().is_empty());
        assert!(login(&accounts, "Rider", "secret").is_ok());
    }

    #[test]
    fn operator_password_reset_matches_the_launcher_key() {
        let accounts = AccountDirectory::in_memory();
        accounts
            .register("Rider", password_key("Rider", "secret"), PEER)
            .unwrap();
        let (_, ticket) = login(&accounts, "Rider", "secret").unwrap();
        assert_eq!(
            accounts.reset_password("RIDER", "new-secret"),
            Ok("Rider".to_owned())
        );
        assert!(accounts.redeem("Rider", &ticket).is_none());
        assert!(login(&accounts, "Rider", "secret").is_err());
        assert!(login(&accounts, "Rider", "new-secret").is_ok());
        assert_eq!(
            accounts.reset_password("Nobody", "new-secret"),
            Err(AccountAdminError::NotFound)
        );
    }

    #[test]
    fn failed_logins_are_throttled_per_address() {
        let accounts = AccountDirectory::in_memory();
        accounts
            .register("Rider", password_key("Rider", "secret"), PEER)
            .unwrap();
        let bad = password_key("Rider", "bad");
        for _ in 0..MAX_FAILED_LOGINS {
            assert_eq!(
                login_with(&accounts, "Rider", &bad),
                Err(AccountFailure::BadCredentials)
            );
        }
        assert_eq!(
            login(&accounts, "Rider", "secret"),
            Err(AccountFailure::RateLimited)
        );
    }

    #[test]
    fn password_change_requires_the_old_password_and_revokes_tickets() {
        let accounts = AccountDirectory::in_memory();
        accounts
            .register("Rider", password_key("Rider", "old1"), PEER)
            .unwrap();
        let (_, ticket) = login(&accounts, "Rider", "old1").unwrap();
        let nonce = [1_u8; 32];
        let proof = login_proof(&password_key("Rider", "old1"), &nonce);
        accounts
            .change_password("Rider", &nonce, &proof, password_key("Rider", "new1"), PEER)
            .unwrap();
        assert!(accounts.redeem("Rider", &ticket).is_none());
        assert_eq!(
            login(&accounts, "Rider", "old1"),
            Err(AccountFailure::BadCredentials)
        );
        assert!(login(&accounts, "Rider", "new1").is_ok());
    }

    #[test]
    fn accounts_survive_a_restart() {
        let root = tempdir().unwrap();
        let accounts = AccountDirectory::open(root.path(), true).unwrap();
        accounts
            .register("라이더", password_key("라이더", "비밀"), PEER)
            .unwrap();
        let reopened = AccountDirectory::open(root.path(), true).unwrap();
        assert!(login(&reopened, "라이더", "비밀").is_ok());
        let closed = AccountDirectory::open(root.path(), false).unwrap();
        assert_eq!(
            closed.register("New", password_key("New", "pass"), PEER),
            Err(AccountFailure::RegistrationClosed)
        );
    }

    async fn exchange(stream: &mut TcpStream, request: &AccountRequest) -> AccountResponse {
        let mut line = request.to_line();
        line.push('\n');
        stream.write_all(line.as_bytes()).await.unwrap();
        let mut reply = Vec::new();
        loop {
            let byte = stream.read_u8().await.unwrap();
            if byte == b'\n' {
                break;
            }
            reply.push(byte);
        }
        AccountResponse::parse(std::str::from_utf8(&reply).unwrap()).unwrap()
    }

    #[tokio::test]
    async fn launcher_registers_and_logs_in_over_tcp() {
        let accounts = Arc::new(AccountDirectory::in_memory());
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let address = listener.local_addr().unwrap();
        let (_stop, shutdown) = watch::channel(false);
        tokio::spawn(accept_loop(listener, Arc::clone(&accounts), None, shutdown));

        let key = password_key("Rider", "secret");
        let mut stream = TcpStream::connect(address).await.unwrap();
        assert_eq!(
            exchange(
                &mut stream,
                &AccountRequest::Register {
                    nickname: "Rider".to_owned(),
                    key
                }
            )
            .await,
            AccountResponse::Registered {
                nickname: "Rider".to_owned()
            }
        );
        assert_eq!(
            exchange(
                &mut stream,
                &AccountRequest::Login {
                    nickname: "Rider".to_owned(),
                    proof: [0; 32]
                }
            )
            .await,
            AccountResponse::Failed(AccountFailure::BadRequest)
        );
        let AccountResponse::Nonce(nonce) = exchange(
            &mut stream,
            &AccountRequest::Challenge {
                nickname: "Rider".to_owned(),
            },
        )
        .await
        else {
            panic!("expected a nonce");
        };
        let AccountResponse::Ticket { nickname, ticket } = exchange(
            &mut stream,
            &AccountRequest::Login {
                nickname: "rider".to_owned(),
                proof: login_proof(&key, &nonce),
            },
        )
        .await
        else {
            panic!("expected a ticket");
        };
        assert_eq!(nickname, "Rider");
        assert!(accounts.redeem("Rider", &ticket).is_some());

        assert_eq!(
            exchange(
                &mut stream,
                &AccountRequest::Presence {
                    nickname: "Rider".to_owned(),
                    ticket: "00".repeat(32),
                }
            )
            .await,
            AccountResponse::Failed(AccountFailure::BadCredentials)
        );
        assert_eq!(
            exchange(
                &mut stream,
                &AccountRequest::Presence {
                    nickname: "Rider".to_owned(),
                    ticket,
                }
            )
            .await,
            AccountResponse::Presence(PlayerPresence::Offline)
        );
    }
}
