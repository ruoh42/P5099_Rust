use crate::{DecodeError, cursor::Cursor};

const START_COLLECT_RECORD_REPLY_HASH: u32 = 0x52A4_07F5;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StartCollectRecordAction {
    pub stored_flag: u8,
    pub collector_gate_argument: bool,
}

impl StartCollectRecordAction {
    #[must_use]
    pub const fn flag_is_nonzero(self) -> bool {
        self.stored_flag != 0
    }
}

pub fn decode_start_collect_record_reply(
    packet: &[u8],
) -> Result<StartCollectRecordAction, DecodeError> {
    let mut reader = Cursor::new(packet);
    reader.expect_hash("PrStartCollectRecord", START_COLLECT_RECORD_REPLY_HASH)?;
    let stored_flag = reader.u8()?;
    reader.finish()?;
    Ok(StartCollectRecordAction {
        stored_flag,
        collector_gate_argument: stored_flag == 0,
    })
}
