#![forbid(unsafe_code)]

mod cursor;
mod legacy_scalar;

pub mod ceremony;
pub mod club;
pub mod evidence;
pub mod final_stage_scheduler;
pub mod game_result;
pub mod item_client_fsm;
pub mod item_operation;
pub mod login;
pub mod protocol_fsm;
pub mod record_collection;
pub mod room;

pub use cursor::DecodeError;
