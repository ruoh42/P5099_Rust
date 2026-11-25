use std::{
    collections::{HashMap, HashSet, VecDeque},
    fmt, io,
    net::{IpAddr, SocketAddr},
    str::FromStr,
    sync::{Arc, Mutex, PoisonError},
    time::{Duration, Instant},
};

use p5099_core::{
    account_protocol::constant_time_eq,
    nickname::canonical_nickname_key,
    rcon::{
        AUTH_FAILED_ID, Packet, SERVERDATA_AUTH, SERVERDATA_AUTH_RESPONSE, SERVERDATA_EXECCOMMAND,
        SERVERDATA_RESPONSE_VALUE, Word, parse_duration, payload_len, response_packets,
        split_command,
    },
};
use serde::Serialize;
use serde_json::{Value, json};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::{Notify, Semaphore, watch},
    time::timeout,
};

use crate::account_directory::unix_now;
use crate::{
    RewardDeadLetter, UserNo, WorldError, WorldHandle,
    account_directory::{AccountAdminError, AccountDirectory},
    world::{AdminKickReport, AdminKickTarget, AdminPlayer, AdminWorldSnapshot, RoomPhase},
};

pub const MIN_ADMIN_TOKEN_CHARS: usize = 16;
const MAX_CONNECTIONS: usize = 16;
const AUTH_TIMEOUT: Duration = Duration::from_secs(15);
const IDLE_TIMEOUT: Duration = Duration::from_mins(5);
const WRITE_TIMEOUT: Duration = Duration::from_secs(15);
const FAILED_AUTH_WINDOW: Duration = Duration::from_mins(10);
const MAX_FAILED_AUTH: usize = 10;
const AUDIT_CAPACITY: usize = 256;

#[derive(Clone)]
pub struct AdminConfig {
    bind: SocketAddr,
    token: Arc<str>,
}

impl AdminConfig {
    #[must_use]
    pub fn new(bind: SocketAddr, token: &str) -> Option<Self> {
        let token = token.trim();
        (token.chars().count() >= MIN_ADMIN_TOKEN_CHARS).then(|| Self {
            bind,
            token: token.into(),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Caller {
    Rcon(IpAddr),
    Named(String),
}

impl fmt::Display for Caller {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Rcon(address) => address.fmt(formatter),
            Self::Named(label) => formatter.write_str(label),
        }
    }
}

impl Serialize for Caller {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

#[derive(Clone)]
pub struct OperatorConsole {
    context: Arc<AdminContext>,
}

impl OperatorConsole {
    pub(crate) fn new(
        accounts: Arc<AccountDirectory>,
        world: WorldHandle,
        require_account: bool,
    ) -> Self {
        Self {
            context: Arc::new(AdminContext {
                accounts,
                world,
                require_account,
                started: Instant::now(),
                failed_auth: Mutex::new(HashMap::new()),
                audit: Mutex::new(VecDeque::new()),
                shutdown_request: Notify::new(),
            }),
        }
    }

    pub async fn execute(&self, caller: &Caller, line: &str) -> Value {
        handle(&self.context, caller, line).await.body
    }

    pub async fn shutdown_requested(&self) {
        self.context.shutdown_request.notified().await;
    }
}

impl fmt::Debug for OperatorConsole {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("OperatorConsole")
            .finish_non_exhaustive()
    }
}

impl fmt::Debug for AdminConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AdminConfig")
            .field("bind", &self.bind)
            .field("token", &"<redacted>")
            .finish_non_exhaustive()
    }
}

struct AdminContext {
    accounts: Arc<AccountDirectory>,
    world: WorldHandle,
    require_account: bool,
    started: Instant,
    failed_auth: Mutex<HashMap<IpAddr, VecDeque<Instant>>>,
    audit: Mutex<VecDeque<AuditEntry>>,
    shutdown_request: Notify,
}

pub(crate) fn spawn_admin_service(
    config: AdminConfig,
    console: OperatorConsole,
    shutdown: watch::Receiver<bool>,
) {
    tokio::spawn(async move {
        let address = config.bind;
        let listener = match TcpListener::bind(address).await {
            Ok(listener) => listener,
            Err(error) => {
                tracing::error!(%address, %error, "operator console could not bind");
                return;
            }
        };
        if !address.ip().is_loopback() {
            tracing::warn!(
                %address,
                "operator console accepts remote connections over unencrypted RCON; \
                 prefer loopback with an SSH tunnel"
            );
        }
        tracing::info!(%address, "operator console (RCON) listening");
        accept_loop(listener, console.context, config.token, shutdown).await;
    });
}

async fn accept_loop(
    listener: TcpListener,
    context: Arc<AdminContext>,
    token: Arc<str>,
    mut shutdown: watch::Receiver<bool>,
) {
    let permits = Arc::new(Semaphore::new(MAX_CONNECTIONS));
    loop {
        tokio::select! {
            changed = shutdown.changed() => {
                if changed.is_err() || *shutdown.borrow() {
                    return;
                }
            }
            accepted = listener.accept() => match accepted {
                Ok((stream, peer)) => {
                    let Ok(permit) = Arc::clone(&permits).try_acquire_owned() else {
                        continue;
                    };
                    let context = Arc::clone(&context);
                    let token = Arc::clone(&token);
                    tokio::spawn(async move {
                        let _permit = permit;
                        if let Err(error) = serve(stream, peer, &context, &token).await {
                            tracing::debug!(%peer, %error, "operator console connection ended");
                        }
                    });
                }
                Err(error) => {
                    tracing::warn!(%error, "operator console accept failed");
                    tokio::time::sleep(Duration::from_millis(250)).await;
                }
            },
        }
    }
}

struct Reply {
    body: Value,
}

impl Reply {
    fn ok(body: Value) -> Self {
        Self { body }
    }

    fn error(message: &str) -> Self {
        Self {
            body: json!({ "error": message }),
        }
    }
}

async fn read_packet<S: AsyncRead + Unpin>(stream: &mut S) -> io::Result<Option<Packet>> {
    let mut size = [0_u8; 4];
    match stream.read_exact(&mut size).await {
        Ok(_) => {}
        Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(error) => return Err(error),
    }
    let length =
        payload_len(size).map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    let mut payload = vec![0_u8; length];
    stream.read_exact(&mut payload).await?;
    Packet::decode(&payload)
        .map(Some)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
}

async fn write_packets<S: AsyncWrite + Unpin>(
    stream: &mut S,
    packets: &[Packet],
) -> io::Result<()> {
    let bytes: Vec<u8> = packets.iter().flat_map(Packet::encode).collect();
    timeout(WRITE_TIMEOUT, stream.write_all(&bytes))
        .await
        .map_err(|_| io::Error::from(io::ErrorKind::TimedOut))?
}

async fn authenticate<S: AsyncWrite + Unpin>(
    stream: &mut S,
    peer: IpAddr,
    context: &AdminContext,
    token: &str,
    packet: &Packet,
) -> io::Result<bool> {
    let refusal = if context.throttled(peer) {
        Some("too many failed attempts")
    } else if context.authorized(peer, &packet.body, token) {
        None
    } else {
        tracing::warn!(target: "p5099_admin", %peer, "rejected operator token");
        Some("wrong console token")
    };
    let id = if refusal.is_some() {
        AUTH_FAILED_ID
    } else {
        packet.id
    };
    write_packets(
        stream,
        &[
            Packet::new(
                packet.id,
                SERVERDATA_RESPONSE_VALUE,
                refusal.unwrap_or_default(),
            ),
            Packet::new(id, SERVERDATA_AUTH_RESPONSE, Vec::new()),
        ],
    )
    .await?;
    Ok(refusal.is_none())
}

async fn serve(
    mut stream: TcpStream,
    peer: SocketAddr,
    context: &AdminContext,
    token: &str,
) -> io::Result<()> {
    let peer = peer.ip();
    let first = match timeout(AUTH_TIMEOUT, read_packet(&mut stream)).await {
        Ok(Ok(Some(packet))) if packet.kind == SERVERDATA_AUTH => packet,
        Ok(Err(error)) => return Err(error),
        _ => return Ok(()),
    };
    if !authenticate(&mut stream, peer, context, token, &first).await? {
        return Ok(());
    }
    loop {
        let packet = match timeout(IDLE_TIMEOUT, read_packet(&mut stream)).await {
            Ok(Ok(Some(packet))) => packet,
            Ok(Err(error)) => return Err(error),
            Ok(Ok(None)) | Err(_) => return Ok(()),
        };
        match packet.kind {
            SERVERDATA_EXECCOMMAND => {
                let reply = match std::str::from_utf8(&packet.body) {
                    Ok(line) => handle(context, &Caller::Rcon(peer), line).await,
                    Err(_) => Reply::error("the command is not UTF-8"),
                };
                let text = serde_json::to_string_pretty(&reply.body).unwrap_or_default();
                write_packets(&mut stream, &response_packets(packet.id, &text)).await?;
            }
            SERVERDATA_RESPONSE_VALUE => {
                write_packets(
                    &mut stream,
                    &[Packet::new(
                        packet.id,
                        SERVERDATA_RESPONSE_VALUE,
                        Vec::new(),
                    )],
                )
                .await?;
            }
            SERVERDATA_AUTH
                if !authenticate(&mut stream, peer, context, token, &packet).await? =>
            {
                return Ok(());
            }
            _ => {}
        }
    }
}

impl AdminContext {
    fn throttled(&self, peer: IpAddr) -> bool {
        let mut failures = self
            .failed_auth
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let events = failures.entry(peer).or_default();
        let now = Instant::now();
        while events
            .front()
            .is_some_and(|event| now.duration_since(*event) > FAILED_AUTH_WINDOW)
        {
            events.pop_front();
        }
        events.len() >= MAX_FAILED_AUTH
    }

    fn authorized(&self, peer: IpAddr, password: &[u8], token: &str) -> bool {
        let presented = std::str::from_utf8(password)
            .map(str::trim)
            .unwrap_or_default();
        let accepted = constant_time_eq(presented.as_bytes(), token.as_bytes());
        if !accepted {
            self.failed_auth
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .entry(peer)
                .or_default()
                .push_back(Instant::now());
        }
        accepted
    }

    fn record(&self, peer: &Caller, action: &'static str, detail: Value) {
        tracing::info!(target: "p5099_admin", %peer, action, %detail, "operator action");
        let mut audit = self.audit.lock().unwrap_or_else(PoisonError::into_inner);
        if audit.len() == AUDIT_CAPACITY {
            audit.pop_front();
        }
        audit.push_back(AuditEntry {
            time: unix_now(),
            peer: peer.clone(),
            action,
            detail,
        });
    }

    fn console_throttles(&self) -> Vec<Value> {
        let now = Instant::now();
        let mut failures = self
            .failed_auth
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        failures.retain(|_, events| {
            events
                .iter()
                .any(|event| now.duration_since(*event) <= FAILED_AUTH_WINDOW)
        });
        let mut addresses: Vec<_> = failures
            .iter()
            .map(|(address, events)| {
                let failures = events
                    .iter()
                    .filter(|event| now.duration_since(**event) <= FAILED_AUTH_WINDOW)
                    .count();
                (*address, failures)
            })
            .collect();
        addresses.sort_unstable();
        addresses
            .into_iter()
            .map(|(address, failures)| {
                json!({
                    "address": address,
                    "failed_tokens": failures,
                    "limited": failures >= MAX_FAILED_AUTH,
                })
            })
            .collect()
    }

    fn clear_console_throttles(&self, address: Option<IpAddr>) -> usize {
        let mut failures = self
            .failed_auth
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if let Some(address) = address {
            usize::from(failures.remove(&address).is_some())
        } else {
            let cleared = failures.len();
            failures.clear();
            cleared
        }
    }
}

#[derive(Debug, Clone, Serialize)]
struct AuditEntry {
    time: u64,
    peer: Caller,
    action: &'static str,
    detail: Value,
}

const COMMANDS: &[&str] = &[
    "help",
    "health",
    "status",
    "players",
    "rooms",
    "accounts",
    "info <nickname>",
    "create <nickname> <password> [--admin]",
    "delete <nickname>",
    "admin <nickname>",
    "unadmin <nickname>",
    "ban <nickname> [--reason <text>] [--duration <30m|12h|7d>]",
    "unban <nickname>",
    "password <nickname> <password>",
    "bans",
    "ban-ip <address> [--reason <text>]",
    "unban-ip <address>",
    "kick <nickname>",
    "kick-ip <address>",
    "kick-room <id>",
    "kick-all [--keep-admins]",
    "registration <open|close>",
    "maintenance [on|off] [--kick]",
    "rewards",
    "retry-reward [<room> <race_epoch>]",
    "throttles",
    "clear-throttle [<address>]",
    "audit [--limit <n>]",
    "shutdown",
];

const VALUE_FLAGS: &[&str] = &["--reason", "--duration", "--limit"];

struct Arguments {
    positional: VecDeque<String>,
    switches: Vec<String>,
    values: Vec<(String, String)>,
}

impl Arguments {
    fn parse(mut words: impl Iterator<Item = Word>) -> Result<Self, Reply> {
        let mut arguments = Self {
            positional: VecDeque::new(),
            switches: Vec::new(),
            values: Vec::new(),
        };
        while let Some(word) = words.next() {
            if word.quoted || !word.text.starts_with("--") {
                arguments.positional.push_back(word.text);
            } else if VALUE_FLAGS.contains(&word.text.as_str()) {
                let Some(value) = words.next() else {
                    return Err(Reply::error(&format!("{} needs a value", word.text)));
                };
                arguments.values.push((word.text, value.text));
            } else {
                arguments.switches.push(word.text);
            }
        }
        Ok(arguments)
    }

    fn required(&mut self, what: &str) -> Result<String, Reply> {
        self.positional
            .pop_front()
            .ok_or_else(|| Reply::error(&format!("missing <{what}>")))
    }

    fn optional(&mut self) -> Option<String> {
        self.positional.pop_front()
    }

    fn switch(&mut self, name: &str) -> bool {
        let before = self.switches.len();
        self.switches.retain(|switch| switch != name);
        self.switches.len() != before
    }

    fn value(&mut self, name: &str) -> Option<String> {
        let index = self.values.iter().position(|(flag, _)| flag == name)?;
        Some(self.values.remove(index).1)
    }

    fn finish(self) -> Result<(), Reply> {
        let unused = self
            .positional
            .into_iter()
            .chain(self.switches)
            .chain(self.values.into_iter().map(|(flag, _)| flag))
            .next();
        match unused {
            Some(word) => Err(Reply::error(&format!("unexpected argument `{word}`"))),
            None => Ok(()),
        }
    }
}

fn parsed<T: FromStr>(text: &str, what: &str) -> Result<T, Reply> {
    text.parse()
        .map_err(|_| Reply::error(&format!("invalid {what} `{text}`")))
}

async fn handle(context: &AdminContext, peer: &Caller, line: &str) -> Reply {
    match Command::parse(line) {
        Ok(command) => execute(context, peer, command).await,
        Err(reply) => reply,
    }
}

enum Command {
    Help,
    Health,
    Status,
    Players,
    Rooms,
    Accounts,
    Info(String),
    Create(CreateAccount),
    Delete(String),
    SetAdmin { nickname: String, admin: bool },
    Ban(SetBan),
    ResetPassword(ResetPassword),
    AddressBans,
    BanAddress(SetAddressBan),
    Kick(String),
    KickAddress(IpAddr),
    KickRoom(u32),
    KickAll { keep_admins: bool },
    Registration(bool),
    Maintenance(Option<SetMaintenance>),
    Rewards,
    RetryReward(RetryReward),
    Throttles,
    ClearThrottles(Option<IpAddr>),
    Audit(usize),
    Shutdown,
}

impl Command {
    fn parse(line: &str) -> Result<Self, Reply> {
        let mut words = split_command(line)
            .map_err(|error| Reply::error(&error.to_string()))?
            .into_iter();
        let name = words
            .next()
            .ok_or_else(|| Reply::error("empty command; try `help`"))?
            .text
            .to_ascii_lowercase();
        let mut args = Arguments::parse(words)?;
        let command = Self::from_words(&name, &mut args)?;
        args.finish()?;
        Ok(command)
    }

    fn from_words(name: &str, args: &mut Arguments) -> Result<Self, Reply> {
        Ok(match name {
            "help" => Self::Help,
            "health" => Self::Health,
            "status" => Self::Status,
            "players" => Self::Players,
            "rooms" => Self::Rooms,
            "accounts" => Self::Accounts,
            "info" => Self::Info(args.required("nickname")?),
            "create" => Self::Create(CreateAccount {
                nickname: args.required("nickname")?,
                password: args.required("password")?,
                admin: args.switch("--admin"),
            }),
            "delete" => Self::Delete(args.required("nickname")?),
            "admin" | "unadmin" => Self::SetAdmin {
                nickname: args.required("nickname")?,
                admin: name == "admin",
            },
            "ban" => Self::Ban(SetBan {
                nickname: args.required("nickname")?,
                banned: true,
                reason: args.value("--reason"),
                duration_seconds: args
                    .value("--duration")
                    .map(|text| parse_duration(&text).map_err(|error| Reply::error(&error)))
                    .transpose()?,
            }),
            "unban" => Self::Ban(SetBan {
                nickname: args.required("nickname")?,
                banned: false,
                reason: None,
                duration_seconds: None,
            }),
            "password" => Self::ResetPassword(ResetPassword {
                nickname: args.required("nickname")?,
                password: args.required("password")?,
            }),
            "bans" => Self::AddressBans,
            "ban-ip" | "unban-ip" => Self::BanAddress(SetAddressBan {
                address: parsed(&args.required("address")?, "address")?,
                banned: name == "ban-ip",
                reason: args.value("--reason"),
            }),
            "kick" => Self::Kick(args.required("nickname")?),
            "kick-ip" => Self::KickAddress(parsed(&args.required("address")?, "address")?),
            "kick-room" => Self::KickRoom(parsed(&args.required("id")?, "room id")?),
            "kick-all" => Self::KickAll {
                keep_admins: args.switch("--keep-admins"),
            },
            "registration" => Self::Registration(match args.required("open|close")?.as_str() {
                "open" => true,
                "close" => false,
                other => {
                    return Err(Reply::error(&format!(
                        "expected open or close, not `{other}`"
                    )));
                }
            }),
            "maintenance" => Self::Maintenance(match args.optional().as_deref() {
                None => None,
                Some(state @ ("on" | "off")) => Some(SetMaintenance {
                    enabled: state == "on",
                    kick: args.switch("--kick"),
                }),
                Some(other) => {
                    return Err(Reply::error(&format!("expected on or off, not `{other}`")));
                }
            }),
            "rewards" => Self::Rewards,
            "retry-reward" => Self::RetryReward(RetryReward {
                room: args
                    .optional()
                    .map(|room| parsed(&room, "room"))
                    .transpose()?,
                race_epoch: args
                    .optional()
                    .map(|epoch| parsed(&epoch, "race epoch"))
                    .transpose()?,
            }),
            "throttles" => Self::Throttles,
            "clear-throttle" => Self::ClearThrottles(
                args.optional()
                    .map(|address| parsed(&address, "address"))
                    .transpose()?,
            ),
            "audit" => Self::Audit(
                args.value("--limit")
                    .map(|limit| parsed(&limit, "limit"))
                    .transpose()?
                    .unwrap_or(50),
            ),
            "shutdown" => Self::Shutdown,
            other => {
                return Err(Reply::error(&format!(
                    "unknown command `{other}`; try `help`"
                )));
            }
        })
    }
}

async fn execute(context: &AdminContext, peer: &Caller, command: Command) -> Reply {
    match command {
        Command::Help => Reply::ok(json!({ "commands": COMMANDS })),
        Command::Health => {
            Reply::ok(json!({ "status": "ok", "version": env!("CARGO_PKG_VERSION") }))
        }
        Command::Status => status(context).await,
        Command::Players => {
            with_snapshot(context, |snapshot| players_json(context, &snapshot)).await
        }
        Command::Rooms => with_snapshot(context, |snapshot| rooms_json(&snapshot)).await,
        Command::Accounts => Reply::ok(json!({ "accounts": context.accounts.summaries() })),
        Command::Info(nickname) => account_info(context, &nickname).await,
        Command::Create(input) => create_account(context, peer, input).await,
        Command::Delete(nickname) => delete_account(context, peer, &nickname).await,
        Command::SetAdmin { nickname, admin } => set_admin(context, peer, &nickname, admin),
        Command::Ban(input) => ban(context, peer, input).await,
        Command::ResetPassword(input) => reset_password(context, peer, input).await,
        Command::AddressBans => address_bans(context),
        Command::BanAddress(input) => ban_address(context, peer, input).await,
        Command::Kick(nickname) => kick(context, peer, &nickname).await,
        Command::KickAddress(address) => {
            kick_many(
                context,
                peer,
                "kick address",
                AdminKickTarget::Address(address),
            )
            .await
        }
        Command::KickRoom(id) => {
            kick_many(context, peer, "kick room", AdminKickTarget::Room(id)).await
        }
        Command::KickAll { keep_admins } => {
            let except = if keep_admins {
                admin_keys(context)
            } else {
                HashSet::new()
            };
            kick_many(
                context,
                peer,
                "kick all",
                AdminKickTarget::Everyone { except },
            )
            .await
        }
        Command::Registration(open) => {
            context.accounts.set_registration_open(open);
            context.record(peer, "registration", json!({ "open": open }));
            Reply::ok(json!({ "registration_open": open }))
        }
        Command::Maintenance(None) => {
            Reply::ok(json!({ "maintenance": context.accounts.maintenance() }))
        }
        Command::Maintenance(Some(input)) => maintenance(context, peer, input).await,
        Command::Rewards => rewards(context).await,
        Command::RetryReward(input) => retry_rewards(context, peer, input).await,
        Command::Throttles => Reply::ok(json!({
            "launcher": context.accounts.throttles(),
            "console": context.console_throttles(),
        })),
        Command::ClearThrottles(address) => {
            let launcher = context.accounts.clear_throttles(address);
            let console = context.clear_console_throttles(address);
            context.record(
                peer,
                "clear throttles",
                json!({ "address": address, "launcher": launcher, "console": console }),
            );
            Reply::ok(json!({ "launcher": launcher, "console": console }))
        }
        Command::Audit(limit) => audit(context, limit),
        Command::Shutdown => {
            context.record(peer, "shutdown", Value::Null);
            context.shutdown_request.notify_one();
            Reply::ok(json!({ "shutdown": "requested" }))
        }
    }
}

struct CreateAccount {
    nickname: String,
    password: String,
    admin: bool,
}

struct SetBan {
    nickname: String,
    banned: bool,
    reason: Option<String>,
    duration_seconds: Option<u64>,
}

struct SetAddressBan {
    address: IpAddr,
    banned: bool,
    reason: Option<String>,
}

struct ResetPassword {
    nickname: String,
    password: String,
}

struct SetMaintenance {
    enabled: bool,
    kick: bool,
}

struct RetryReward {
    room: Option<u32>,
    race_epoch: Option<u64>,
}

fn set_admin(context: &AdminContext, peer: &Caller, nickname: &str, admin: bool) -> Reply {
    match context.accounts.set_admin(nickname, admin) {
        Ok(nickname) => {
            context.record(
                peer,
                "set admin",
                json!({ "nickname": nickname, "admin": admin }),
            );
            Reply::ok(json!({ "nickname": nickname, "admin": admin }))
        }
        Err(error) => account_error(&error),
    }
}

fn audit(context: &AdminContext, limit: usize) -> Reply {
    let limit = limit.min(AUDIT_CAPACITY);
    let audit = context.audit.lock().unwrap_or_else(PoisonError::into_inner);
    let entries: Vec<_> = audit
        .iter()
        .skip(audit.len().saturating_sub(limit))
        .collect();
    Reply::ok(json!({ "entries": entries }))
}

fn snake_name(value: &impl fmt::Debug) -> String {
    let mut name = String::new();
    for (index, character) in format!("{value:?}").chars().enumerate() {
        if character.is_ascii_uppercase() {
            if index > 0 {
                name.push('_');
            }
            name.push(character.to_ascii_lowercase());
        } else {
            name.push(character);
        }
    }
    name
}

fn world_unavailable() -> Reply {
    Reply::error("the game world is stopping")
}

async fn with_snapshot(
    context: &AdminContext,
    render: impl FnOnce(AdminWorldSnapshot) -> Value,
) -> Reply {
    match context.world.admin_snapshot().await {
        Ok(snapshot) => Reply::ok(render(snapshot)),
        Err(_) => world_unavailable(),
    }
}

async fn status(context: &AdminContext) -> Reply {
    let Ok(snapshot) = context.world.admin_snapshot().await else {
        return world_unavailable();
    };
    let rewards = context.world.reward_drain_status().await.ok();
    let accounts = context.accounts.summaries();
    Reply::ok(json!({
        "version": env!("CARGO_PKG_VERSION"),
        "server_time": unix_now(),
        "uptime_seconds": context.started.elapsed().as_secs(),
        "sessions": snapshot.sessions,
        "players_online": snapshot.players.len(),
        "observers_online": snapshot.rooms.iter().map(|room| room.observers.len()).sum::<usize>(),
        "rooms": snapshot.rooms.len(),
        "racing_rooms": snapshot.rooms.iter().filter(|room| room.phase != RoomPhase::Lobby).count(),
        "accounts": accounts.len(),
        "admin_accounts": accounts.iter().filter(|account| account.admin).count(),
        "banned_accounts": accounts.iter().filter(|account| account.banned).count(),
        "banned_addresses": context.accounts.address_bans().len(),
        "require_account": context.require_account,
        "registration_open": context.accounts.registration_open(),
        "maintenance": context.accounts.maintenance(),
        "reward_recovery": rewards.map(|status| json!({
            "outstanding_lanes": status.outstanding_lanes().len(),
            "dead_letters": status.dead_letters().len(),
        })),
    }))
}

fn player_json(context: &AdminContext, player: &AdminPlayer) -> Value {
    json!({
        "nickname": player.nickname,
        "user_no": player.user_no,
        "address": player.address.to_string(),
        "channel": player.channel,
        "room": player.room,
        "connected_seconds": player.connected.as_secs(),
        "admin": context.accounts.is_admin(&player.nickname),
    })
}

fn players_json(context: &AdminContext, snapshot: &AdminWorldSnapshot) -> Value {
    let players: Vec<_> = snapshot
        .players
        .iter()
        .map(|player| player_json(context, player))
        .collect();
    json!({ "players": players })
}

const fn phase_name(phase: RoomPhase) -> &'static str {
    match phase {
        RoomPhase::Lobby => "lobby",
        RoomPhase::Loading => "loading",
        RoomPhase::Running => "running",
        RoomPhase::Settling => "settling",
    }
}

fn rooms_json(snapshot: &AdminWorldSnapshot) -> Value {
    let rooms: Vec<_> = snapshot
        .rooms
        .iter()
        .map(|room| {
            json!({
                "id": room.id,
                "name": room.name,
                "channel": room.channel,
                "game_type": room.game_type,
                "speed_type": room.speed_type,
                "track": room.track,
                "phase": phase_name(room.phase),
                "locked": room.locked,
                "players": room.players,
                "observers": room.observers,
                "ais": room.ais,
                "capacity": room.capacity,
            })
        })
        .collect();
    json!({ "rooms": rooms })
}

async fn account_info(context: &AdminContext, nickname: &str) -> Reply {
    let Some(detail) = context.accounts.detail(nickname) else {
        return Reply::error("no such account");
    };
    let Ok(snapshot) = context.world.admin_snapshot().await else {
        return world_unavailable();
    };
    let key = canonical_nickname_key(&detail.summary.nickname);
    let online = snapshot
        .players
        .iter()
        .find(|player| canonical_nickname_key(&player.nickname) == key)
        .map(|player| player_json(context, player));
    let mut reply = serde_json::to_value(&detail).unwrap_or_default();
    reply["online"] = online.unwrap_or(Value::Null);
    Reply::ok(reply)
}

async fn kick(context: &AdminContext, peer: &Caller, nickname: &str) -> Reply {
    match context.world.admin_kick(nickname).await {
        Ok(true) => {
            context.record(peer, "kick", json!({ "nickname": nickname }));
            Reply::ok(json!({ "kicked": nickname }))
        }
        Ok(false) => Reply::error("no rider by that nickname is online"),
        Err(_) => world_unavailable(),
    }
}

fn kick_report_json(report: &AdminKickReport) -> Value {
    json!({ "sessions": report.sessions, "nicknames": report.nicknames })
}

async fn kick_many(
    context: &AdminContext,
    peer: &Caller,
    action: &'static str,
    target: AdminKickTarget,
) -> Reply {
    let described = match &target {
        AdminKickTarget::Room(id) => json!({ "room": id }),
        AdminKickTarget::Address(address) => json!({ "address": address }),
        AdminKickTarget::Everyone { except } => json!({ "kept": except.len() }),
    };
    match context.world.admin_kick_many(target).await {
        Ok(Some(report)) => {
            let kicked = kick_report_json(&report);
            let mut detail = described;
            detail["kicked"] = kicked.clone();
            context.record(peer, action, detail);
            Reply::ok(json!({ "kicked": kicked }))
        }
        Ok(None) => Reply::error("no such room"),
        Err(_) => world_unavailable(),
    }
}

fn admin_keys(context: &AdminContext) -> HashSet<String> {
    context
        .accounts
        .summaries()
        .into_iter()
        .filter(|account| account.admin)
        .map(|account| canonical_nickname_key(&account.nickname))
        .collect()
}

fn account_error(error: &AccountAdminError) -> Reply {
    match error {
        AccountAdminError::NotFound => Reply::error("no such account"),
        AccountAdminError::Exists => Reply::error("that nickname is taken"),
        AccountAdminError::InvalidNickname => {
            Reply::error("the nickname breaks the client's rules")
        }
        AccountAdminError::InvalidPassword => Reply::error("password length is out of range"),
        AccountAdminError::Unavailable => Reply::error("could not save the account directory"),
    }
}

async fn create_account(context: &AdminContext, peer: &Caller, input: CreateAccount) -> Reply {
    let accounts = Arc::clone(&context.accounts);
    let admin = input.admin;
    let result = tokio::task::spawn_blocking(move || {
        accounts.create(&input.nickname, &input.password, input.admin)
    })
    .await
    .unwrap_or(Err(AccountAdminError::Unavailable));
    match result {
        Ok(nickname) => {
            context.record(
                peer,
                "create account",
                json!({ "nickname": nickname, "admin": admin }),
            );
            Reply::ok(json!({ "nickname": nickname, "admin": admin }))
        }
        Err(error) => account_error(&error),
    }
}

async fn delete_account(context: &AdminContext, peer: &Caller, nickname: &str) -> Reply {
    let nickname = match context.accounts.delete(nickname) {
        Ok(nickname) => nickname,
        Err(error) => return account_error(&error),
    };
    let kicked = context.world.admin_kick(&nickname).await.unwrap_or(false);
    context.record(
        peer,
        "delete account",
        json!({ "nickname": nickname, "kicked": kicked }),
    );
    Reply::ok(json!({ "nickname": nickname, "deleted": true, "kicked": kicked }))
}

async fn ban(context: &AdminContext, peer: &Caller, input: SetBan) -> Reply {
    let until = input
        .duration_seconds
        .filter(|_| input.banned)
        .map(|seconds| unix_now().saturating_add(seconds));
    let nickname = match context.accounts.set_banned(
        &input.nickname,
        input.banned,
        input.reason.clone(),
        until,
    ) {
        Ok(nickname) => nickname,
        Err(error) => return account_error(&error),
    };
    let kicked = input.banned && context.world.admin_kick(&nickname).await.unwrap_or(false);
    context.record(
        peer,
        "ban",
        json!({
            "nickname": nickname,
            "banned": input.banned,
            "reason": input.reason,
            "banned_until": until,
            "kicked": kicked,
        }),
    );
    Reply::ok(json!({
        "nickname": nickname,
        "banned": input.banned,
        "banned_until": until,
        "kicked": kicked,
    }))
}

async fn reset_password(context: &AdminContext, peer: &Caller, input: ResetPassword) -> Reply {
    let accounts = Arc::clone(&context.accounts);
    let result = tokio::task::spawn_blocking(move || {
        accounts.reset_password(&input.nickname, &input.password)
    })
    .await
    .unwrap_or(Err(AccountAdminError::Unavailable));
    match result {
        Ok(nickname) => {
            context.record(peer, "reset password", json!({ "nickname": nickname }));
            Reply::ok(json!({ "nickname": nickname }))
        }
        Err(error) => account_error(&error),
    }
}

fn address_bans(context: &AdminContext) -> Reply {
    let addresses: Vec<_> = context
        .accounts
        .address_bans()
        .into_iter()
        .map(|ban| json!({ "address": ban.address, "reason": ban.reason, "created": ban.created }))
        .collect();
    Reply::ok(json!({ "addresses": addresses }))
}

async fn ban_address(context: &AdminContext, peer: &Caller, input: SetAddressBan) -> Reply {
    let changed =
        match context
            .accounts
            .set_address_banned(input.address, input.banned, input.reason.clone())
        {
            Ok(changed) => changed,
            Err(error) => return account_error(&error),
        };
    let kicked = if input.banned {
        match context
            .world
            .admin_kick_many(AdminKickTarget::Address(input.address))
            .await
        {
            Ok(Some(report)) => kick_report_json(&report),
            _ => Value::Null,
        }
    } else {
        Value::Null
    };
    context.record(
        peer,
        "address ban",
        json!({
            "address": input.address,
            "banned": input.banned,
            "reason": input.reason,
            "changed": changed,
            "kicked": kicked,
        }),
    );
    Reply::ok(json!({
        "address": input.address,
        "banned": input.banned,
        "changed": changed,
        "kicked": kicked,
    }))
}

async fn maintenance(context: &AdminContext, peer: &Caller, input: SetMaintenance) -> Reply {
    context.accounts.set_maintenance(input.enabled);
    let kicked = if input.enabled && input.kick {
        let except = admin_keys(context);
        match context
            .world
            .admin_kick_many(AdminKickTarget::Everyone { except })
            .await
        {
            Ok(Some(report)) => kick_report_json(&report),
            _ => Value::Null,
        }
    } else {
        Value::Null
    };
    context.record(
        peer,
        "maintenance",
        json!({ "enabled": input.enabled, "kicked": kicked }),
    );
    Reply::ok(json!({ "maintenance": input.enabled, "kicked": kicked }))
}

fn dead_letter_json(dead_letter: &RewardDeadLetter) -> Value {
    let fence = dead_letter.fence();
    json!({
        "room": fence.room_id().0,
        "race_epoch": fence.race_epoch().get(),
        "nickname": dead_letter.failed_nickname(),
        "user_no": dead_letter.failed_user_no().map(UserNo::get),
        "reason": snake_name(&dead_letter.reason()),
        "retryable": dead_letter.reason().permits_persistence_retry(),
    })
}

async fn rewards(context: &AdminContext) -> Reply {
    let Ok(status) = context.world.reward_drain_status().await else {
        return world_unavailable();
    };
    let outstanding: Vec<_> = status
        .outstanding_lanes()
        .iter()
        .map(|lane| {
            json!({
                "room": lane.fence().room_id().0,
                "race_epoch": lane.fence().race_epoch().get(),
                "nickname": lane.nickname(),
                "user_no": lane.user_no().get(),
                "phase": snake_name(&lane.phase()),
            })
        })
        .collect();
    let dead_letters: Vec<_> = status.dead_letters().iter().map(dead_letter_json).collect();
    Reply::ok(json!({ "outstanding": outstanding, "dead_letters": dead_letters }))
}

async fn retry_rewards(context: &AdminContext, peer: &Caller, input: RetryReward) -> Reply {
    let Ok(status) = context.world.reward_drain_status().await else {
        return world_unavailable();
    };
    let chosen: Vec<RewardDeadLetter> = match (input.room, input.race_epoch) {
        (None, None) => status
            .dead_letters()
            .iter()
            .filter(|dead_letter| dead_letter.reason().permits_persistence_retry())
            .cloned()
            .collect(),
        (Some(room), Some(epoch)) => {
            let found = status.dead_letters().iter().find(|dead_letter| {
                dead_letter.fence().room_id().0 == room
                    && dead_letter.fence().race_epoch().get() == epoch
            });
            match found {
                Some(dead_letter) => vec![dead_letter.clone()],
                None => return Reply::error("no such dead letter"),
            }
        }
        _ => {
            return Reply::error("give both room and race_epoch, or neither to retry all");
        }
    };
    let mut retried = Vec::new();
    let mut failed = Vec::new();
    for dead_letter in chosen {
        let race = json!({
            "room": dead_letter.fence().room_id().0,
            "race_epoch": dead_letter.fence().race_epoch().get(),
        });
        match context.world.retry_reward_dead_letter(dead_letter).await {
            Ok(()) => retried.push(race),
            Err(WorldError::Stopped) => return world_unavailable(),
            Err(error) => {
                let mut race = race;
                race["error"] = json!(error.to_string());
                failed.push(race);
            }
        }
    }
    context.record(
        peer,
        "retry reward",
        json!({ "retried": retried, "failed": failed }),
    );
    Reply::ok(json!({ "retried": retried, "failed": failed }))
}

#[cfg(test)]
mod tests {
    use std::net::Ipv4Addr;

    use p5099_core::{account_protocol::password_key, rcon::quote};

    use super::*;

    const TOKEN: &str = "0123456789abcdef-test";
    const RIDER: IpAddr = IpAddr::V4(Ipv4Addr::new(10, 0, 0, 2));

    struct Console {
        address: SocketAddr,
        operator: OperatorConsole,
        accounts: Arc<AccountDirectory>,
        world: WorldHandle,
        _shutdown: watch::Sender<bool>,
    }

    async fn start() -> Console {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let config = AdminConfig::new(address, TOKEN).unwrap();
        let accounts = Arc::new(AccountDirectory::in_memory());
        let (world, _task) = WorldHandle::spawn(16).unwrap();
        let (shutdown, receiver) = watch::channel(false);
        let operator = OperatorConsole::new(Arc::clone(&accounts), world.clone(), true);
        tokio::spawn(accept_loop(
            listener,
            Arc::clone(&operator.context),
            config.token,
            receiver,
        ));
        Console {
            address,
            operator,
            accounts,
            world,
            _shutdown: shutdown,
        }
    }

    async fn send(stream: &mut TcpStream, packet: Packet) {
        stream.write_all(&packet.encode()).await.unwrap();
    }

    async fn receive(stream: &mut TcpStream) -> Packet {
        read_packet(stream).await.unwrap().unwrap()
    }

    async fn login(console: &Console, token: &str) -> Result<TcpStream, String> {
        let mut stream = TcpStream::connect(console.address).await.unwrap();
        send(&mut stream, Packet::new(9, SERVERDATA_AUTH, token)).await;
        let empty = receive(&mut stream).await;
        assert_eq!(empty.kind, SERVERDATA_RESPONSE_VALUE);
        let auth = receive(&mut stream).await;
        assert_eq!(auth.kind, SERVERDATA_AUTH_RESPONSE);
        if auth.id == 9 {
            Ok(stream)
        } else {
            assert_eq!(auth.id, AUTH_FAILED_ID);
            Err(String::from_utf8(empty.body).unwrap())
        }
    }

    async fn exec(stream: &mut TcpStream, line: &str) -> Value {
        send(stream, Packet::new(1, SERVERDATA_EXECCOMMAND, line)).await;
        send(
            stream,
            Packet::new(2, SERVERDATA_RESPONSE_VALUE, Vec::new()),
        )
        .await;
        let mut body = Vec::new();
        loop {
            let packet = receive(stream).await;
            if packet.id == 2 {
                break;
            }
            assert_eq!((packet.id, packet.kind), (1, SERVERDATA_RESPONSE_VALUE));
            body.extend(packet.body);
        }
        serde_json::from_slice(&body).unwrap()
    }

    async fn call(console: &Console, line: &str) -> Value {
        let mut stream = login(console, TOKEN).await.unwrap();
        exec(&mut stream, line).await
    }

    fn error(reply: &Value) -> &str {
        reply["error"].as_str().unwrap_or_default()
    }

    #[test]
    fn short_tokens_are_refused_and_debug_hides_the_token() {
        let address = "127.0.0.1:1".parse().unwrap();
        assert!(AdminConfig::new(address, "short").is_none());
        let config = AdminConfig::new(address, TOKEN).unwrap();
        assert!(!format!("{config:?}").contains(TOKEN));
    }

    #[tokio::test]
    async fn requires_the_token_and_throttles_guessing() {
        let console = start().await;
        assert_eq!(
            login(&console, "wrong").await.unwrap_err(),
            "wrong console token"
        );
        let body = call(&console, "status").await;
        assert_eq!(body["players_online"], 0);
        assert_eq!(body["registration_open"], true);
        for _ in 1..MAX_FAILED_AUTH {
            login(&console, "wrong").await.unwrap_err();
        }
        assert_eq!(
            login(&console, TOKEN).await.unwrap_err(),
            "too many failed attempts"
        );
    }

    #[tokio::test]
    async fn commands_before_auth_close_the_connection() {
        let console = start().await;
        let mut stream = TcpStream::connect(console.address).await.unwrap();
        send(
            &mut stream,
            Packet::new(1, SERVERDATA_EXECCOMMAND, "status"),
        )
        .await;
        assert!(read_packet(&mut stream).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn one_connection_runs_many_commands() {
        let console = start().await;
        let mut stream = login(&console, TOKEN).await.unwrap();
        assert_eq!(exec(&mut stream, "health").await["status"], "ok");
        assert!(
            exec(&mut stream, "HELP").await["commands"]
                .as_array()
                .unwrap()
                .contains(&json!("maintenance [on|off] [--kick]"))
        );
        assert_eq!(
            error(&exec(&mut stream, "").await),
            "empty command; try `help`"
        );
        assert_eq!(
            error(&exec(&mut stream, "nothing").await),
            "unknown command `nothing`; try `help`"
        );
        assert_eq!(
            error(&exec(&mut stream, "kick").await),
            "missing <nickname>"
        );
        assert_eq!(
            error(&exec(&mut stream, "status extra").await),
            "unexpected argument `extra`"
        );
        assert_eq!(
            error(&exec(&mut stream, r#"info "x"#).await),
            "unterminated quote"
        );
    }

    #[tokio::test]
    async fn long_replies_span_packets() {
        let console = start().await;
        for host in 0..80 {
            let address = IpAddr::V4(Ipv4Addr::new(192, 0, 2, host));
            console
                .accounts
                .set_address_banned(
                    address,
                    true,
                    Some("a reason long enough to add up".to_owned()),
                )
                .unwrap();
        }
        let body = call(&console, "bans").await;
        assert_eq!(body["addresses"].as_array().unwrap().len(), 80);
    }

    #[tokio::test]
    async fn manages_accounts() {
        let console = start().await;
        console
            .accounts
            .register("Rider", password_key("Rider", "secret"), RIDER)
            .unwrap();

        let body = call(&console, "accounts").await;
        assert_eq!(body["accounts"][0]["nickname"], "Rider");
        assert!(body["accounts"][0].get("key").is_none());

        assert_eq!(call(&console, "admin rider").await["admin"], true);
        let body = call(&console, "ban RIDER --reason cheating").await;
        assert_eq!(body["kicked"], false);
        let summary = &console.accounts.summaries()[0];
        assert!(summary.admin && summary.banned);
        assert_eq!(summary.ban_reason.as_deref(), Some("cheating"));

        assert_eq!(
            error(&call(&console, "password Rider x").await),
            "password length is out of range"
        );
        assert_eq!(
            error(&call(&console, "password Nobody long-enough").await),
            "no such account"
        );
        assert_eq!(
            call(
                &console,
                &format!("password Rider {}", quote("--long-enough"))
            )
            .await["nickname"],
            "Rider"
        );

        let body = call(&console, "registration close").await;
        assert_eq!(body["registration_open"], false);
        assert!(!console.accounts.registration_open());
        assert!(error(&call(&console, "registration maybe").await).contains("open or close"));
        assert!(error(&call(&console, "ban Rider --duration 5w").await).contains("unknown unit"));
        assert_eq!(
            error(&call(&console, "ban Rider --reason").await),
            "--reason needs a value"
        );
    }

    #[tokio::test]
    async fn lists_and_kicks_online_riders() {
        let console = start().await;
        let session = console
            .world
            .register_session(SocketAddr::new(RIDER, 50_000))
            .await
            .unwrap();
        console
            .world
            .claim_identity(session, "Rider")
            .await
            .unwrap();

        let body = call(&console, "players").await;
        assert_eq!(body["players"][0]["nickname"], "Rider");
        assert_eq!(body["players"][0]["address"], "10.0.0.2");
        assert_eq!(call(&console, "rooms").await["rooms"], json!([]));

        assert_eq!(call(&console, "kick rider").await["kicked"], "rider");
        assert_eq!(
            error(&call(&console, "kick Ghost").await),
            "no rider by that nickname is online"
        );
    }

    #[tokio::test]
    async fn creates_inspects_and_deletes_accounts() {
        let console = start().await;
        console.accounts.set_registration_open(false);
        let body = call(&console, r#"create " 라이더 " long-enough --admin"#).await;
        assert_eq!(body["nickname"], "라이더");
        assert_eq!(body["admin"], true);
        assert_eq!(
            error(&call(&console, "create 라이더 long-enough").await),
            "that nickname is taken"
        );

        let body = call(&console, "info 라이더").await;
        assert_eq!(body["nickname"], "라이더");
        assert_eq!(body["admin"], true);
        assert_eq!(body["ticket_active"], false);
        assert_eq!(body["online"], Value::Null);
        assert_eq!(error(&call(&console, "info").await), "missing <nickname>");

        let body = call(&console, "ban 라이더 --duration 1h").await;
        let until = body["banned_until"].as_u64().unwrap();
        assert!(until > unix_now());
        assert_eq!(console.accounts.summaries()[0].banned_until, Some(until));

        assert!(error(&call(&console, "delete 라이더 now").await).contains("unexpected"));
        let body = call(&console, "delete 라이더").await;
        assert_eq!(body["deleted"], true);
        assert!(console.accounts.summaries().is_empty());

        let body = call(&console, "audit --limit 2").await;
        let entries = body["entries"].as_array().unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0]["action"], "ban");
        assert_eq!(entries[1]["action"], "delete account");
        assert!(!body.to_string().contains("long-enough"));
    }

    #[tokio::test]
    async fn bans_addresses_and_kicks_groups() {
        let console = start().await;
        for (port, nickname) in [(50_000, "Rider"), (50_001, "Other")] {
            let session = console
                .world
                .register_session(SocketAddr::new(RIDER, port))
                .await
                .unwrap();
            console
                .world
                .claim_identity(session, nickname)
                .await
                .unwrap();
        }
        console
            .world
            .register_session(SocketAddr::new(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 9)), 1))
            .await
            .unwrap();

        let body = call(&console, "players").await;
        assert!(body["players"][0]["connected_seconds"].is_u64());

        let body = call(&console, "ban-ip 10.0.0.2 --reason flood").await;
        assert_eq!(body["kicked"]["sessions"], 2);
        assert_eq!(body["kicked"]["nicknames"], json!(["Other", "Rider"]));
        assert!(console.accounts.address_banned(RIDER));
        assert_eq!(
            call(&console, "bans").await["addresses"][0]["reason"],
            "flood"
        );

        assert_eq!(call(&console, "kick-all").await["kicked"]["sessions"], 3);
        assert_eq!(error(&call(&console, "kick-room 77").await), "no such room");
        assert_eq!(
            error(&call(&console, "kick-ip not-an-ip").await),
            "invalid address `not-an-ip`"
        );
    }

    #[tokio::test]
    async fn maintenance_rewards_and_throttles() {
        let console = start().await;
        let body = call(&console, "maintenance on --kick").await;
        assert_eq!(body["kicked"]["sessions"], 0);
        assert!(console.accounts.maintenance());
        assert_eq!(call(&console, "maintenance").await["maintenance"], true);
        assert_eq!(call(&console, "status").await["maintenance"], true);

        assert_eq!(call(&console, "rewards").await["dead_letters"], json!([]));
        assert_eq!(call(&console, "retry-reward").await["retried"], json!([]));
        assert!(error(&call(&console, "retry-reward 1").await).contains("give both"));

        login(&console, "wrong").await.unwrap_err();
        let body = call(&console, "throttles").await;
        assert_eq!(body["console"][0]["failed_tokens"], 1);
        assert_eq!(call(&console, "clear-throttle").await["console"], 1);
        assert_eq!(call(&console, "throttles").await["console"], json!([]));
    }

    #[test]
    fn names_variants() {
        assert_eq!(snake_name(&RoomPhase::Lobby), "lobby");
        assert_eq!(
            snake_name(&crate::RewardTerminalReason::RewardPersistence),
            "reward_persistence"
        );
    }

    #[tokio::test]
    async fn shutdown_request_reaches_the_waiter() {
        let console = start().await;
        assert_eq!(call(&console, "shutdown").await["shutdown"], "requested");
        timeout(
            Duration::from_secs(1),
            console.operator.shutdown_requested(),
        )
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn in_process_commands_match_rcon_and_name_their_caller() {
        let console = start().await;
        let caller = Caller::Named("discord:operator (42)".to_owned());
        assert_eq!(
            console.operator.execute(&caller, "status").await["require_account"],
            call(&console, "status").await["require_account"]
        );
        let body = console
            .operator
            .execute(&caller, "registration close")
            .await;
        assert_eq!(body["error"], Value::Null, "{body}");
        assert!(!console.accounts.registration_open());
        assert_eq!(
            error(&console.operator.execute(&caller, "no-such-command").await),
            "unknown command `no-such-command`; try `help`"
        );
        let audit = call(&console, "audit").await;
        assert_eq!(
            audit["entries"][0]["peer"], "discord:operator (42)",
            "{audit}"
        );
        assert_eq!(audit["entries"][0]["action"], "registration");
    }
}
