use crate::packet::{PacketError, PacketWriter};

pub const CLIENT_VERSION: u16 = 5_099;
pub const LOCALE_ID: u16 = 1_002;
pub const CLIENT_LOCATION: u8 = 118;
pub const FIRST_KEY: u32 = 2_919_676_295;
pub const SECOND_KEY: u32 = 263_300_380;
pub const FIRST_KEY_TEXT: &str = "QyvKvO60jogWDupzJ7gm0kRQdooFjWRjSjlq0gu/x2k=";
pub const SECOND_KEY_TEXT: &str = "GXQstj1A95XiHvjrOGuPkzdyL+7qxETl/cPlUZk2KA4=";
pub const PATCH_URL: &str = "http://kart.dn.nexoncdn.co.kr/patch";

pub const COMPATIBILITY_BLOCK: [u8; 31] = [
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x0f, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x2e, 0x31, 0x2e, 0x31, 0x37, 0x2e, 0x36, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
];

#[must_use]
pub const fn initial_iv() -> u32 {
    FIRST_KEY ^ SECOND_KEY
}

pub fn first_message_payload() -> Result<Vec<u8>, PacketError> {
    let mut packet = PacketWriter::named("PcFirstMessage");
    packet.write_u16(LOCALE_ID);
    packet.write_u16(1);
    packet.write_u16(CLIENT_VERSION);
    packet.write_utf16(PATCH_URL)?;
    packet.write_u32(FIRST_KEY);
    packet.write_u32(SECOND_KEY);
    packet.write_u8(CLIENT_LOCATION);
    packet.write_utf16(FIRST_KEY_TEXT)?;
    packet.write_bytes(&COMPATIBILITY_BLOCK);
    packet.write_utf16(SECOND_KEY_TEXT)?;
    Ok(packet.into_inner())
}

#[cfg(test)]
mod tests {
    use crate::{adler32, packet::PacketReader};
    use sha2::{Digest, Sha256};

    use super::{
        CLIENT_LOCATION, CLIENT_VERSION, COMPATIBILITY_BLOCK, FIRST_KEY, FIRST_KEY_TEXT, LOCALE_ID,
        PATCH_URL, SECOND_KEY, SECOND_KEY_TEXT, first_message_payload, initial_iv,
    };

    #[test]
    fn first_message_matches_the_csharp_field_layout() {
        let payload = first_message_payload().unwrap();
        assert_eq!(payload.len(), 308);
        assert_eq!(
            format!("{:X}", Sha256::digest(&payload)),
            "FAA785A13D8A99CEB6552C99223D2AB17130F14F685504C52794F91BBF0512D7"
        );

        let mut reader = PacketReader::new(&payload);
        assert_eq!(
            reader.read_u32().unwrap(),
            adler32::packet_hash("PcFirstMessage")
        );
        assert_eq!(reader.read_u16().unwrap(), LOCALE_ID);
        assert_eq!(reader.read_u16().unwrap(), 1);
        assert_eq!(reader.read_u16().unwrap(), CLIENT_VERSION);
        assert_eq!(reader.read_utf16().unwrap(), PATCH_URL);
        assert_eq!(reader.read_u32().unwrap(), FIRST_KEY);
        assert_eq!(reader.read_u32().unwrap(), SECOND_KEY);
        assert_eq!(reader.read_u8().unwrap(), CLIENT_LOCATION);
        assert_eq!(reader.read_utf16().unwrap(), FIRST_KEY_TEXT);
        assert_eq!(
            &reader.remaining()[..COMPATIBILITY_BLOCK.len()],
            &COMPATIBILITY_BLOCK
        );

        let rest = &reader.remaining()[COMPATIBILITY_BLOCK.len()..];
        let mut final_reader = PacketReader::new(rest);
        assert_eq!(final_reader.read_utf16().unwrap(), SECOND_KEY_TEXT);
        assert_eq!(final_reader.remaining(), []);
        assert_eq!(initial_iv(), 0xa1b7_1c9b);

        let wire = crate::frame::encode_plain(&payload, crate::frame::DEFAULT_MAX_PAYLOAD).unwrap();
        assert_eq!(wire.len(), 312);
        assert_eq!(
            format!("{:X}", Sha256::digest(&wire)),
            "D33BC87060397255DCBDA663D35DF585F9A2BD9EAC198933816C74CA1FF2BA68"
        );
    }
}
