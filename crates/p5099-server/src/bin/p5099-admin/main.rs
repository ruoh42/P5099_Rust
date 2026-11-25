use std::{
    fmt::Write as _,
    io::{self, BufRead, Read, Write},
    net::{IpAddr, TcpStream},
    path::PathBuf,
    time::Duration,
};

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use p5099_core::{
    ports::{DEFAULT_CONFIGURED_PORT, PortTopology},
    rcon::{
        AUTH_FAILED_ID, Packet, SERVERDATA_AUTH, SERVERDATA_AUTH_RESPONSE, SERVERDATA_EXECCOMMAND,
        SERVERDATA_RESPONSE_VALUE, parse_duration, payload_len, quote,
    },
};
use serde_json::Value;

const TOKEN_ENV: &str = "P5099_ADMIN_TOKEN";
const TIMEOUT: Duration = Duration::from_secs(20);

/// Manage a running p5099-server through its operator console.
#[derive(Debug, Parser)]
#[command(name = "p5099-admin", version)]
struct Args {
    /// Console host. Use an SSH tunnel to reach a remote server.
    #[arg(long, default_value = "127.0.0.1")]
    host: String,

    /// Console port (defaults to the server's base port + 7).
    #[arg(long)]
    port: Option<u16>,

    /// File holding the console token; defaults to `P5099_ADMIN_TOKEN`.
    #[arg(long, value_name = "PATH")]
    token_file: Option<PathBuf>,

    /// Print the console's JSON reply as is.
    #[arg(long)]
    json: bool,

    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Whether the console answers and accepts the token.
    Health,
    /// Server summary: uptime, players, rooms, accounts, maintenance.
    Status,
    /// Riders online now.
    Players,
    /// Open rooms and who is in them.
    Rooms,
    /// Registered accounts.
    Accounts,
    /// One account: flags, latest launcher login, where the rider is.
    Info { nickname: String },
    /// Create an account (even while registration is closed); the password
    /// is read from standard input.
    Create {
        nickname: String,
        /// Make it an administrator.
        #[arg(long)]
        admin: bool,
    },
    /// Delete an account and disconnect the rider. The profile stays.
    Delete {
        nickname: String,
        /// Required: confirms the deletion.
        #[arg(long)]
        yes: bool,
    },
    /// Disconnect an online rider.
    Kick { nickname: String },
    /// Disconnect every connection from an address.
    KickIp { address: IpAddr },
    /// Disconnect everyone in a room.
    KickRoom { id: u32 },
    /// Disconnect everyone.
    KickAll {
        /// Keep administrator accounts online.
        #[arg(long)]
        keep_admins: bool,
    },
    /// Suspend an account and disconnect the rider.
    Ban {
        nickname: String,
        #[arg(long)]
        reason: Option<String>,
        /// Lift the ban automatically after this long, e.g. 30m, 12h, 7d.
        #[arg(long, value_parser = parse_duration)]
        duration: Option<u64>,
    },
    /// Lift a suspension.
    Unban { nickname: String },
    /// Refuse an address and disconnect its connections.
    BanIp {
        address: IpAddr,
        #[arg(long)]
        reason: Option<String>,
    },
    /// Lift an address ban.
    UnbanIp { address: IpAddr },
    /// Banned addresses.
    Bans,
    /// Allow the account to request observer and league roles.
    Admin { nickname: String },
    /// Remove those roles.
    Unadmin { nickname: String },
    /// Set a new password; it is read from standard input.
    Password { nickname: String },
    /// Open or close launcher registration until the next restart.
    Registration {
        #[arg(value_parser = ["open", "close"])]
        state: String,
    },
    /// Show maintenance mode, or turn it on or off until the next restart.
    /// While on, only administrators can log in.
    Maintenance {
        #[arg(value_parser = ["on", "off"])]
        state: Option<String>,
        /// When turning it on, also disconnect everyone but administrators.
        #[arg(long)]
        kick: bool,
    },
    /// Race rewards still being saved and failed saves (dead letters).
    Rewards,
    /// Retry a failed reward save, or every retryable one.
    RetryReward {
        room: Option<u32>,
        race_epoch: Option<u64>,
    },
    /// Addresses near or over the login and console attempt limits.
    Throttles,
    /// Forget attempt history for one address, or for all.
    ClearThrottle { address: Option<IpAddr> },
    /// Recent operator changes since the server started.
    Audit {
        #[arg(long, default_value_t = 50)]
        limit: usize,
    },
    /// Stop the server gracefully.
    Shutdown,
}

fn main() -> Result<()> {
    let args = Args::parse();
    let token = read_token(&args)?;
    let port = args.port.unwrap_or_else(|| {
        PortTopology::new(DEFAULT_CONFIGURED_PORT).map_or(0, PortTopology::admin_tcp)
    });
    let command = command_line(&args.command)?;
    let reply = send(&args.host, port, &token, &command)?;
    if args.json {
        println!("{}", serde_json::to_string_pretty(&reply)?);
    } else {
        print_reply(&args.command, &reply);
    }
    Ok(())
}

fn read_token(args: &Args) -> Result<String> {
    let token = match &args.token_file {
        Some(path) => std::fs::read_to_string(path)
            .with_context(|| format!("failed to read {}", path.display()))?,
        None => std::env::var(TOKEN_ENV)
            .with_context(|| format!("set {TOKEN_ENV} or pass --token-file"))?,
    };
    Ok(token.trim().to_owned())
}

fn read_password(nickname: &str) -> Result<String> {
    eprint!("New password for {nickname}: ");
    io::stderr().flush()?;
    let mut password = String::new();
    io::stdin().lock().read_line(&mut password)?;
    let password = password.trim_end_matches(['\r', '\n']);
    if password.is_empty() {
        bail!("no password given");
    }
    Ok(password.to_owned())
}

fn command_line(command: &Command) -> Result<String> {
    let nick = |verb: &str, nickname: &str| format!("{verb} {}", quote(nickname));
    let line = match command {
        Command::Health => "health".to_owned(),
        Command::Status => "status".to_owned(),
        Command::Players => "players".to_owned(),
        Command::Rooms => "rooms".to_owned(),
        Command::Accounts => "accounts".to_owned(),
        Command::Info { nickname } => nick("info", nickname),
        Command::Create { nickname, admin } => {
            let password = read_password(nickname)?;
            let mut line = format!("create {} {}", quote(nickname), quote(&password));
            if *admin {
                line.push_str(" --admin");
            }
            line
        }
        Command::Delete { nickname, yes } => {
            if !yes {
                bail!("deleting {nickname} cannot be undone; add --yes to confirm");
            }
            nick("delete", nickname)
        }
        Command::Kick { nickname } => nick("kick", nickname),
        Command::KickIp { address } => format!("kick-ip {address}"),
        Command::KickRoom { id } => format!("kick-room {id}"),
        Command::KickAll { keep_admins } => {
            if *keep_admins {
                "kick-all --keep-admins".to_owned()
            } else {
                "kick-all".to_owned()
            }
        }
        Command::Ban {
            nickname,
            reason,
            duration,
        } => {
            let mut line = nick("ban", nickname);
            if let Some(reason) = reason {
                let _ = write!(line, " --reason {}", quote(reason));
            }
            if let Some(seconds) = duration {
                let _ = write!(line, " --duration {seconds}");
            }
            line
        }
        Command::Unban { nickname } => nick("unban", nickname),
        Command::Admin { nickname } => nick("admin", nickname),
        Command::Unadmin { nickname } => nick("unadmin", nickname),
        Command::BanIp { address, reason } => {
            let mut line = format!("ban-ip {address}");
            if let Some(reason) = reason {
                let _ = write!(line, " --reason {}", quote(reason));
            }
            line
        }
        Command::UnbanIp { address } => format!("unban-ip {address}"),
        Command::Bans => "bans".to_owned(),
        Command::Password { nickname } => {
            let password = read_password(nickname)?;
            format!("password {} {}", quote(nickname), quote(&password))
        }
        Command::Registration { state } => format!("registration {state}"),
        Command::Maintenance { state: None, .. } => "maintenance".to_owned(),
        Command::Maintenance {
            state: Some(state),
            kick,
        } => {
            if *kick {
                format!("maintenance {state} --kick")
            } else {
                format!("maintenance {state}")
            }
        }
        Command::Rewards => "rewards".to_owned(),
        Command::RetryReward { room, race_epoch } => match (room, race_epoch) {
            (None, None) => "retry-reward".to_owned(),
            (Some(room), Some(epoch)) => format!("retry-reward {room} {epoch}"),
            _ => bail!("give both ROOM and RACE_EPOCH, or neither to retry every retryable save"),
        },
        Command::Throttles => "throttles".to_owned(),
        Command::ClearThrottle { address: None } => "clear-throttle".to_owned(),
        Command::ClearThrottle {
            address: Some(address),
        } => format!("clear-throttle {address}"),
        Command::Audit { limit } => format!("audit --limit {limit}"),
        Command::Shutdown => "shutdown".to_owned(),
    };
    Ok(line)
}

const AUTH_ID: i32 = 1;
const COMMAND_ID: i32 = 2;
const END_ID: i32 = 3;

fn write_packet(stream: &mut TcpStream, packet: &Packet) -> Result<()> {
    stream.write_all(&packet.encode())?;
    Ok(())
}

fn read_packet(stream: &mut TcpStream) -> Result<Packet> {
    let mut size = [0_u8; 4];
    stream
        .read_exact(&mut size)
        .context("the console closed the connection")?;
    let mut payload = vec![0_u8; payload_len(size)?];
    stream.read_exact(&mut payload)?;
    Ok(Packet::decode(&payload)?)
}

fn send(host: &str, port: u16, token: &str, command: &str) -> Result<Value> {
    let mut stream = TcpStream::connect((host, port))
        .with_context(|| format!("could not reach the operator console at {host}:{port}"))?;
    stream.set_read_timeout(Some(TIMEOUT))?;
    stream.set_write_timeout(Some(TIMEOUT))?;

    write_packet(&mut stream, &Packet::new(AUTH_ID, SERVERDATA_AUTH, token))?;
    let mut refusal = String::new();
    loop {
        let packet = read_packet(&mut stream)?;
        match packet.kind {
            SERVERDATA_AUTH_RESPONSE if packet.id == AUTH_FAILED_ID => {
                bail!(
                    "{}",
                    if refusal.is_empty() {
                        "wrong console token"
                    } else {
                        &refusal
                    }
                );
            }
            SERVERDATA_AUTH_RESPONSE => break,
            _ => refusal = String::from_utf8_lossy(&packet.body).into_owned(),
        }
    }

    write_packet(
        &mut stream,
        &Packet::new(COMMAND_ID, SERVERDATA_EXECCOMMAND, command),
    )?;
    write_packet(
        &mut stream,
        &Packet::new(END_ID, SERVERDATA_RESPONSE_VALUE, Vec::new()),
    )?;
    let mut body = Vec::new();
    loop {
        let packet = read_packet(&mut stream)?;
        match packet.id {
            END_ID => break,
            COMMAND_ID => body.extend(packet.body),
            _ => {}
        }
    }
    let reply: Value = serde_json::from_slice(&body).context("the console reply is not JSON")?;
    if let Some(message) = reply["error"].as_str() {
        bail!("{message}");
    }
    Ok(reply)
}

fn text(value: &Value) -> String {
    match value {
        Value::Null => "-".to_owned(),
        Value::String(text) => text.clone(),
        other => other.to_string(),
    }
}

fn elapsed(value: &Value) -> String {
    let Some(seconds) = value.as_u64() else {
        return "-".to_owned();
    };
    match seconds {
        0..60 => format!("{seconds}s"),
        60..3_600 => format!("{}m", seconds / 60),
        3_600..86_400 => format!("{}h {}m", seconds / 3_600, seconds % 3_600 / 60),
        _ => format!("{}d {}h", seconds / 86_400, seconds % 86_400 / 3_600),
    }
}

fn timestamp(value: &Value) -> String {
    let Some(seconds) = value.as_u64() else {
        return "-".to_owned();
    };
    let days = i64::try_from(seconds / 86_400).unwrap_or(0);
    let rest = seconds % 86_400;
    let shifted = days + 719_468;
    let era = shifted.div_euclid(146_097);
    let day_of_era = shifted.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_index + 2) / 5 + 1;
    let month = if month_index < 10 {
        month_index + 3
    } else {
        month_index - 9
    };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02} UTC",
        rest / 3_600,
        rest % 3_600 / 60
    )
}

fn list<'a>(reply: &'a Value, key: &str) -> &'a [Value] {
    reply[key].as_array().map_or(&[][..], Vec::as_slice)
}

fn kicked(reply: &Value) -> String {
    let kicked = &reply["kicked"];
    match kicked {
        Value::Object(_) => {
            let names = list(kicked, "nicknames")
                .iter()
                .map(text)
                .collect::<Vec<_>>()
                .join(", ");
            format!(
                "{} connection(s) closed {}",
                text(&kicked["sessions"]),
                names
            )
            .trim_end()
            .to_owned()
        }
        Value::Bool(true) => "disconnected".to_owned(),
        Value::String(nickname) => format!("{nickname} disconnected"),
        _ => "nobody disconnected".to_owned(),
    }
}

fn account_flags(account: &Value) -> String {
    let mut flags = Vec::new();
    if account["admin"] == true {
        flags.push("admin".to_owned());
    }
    if account["banned"] == true {
        let mut flag = format!(
            "banned ({})",
            account["ban_reason"].as_str().unwrap_or("no reason")
        );
        if !account["banned_until"].is_null() {
            let _ = write!(flag, " until {}", timestamp(&account["banned_until"]));
        }
        flags.push(flag);
    }
    flags.join(", ")
}

#[allow(clippy::too_many_lines, reason = "one arm per command")]
fn print_reply(command: &Command, reply: &Value) {
    match command {
        Command::Status => {
            if let Some(fields) = reply.as_object() {
                for (name, value) in fields {
                    let value = match name.as_str() {
                        "uptime_seconds" => elapsed(value),
                        "server_time" => timestamp(value),
                        _ => text(value),
                    };
                    println!("{name:<20} {value}");
                }
            }
        }
        Command::Players => {
            let players = list(reply, "players");
            println!("{} online", players.len());
            for player in players {
                println!(
                    "  {:<24} #{:<8} {:<16} channel {:<4} room {:<6} {:>8}{}",
                    text(&player["nickname"]),
                    text(&player["user_no"]),
                    text(&player["address"]),
                    text(&player["channel"]),
                    text(&player["room"]),
                    elapsed(&player["connected_seconds"]),
                    if player["admin"] == true {
                        "  admin"
                    } else {
                        ""
                    },
                );
            }
        }
        Command::Rooms => {
            let rooms = list(reply, "rooms");
            println!("{} rooms", rooms.len());
            for room in rooms {
                let names = |key: &str| {
                    list(room, key)
                        .iter()
                        .map(text)
                        .collect::<Vec<_>>()
                        .join(", ")
                };
                println!(
                    "  #{:<5} {:<24} {:<8} game {} speed {} track {} {}{}/{}  [{}]",
                    text(&room["id"]),
                    text(&room["name"]),
                    text(&room["phase"]),
                    text(&room["game_type"]),
                    text(&room["speed_type"]),
                    text(&room["track"]),
                    if room["locked"] == true {
                        "locked "
                    } else {
                        ""
                    },
                    list(room, "players").len(),
                    text(&room["capacity"]),
                    names("players"),
                );
                let observers = names("observers");
                if !observers.is_empty() {
                    println!("         observers: {observers}");
                }
            }
        }
        Command::Accounts => {
            let accounts = list(reply, "accounts");
            println!("{} accounts", accounts.len());
            for account in accounts {
                println!(
                    "  {:<24} {}",
                    text(&account["nickname"]),
                    account_flags(account)
                );
            }
        }
        Command::Info { .. } => {
            println!("{}  {}", text(&reply["nickname"]), account_flags(reply));
            println!("  created       {}", timestamp(&reply["created"]));
            println!(
                "  last login    {} from {}{}",
                timestamp(&reply["last_login"]),
                text(&reply["last_login_address"]),
                if reply["ticket_active"] == true {
                    " (ticket active)"
                } else {
                    ""
                },
            );
            let online = &reply["online"];
            if online.is_null() {
                println!("  offline");
            } else {
                println!(
                    "  online        {} from {}, channel {}, room {}",
                    elapsed(&online["connected_seconds"]),
                    text(&online["address"]),
                    text(&online["channel"]),
                    text(&online["room"]),
                );
            }
        }
        Command::Bans => {
            let addresses = list(reply, "addresses");
            println!("{} banned addresses", addresses.len());
            for ban in addresses {
                println!(
                    "  {:<40} {}  {}",
                    text(&ban["address"]),
                    timestamp(&ban["created"]),
                    ban["reason"].as_str().unwrap_or("")
                );
            }
        }
        Command::Kick { .. }
        | Command::KickIp { .. }
        | Command::KickRoom { .. }
        | Command::KickAll { .. } => println!("{}", kicked(reply)),
        Command::Ban { .. } => {
            let until = if reply["banned_until"].is_null() {
                String::new()
            } else {
                format!(" until {}", timestamp(&reply["banned_until"]))
            };
            println!(
                "{} banned{until}; {}",
                text(&reply["nickname"]),
                kicked(reply)
            );
        }
        Command::BanIp { .. } => {
            println!("{} banned; {}", text(&reply["address"]), kicked(reply));
        }
        Command::UnbanIp { .. } => println!(
            "{} {}",
            text(&reply["address"]),
            if reply["changed"] == true {
                "unbanned"
            } else {
                "was not banned"
            }
        ),
        Command::Delete { .. } => {
            println!("{} deleted; {}", text(&reply["nickname"]), kicked(reply));
        }
        Command::Maintenance { .. } => {
            let on = reply["maintenance"] == true;
            println!("maintenance {}", if on { "on" } else { "off" });
            if on && !reply["kicked"].is_null() {
                println!("{}", kicked(reply));
            }
        }
        Command::Rewards => {
            let outstanding = list(reply, "outstanding");
            println!("{} reward saves in progress", outstanding.len());
            for lane in outstanding {
                println!(
                    "  room {:<6} race {:<8} {:<24} {}",
                    text(&lane["room"]),
                    text(&lane["race_epoch"]),
                    text(&lane["nickname"]),
                    text(&lane["phase"]),
                );
            }
            let dead_letters = list(reply, "dead_letters");
            println!("{} failed reward saves", dead_letters.len());
            for dead_letter in dead_letters {
                println!(
                    "  room {:<6} race {:<8} {:<24} {}{}",
                    text(&dead_letter["room"]),
                    text(&dead_letter["race_epoch"]),
                    text(&dead_letter["nickname"]),
                    text(&dead_letter["reason"]),
                    if dead_letter["retryable"] == true {
                        "  (retryable)"
                    } else {
                        ""
                    },
                );
            }
        }
        Command::RetryReward { .. } => {
            for race in list(reply, "retried") {
                println!(
                    "retried room {} race {}",
                    text(&race["room"]),
                    text(&race["race_epoch"])
                );
            }
            for race in list(reply, "failed") {
                println!(
                    "failed  room {} race {}: {}",
                    text(&race["room"]),
                    text(&race["race_epoch"]),
                    text(&race["error"])
                );
            }
            if list(reply, "retried").is_empty() && list(reply, "failed").is_empty() {
                println!("nothing to retry");
            }
        }
        Command::Throttles => {
            for (key, label, count) in [
                ("launcher", "launcher", "failed_logins"),
                ("console", "console", "failed_tokens"),
            ] {
                let entries = list(reply, key);
                println!("{label}: {} address(es)", entries.len());
                for entry in entries {
                    let mut line = format!(
                        "  {:<40} {count} {}",
                        text(&entry["address"]),
                        text(&entry[count])
                    );
                    if !entry["registrations"].is_null() {
                        let _ = write!(line, " registrations {}", text(&entry["registrations"]));
                    }
                    if entry["limited"] == true {
                        line.push_str("  LIMITED");
                    }
                    println!("{line}");
                }
            }
        }
        Command::Audit { .. } => {
            for entry in list(reply, "entries") {
                println!(
                    "{}  {:<16} {:<16} {}",
                    timestamp(&entry["time"]),
                    text(&entry["peer"]),
                    text(&entry["action"]),
                    if entry["detail"].is_null() {
                        String::new()
                    } else {
                        entry["detail"].to_string()
                    },
                );
            }
        }
        _ => println!("{reply}"),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn builds_quoted_commands() {
        let command = Command::Ban {
            nickname: "라 이더".to_owned(),
            reason: Some("--spam".to_owned()),
            duration: Some(60),
        };
        assert_eq!(
            command_line(&command).unwrap(),
            r#"ban "라 이더" --reason "--spam" --duration 60"#
        );
        assert!(
            command_line(&Command::RetryReward {
                room: Some(1),
                race_epoch: None
            })
            .is_err()
        );
    }

    #[test]
    fn formats_times() {
        assert_eq!(timestamp(&json!(0)), "1970-01-01 00:00 UTC");
        assert_eq!(timestamp(&json!(1_791_504_000)), "2026-10-09 00:00 UTC");
        assert_eq!(elapsed(&json!(3_725)), "1h 2m");
    }
}
