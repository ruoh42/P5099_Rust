use std::{
    collections::BTreeMap,
    io::Write as _,
    path::{Path, PathBuf},
};

use flate2::{Compression, write::ZlibEncoder};
use p5099_core::{adler32, startup};
use p5099_rho5::{Rho5Directory, Rho5Error, Rho5Limits};
use quick_xml::{Reader, events::Event};
use thiserror::Error;

const CLIENT_CHANNEL_XML_PATH: &str = "zeta_/kr/content/channel.xml";

const BLOCK_MAGIC: u8 = b'S';
const BLOCK_FLAG_ZLIB: u8 = 0x01;
const BODY_LEAD_BYTE: u8 = 1;

const MAX_CHANNEL_XML_BYTES: usize = 1024 * 1024;
const MAX_TABLE_PLAINTEXT_BYTES: usize = 256 * 1024;
const MAX_CATEGORIES: usize = 512;
const MAX_CHANNELS: usize = 512;
const MAX_NAME_UTF16_UNITS: usize = 128;
const MAX_XML_EVENTS: usize = 100_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChannelRecord {
    pub channel_id: u16,
    pub name: String,
    pub category_id: u32,
    pub flag: u8,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChannelTable {
    categories: BTreeMap<u8, String>,
    channels: Vec<ChannelRecord>,
    body: Vec<u8>,
}

#[derive(Debug, Error)]
pub enum ChannelTableError {
    #[error("channel table {kind} count {actual} exceeds configured maximum {maximum}")]
    LimitExceeded {
        kind: &'static str,
        actual: usize,
        maximum: usize,
    },

    #[error("channel table ended at byte {offset}; needed {needed} more bytes")]
    Truncated { offset: usize, needed: usize },

    #[error("channel table block magic 0x{actual:02X} is not 'S'")]
    InvalidBlockMagic { actual: u8 },

    #[error("channel table block flags 0x{actual:02X} are not the expected zlib encoding")]
    UnsupportedBlockFlags { actual: u8 },

    #[error("channel table checksum mismatch: expected 0x{expected:08X}, got 0x{actual:08X}")]
    ChecksumMismatch { expected: u32, actual: u32 },

    #[error("channel table name is not valid UTF-16")]
    InvalidName,

    #[error("zlib codec failure for the channel table")]
    Zlib(#[source] std::io::Error),

    #[error("failed to read {path}")]
    ReadClientChannels {
        path: PathBuf,
        #[source]
        source: Box<Rho5Error>,
    },

    #[error("the client archives do not contain {0}")]
    MissingClientChannels(&'static str),

    #[error("{path} is {actual} bytes, above the {maximum}-byte bound")]
    ClientChannelsTooLarge {
        path: &'static str,
        actual: usize,
        maximum: usize,
    },

    #[error("{0} is not valid UTF-16")]
    ClientChannelsEncoding(&'static str),

    #[error("invalid XML in {path}")]
    ClientChannelsXml {
        path: &'static str,
        #[source]
        source: Box<quick_xml::Error>,
    },

    #[error("{path} exceeds the bounded {maximum} XML event budget")]
    ClientChannelsTooComplex { path: &'static str, maximum: usize },

    #[error("{path} declares no <Channel> entries")]
    NoClientChannels { path: &'static str },

    #[error("client channel {name:?} has no category in the advertised channel table")]
    UnknownCategory { name: String },
}

impl ChannelTable {
    pub fn stock() -> Result<Self, ChannelTableError> {
        Self::decode(startup::channel_static_reply_body())
    }

    pub fn from_client_data(data_directory: &Path) -> Result<Self, ChannelTableError> {
        let stock = Self::stock()?;
        let names = read_client_channel_names(data_directory)?;
        stock.with_client_channels(&names)
    }

    fn with_client_channels(&self, names: &[String]) -> Result<Self, ChannelTableError> {
        enforce_limit("channel", names.len(), MAX_CHANNELS)?;
        let by_name = self
            .categories
            .iter()
            .map(|(id, name)| (name.as_str(), u32::from(*id)))
            .collect::<BTreeMap<_, _>>();

        let mut channels = Vec::with_capacity(names.len());
        for (index, name) in names.iter().enumerate() {
            let category_id = *by_name
                .get(name.as_str())
                .ok_or_else(|| ChannelTableError::UnknownCategory { name: name.clone() })?;
            let channel_id =
                u16::try_from(index + 1).map_err(|_| ChannelTableError::LimitExceeded {
                    kind: "channel",
                    actual: index + 1,
                    maximum: MAX_CHANNELS,
                })?;
            channels.push(ChannelRecord {
                channel_id,
                name: name.clone(),
                category_id,
                flag: 0,
            });
        }

        let body = encode_body(&self.categories, &channels)?;
        Ok(Self {
            categories: self.categories.clone(),
            channels,
            body,
        })
    }

    #[must_use]
    pub fn body(&self) -> &[u8] {
        &self.body
    }

    #[must_use]
    pub fn channels(&self) -> &[ChannelRecord] {
        &self.channels
    }

    #[must_use]
    pub fn resolve_channel_id(
        &self,
        requested_category: u8,
        preferred_channel_id: u16,
    ) -> Option<u16> {
        let category = u32::from(requested_category);
        let mut matching = self
            .channels
            .iter()
            .filter(|channel| channel.category_id == category);
        let first = matching.clone().next()?;
        if preferred_channel_id != 0
            && matching.any(|channel| channel.channel_id == preferred_channel_id)
        {
            return Some(preferred_channel_id);
        }
        Some(first.channel_id)
    }

    fn decode(body: &[u8]) -> Result<Self, ChannelTableError> {
        let mut reader = ByteReader::new(body);
        let _lead = reader.u8()?;
        let block_length = usize::try_from(reader.u32()?).unwrap_or(usize::MAX);
        let block = reader.take(block_length)?;

        let mut block_reader = ByteReader::new(block);
        let magic = block_reader.u8()?;
        if magic != BLOCK_MAGIC {
            return Err(ChannelTableError::InvalidBlockMagic { actual: magic });
        }
        let flags = block_reader.u8()?;
        if flags != BLOCK_FLAG_ZLIB {
            return Err(ChannelTableError::UnsupportedBlockFlags { actual: flags });
        }
        let expected_checksum = block_reader.u32()?;
        let declared = usize::try_from(block_reader.u32()?).unwrap_or(usize::MAX);
        enforce_limit("plaintext byte", declared, MAX_TABLE_PLAINTEXT_BYTES)?;

        let plaintext = inflate(block_reader.remaining(), declared)?;
        let actual_checksum = adler32::hash(&plaintext, 0);
        if actual_checksum != expected_checksum {
            return Err(ChannelTableError::ChecksumMismatch {
                expected: expected_checksum,
                actual: actual_checksum,
            });
        }

        let (categories, channels) = decode_plaintext(&plaintext)?;
        Ok(Self {
            categories,
            channels,
            body: body.to_vec(),
        })
    }
}

fn decode_plaintext(
    plaintext: &[u8],
) -> Result<(BTreeMap<u8, String>, Vec<ChannelRecord>), ChannelTableError> {
    let mut reader = ByteReader::new(plaintext);
    let category_count = usize::try_from(reader.u32()?).unwrap_or(usize::MAX);
    enforce_limit("category", category_count, MAX_CATEGORIES)?;
    let mut categories = BTreeMap::new();
    for _ in 0..category_count {
        let id = reader.u8()?;
        categories.insert(id, reader.utf16()?);
    }

    let channel_count = usize::try_from(reader.u32()?).unwrap_or(usize::MAX);
    enforce_limit("channel", channel_count, MAX_CHANNELS)?;
    let mut channels = Vec::with_capacity(channel_count);
    for _ in 0..channel_count {
        channels.push(ChannelRecord {
            channel_id: reader.u16()?,
            name: reader.utf16()?,
            category_id: reader.u32()?,
            flag: reader.u8()?,
        });
    }
    Ok((categories, channels))
}

fn encode_body(
    categories: &BTreeMap<u8, String>,
    channels: &[ChannelRecord],
) -> Result<Vec<u8>, ChannelTableError> {
    let mut plaintext = Vec::new();
    write_u32(&mut plaintext, count_u32("category", categories.len())?);
    for (id, name) in categories {
        plaintext.push(*id);
        write_utf16(&mut plaintext, name)?;
    }
    write_u32(&mut plaintext, count_u32("channel", channels.len())?);
    for channel in channels {
        plaintext.extend_from_slice(&channel.channel_id.to_le_bytes());
        write_utf16(&mut plaintext, &channel.name)?;
        write_u32(&mut plaintext, channel.category_id);
        plaintext.push(channel.flag);
    }
    enforce_limit("plaintext byte", plaintext.len(), MAX_TABLE_PLAINTEXT_BYTES)?;

    let checksum = adler32::hash(&plaintext, 0);
    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::best());
    encoder
        .write_all(&plaintext)
        .map_err(ChannelTableError::Zlib)?;
    let compressed = encoder.finish().map_err(ChannelTableError::Zlib)?;

    let mut block = Vec::with_capacity(compressed.len() + 10);
    block.push(BLOCK_MAGIC);
    block.push(BLOCK_FLAG_ZLIB);
    block.extend_from_slice(&checksum.to_le_bytes());
    write_u32(&mut block, count_u32("plaintext byte", plaintext.len())?);
    block.extend_from_slice(&compressed);

    let mut body = Vec::with_capacity(block.len() + 5);
    body.push(BODY_LEAD_BYTE);
    write_u32(&mut body, count_u32("block byte", block.len())?);
    body.extend_from_slice(&block);
    Ok(body)
}

fn read_client_channel_names(data_directory: &Path) -> Result<Vec<String>, ChannelTableError> {
    let directory =
        Rho5Directory::scan_kr(data_directory, Rho5Limits::default()).map_err(|source| {
            ChannelTableError::ReadClientChannels {
                path: data_directory.to_path_buf(),
                source: Box::new(source),
            }
        })?;
    let entry = directory
        .entries()
        .iter()
        .find(|entry| entry.normalized_path() == CLIENT_CHANNEL_XML_PATH)
        .ok_or(ChannelTableError::MissingClientChannels(
            CLIENT_CHANNEL_XML_PATH,
        ))?
        .clone();
    if entry.plaintext_size() > MAX_CHANNEL_XML_BYTES {
        return Err(ChannelTableError::ClientChannelsTooLarge {
            path: CLIENT_CHANNEL_XML_PATH,
            actual: entry.plaintext_size(),
            maximum: MAX_CHANNEL_XML_BYTES,
        });
    }
    let bytes = directory
        .extract_entry_with_legacy_padding(&entry)
        .map_err(|source| ChannelTableError::ReadClientChannels {
            path: data_directory.join(CLIENT_CHANNEL_XML_PATH),
            source: Box::new(source),
        })?;
    parse_client_channel_names(&bytes)
}

fn parse_client_channel_names(bytes: &[u8]) -> Result<Vec<String>, ChannelTableError> {
    let text = decode_utf16(bytes)?;
    let mut reader = Reader::from_str(&text);
    reader.config_mut().trim_text(true);
    let mut names = Vec::new();
    let mut events = 0_usize;
    loop {
        events += 1;
        if events > MAX_XML_EVENTS {
            return Err(ChannelTableError::ClientChannelsTooComplex {
                path: CLIENT_CHANNEL_XML_PATH,
                maximum: MAX_XML_EVENTS,
            });
        }
        let event = reader
            .read_event()
            .map_err(|source| ChannelTableError::ClientChannelsXml {
                path: CLIENT_CHANNEL_XML_PATH,
                source: Box::new(source),
            })?;
        match event {
            Event::Eof => break,
            Event::Start(element) | Event::Empty(element) => {
                if element.name().as_ref() != b"Channel" {
                    continue;
                }
                for attribute in element.attributes().flatten() {
                    if attribute.key.as_ref() != b"name" {
                        continue;
                    }
                    let value = String::from_utf8_lossy(attribute.value.as_ref()).into_owned();
                    if !value.is_empty()
                        && value.encode_utf16().count() <= MAX_NAME_UTF16_UNITS
                        && !names.contains(&value)
                    {
                        names.push(value);
                    }
                }
            }
            _ => {}
        }
    }
    if names.is_empty() {
        return Err(ChannelTableError::NoClientChannels {
            path: CLIENT_CHANNEL_XML_PATH,
        });
    }
    enforce_limit("channel", names.len(), MAX_CHANNELS)?;
    Ok(names)
}

fn decode_utf16(bytes: &[u8]) -> Result<String, ChannelTableError> {
    let body = bytes.strip_prefix(&[0xFF, 0xFE]).unwrap_or(bytes);
    if !body.len().is_multiple_of(2) {
        return Err(ChannelTableError::ClientChannelsEncoding(
            CLIENT_CHANNEL_XML_PATH,
        ));
    }
    let units = body
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
        .collect::<Vec<_>>();
    String::from_utf16(&units)
        .map_err(|_| ChannelTableError::ClientChannelsEncoding(CLIENT_CHANNEL_XML_PATH))
}

fn inflate(compressed: &[u8], expected: usize) -> Result<Vec<u8>, ChannelTableError> {
    use std::io::Read as _;

    let mut decoder = flate2::read::ZlibDecoder::new(compressed)
        .take(u64::try_from(expected).unwrap_or(u64::MAX));
    let mut plaintext = Vec::with_capacity(expected);
    decoder
        .read_to_end(&mut plaintext)
        .map_err(ChannelTableError::Zlib)?;
    Ok(plaintext)
}

fn count_u32(kind: &'static str, value: usize) -> Result<u32, ChannelTableError> {
    u32::try_from(value).map_err(|_| ChannelTableError::LimitExceeded {
        kind,
        actual: value,
        maximum: usize::try_from(u32::MAX).unwrap_or(usize::MAX),
    })
}

fn enforce_limit(
    kind: &'static str,
    actual: usize,
    maximum: usize,
) -> Result<(), ChannelTableError> {
    if actual > maximum {
        Err(ChannelTableError::LimitExceeded {
            kind,
            actual,
            maximum,
        })
    } else {
        Ok(())
    }
}

fn write_u32(buffer: &mut Vec<u8>, value: u32) {
    buffer.extend_from_slice(&value.to_le_bytes());
}

fn write_utf16(buffer: &mut Vec<u8>, value: &str) -> Result<(), ChannelTableError> {
    let units = value.encode_utf16().collect::<Vec<_>>();
    enforce_limit("name code unit", units.len(), MAX_NAME_UTF16_UNITS)?;
    write_u32(buffer, count_u32("name code unit", units.len())?);
    for unit in units {
        buffer.extend_from_slice(&unit.to_le_bytes());
    }
    Ok(())
}

struct ByteReader<'a> {
    input: &'a [u8],
    offset: usize,
}

impl<'a> ByteReader<'a> {
    const fn new(input: &'a [u8]) -> Self {
        Self { input, offset: 0 }
    }

    fn take(&mut self, length: usize) -> Result<&'a [u8], ChannelTableError> {
        let end = self
            .offset
            .checked_add(length)
            .ok_or(ChannelTableError::Truncated {
                offset: self.offset,
                needed: length,
            })?;
        if end > self.input.len() {
            return Err(ChannelTableError::Truncated {
                offset: self.offset,
                needed: end - self.input.len(),
            });
        }
        let slice = &self.input[self.offset..end];
        self.offset = end;
        Ok(slice)
    }

    fn u8(&mut self) -> Result<u8, ChannelTableError> {
        Ok(self.take(1)?[0])
    }

    fn u16(&mut self) -> Result<u16, ChannelTableError> {
        let bytes = self.take(2)?;
        Ok(u16::from_le_bytes([bytes[0], bytes[1]]))
    }

    fn u32(&mut self) -> Result<u32, ChannelTableError> {
        let bytes = self.take(4)?;
        Ok(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }

    fn utf16(&mut self) -> Result<String, ChannelTableError> {
        let length = usize::try_from(self.u32()?).unwrap_or(usize::MAX);
        enforce_limit("name code unit", length, MAX_NAME_UTF16_UNITS)?;
        let bytes = self.take(length * 2)?;
        let units = bytes
            .as_chunks::<2>()
            .0
            .iter()
            .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
            .collect::<Vec<_>>();
        String::from_utf16(&units).map_err(|_| ChannelTableError::InvalidName)
    }

    const fn remaining(&self) -> &'a [u8] {
        self.input.split_at(self.offset).1
    }
}

#[cfg(test)]
mod tests {
    use super::{ChannelTable, ChannelTableError, parse_client_channel_names};

    fn utf16_xml(body: &str) -> Vec<u8> {
        let mut bytes = vec![0xFF, 0xFE];
        for unit in body.encode_utf16() {
            bytes.extend_from_slice(&unit.to_le_bytes());
        }
        bytes
    }

    #[test]
    fn stock_table_round_trips_its_captured_envelope() {
        let stock = ChannelTable::stock().expect("stock channel table decodes");
        assert_eq!(stock.channels().len(), 24);
        assert!(
            stock
                .channels()
                .iter()
                .any(|channel| channel.name == "speedIndiInfinitA")
        );
        assert_eq!(stock.resolve_channel_id(23, 0), Some(13));
        assert_eq!(stock.resolve_channel_id(23, 15), Some(15));
        assert_eq!(stock.resolve_channel_id(0, 0), None);
    }

    #[test]
    fn client_channels_replace_the_lettered_sub_channels() {
        let stock = ChannelTable::stock().unwrap();
        let names = ["battle", "speedIndiInfinit", "speedTeamInfinit"]
            .map(str::to_owned)
            .to_vec();
        let rebuilt = stock.with_client_channels(&names).unwrap();

        assert_eq!(rebuilt.channels().len(), 3);
        assert_eq!(rebuilt.channels()[0].name, "battle");
        assert_eq!(rebuilt.channels()[0].channel_id, 1);
        assert_eq!(rebuilt.channels()[0].category_id, 20);
        assert_eq!(rebuilt.channels()[1].category_id, 23);
        assert_eq!(rebuilt.resolve_channel_id(23, 0), Some(2));
        assert_eq!(rebuilt.resolve_channel_id(24, 0), Some(3));
        assert_eq!(rebuilt.resolve_channel_id(67, 0), None);

        let decoded = ChannelTable::decode(rebuilt.body()).unwrap();
        assert_eq!(decoded.channels(), rebuilt.channels());
    }

    #[test]
    fn a_client_channel_without_a_category_is_rejected() {
        let stock = ChannelTable::stock().unwrap();
        let error = stock
            .with_client_channels(&["notAChannelCategory".to_owned()])
            .unwrap_err();
        assert!(matches!(
            error,
            ChannelTableError::UnknownCategory { ref name } if name == "notAChannelCategory"
        ));
    }

    #[test]
    fn channel_names_are_read_in_document_order_without_duplicates() {
        let xml = utf16_xml(
            "<?xml version='1.0' encoding='UTF-16'?>\
             <Class><Multiplay>\
             <Channel name='battle' gameType='17' createSpeed='1'/>\
             <Channel name='speedIndiInfinit' gameType='1' createSpeed='4'/>\
             <Channel name='battle' gameType='17' createSpeed='1'/>\
             </Multiplay></Class>",
        );
        assert_eq!(
            parse_client_channel_names(&xml).unwrap(),
            vec!["battle".to_owned(), "speedIndiInfinit".to_owned()]
        );
    }

    #[test]
    fn a_definition_without_channels_is_rejected() {
        let xml = utf16_xml("<Class><Multiplay/></Class>");
        assert!(matches!(
            parse_client_channel_names(&xml).unwrap_err(),
            ChannelTableError::NoClientChannels { .. }
        ));
    }
}
