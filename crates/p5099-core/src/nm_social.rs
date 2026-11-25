use crate::nm_protocol::{NM_FUNC_CLASS, NmError, NmReader, NmWriter, length_u32};

pub const GET_SOCIAL_FRIEND_LIST: u32 = 0x200E;
pub const GET_GAME_FRIEND_LIST: u32 = 0x200F;
pub const DELETE_SOCIAL_FRIEND: u32 = 0x2010;
pub const DELETE_GAME_FRIEND: u32 = 0x2011;
pub const GET_RECEIVED_FRIEND_REQUESTS: u32 = 0x2012;
pub const ACCEPT_FRIEND_REQUEST: u32 = 0x2013;
pub const REJECT_FRIEND_REQUEST: u32 = 0x2014;
pub const GET_SENT_FRIEND_REQUESTS: u32 = 0x2015;
pub const CANCEL_FRIEND_REQUEST: u32 = 0x2016;
pub const SEND_FRIEND_REQUEST: u32 = 0x2017;
pub const GET_BLOCKED_USER_LIST: u32 = 0x2018;

const PRESENCE_CLASS: u16 = 0x5330;
const REQUEST_DETAIL_CLASS: u16 = 0x5331;
const USER_CLASS: u16 = 0x5332;
const FRIEND_CLASS: u16 = 0x5333;
const REQUEST_CLASS: u16 = 0x5335;

pub const SOCIAL_OK: u32 = 0;
pub const SOCIAL_NOT_EXIST_USER: u32 = 1000;
pub const SOCIAL_ALREADY_FRIEND: u32 = 1004;
pub const SOCIAL_REQUEST_ERROR: u32 = 1005;
pub const SOCIAL_MAX_REQUEST_LIMIT: u32 = 1006;
pub const SOCIAL_ALREADY_REQUEST: u32 = 1013;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SocialUser {
    pub id: u64,
    pub nickname: String,
}

fn write_user(writer: &mut NmWriter, user: &SocialUser) {
    writer
        .begin(USER_CLASS, 1, 1)
        .u64(user.id)
        .wstr(&user.nickname)
        .wstr(&user.nickname)
        .wstr("")
        .end();
}

fn write_presence(writer: &mut NmWriter) {
    writer.begin(PRESENCE_CLASS, 1, 1).u32(0).wstr("").end();
}

fn write_friend(writer: &mut NmWriter, user: &SocialUser, online: bool) {
    writer.begin(FRIEND_CLASS, 1, 1);
    write_user(writer, user);
    write_presence(writer);
    writer.u8(u8::from(online)).u8(0).u32(0).end();
}

fn write_request(writer: &mut NmWriter, user: &SocialUser, message: &str) {
    writer.begin(REQUEST_CLASS, 1, 1);
    write_user(writer, user);
    writer.begin(REQUEST_DETAIL_CLASS, 1, 1);
    write_presence(writer);
    writer.wstr("").end();
    writer.wstr(message).end();
}

fn reply(request: &[u8], fields: impl FnOnce(&mut NmWriter)) -> Vec<u8> {
    let (version, flag) = NmReader::new(request)
        .header()
        .map_or((1, 1), |header| (header.version, header.flag));
    let mut writer = NmWriter::new();
    writer.begin(NM_FUNC_CLASS, version, flag).u32(1);
    fields(&mut writer);
    writer.end();
    writer.into_bytes()
}

#[must_use]
pub fn success_reply(request: &[u8]) -> Vec<u8> {
    reply(request, |_| {})
}

#[must_use]
pub fn friend_list_reply(request: &[u8], friends: &[(SocialUser, bool)]) -> Vec<u8> {
    reply(request, |writer| {
        writer.u32(length_u32(friends.len()));
        for (user, online) in friends {
            write_friend(writer, user, *online);
        }
    })
}

#[must_use]
pub fn request_list_reply(request: &[u8], requests: &[(SocialUser, String)]) -> Vec<u8> {
    reply(request, |writer| {
        writer.u32(length_u32(requests.len()));
        for (user, message) in requests {
            write_request(writer, user, message);
        }
    })
}

#[must_use]
pub fn empty_list_reply(request: &[u8]) -> Vec<u8> {
    reply(request, |writer| {
        writer.u32(0);
    })
}

#[must_use]
pub fn code_reply(request: &[u8], code: u32) -> Vec<u8> {
    reply(request, |writer| {
        writer.u32(code);
    })
}

fn request_fields(request: &[u8]) -> Result<NmReader<'_>, NmError> {
    let mut reader = NmReader::new(request);
    reader.header()?;
    reader.u32()?;
    Ok(reader)
}

pub fn parse_target_id(request: &[u8]) -> Result<u64, NmError> {
    request_fields(request)?.u64()
}

pub fn parse_send_request(request: &[u8]) -> Result<(u64, String), NmError> {
    let mut reader = request_fields(request)?;
    Ok((reader.u64()?, reader.wstr()?))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request_with(fields: impl FnOnce(&mut NmWriter)) -> Vec<u8> {
        let mut writer = NmWriter::new();
        writer.begin(NM_FUNC_CLASS, 1, 1).u32(0);
        fields(&mut writer);
        writer.end();
        writer.into_bytes()
    }

    #[test]
    fn requests_parse_their_target() {
        let send = request_with(|writer| {
            writer.u64(0).wstr("친구");
        });
        assert_eq!(parse_send_request(&send).unwrap(), (0, "친구".to_owned()));
        let accept = request_with(|writer| {
            writer.u64(1_000_002);
        });
        assert_eq!(parse_target_id(&accept).unwrap(), 1_000_002);
    }

    #[test]
    fn friend_list_reply_nests_complete_objects() {
        let request = request_with(|_| {});
        let user = SocialUser {
            id: 1_000_001,
            nickname: "Rider".to_owned(),
        };
        let reply = friend_list_reply(&request, &[(user, true)]);
        let mut reader = NmReader::new(&reply);
        let header = reader.header().unwrap();
        assert_eq!(header.length as usize, reply.len() - 8);
        assert_eq!(reader.u32().unwrap(), 1);
        assert_eq!(reader.u32().unwrap(), 1);
        let friend = reader.header().unwrap();
        assert_eq!(friend.class, FRIEND_CLASS);
        let user = reader.header().unwrap();
        assert_eq!(user.class, USER_CLASS);
        assert_eq!(reader.u64().unwrap(), 1_000_001);
        assert_eq!(reader.wstr().unwrap(), "Rider");
        assert_eq!(reader.wstr().unwrap(), "Rider");
        assert_eq!(reader.wstr().unwrap(), "");
        assert_eq!(reader.header().unwrap().class, PRESENCE_CLASS);
        assert_eq!(reader.u32().unwrap(), 0);
        assert_eq!(reader.wstr().unwrap(), "");
        assert_eq!(reader.u8().unwrap(), 1);
        assert_eq!(reader.u8().unwrap(), 0);
        assert_eq!(reader.u32().unwrap(), 0);
        assert_eq!(reader.remaining(), [] as [u8; 0]);
    }

    #[test]
    fn code_reply_carries_the_result_after_the_base() {
        let reply = code_reply(&request_with(|_| {}), SOCIAL_ALREADY_FRIEND);
        let mut reader = NmReader::new(&reply);
        reader.header().unwrap();
        assert_eq!(reader.u32().unwrap(), 1);
        assert_eq!(reader.u32().unwrap(), SOCIAL_ALREADY_FRIEND);
    }
}
