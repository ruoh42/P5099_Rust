use std::{
    io,
    net::{Ipv4Addr, SocketAddr, SocketAddrV4},
    time::Duration,
};

use p5099_core::{
    account_protocol::{
        AccountFailure, AccountKey, AccountRequest, AccountResponse, MAX_ACCOUNT_LINE_BYTES,
        PasswordError, PlayerPresence, login_proof, password_key, validate_password,
    },
    nickname::{NicknameError, normalize_nickname},
    ports::PortTopology,
};
use thiserror::Error;
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    net::TcpStream,
    time,
};

pub const DEFAULT_ACCOUNT_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchTicket {
    pub nickname: String,
    pub ticket: String,
}

#[derive(Debug, Error)]
pub enum AccountError {
    #[error("invalid nickname")]
    Nickname(#[from] NicknameError),

    #[error("invalid password")]
    Password(#[from] PasswordError),

    #[error("account server {endpoint} did not answer within {timeout:?}")]
    Timeout {
        endpoint: SocketAddr,
        timeout: Duration,
    },

    #[error("cannot reach the account server {endpoint}")]
    Connect {
        endpoint: SocketAddr,
        #[source]
        source: io::Error,
    },

    #[error("account server connection failed")]
    Io(#[from] io::Error),

    #[error("the account server sent an unexpected reply")]
    Protocol,

    #[error("the account server refused the request ({})", .0.code())]
    Refused(AccountFailure),
}

impl AccountError {
    #[must_use]
    pub const fn failure(&self) -> Option<AccountFailure> {
        match self {
            Self::Refused(failure) => Some(*failure),
            _ => None,
        }
    }
}

struct Connection {
    stream: BufReader<TcpStream>,
}

impl Connection {
    async fn open(endpoint: SocketAddr) -> Result<Self, AccountError> {
        let stream = TcpStream::connect(endpoint)
            .await
            .map_err(|source| AccountError::Connect { endpoint, source })?;
        Ok(Self {
            stream: BufReader::new(stream),
        })
    }

    async fn exchange(
        &mut self,
        request: &AccountRequest,
    ) -> Result<AccountResponse, AccountError> {
        let mut line = request.to_line();
        line.push('\n');
        self.stream.get_mut().write_all(line.as_bytes()).await?;
        let mut reply = Vec::new();
        (&mut self.stream)
            .take(u64::try_from(MAX_ACCOUNT_LINE_BYTES + 1).unwrap_or(u64::MAX))
            .read_until(b'\n', &mut reply)
            .await?;
        if reply.last() != Some(&b'\n') {
            return Err(AccountError::Protocol);
        }
        let reply = std::str::from_utf8(&reply).map_err(|_| AccountError::Protocol)?;
        match AccountResponse::parse(reply.trim_end_matches(['\r', '\n'])) {
            Ok(AccountResponse::Failed(failure)) => Err(AccountError::Refused(failure)),
            Ok(response) => Ok(response),
            Err(_) => Err(AccountError::Protocol),
        }
    }

    async fn challenge(&mut self, nickname: &str) -> Result<[u8; 32], AccountError> {
        match self
            .exchange(&AccountRequest::Challenge {
                nickname: nickname.to_owned(),
            })
            .await?
        {
            AccountResponse::Nonce(nonce) => Ok(nonce),
            _ => Err(AccountError::Protocol),
        }
    }
}

fn endpoint(server: Ipv4Addr, ports: PortTopology) -> SocketAddr {
    SocketAddr::V4(SocketAddrV4::new(server, ports.account_tcp()))
}

async fn stretch(nickname: &str, password: &str) -> Result<AccountKey, AccountError> {
    let nickname = nickname.to_owned();
    let password = password.to_owned();
    tokio::task::spawn_blocking(move || password_key(&nickname, &password))
        .await
        .map_err(|error| AccountError::Io(io::Error::other(error)))
}

async fn with_timeout<T>(
    endpoint: SocketAddr,
    timeout: Duration,
    operation: impl Future<Output = Result<T, AccountError>>,
) -> Result<T, AccountError> {
    time::timeout(timeout, operation)
        .await
        .map_err(|_| AccountError::Timeout { endpoint, timeout })?
}

pub async fn register(
    server: Ipv4Addr,
    ports: PortTopology,
    nickname: &str,
    password: &str,
    timeout: Duration,
) -> Result<String, AccountError> {
    let nickname = normalize_nickname(nickname)?;
    validate_password(password)?;
    let key = stretch(&nickname, password).await?;
    let endpoint = endpoint(server, ports);
    with_timeout(endpoint, timeout, async {
        let mut connection = Connection::open(endpoint).await?;
        match connection
            .exchange(&AccountRequest::Register { nickname, key })
            .await?
        {
            AccountResponse::Registered { nickname } => Ok(nickname),
            _ => Err(AccountError::Protocol),
        }
    })
    .await
}

pub async fn account_key(nickname: &str, password: &str) -> Result<AccountKey, AccountError> {
    stretch(&normalize_nickname(nickname)?, password).await
}

pub async fn login_with_key(
    server: Ipv4Addr,
    ports: PortTopology,
    nickname: &str,
    key: &AccountKey,
    timeout: Duration,
) -> Result<LaunchTicket, AccountError> {
    let nickname = normalize_nickname(nickname)?;
    let endpoint = endpoint(server, ports);
    with_timeout(endpoint, timeout, async {
        let mut connection = Connection::open(endpoint).await?;
        let nonce = connection.challenge(&nickname).await?;
        match connection
            .exchange(&AccountRequest::Login {
                proof: login_proof(key, &nonce),
                nickname,
            })
            .await?
        {
            AccountResponse::Ticket { nickname, ticket } => Ok(LaunchTicket { nickname, ticket }),
            _ => Err(AccountError::Protocol),
        }
    })
    .await
}

pub async fn change_password(
    server: Ipv4Addr,
    ports: PortTopology,
    nickname: &str,
    password: &str,
    new_password: &str,
    timeout: Duration,
) -> Result<(), AccountError> {
    let nickname = normalize_nickname(nickname)?;
    validate_password(new_password)?;
    let key = stretch(&nickname, password).await?;
    let new_key = stretch(&nickname, new_password).await?;
    let endpoint = endpoint(server, ports);
    with_timeout(endpoint, timeout, async {
        let mut connection = Connection::open(endpoint).await?;
        let nonce = connection.challenge(&nickname).await?;
        match connection
            .exchange(&AccountRequest::ChangePassword {
                proof: login_proof(&key, &nonce),
                nickname,
                new_key,
            })
            .await?
        {
            AccountResponse::PasswordChanged => Ok(()),
            _ => Err(AccountError::Protocol),
        }
    })
    .await
}

pub async fn presence(
    server: Ipv4Addr,
    ports: PortTopology,
    ticket: &LaunchTicket,
    timeout: Duration,
) -> Result<PlayerPresence, AccountError> {
    let endpoint = endpoint(server, ports);
    with_timeout(endpoint, timeout, async {
        let mut connection = Connection::open(endpoint).await?;
        match connection
            .exchange(&AccountRequest::Presence {
                nickname: ticket.nickname.clone(),
                ticket: ticket.ticket.clone(),
            })
            .await?
        {
            AccountResponse::Presence(presence) => Ok(presence),
            _ => Err(AccountError::Protocol),
        }
    })
    .await
}

#[cfg(test)]
mod tests {
    use p5099_core::account_protocol::to_hex;
    use tokio::net::TcpListener;

    use super::*;

    async fn fake_server(key: AccountKey) -> PortTopology {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut stream = BufReader::new(stream);
            let nonce = [8_u8; 32];
            loop {
                let mut line = String::new();
                if stream.read_line(&mut line).await.unwrap() == 0 {
                    return;
                }
                let response = match AccountRequest::parse(line.trim_end()).unwrap() {
                    AccountRequest::Challenge { .. } => AccountResponse::Nonce(nonce),
                    AccountRequest::Login { nickname, proof } => {
                        if proof == login_proof(&key, &nonce) {
                            AccountResponse::Ticket {
                                nickname,
                                ticket: to_hex(&[1; 32]),
                            }
                        } else {
                            AccountResponse::Failed(AccountFailure::BadCredentials)
                        }
                    }
                    _ => AccountResponse::Failed(AccountFailure::BadRequest),
                };
                let mut reply = response.to_line();
                reply.push('\n');
                stream.get_mut().write_all(reply.as_bytes()).await.unwrap();
            }
        });
        PortTopology::new(port - 5).unwrap()
    }

    #[tokio::test]
    async fn login_proves_the_password_and_returns_the_ticket() {
        let key = account_key(" Rider ", "secret").await.unwrap();
        assert_eq!(key, password_key("Rider", "secret"));
        let ports = fake_server(key).await;
        let ticket = login_with_key(
            Ipv4Addr::LOCALHOST,
            ports,
            " Rider ",
            &key,
            DEFAULT_ACCOUNT_TIMEOUT,
        )
        .await
        .unwrap();
        assert_eq!(ticket.nickname, "Rider");
        assert_eq!(ticket.ticket, to_hex(&[1; 32]));

        let wrong = account_key("Rider", "wrong").await.unwrap();
        let ports = fake_server(key).await;
        let error = login_with_key(
            Ipv4Addr::LOCALHOST,
            ports,
            "Rider",
            &wrong,
            DEFAULT_ACCOUNT_TIMEOUT,
        )
        .await
        .unwrap_err();
        assert_eq!(error.failure(), Some(AccountFailure::BadCredentials));
    }

    #[tokio::test]
    async fn short_passwords_are_rejected_before_connecting() {
        let error = register(
            Ipv4Addr::LOCALHOST,
            PortTopology::default(),
            "Rider",
            "abc",
            DEFAULT_ACCOUNT_TIMEOUT,
        )
        .await
        .unwrap_err();
        assert!(matches!(
            error,
            AccountError::Password(PasswordError::TooShort)
        ));
    }
}
