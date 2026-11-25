use std::{
    fmt::{self, Write as _},
    future::Future,
    pin::Pin,
    sync::Arc,
    time::Duration,
};

use p5099_core::{
    account_protocol::{MAX_PASSWORD_CHARS, MIN_PASSWORD_CHARS},
    rcon::quote,
};
use serde::{Deserialize, de::DeserializeOwned};
use serde_json::Value;

const TIMEOUT: Duration = Duration::from_secs(20);

#[derive(Debug)]
pub enum ConsoleError {
    Unreachable(String),
    Api(String),
    Decode(String),
}

impl fmt::Display for ConsoleError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unreachable(error) => {
                write!(formatter, "서버 연결 실패 ({error})")
            }
            Self::Api(message) => formatter.write_str(&korean_error(message)),
            Self::Decode(error) => {
                write!(formatter, "서버 응답 오류 ({error})")
            }
        }
    }
}

impl std::error::Error for ConsoleError {}

fn korean_error(message: &str) -> String {
    let translated = match message {
        "no such account" => "계정 없음",
        "that nickname is taken" => "이미 있는 닉네임",
        "the nickname breaks the client's rules" => "쓸 수 없는 닉네임",
        "password length is out of range" => {
            return format!("비밀번호는 {MIN_PASSWORD_CHARS}~{MAX_PASSWORD_CHARS}자");
        }
        "could not save the account directory" => "계정 저장 실패",
        "no rider by that nickname is online" => "접속 중이 아님",
        "no such room" => "방 없음",
        "no such dead letter" => "기록 없음",
        "the game world is stopping" => "서버 종료 중",
        "wrong console token" => "콘솔 토큰 불일치",
        "too many failed attempts" => "콘솔 시도 제한됨",
        _ => return message.to_owned(),
    };
    translated.to_owned()
}

pub type ConsoleResult<T> = Result<T, ConsoleError>;

pub type Reply = Pin<Box<dyn Future<Output = ConsoleResult<Value>> + Send>>;

pub trait Backend: Send + Sync {
    fn execute(&self, caller: String, line: String) -> Reply;
}

#[derive(Clone)]
pub struct Console {
    backend: Arc<dyn Backend>,
    caller: String,
}

impl Console {
    pub fn new(backend: Arc<dyn Backend>) -> Self {
        Self {
            backend,
            caller: "discord".to_owned(),
        }
    }

    pub fn acting_as(&self, user: &serenity::model::user::User) -> Self {
        Self {
            backend: Arc::clone(&self.backend),
            caller: format!("discord:{} ({})", user.name, user.id),
        }
    }

    async fn run<T: DeserializeOwned>(&self, line: String) -> ConsoleResult<T> {
        let body = tokio::time::timeout(TIMEOUT, self.backend.execute(self.caller.clone(), line))
            .await
            .map_err(|_| ConsoleError::Unreachable("시간 초과".to_owned()))??;
        if let Some(message) = body["error"].as_str() {
            return Err(ConsoleError::Api(message.to_owned()));
        }
        serde_json::from_value(body).map_err(|error| ConsoleError::Decode(error.to_string()))
    }

    async fn send(&self, line: String) -> ConsoleResult<()> {
        self.run::<Value>(line).await.map(drop)
    }

    async fn list<T: DeserializeOwned>(&self, line: &str, key: &str) -> ConsoleResult<Vec<T>> {
        let mut reply: Value = self.run(line.to_owned()).await?;
        serde_json::from_value(reply[key].take())
            .map_err(|error| ConsoleError::Decode(error.to_string()))
    }

    pub async fn status(&self) -> ConsoleResult<Status> {
        self.run("status".to_owned()).await
    }

    pub async fn players(&self) -> ConsoleResult<Vec<Player>> {
        self.list("players", "players").await
    }

    pub async fn rooms(&self) -> ConsoleResult<Vec<Room>> {
        self.list("rooms", "rooms").await
    }

    pub async fn accounts(&self) -> ConsoleResult<Vec<Account>> {
        self.list("accounts", "accounts").await
    }

    pub async fn account_info(&self, nickname: &str) -> ConsoleResult<AccountInfo> {
        self.run(format!("info {}", quote(nickname))).await
    }

    pub async fn create_account(
        &self,
        nickname: &str,
        password: &str,
        admin: bool,
    ) -> ConsoleResult<NicknameReply> {
        let mut line = format!("create {} {}", quote(nickname), quote(password));
        if admin {
            line.push_str(" --admin");
        }
        self.run(line).await
    }

    pub async fn delete_account(&self, nickname: &str) -> ConsoleResult<KickedReply> {
        self.run(format!("delete {}", quote(nickname))).await
    }

    pub async fn set_admin(&self, nickname: &str, admin: bool) -> ConsoleResult<NicknameReply> {
        let verb = if admin { "admin" } else { "unadmin" };
        self.run(format!("{verb} {}", quote(nickname))).await
    }

    pub async fn ban(
        &self,
        nickname: &str,
        reason: Option<&str>,
        duration_seconds: Option<u64>,
    ) -> ConsoleResult<BanReply> {
        let mut line = format!("ban {}", quote(nickname));
        if let Some(reason) = reason {
            line.push_str(" --reason ");
            line.push_str(&quote(reason));
        }
        if let Some(seconds) = duration_seconds {
            let _ = write!(line, " --duration {seconds}");
        }
        self.run(line).await
    }

    pub async fn unban(&self, nickname: &str) -> ConsoleResult<BanReply> {
        self.run(format!("unban {}", quote(nickname))).await
    }

    pub async fn reset_password(
        &self,
        nickname: &str,
        password: &str,
    ) -> ConsoleResult<NicknameReply> {
        self.run(format!("password {} {}", quote(nickname), quote(password)))
            .await
    }

    pub async fn address_bans(&self) -> ConsoleResult<Vec<AddressBan>> {
        self.list("bans", "addresses").await
    }

    pub async fn ban_address(
        &self,
        address: &str,
        reason: Option<&str>,
    ) -> ConsoleResult<AddressBanReply> {
        let mut line = format!("ban-ip {}", quote(address));
        if let Some(reason) = reason {
            line.push_str(" --reason ");
            line.push_str(&quote(reason));
        }
        self.run(line).await
    }

    pub async fn unban_address(&self, address: &str) -> ConsoleResult<AddressBanReply> {
        self.run(format!("unban-ip {}", quote(address))).await
    }

    pub async fn kick(&self, nickname: &str) -> ConsoleResult<KickedReply> {
        self.run(format!("kick {}", quote(nickname))).await
    }

    pub async fn kick_address(&self, address: &str) -> ConsoleResult<KickedReply> {
        self.run(format!("kick-ip {}", quote(address))).await
    }

    pub async fn kick_room(&self, id: u32) -> ConsoleResult<KickedReply> {
        self.run(format!("kick-room {id}")).await
    }

    pub async fn kick_all(&self, keep_admins: bool) -> ConsoleResult<KickedReply> {
        let line = if keep_admins {
            "kick-all --keep-admins"
        } else {
            "kick-all"
        };
        self.run(line.to_owned()).await
    }

    pub async fn set_maintenance(&self, enabled: bool, kick: bool) -> ConsoleResult<KickedReply> {
        let mut line = format!("maintenance {}", if enabled { "on" } else { "off" });
        if kick {
            line.push_str(" --kick");
        }
        self.run(line).await
    }

    pub async fn set_registration(&self, open: bool) -> ConsoleResult<()> {
        let state = if open { "open" } else { "close" };
        self.send(format!("registration {state}")).await
    }

    pub async fn rewards(&self) -> ConsoleResult<Rewards> {
        self.run("rewards".to_owned()).await
    }

    pub async fn retry_rewards(&self, race: Option<(u32, u64)>) -> ConsoleResult<RetryReply> {
        let line = match race {
            Some((room, race_epoch)) => format!("retry-reward {room} {race_epoch}"),
            None => "retry-reward".to_owned(),
        };
        self.run(line).await
    }

    pub async fn throttles(&self) -> ConsoleResult<Throttles> {
        self.run("throttles".to_owned()).await
    }

    pub async fn clear_throttles(&self, address: Option<&str>) -> ConsoleResult<()> {
        let line = match address {
            Some(address) => format!("clear-throttle {}", quote(address)),
            None => "clear-throttle".to_owned(),
        };
        self.send(line).await
    }

    pub async fn audit(&self, limit: usize) -> ConsoleResult<Vec<AuditEntry>> {
        self.list(&format!("audit --limit {limit}"), "entries")
            .await
    }

    pub async fn shutdown(&self) -> ConsoleResult<()> {
        self.send("shutdown".to_owned()).await
    }
}

#[cfg(test)]
pub(crate) mod rcon {
    use p5099_core::rcon::{
        AUTH_FAILED_ID, Packet, SERVERDATA_AUTH, SERVERDATA_AUTH_RESPONSE, SERVERDATA_EXECCOMMAND,
        SERVERDATA_RESPONSE_VALUE, payload_len,
    };
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpStream,
    };

    use super::{ConsoleError, ConsoleResult};

    const AUTH_ID: i32 = 1;
    const COMMAND_ID: i32 = 2;
    const END_ID: i32 = 3;

    fn io_error(error: &std::io::Error) -> ConsoleError {
        ConsoleError::Unreachable(match error.kind() {
            std::io::ErrorKind::ConnectionRefused => "연결 거부".to_owned(),
            std::io::ErrorKind::UnexpectedEof => "연결 끊김".to_owned(),
            _ => error.to_string(),
        })
    }

    async fn read(stream: &mut TcpStream) -> ConsoleResult<Packet> {
        let mut size = [0_u8; 4];
        stream
            .read_exact(&mut size)
            .await
            .map_err(|error| io_error(&error))?;
        let size = payload_len(size).map_err(|error| ConsoleError::Decode(error.to_string()))?;
        let mut payload = vec![0_u8; size];
        stream
            .read_exact(&mut payload)
            .await
            .map_err(|error| io_error(&error))?;
        Packet::decode(&payload).map_err(|error| ConsoleError::Decode(error.to_string()))
    }

    async fn write(stream: &mut TcpStream, packets: &[Packet]) -> ConsoleResult<()> {
        let bytes: Vec<u8> = packets.iter().flat_map(Packet::encode).collect();
        stream
            .write_all(&bytes)
            .await
            .map_err(|error| io_error(&error))
    }

    pub(crate) async fn exchange(address: &str, token: &str, line: &str) -> ConsoleResult<Vec<u8>> {
        let mut stream = TcpStream::connect(address)
            .await
            .map_err(|error| io_error(&error))?;
        write(&mut stream, &[Packet::new(AUTH_ID, SERVERDATA_AUTH, token)]).await?;
        let mut refusal = Vec::new();
        loop {
            let packet = read(&mut stream).await?;
            match packet.kind {
                SERVERDATA_AUTH_RESPONSE if packet.id == AUTH_FAILED_ID => {
                    let message = String::from_utf8_lossy(&refusal);
                    return Err(ConsoleError::Api(if message.is_empty() {
                        "wrong console token".to_owned()
                    } else {
                        message.into_owned()
                    }));
                }
                SERVERDATA_AUTH_RESPONSE => break,
                _ => refusal = packet.body,
            }
        }
        write(
            &mut stream,
            &[
                Packet::new(COMMAND_ID, SERVERDATA_EXECCOMMAND, line),
                Packet::new(END_ID, SERVERDATA_RESPONSE_VALUE, Vec::new()),
            ],
        )
        .await?;
        let mut reply = Vec::new();
        loop {
            let packet = read(&mut stream).await?;
            match packet.id {
                END_ID => return Ok(reply),
                COMMAND_ID => reply.extend(packet.body),
                _ => {}
            }
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct Status {
    pub version: String,
    pub server_time: u64,
    pub uptime_seconds: u64,
    pub players_online: u64,
    pub rooms: u64,
    #[serde(default)]
    pub racing_rooms: u64,
    pub accounts: u64,
    #[serde(default)]
    pub require_account: bool,
    #[serde(default)]
    pub registration_open: bool,
    #[serde(default)]
    pub maintenance: bool,
    #[serde(default)]
    pub reward_recovery: Option<RewardRecovery>,
}

impl Status {
    pub fn started(&self) -> u64 {
        self.server_time.saturating_sub(self.uptime_seconds)
    }

    pub fn dead_letters(&self) -> u64 {
        self.reward_recovery
            .as_ref()
            .map_or(0, |recovery| recovery.dead_letters)
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct RewardRecovery {
    #[serde(default)]
    pub dead_letters: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Player {
    pub nickname: String,
    pub user_no: u32,
    pub address: String,
    pub room: Option<u32>,
    #[serde(default)]
    pub admin: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Room {
    pub id: u32,
    pub name: String,
    pub game_type: u8,
    pub speed_type: u8,
    pub track: u32,
    pub phase: String,
    #[serde(default)]
    pub locked: bool,
    #[serde(default)]
    pub players: Vec<String>,
    #[serde(default)]
    pub observers: Vec<String>,
    pub capacity: usize,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Account {
    pub nickname: String,
    pub created: u64,
    #[serde(default)]
    pub admin: bool,
    #[serde(default)]
    pub banned: bool,
    #[serde(default)]
    pub ban_reason: Option<String>,
    #[serde(default)]
    pub banned_until: Option<u64>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AccountInfo {
    #[serde(flatten)]
    pub account: Account,
    #[serde(default)]
    pub last_login: Option<u64>,
    #[serde(default)]
    pub last_login_address: Option<String>,
    #[serde(default)]
    pub online: Option<Player>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct NicknameReply {
    pub nickname: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct KickedReply {
    #[serde(default)]
    pub nickname: Option<String>,
    #[serde(default)]
    pub kicked: Value,
}

impl KickedReply {
    pub fn report(&self) -> (u64, Vec<String>) {
        match &self.kicked {
            Value::Object(report) => (
                report.get("sessions").and_then(Value::as_u64).unwrap_or(0),
                report
                    .get("nicknames")
                    .and_then(Value::as_array)
                    .map(|names| {
                        names
                            .iter()
                            .filter_map(|name| name.as_str().map(str::to_owned))
                            .collect()
                    })
                    .unwrap_or_default(),
            ),
            Value::String(nickname) => (1, vec![nickname.clone()]),
            Value::Bool(true) => (1, Vec::new()),
            _ => (0, Vec::new()),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct BanReply {
    pub nickname: String,
    #[serde(default)]
    pub banned_until: Option<u64>,
    #[serde(flatten)]
    pub kicked: KickedReply,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AddressBan {
    pub address: String,
    #[serde(default)]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AddressBanReply {
    pub address: String,
    #[serde(default)]
    pub changed: bool,
    #[serde(flatten)]
    pub kicked: KickedReply,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct Rewards {
    #[serde(default)]
    pub outstanding: Vec<RewardLane>,
    #[serde(default)]
    pub dead_letters: Vec<DeadLetter>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RewardLane {
    pub room: u32,
    pub race_epoch: u64,
    #[serde(default)]
    pub nickname: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DeadLetter {
    pub room: u32,
    pub race_epoch: u64,
    #[serde(default)]
    pub nickname: Option<String>,
    #[serde(default)]
    pub retryable: bool,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct RetryReply {
    #[serde(default)]
    pub retried: Vec<RetriedRace>,
    #[serde(default)]
    pub failed: Vec<RetriedRace>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RetriedRace {
    pub room: u32,
    pub race_epoch: u64,
    #[serde(default)]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct Throttles {
    #[serde(default)]
    pub launcher: Vec<LauncherThrottle>,
    #[serde(default)]
    pub console: Vec<ConsoleThrottle>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LauncherThrottle {
    pub address: String,
    #[serde(default)]
    pub failed_logins: u64,
    #[serde(default)]
    pub registrations: u64,
    #[serde(default)]
    pub limited: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ConsoleThrottle {
    pub address: String,
    #[serde(default)]
    pub failed_tokens: u64,
    #[serde(default)]
    pub limited: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AuditEntry {
    pub time: u64,
    pub peer: String,
    pub action: String,
    #[serde(default)]
    pub detail: Value,
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn reads_kick_reports() {
        let report: KickedReply =
            serde_json::from_value(json!({ "kicked": { "sessions": 2, "nicknames": ["a"] } }))
                .unwrap();
        assert_eq!(report.report(), (2, vec!["a".to_owned()]));
        let single: KickedReply = serde_json::from_value(json!({ "kicked": "rider" })).unwrap();
        assert_eq!(single.report(), (1, vec!["rider".to_owned()]));
        let none: KickedReply = serde_json::from_value(json!({ "kicked": null })).unwrap();
        assert_eq!(none.report(), (0, Vec::new()));
    }

    #[test]
    fn reads_account_info() {
        let info: AccountInfo = serde_json::from_value(json!({
            "nickname": "라이더", "created": 1, "admin": true, "banned": false,
            "last_login": 2, "last_login_address": "127.0.0.1",
            "online": null,
        }))
        .unwrap();
        assert_eq!(info.account.nickname, "라이더");
        assert!(info.account.admin && info.online.is_none());
    }

    #[test]
    fn translates_errors() {
        assert_eq!(korean_error("no such account"), "계정 없음");
        assert_eq!(korean_error("something new"), "something new");
    }
}
