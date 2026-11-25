use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::nickname::canonical_nickname_key;

pub const ACCOUNT_KDF_ITERATIONS: u32 = 100_000;
pub const MAX_ACCOUNT_LINE_BYTES: usize = 512;
pub const MIN_PASSWORD_CHARS: usize = 8;
pub const MAX_PASSWORD_CHARS: usize = 64;
pub const ACCOUNT_KEY_BYTES: usize = 32;
pub const ACCOUNT_NONCE_BYTES: usize = 32;
pub const LAUNCH_TICKET_BYTES: usize = 32;

const SALT_CONTEXT: &[u8] = b"P5099-account-salt-v1\0";
const PROOF_CONTEXT: &[u8] = b"P5099-login-proof-v1\0";
const SHA256_BLOCK: usize = 64;

pub type AccountKey = [u8; ACCOUNT_KEY_BYTES];

#[must_use]
pub fn hmac_sha256(key: &[u8], message: &[u8]) -> [u8; 32] {
    let mut block = [0_u8; SHA256_BLOCK];
    if key.len() > SHA256_BLOCK {
        block[..32].copy_from_slice(&Sha256::digest(key));
    } else {
        block[..key.len()].copy_from_slice(key);
    }
    let mut inner = Sha256::new();
    inner.update(block.map(|byte| byte ^ 0x36));
    inner.update(message);
    let inner = inner.finalize();
    let mut outer = Sha256::new();
    outer.update(block.map(|byte| byte ^ 0x5C));
    outer.update(inner);
    outer.finalize().into()
}

#[must_use]
pub fn pbkdf2_sha256(password: &[u8], salt: &[u8], iterations: u32) -> [u8; 32] {
    let mut first = Vec::with_capacity(salt.len() + 4);
    first.extend_from_slice(salt);
    first.extend_from_slice(&1_u32.to_be_bytes());
    let mut block = hmac_sha256(password, &first);
    let mut output = block;
    for _ in 1..iterations {
        block = hmac_sha256(password, &block);
        for (out, byte) in output.iter_mut().zip(block) {
            *out ^= byte;
        }
    }
    output
}

#[must_use]
pub fn account_salt(nickname: &str) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(SALT_CONTEXT);
    hasher.update(canonical_nickname_key(nickname).as_bytes());
    hasher.finalize().into()
}

#[must_use]
pub fn password_key(nickname: &str, password: &str) -> AccountKey {
    pbkdf2_sha256(
        password.as_bytes(),
        &account_salt(nickname),
        ACCOUNT_KDF_ITERATIONS,
    )
}

#[must_use]
pub fn login_proof(key: &AccountKey, nonce: &[u8]) -> [u8; 32] {
    let mut message = Vec::with_capacity(PROOF_CONTEXT.len() + nonce.len());
    message.extend_from_slice(PROOF_CONTEXT);
    message.extend_from_slice(nonce);
    hmac_sha256(key, &message)
}

#[must_use]
pub fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    left.len() == right.len()
        && left
            .iter()
            .zip(right)
            .fold(0_u8, |difference, (a, b)| difference | (a ^ b))
            == 0
}

#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum PasswordError {
    #[error("password must be at least {MIN_PASSWORD_CHARS} characters")]
    TooShort,
    #[error("password must be at most {MAX_PASSWORD_CHARS} characters")]
    TooLong,
}

pub fn validate_password(password: &str) -> Result<(), PasswordError> {
    let length = password.chars().count();
    if length < MIN_PASSWORD_CHARS {
        Err(PasswordError::TooShort)
    } else if length > MAX_PASSWORD_CHARS {
        Err(PasswordError::TooLong)
    } else {
        Ok(())
    }
}

#[must_use]
pub fn to_hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut text = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        text.push(char::from(DIGITS[usize::from(byte >> 4)]));
        text.push(char::from(DIGITS[usize::from(byte & 0x0F)]));
    }
    text
}

pub fn from_hex<const N: usize>(text: &str) -> Result<[u8; N], AccountProtocolError> {
    let bytes = text.as_bytes();
    if bytes.len() != N * 2 {
        return Err(AccountProtocolError::BadHex);
    }
    let mut output = [0_u8; N];
    for (out, pair) in output.iter_mut().zip(bytes.as_chunks::<2>().0) {
        let digit = |value: u8| match value {
            b'0'..=b'9' => Ok(value - b'0'),
            b'a'..=b'f' => Ok(value - b'a' + 10),
            b'A'..=b'F' => Ok(value - b'A' + 10),
            _ => Err(AccountProtocolError::BadHex),
        };
        *out = (digit(pair[0])? << 4) | digit(pair[1])?;
    }
    Ok(output)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccountFailure {
    Exists,
    InvalidNickname,
    BadCredentials,
    RateLimited,
    RegistrationClosed,
    BadRequest,
    Unavailable,
    Banned,
    Maintenance,
}

impl AccountFailure {
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::Exists => "exists",
            Self::InvalidNickname => "invalid_nickname",
            Self::BadCredentials => "bad_credentials",
            Self::RateLimited => "rate_limited",
            Self::RegistrationClosed => "registration_closed",
            Self::BadRequest => "bad_request",
            Self::Unavailable => "unavailable",
            Self::Banned => "banned",
            Self::Maintenance => "maintenance",
        }
    }

    #[must_use]
    pub fn from_code(code: &str) -> Self {
        match code {
            "exists" => Self::Exists,
            "invalid_nickname" => Self::InvalidNickname,
            "bad_credentials" => Self::BadCredentials,
            "rate_limited" => Self::RateLimited,
            "registration_closed" => Self::RegistrationClosed,
            "unavailable" => Self::Unavailable,
            "banned" => Self::Banned,
            "maintenance" => Self::Maintenance,
            _ => Self::BadRequest,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlayerPresence {
    Offline,
    Lobby,
    Room(RoomPresenceInfo),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoomPresenceInfo {
    pub name: String,
    pub game_type: u8,
    pub speed_type: u8,
    pub racing: bool,
    pub players: u8,
    pub capacity: u8,
}

impl RoomPresenceInfo {
    #[must_use]
    pub const fn mode_label(&self) -> &'static str {
        match (self.game_type, self.speed_type) {
            (1, 4) => "무한부스터 개인전",
            (3, 4) => "무한부스터 팀전",
            (1, _) => "스피드 개인전",
            (3, _) => "스피드 팀전",
            (2, _) => "아이템 개인전",
            (4, _) => "아이템 팀전",
            _ => "카트라이더",
        }
    }
}

fn presence_text(value: &str) -> String {
    value
        .chars()
        .map(|character| {
            if character.is_control() {
                ' '
            } else {
                character
            }
        })
        .collect()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AccountRequest {
    Register {
        nickname: String,
        key: AccountKey,
    },
    Challenge {
        nickname: String,
    },
    Login {
        nickname: String,
        proof: [u8; 32],
    },
    ChangePassword {
        nickname: String,
        proof: [u8; 32],
        new_key: AccountKey,
    },
    Presence {
        nickname: String,
        ticket: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AccountResponse {
    Registered { nickname: String },
    Nonce([u8; ACCOUNT_NONCE_BYTES]),
    Ticket { nickname: String, ticket: String },
    PasswordChanged,
    Presence(PlayerPresence),
    Failed(AccountFailure),
}

#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum AccountProtocolError {
    #[error("account line is empty or has the wrong number of fields")]
    Malformed,
    #[error("unknown account command")]
    UnknownCommand,
    #[error("account field is not the expected hexadecimal value")]
    BadHex,
}

impl AccountRequest {
    #[must_use]
    pub fn to_line(&self) -> String {
        match self {
            Self::Register { nickname, key } => format!("REGISTER\t{nickname}\t{}", to_hex(key)),
            Self::Challenge { nickname } => format!("CHALLENGE\t{nickname}"),
            Self::Login { nickname, proof } => format!("LOGIN\t{nickname}\t{}", to_hex(proof)),
            Self::ChangePassword {
                nickname,
                proof,
                new_key,
            } => format!(
                "PASSWORD\t{nickname}\t{}\t{}",
                to_hex(proof),
                to_hex(new_key)
            ),
            Self::Presence { nickname, ticket } => format!("PRESENCE\t{nickname}\t{ticket}"),
        }
    }

    pub fn parse(line: &str) -> Result<Self, AccountProtocolError> {
        let fields: Vec<&str> = line.split('\t').collect();
        let nickname = || (*fields.get(1).unwrap_or(&"")).to_owned();
        match (fields[0], fields.len()) {
            ("REGISTER", 3) => Ok(Self::Register {
                nickname: nickname(),
                key: from_hex(fields[2])?,
            }),
            ("CHALLENGE", 2) => Ok(Self::Challenge {
                nickname: nickname(),
            }),
            ("LOGIN", 3) => Ok(Self::Login {
                nickname: nickname(),
                proof: from_hex(fields[2])?,
            }),
            ("PASSWORD", 4) => Ok(Self::ChangePassword {
                nickname: nickname(),
                proof: from_hex(fields[2])?,
                new_key: from_hex(fields[3])?,
            }),
            ("PRESENCE", 3) => Ok(Self::Presence {
                nickname: nickname(),
                ticket: fields[2].to_owned(),
            }),
            ("REGISTER" | "CHALLENGE" | "LOGIN" | "PASSWORD" | "PRESENCE", _) => {
                Err(AccountProtocolError::Malformed)
            }
            _ => Err(AccountProtocolError::UnknownCommand),
        }
    }
}

impl AccountResponse {
    #[must_use]
    pub fn to_line(&self) -> String {
        match self {
            Self::Registered { nickname } => format!("REGISTERED\t{nickname}"),
            Self::Nonce(nonce) => format!("NONCE\t{}", to_hex(nonce)),
            Self::Ticket { nickname, ticket } => format!("TICKET\t{nickname}\t{ticket}"),
            Self::PasswordChanged => "PASSWORD_CHANGED".to_owned(),
            Self::Presence(PlayerPresence::Offline) => "PRESENCE\toffline".to_owned(),
            Self::Presence(PlayerPresence::Lobby) => "PRESENCE\tlobby".to_owned(),
            Self::Presence(PlayerPresence::Room(room)) => format!(
                "PRESENCE\troom\t{}\t{}\t{}\t{}\t{}\t{}",
                u8::from(room.racing),
                room.game_type,
                room.speed_type,
                room.players,
                room.capacity,
                presence_text(&room.name)
            ),
            Self::Failed(failure) => format!("ERROR\t{}", failure.code()),
        }
    }

    pub fn parse(line: &str) -> Result<Self, AccountProtocolError> {
        let fields: Vec<&str> = line.split('\t').collect();
        match (fields[0], fields.len()) {
            ("REGISTERED", 2) => Ok(Self::Registered {
                nickname: fields[1].to_owned(),
            }),
            ("NONCE", 2) => Ok(Self::Nonce(from_hex(fields[1])?)),
            ("TICKET", 3) => {
                from_hex::<LAUNCH_TICKET_BYTES>(fields[2])?;
                Ok(Self::Ticket {
                    nickname: fields[1].to_owned(),
                    ticket: fields[2].to_owned(),
                })
            }
            ("PASSWORD_CHANGED", 1) => Ok(Self::PasswordChanged),
            ("PRESENCE", 2) if fields[1] == "offline" => {
                Ok(Self::Presence(PlayerPresence::Offline))
            }
            ("PRESENCE", 2) if fields[1] == "lobby" => Ok(Self::Presence(PlayerPresence::Lobby)),
            ("PRESENCE", 8) if fields[1] == "room" => {
                let number = |index: usize| {
                    fields[index]
                        .parse::<u8>()
                        .map_err(|_| AccountProtocolError::Malformed)
                };
                Ok(Self::Presence(PlayerPresence::Room(RoomPresenceInfo {
                    racing: number(2)? != 0,
                    game_type: number(3)?,
                    speed_type: number(4)?,
                    players: number(5)?,
                    capacity: number(6)?,
                    name: fields[7].to_owned(),
                })))
            }
            ("ERROR", 2) => Ok(Self::Failed(AccountFailure::from_code(fields[1]))),
            ("REGISTERED" | "NONCE" | "TICKET" | "PASSWORD_CHANGED" | "PRESENCE" | "ERROR", _) => {
                Err(AccountProtocolError::Malformed)
            }
            _ => Err(AccountProtocolError::UnknownCommand),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hmac_matches_rfc_4231_case_2() {
        assert_eq!(
            to_hex(&hmac_sha256(b"Jefe", b"what do ya want for nothing?")),
            "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843"
        );
    }

    #[test]
    fn hmac_hashes_long_keys_first() {
        assert_eq!(
            to_hex(&hmac_sha256(
                &[0xAA; 131],
                b"Test Using Larger Than Block-Size Key - Hash Key First"
            )),
            "60e431591ee0b67f0d8a26aacbf5b77f8e0bc6213728c5140546040f0ee37f54"
        );
    }

    #[test]
    fn pbkdf2_matches_published_sha256_vectors() {
        assert_eq!(
            to_hex(&pbkdf2_sha256(b"password", b"salt", 1)),
            "120fb6cffcf8b32c43e7225256c4f837a86548c92ccc35480805987cb70be17b"
        );
        assert_eq!(
            to_hex(&pbkdf2_sha256(b"password", b"salt", 4096)),
            "c5e478d59288c841aa530db6845c4c8d962893a001ce4e11a4963873aa98134a"
        );
    }

    #[test]
    fn keys_ignore_nickname_case_but_not_password_case() {
        assert_eq!(
            password_key("Rider", "secret"),
            password_key("rIDER", "secret")
        );
        assert_ne!(
            password_key("Rider", "secret"),
            password_key("Rider", "Secret")
        );
        assert_ne!(
            password_key("Rider", "secret"),
            password_key("Other", "secret")
        );
    }

    #[test]
    fn requests_and_responses_round_trip() {
        let key = password_key("라이더", "비밀번호1");
        let requests = [
            AccountRequest::Register {
                nickname: "라이더".to_owned(),
                key,
            },
            AccountRequest::Challenge {
                nickname: "Rider One".to_owned(),
            },
            AccountRequest::Login {
                nickname: "Rider".to_owned(),
                proof: login_proof(&key, &[7; 32]),
            },
            AccountRequest::ChangePassword {
                nickname: "Rider".to_owned(),
                proof: [1; 32],
                new_key: [2; 32],
            },
            AccountRequest::Presence {
                nickname: "Rider".to_owned(),
                ticket: to_hex(&[3; 32]),
            },
        ];
        for request in requests {
            assert_eq!(AccountRequest::parse(&request.to_line()).unwrap(), request);
        }
        let responses = [
            AccountResponse::Registered {
                nickname: "Rider".to_owned(),
            },
            AccountResponse::Nonce([9; 32]),
            AccountResponse::Ticket {
                nickname: "Rider".to_owned(),
                ticket: to_hex(&[3; 32]),
            },
            AccountResponse::PasswordChanged,
            AccountResponse::Presence(PlayerPresence::Offline),
            AccountResponse::Presence(PlayerPresence::Lobby),
            AccountResponse::Presence(PlayerPresence::Room(RoomPresenceInfo {
                name: "초보만 S2".to_owned(),
                game_type: 4,
                speed_type: 7,
                racing: true,
                players: 6,
                capacity: 8,
            })),
            AccountResponse::Failed(AccountFailure::RateLimited),
        ];
        for response in responses {
            assert_eq!(
                AccountResponse::parse(&response.to_line()).unwrap(),
                response
            );
        }
    }

    #[test]
    fn room_names_cannot_break_the_line_and_modes_are_named() {
        let room = RoomPresenceInfo {
            name: "a\tb\nc".to_owned(),
            game_type: 1,
            speed_type: 4,
            racing: false,
            players: 1,
            capacity: 8,
        };
        assert_eq!(room.mode_label(), "무한부스터 개인전");
        let line = AccountResponse::Presence(PlayerPresence::Room(room)).to_line();
        assert!(!line.contains('\n'));
        let AccountResponse::Presence(PlayerPresence::Room(parsed)) =
            AccountResponse::parse(&line).unwrap()
        else {
            panic!("expected a room");
        };
        assert_eq!(parsed.name, "a b c");
    }

    #[test]
    fn malformed_lines_are_rejected() {
        assert_eq!(
            AccountRequest::parse("LOGIN\tRider"),
            Err(AccountProtocolError::Malformed)
        );
        assert_eq!(
            AccountRequest::parse("LOGIN\tRider\tzz"),
            Err(AccountProtocolError::BadHex)
        );
        assert_eq!(
            AccountRequest::parse("DROP\tall"),
            Err(AccountProtocolError::UnknownCommand)
        );
    }

    #[test]
    fn password_length_is_bounded() {
        assert_eq!(validate_password("abcdefg"), Err(PasswordError::TooShort));
        assert!(validate_password("abcdefgh").is_ok());
        assert_eq!(
            validate_password(&"x".repeat(65)),
            Err(PasswordError::TooLong)
        );
    }
}
