use thiserror::Error;

pub const SERVERDATA_AUTH: i32 = 3;
pub const SERVERDATA_AUTH_RESPONSE: i32 = 2;
pub const SERVERDATA_EXECCOMMAND: i32 = 2;
pub const SERVERDATA_RESPONSE_VALUE: i32 = 0;
pub const AUTH_FAILED_ID: i32 = -1;

pub const MIN_PACKET_SIZE: usize = 10;
pub const MAX_PACKET_SIZE: usize = 4096;
pub const MAX_BODY_BYTES: usize = MAX_PACKET_SIZE - MIN_PACKET_SIZE;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum RconError {
    #[error("packet size {0} is out of range")]
    Size(i32),
    #[error("packet is not NUL-terminated")]
    Unterminated,
    #[error("unterminated quote")]
    UnterminatedQuote,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Packet {
    pub id: i32,
    pub kind: i32,
    pub body: Vec<u8>,
}

impl Packet {
    #[must_use]
    pub fn new(id: i32, kind: i32, body: impl Into<Vec<u8>>) -> Self {
        Self {
            id,
            kind,
            body: body.into(),
        }
    }

    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let size = i32::try_from(self.body.len() + MIN_PACKET_SIZE).unwrap_or(i32::MAX);
        let mut bytes = Vec::with_capacity(self.body.len() + MIN_PACKET_SIZE + 4);
        bytes.extend_from_slice(&size.to_le_bytes());
        bytes.extend_from_slice(&self.id.to_le_bytes());
        bytes.extend_from_slice(&self.kind.to_le_bytes());
        bytes.extend_from_slice(&self.body);
        bytes.extend_from_slice(&[0, 0]);
        bytes
    }

    pub fn decode(payload: &[u8]) -> Result<Self, RconError> {
        if payload.len() < MIN_PACKET_SIZE - 1 || payload.last() != Some(&0) {
            return Err(RconError::Unterminated);
        }
        let id = i32::from_le_bytes([payload[0], payload[1], payload[2], payload[3]]);
        let kind = i32::from_le_bytes([payload[4], payload[5], payload[6], payload[7]]);
        let mut body = &payload[8..];
        for _ in 0..2 {
            if let Some(rest) = body.strip_suffix(&[0]) {
                body = rest;
            }
        }
        Ok(Self::new(id, kind, body))
    }
}

pub fn payload_len(size_field: [u8; 4]) -> Result<usize, RconError> {
    let size = i32::from_le_bytes(size_field);
    usize::try_from(size)
        .ok()
        .filter(|size| (MIN_PACKET_SIZE - 1..=MAX_PACKET_SIZE).contains(size))
        .ok_or(RconError::Size(size))
}

#[must_use]
pub fn response_packets(id: i32, text: &str) -> Vec<Packet> {
    let mut packets = Vec::new();
    let mut rest = text;
    loop {
        let mut end = rest.len().min(MAX_BODY_BYTES);
        while !rest.is_char_boundary(end) {
            end -= 1;
        }
        let (chunk, tail) = rest.split_at(end);
        packets.push(Packet::new(id, SERVERDATA_RESPONSE_VALUE, chunk));
        if tail.is_empty() {
            return packets;
        }
        rest = tail;
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Word {
    pub text: String,
    pub quoted: bool,
}

pub fn split_command(line: &str) -> Result<Vec<Word>, RconError> {
    let mut words = Vec::new();
    let mut current: Option<Word> = None;
    let mut characters = line.chars();
    while let Some(character) = characters.next() {
        match character {
            '"' | '\'' => {
                let word = current.get_or_insert_with(|| Word {
                    text: String::new(),
                    quoted: false,
                });
                word.quoted = true;
                loop {
                    match characters.next() {
                        None => return Err(RconError::UnterminatedQuote),
                        Some(close) if close == character => break,
                        Some('\\') if character == '"' => {
                            word.text
                                .push(characters.next().ok_or(RconError::UnterminatedQuote)?);
                        }
                        Some(other) => word.text.push(other),
                    }
                }
            }
            '\\' => {
                let word = current.get_or_insert_with(|| Word {
                    text: String::new(),
                    quoted: false,
                });
                if let Some(escaped) = characters.next() {
                    word.text.push(escaped);
                }
            }
            whitespace if whitespace.is_whitespace() => words.extend(current.take()),
            other => current
                .get_or_insert_with(|| Word {
                    text: String::new(),
                    quoted: false,
                })
                .text
                .push(other),
        }
    }
    words.extend(current);
    Ok(words)
}

#[must_use]
pub fn quote(word: &str) -> String {
    let plain = !word.is_empty()
        && !word.starts_with('-')
        && !word
            .chars()
            .any(|character| character.is_whitespace() || matches!(character, '"' | '\'' | '\\'));
    if plain {
        return word.to_owned();
    }
    let mut quoted = String::with_capacity(word.len() + 2);
    quoted.push('"');
    for character in word.chars() {
        if matches!(character, '"' | '\\') {
            quoted.push('\\');
        }
        quoted.push(character);
    }
    quoted.push('"');
    quoted
}

pub fn parse_duration(text: &str) -> Result<u64, String> {
    let text = text.trim();
    let split = text
        .find(|character: char| !character.is_ascii_digit())
        .unwrap_or(text.len());
    let (number, unit) = text.split_at(split);
    let number: u64 = number
        .parse()
        .map_err(|_| format!("`{text}` is not a duration like 30m, 12h or 7d"))?;
    let scale = match unit {
        "" | "s" => 1,
        "m" => 60,
        "h" => 60 * 60,
        "d" => 24 * 60 * 60,
        _ => return Err(format!("unknown unit `{unit}`; use s, m, h or d")),
    };
    number
        .checked_mul(scale)
        .filter(|seconds| *seconds > 0)
        .ok_or_else(|| format!("`{text}` is out of range"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texts(line: &str) -> Vec<String> {
        split_command(line)
            .unwrap()
            .into_iter()
            .map(|word| word.text)
            .collect()
    }

    #[test]
    fn packets_round_trip() {
        let packet = Packet::new(7, SERVERDATA_EXECCOMMAND, "status");
        let bytes = packet.encode();
        assert_eq!(&bytes[..4], &16_i32.to_le_bytes());
        let size = payload_len(bytes[..4].try_into().unwrap()).unwrap();
        assert_eq!(size, bytes.len() - 4);
        assert_eq!(Packet::decode(&bytes[4..]).unwrap(), packet);
        assert_eq!(Packet::decode(&bytes[4..bytes.len() - 1]).unwrap(), packet);
        assert!(Packet::decode(&[0; 8]).is_err());
        assert!(payload_len(5_i32.to_le_bytes()).is_err());
        assert!(payload_len(4097_i32.to_le_bytes()).is_err());
        assert!(payload_len((-1_i32).to_le_bytes()).is_err());
    }

    #[test]
    fn long_replies_split_on_characters() {
        let text = "가".repeat(3_000);
        let packets = response_packets(4, &text);
        assert_eq!(packets.len(), 3);
        assert!(
            packets
                .iter()
                .all(|packet| packet.body.len() <= MAX_BODY_BYTES)
        );
        let joined: Vec<u8> = packets.into_iter().flat_map(|packet| packet.body).collect();
        assert_eq!(joined, text.as_bytes());
        assert_eq!(response_packets(1, "").len(), 1);
    }

    #[test]
    fn splits_and_quotes_words() {
        assert_eq!(
            texts(r#"ban "라 이더" --reason 'a "b"' x\ y"#),
            ["ban", "라 이더", "--reason", r#"a "b""#, "x y"]
        );
        assert_eq!(texts("  "), Vec::<String>::new());
        assert_eq!(texts(r#""""#), [""]);
        assert!(split_command(r#"ban "x"#).is_err());
        for word in ["plain", "", "two words", r#"q"u\o'te"#, "--flag", "라이더"] {
            let words = split_command(&format!("cmd {}", quote(word))).unwrap();
            assert_eq!(words[1].text, word);
        }
        assert!(split_command(&quote("--secret")).unwrap()[0].quoted);
    }

    #[test]
    fn parses_durations() {
        assert_eq!(parse_duration("90"), Ok(90));
        assert_eq!(parse_duration("30m"), Ok(1_800));
        assert_eq!(parse_duration("12h"), Ok(43_200));
        assert_eq!(parse_duration("7d"), Ok(604_800));
        assert!(parse_duration("0").is_err());
        assert!(parse_duration("5w").is_err());
        assert!(parse_duration("h").is_err());
    }
}
