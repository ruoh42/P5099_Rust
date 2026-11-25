use thiserror::Error;

use crate::{
    login::LegacyTime,
    packet::{PacketError, PacketReader, PacketWriter},
    room_protocol::MAX_CLUB_NAME_UTF16_UNITS,
};

pub const CLUB_GRADE_MASTER: u16 = 5;
pub const CLUB_GRADE_MANAGER: u16 = 4;
pub const CLUB_GRADE_MEMBER: u16 = 2;

pub const LEAVE_DATE_OK: u32 = 0;
pub const LEAVE_DATE_TOO_RECENT: u32 = 3;

pub const CREATE_CONDITION_OK: u32 = 0;
pub const CREATE_CONDITION_REFRESH: u32 = 4;

pub const CREATE_CLUB_SUCCESS: u32 = 0;
pub const CREATE_CLUB_NAME_DUPLICATE: u32 = 1;
pub const CREATE_CLUB_ALREADY_JOINED: u32 = 6;
pub const CREATE_CLUB_FAILED: u32 = 7;

pub const JOIN_CLUB_JOINED: u32 = 0;
pub const JOIN_CLUB_WAITING: u32 = 1;
pub const JOIN_CLUB_CREW_FULL: u32 = 2;
pub const JOIN_CLUB_LEAVE_DATE_NOT_OVER: u32 = 4;
pub const JOIN_CLUB_FAILED: u32 = 6;

pub const CLUB_RESULT_OK: u32 = 0;
pub const CLUB_RESULT_REFRESH: u32 = 2;
pub const KICK_RESULT_MASTER: u32 = 3;

pub const CLUB_LIST_PAGE_SIZE: usize = 12;
pub const CLUB_CREW_PAGE_SIZE: usize = 10;
pub const MAX_CLUB_INTRO_UTF16_UNITS: usize = 256;

const NO_DATE: LegacyTime = LegacyTime {
    days_since_1900: u16::MAX,
    quarter_seconds: 0,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClubRequest {
    CheckMyLeaveDate,
    CreateClub,
    JoinClub,
    CancelClubWaiting,
    GetClubAutoJoinState,
    GetClubInfo,
    ClubInitFirst,
    CheckClubDirtyTime,
    GetClubRealCrewList,
    GetClubRecord,
    GetClubWaitingCrewList,
    InitClubHouse,
    InitClubHq,
    InitClubBank,
    LeaveClub,
    BreakUpClub,
    ChangeClubIntro,
    ChangeClubAutoJoinState,
    KickOutCrew,
    UpdateCrewGrade,
    WaitingCrewProcess,
    ChangeClubName,
    ChangeClubMark,
    InitRacingCenter,
    InitRiderCenter,
    ClubSupport,
}

impl ClubRequest {
    pub const ALL: [Self; 26] = [
        Self::CheckMyLeaveDate,
        Self::CreateClub,
        Self::JoinClub,
        Self::CancelClubWaiting,
        Self::GetClubAutoJoinState,
        Self::GetClubInfo,
        Self::ClubInitFirst,
        Self::CheckClubDirtyTime,
        Self::GetClubRealCrewList,
        Self::GetClubRecord,
        Self::GetClubWaitingCrewList,
        Self::InitClubHouse,
        Self::InitClubHq,
        Self::InitClubBank,
        Self::LeaveClub,
        Self::BreakUpClub,
        Self::ChangeClubIntro,
        Self::ChangeClubAutoJoinState,
        Self::KickOutCrew,
        Self::UpdateCrewGrade,
        Self::WaitingCrewProcess,
        Self::ChangeClubName,
        Self::ChangeClubMark,
        Self::InitRacingCenter,
        Self::InitRiderCenter,
        Self::ClubSupport,
    ];

    #[must_use]
    pub const fn request_name(self) -> &'static str {
        match self {
            Self::CheckMyLeaveDate => "PqCheckMyLeaveDatePacket",
            Self::CreateClub => "PqCreateClubPacket",
            Self::JoinClub => "PqJoinClubPacket",
            Self::CancelClubWaiting => "PqCancelClubWaitingPacket",
            Self::GetClubAutoJoinState => "PqGetClubAutoJoinStatePacket",
            Self::GetClubInfo => "PqGetClubInfoPacket",
            Self::ClubInitFirst => "PqClubInitFirstPacket",
            Self::CheckClubDirtyTime => "PqCheckClubDirtyTimePacket",
            Self::GetClubRealCrewList => "PqGetClubRealCrewListPacket",
            Self::GetClubRecord => "PqGetClubRecordPacket",
            Self::GetClubWaitingCrewList => "PqGetClubWaitingCrewListPacket",
            Self::InitClubHouse => "PqInitClubHousePacket",
            Self::InitClubHq => "PqInitClubHQPacket",
            Self::InitClubBank => "PqInitClubBankPacket",
            Self::LeaveClub => "PqLeaveClubPacket",
            Self::BreakUpClub => "PqBreakUpClubPacket",
            Self::ChangeClubIntro => "PqChangeClubIntroPacket",
            Self::ChangeClubAutoJoinState => "PqChangeClubAutoJoinStatePacket",
            Self::KickOutCrew => "PqKickOutCrewPacket",
            Self::UpdateCrewGrade => "PqUpdateCrewGradePacket",
            Self::WaitingCrewProcess => "PqWaitingCrewProcessPacket",
            Self::ChangeClubName => "PqChangeClubNamePacket",
            Self::ChangeClubMark => "PqChangeClubMarkPacket",
            Self::InitRacingCenter => "PqInitRacingCenterPacket",
            Self::InitRiderCenter => "PqInitRiderCenterPacket",
            Self::ClubSupport => "PqClubSupportPacket",
        }
    }

    #[must_use]
    pub fn reply_name(self) -> String {
        format!("Pr{}", &self.request_name()[2..])
    }
}

#[must_use]
pub fn classify_club_request(hash: u32) -> Option<ClubRequest> {
    ClubRequest::ALL
        .into_iter()
        .find(|request| crate::adler32::packet_hash(request.request_name()) == hash)
}

#[derive(Clone, PartialEq, Eq)]
pub enum ParsedClubRequest {
    CheckMyLeaveDate,
    CreateClub {
        name: String,
        intro: String,
        mark_logo: u32,
        mark_line: u32,
    },
    JoinClub {
        club_code: u32,
    },
    CancelClubWaiting {
        club_code: u32,
    },
    GetClubAutoJoinState {
        club_code: u32,
    },
    GetClubInfo {
        club_code: u32,
    },
    ClubInitFirst,
    CheckClubDirtyTime,
    GetClubRealCrewList {
        club_code: u32,
        page: u32,
    },
    GetClubRecord {
        club_code: u32,
    },
    GetClubWaitingCrewList {
        club_code: u32,
        page: u32,
    },
    InitClubHouse,
    InitClubHq {
        kind: u8,
    },
    InitClubBank,
    LeaveClub {
        club_code: u32,
    },
    BreakUpClub {
        club_code: u32,
    },
    ChangeClubIntro {
        intro: String,
    },
    ChangeClubAutoJoinState {
        auto_join: bool,
    },
    KickOutCrew {
        club_code: u32,
        user_no: u32,
    },
    UpdateCrewGrade {
        user_no: u32,
        grade: u16,
    },
    WaitingCrewProcess {
        user_no: u32,
        accept: bool,
    },
    ChangeClubName {
        name: String,
    },
    ChangeClubMark {
        mark_logo: u32,
        mark_line: u32,
    },
    InitRacingCenter,
    InitRiderCenter {
        kind: u8,
    },
    ClubSupport {
        amount: u32,
    },
}

impl ParsedClubRequest {
    #[must_use]
    pub const fn kind(&self) -> ClubRequest {
        match self {
            Self::CheckMyLeaveDate => ClubRequest::CheckMyLeaveDate,
            Self::CreateClub { .. } => ClubRequest::CreateClub,
            Self::JoinClub { .. } => ClubRequest::JoinClub,
            Self::CancelClubWaiting { .. } => ClubRequest::CancelClubWaiting,
            Self::GetClubAutoJoinState { .. } => ClubRequest::GetClubAutoJoinState,
            Self::GetClubInfo { .. } => ClubRequest::GetClubInfo,
            Self::ClubInitFirst => ClubRequest::ClubInitFirst,
            Self::CheckClubDirtyTime => ClubRequest::CheckClubDirtyTime,
            Self::GetClubRealCrewList { .. } => ClubRequest::GetClubRealCrewList,
            Self::GetClubRecord { .. } => ClubRequest::GetClubRecord,
            Self::GetClubWaitingCrewList { .. } => ClubRequest::GetClubWaitingCrewList,
            Self::InitClubHouse => ClubRequest::InitClubHouse,
            Self::InitClubHq { .. } => ClubRequest::InitClubHq,
            Self::InitClubBank => ClubRequest::InitClubBank,
            Self::LeaveClub { .. } => ClubRequest::LeaveClub,
            Self::BreakUpClub { .. } => ClubRequest::BreakUpClub,
            Self::ChangeClubIntro { .. } => ClubRequest::ChangeClubIntro,
            Self::ChangeClubAutoJoinState { .. } => ClubRequest::ChangeClubAutoJoinState,
            Self::KickOutCrew { .. } => ClubRequest::KickOutCrew,
            Self::UpdateCrewGrade { .. } => ClubRequest::UpdateCrewGrade,
            Self::WaitingCrewProcess { .. } => ClubRequest::WaitingCrewProcess,
            Self::ChangeClubName { .. } => ClubRequest::ChangeClubName,
            Self::ChangeClubMark { .. } => ClubRequest::ChangeClubMark,
            Self::InitRacingCenter => ClubRequest::InitRacingCenter,
            Self::InitRiderCenter { .. } => ClubRequest::InitRiderCenter,
            Self::ClubSupport { .. } => ClubRequest::ClubSupport,
        }
    }
}

#[derive(Debug, Error)]
pub enum ClubProtocolError {
    #[error(transparent)]
    Packet(#[from] PacketError),

    #[error("unsupported P5099 club packet hash 0x{actual:08X}")]
    UnsupportedPacketHash { actual: u32 },

    #[error("packet {name} has {count} unexpected trailing bytes")]
    TrailingBytes { name: &'static str, count: usize },
}

// One flat match over every club packet kind; splitting it would only scatter
// the dispatch table.
#[allow(clippy::too_many_lines)]
pub fn parse_club_request(packet: &[u8]) -> Result<ParsedClubRequest, ClubProtocolError> {
    let mut reader = PacketReader::new(packet);
    let hash = reader.read_u32()?;
    let kind = classify_club_request(hash)
        .ok_or(ClubProtocolError::UnsupportedPacketHash { actual: hash })?;
    let parsed = match kind {
        ClubRequest::CheckMyLeaveDate => ParsedClubRequest::CheckMyLeaveDate,
        ClubRequest::CreateClub => ParsedClubRequest::CreateClub {
            name: reader.read_utf16_bounded(MAX_CLUB_NAME_UTF16_UNITS)?,
            intro: reader.read_utf16_bounded(MAX_CLUB_INTRO_UTF16_UNITS)?,
            mark_logo: reader.read_u32()?,
            mark_line: reader.read_u32()?,
        },
        ClubRequest::JoinClub => ParsedClubRequest::JoinClub {
            club_code: reader.read_u32()?,
        },
        ClubRequest::CancelClubWaiting => ParsedClubRequest::CancelClubWaiting {
            club_code: reader.read_u32()?,
        },
        ClubRequest::GetClubAutoJoinState => ParsedClubRequest::GetClubAutoJoinState {
            club_code: reader.read_u32()?,
        },
        ClubRequest::GetClubInfo => ParsedClubRequest::GetClubInfo {
            club_code: reader.read_u32()?,
        },
        ClubRequest::ClubInitFirst => ParsedClubRequest::ClubInitFirst,
        ClubRequest::CheckClubDirtyTime => ParsedClubRequest::CheckClubDirtyTime,
        ClubRequest::GetClubRealCrewList => ParsedClubRequest::GetClubRealCrewList {
            club_code: reader.read_u32()?,
            page: reader.read_u32()?,
        },
        ClubRequest::GetClubRecord => ParsedClubRequest::GetClubRecord {
            club_code: reader.read_u32()?,
        },
        ClubRequest::GetClubWaitingCrewList => ParsedClubRequest::GetClubWaitingCrewList {
            club_code: reader.read_u32()?,
            page: reader.read_u32()?,
        },
        ClubRequest::InitClubHouse => ParsedClubRequest::InitClubHouse,
        ClubRequest::InitClubHq => ParsedClubRequest::InitClubHq {
            kind: reader.read_u8()?,
        },
        ClubRequest::InitClubBank => ParsedClubRequest::InitClubBank,
        ClubRequest::LeaveClub => ParsedClubRequest::LeaveClub {
            club_code: reader.read_u32()?,
        },
        ClubRequest::BreakUpClub => {
            let club_code = reader.read_u32()?;
            let _reserved = reader.read_u32()?;
            ParsedClubRequest::BreakUpClub { club_code }
        }
        ClubRequest::ChangeClubIntro => {
            let intro = reader.read_utf16_bounded(MAX_CLUB_INTRO_UTF16_UNITS)?;
            let _club_code = reader.read_u32()?;
            let _reserved = reader.read_u32()?;
            ParsedClubRequest::ChangeClubIntro { intro }
        }
        ClubRequest::ChangeClubAutoJoinState => ParsedClubRequest::ChangeClubAutoJoinState {
            auto_join: reader.read_u8()? != 0,
        },
        ClubRequest::KickOutCrew => {
            let club_code = reader.read_u32()?;
            let user_no = reader.read_u32()?;
            let _grade = reader.read_u32()?;
            ParsedClubRequest::KickOutCrew { club_code, user_no }
        }
        ClubRequest::UpdateCrewGrade => {
            let user_no = reader.read_u32()?;
            let grade = reader.read_u16()?;
            let _previous_grade = reader.read_u16()?;
            ParsedClubRequest::UpdateCrewGrade { user_no, grade }
        }
        ClubRequest::WaitingCrewProcess => ParsedClubRequest::WaitingCrewProcess {
            user_no: reader.read_u32()?,
            accept: reader.read_u32()? != 0,
        },
        ClubRequest::ChangeClubName => {
            let name = reader.read_utf16_bounded(MAX_CLUB_NAME_UTF16_UNITS)?;
            let _previous_name = reader.read_utf16_bounded(MAX_CLUB_NAME_UTF16_UNITS)?;
            ParsedClubRequest::ChangeClubName { name }
        }
        ClubRequest::ChangeClubMark => {
            let _current_logo = reader.read_u32()?;
            let mark_logo = reader.read_u32()?;
            let _current_line = reader.read_u32()?;
            let mark_line = reader.read_u32()?;
            ParsedClubRequest::ChangeClubMark {
                mark_logo,
                mark_line,
            }
        }
        ClubRequest::InitRacingCenter => ParsedClubRequest::InitRacingCenter,
        ClubRequest::InitRiderCenter => ParsedClubRequest::InitRiderCenter {
            kind: reader.read_u8()?,
        },
        ClubRequest::ClubSupport => ParsedClubRequest::ClubSupport {
            amount: reader.read_u32()?,
        },
    };
    let trailing = reader.remaining().len();
    if trailing != 0 {
        return Err(ClubProtocolError::TrailingBytes {
            name: kind.request_name(),
            count: trailing,
        });
    }
    Ok(parsed)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClubInfoView {
    pub club_code: u32,
    pub name: String,
    pub created: LegacyTime,
    pub master_user_no: u32,
    pub master_nickname: String,
    pub intro: String,
    pub mark_logo: u32,
    pub mark_line: u32,
    pub member_count: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClubMemberView {
    pub club_code: u32,
    pub user_no: u32,
    pub nickname: String,
    pub rp: u32,
    pub grade: u16,
    pub joined: LegacyTime,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClubListEntry {
    pub info: ClubInfoView,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClubWaitingCrewView {
    pub user_no: u32,
    pub nickname: String,
    pub rp: u32,
}

pub const CLUB_LEVEL: u8 = 5;

pub const RIDER_CENTER_MAX_MEMBERS: [u32; 5] = [30, 60, 80, 120, 150];

#[must_use]
pub const fn club_max_members(level: u8) -> u32 {
    let index = if level == 0 {
        0
    } else if level as usize > RIDER_CENTER_MAX_MEMBERS.len() {
        RIDER_CENTER_MAX_MEMBERS.len() - 1
    } else {
        level as usize - 1
    };
    RIDER_CENTER_MAX_MEMBERS[index]
}

pub const CLUB_MAX_MEMBERS: u32 = club_max_members(CLUB_LEVEL);

fn write_time(packet: &mut PacketWriter, time: LegacyTime) {
    packet.write_u16(time.days_since_1900);
    packet.write_u16(time.quarter_seconds);
}

fn write_club_info(packet: &mut PacketWriter, info: &ClubInfoView) -> Result<(), PacketError> {
    packet.write_u32(info.club_code);
    packet.write_utf16(&info.name)?;
    write_time(packet, info.created);
    packet.write_u8(CLUB_LEVEL);
    packet.write_u8(1);
    packet.write_u32(info.master_user_no);
    packet.write_utf16(&info.master_nickname)?;
    packet.write_u32(CLUB_MAX_MEMBERS);
    packet.write_u32(0);
    packet.write_u32(1_000);
    packet.write_u8(4);
    write_time(packet, NO_DATE);
    packet.write_utf16(&info.intro)?;
    packet.write_u32(info.mark_logo);
    packet.write_u32(info.mark_line);
    packet.write_u32(0);
    packet.write_u32(0);
    packet.write_u32(0);
    packet.write_u32(0);
    packet.write_u32(info.member_count);
    Ok(())
}

fn write_member(packet: &mut PacketWriter, member: &ClubMemberView) -> Result<(), PacketError> {
    packet.write_u32(member.club_code);
    packet.write_u32(member.user_no);
    packet.write_utf16(&member.nickname)?;
    packet.write_u32(member.rp);
    packet.write_u16(member.grade);
    packet.write_u32(0);
    packet.write_u32(0);
    write_time(packet, member.joined);
    write_time(packet, NO_DATE);
    write_time(packet, member.joined);
    write_time(packet, member.joined);
    packet.write_u32(0);
    Ok(())
}

fn count(length: usize) -> u32 {
    u32::try_from(length).unwrap_or(u32::MAX)
}

pub fn serialize_club_info(
    reply_name: &str,
    info: Option<&ClubInfoView>,
) -> Result<Vec<u8>, PacketError> {
    let empty = ClubInfoView {
        club_code: 0,
        name: String::new(),
        created: NO_DATE,
        master_user_no: 0,
        master_nickname: String::new(),
        intro: String::new(),
        mark_logo: 0,
        mark_line: 0,
        member_count: 0,
    };
    let mut packet = PacketWriter::named(reply_name);
    write_club_info(&mut packet, info.unwrap_or(&empty))?;
    packet.write_u32(0);
    packet.write_bytes(&[0; 8]);
    Ok(packet.into_inner())
}

pub fn serialize_member_club_state(
    info: &ClubInfoView,
    grade: u16,
    nickname: &str,
) -> Result<Vec<u8>, PacketError> {
    let mut packet = PacketWriter::named("PrCheckMyClubStatePacket");
    packet.write_u32(info.club_code);
    packet.write_utf16(&info.name)?;
    packet.write_u32(info.mark_logo);
    packet.write_u32(info.mark_line);
    packet.write_u16(grade);
    packet.write_utf16(nickname)?;
    packet.write_u32(1);
    packet.write_u8(CLUB_LEVEL);
    Ok(packet.into_inner())
}

pub fn serialize_waiting_join(club_code: u32, club_name: &str) -> Result<Vec<u8>, PacketError> {
    let mut packet = PacketWriter::named("PrGetUserWaitingJoinClubPacket");
    packet.write_u32(1);
    packet.write_u32(club_code);
    packet.write_utf16(club_name)?;
    Ok(packet.into_inner())
}

#[must_use]
pub fn serialize_u32_reply(reply_name: &str, value: u32) -> Vec<u8> {
    let mut packet = PacketWriter::named(reply_name);
    packet.write_u32(value);
    packet.into_inner()
}

#[must_use]
pub fn serialize_u32_pair_reply(reply_name: &str, first: u32, second: u32) -> Vec<u8> {
    let mut packet = PacketWriter::named(reply_name);
    packet.write_u32(first);
    packet.write_u32(second);
    packet.into_inner()
}

#[must_use]
pub fn serialize_create_club(club_code: u32, lucci: u32, result: u32) -> Vec<u8> {
    let mut packet = PacketWriter::named("PrCreateClubPacket");
    packet.write_u32(club_code);
    packet.write_u32(lucci);
    packet.write_u32(result);
    packet.into_inner()
}

pub fn serialize_club_list(entries: &[ClubListEntry]) -> Result<Vec<u8>, PacketError> {
    let mut packet = PacketWriter::named("PrSearchClubListPacket");
    packet.write_u32(count(entries.len()));
    for entry in entries {
        let info = &entry.info;
        packet.write_u32(info.club_code);
        packet.write_utf16(&info.name)?;
        packet.write_u8(CLUB_LEVEL);
        packet.write_u32(CLUB_MAX_MEMBERS);
        packet.write_u32(u32::MAX);
        packet.write_u32(0);
        packet.write_u32(0);
        packet.write_u32(info.mark_logo);
        packet.write_u32(info.mark_line);
        packet.write_utf16(&info.master_nickname)?;
        packet.write_u32(info.member_count);
        write_time(&mut packet, info.created);
        packet.write_utf16(&info.intro)?;
        packet.write_u8(0);
        packet.write_u8(0);
        packet.write_u32(0);
    }
    packet.write_u8(1);
    Ok(packet.into_inner())
}

#[must_use]
pub fn serialize_auto_join_state(club_code: u32, auto_join: bool) -> Vec<u8> {
    let mut packet = PacketWriter::named("PrGetClubAutoJoinStatePacket");
    packet.write_u32(club_code);
    packet.write_u8(u8::from(auto_join));
    packet.into_inner()
}

pub fn serialize_join_club(
    club_code: u32,
    club_name: &str,
    grade: u16,
    result: u32,
) -> Result<Vec<u8>, PacketError> {
    let mut packet = PacketWriter::named("PrJoinClubPacket");
    packet.write_u32(club_code);
    packet.write_utf16(club_name)?;
    packet.write_u16(grade);
    packet.write_u32(result);
    Ok(packet.into_inner())
}

pub fn serialize_club_init_first(
    info: &ClubInfoView,
    crew: &[ClubMemberView],
    me: &ClubMemberView,
) -> Result<Vec<u8>, PacketError> {
    let mut packet = PacketWriter::named("PrClubInitFirstPacket");
    write_club_info(&mut packet, info)?;
    packet.write_u32(0);
    packet.write_bytes(&[0; 16]);
    packet.write_u32(count(crew.len()));
    for member in crew {
        write_member(&mut packet, member)?;
    }
    write_member(&mut packet, me)?;
    packet.write_u32(0);
    packet.write_bytes(&[0; 32]);
    packet.write_u32(0);
    Ok(packet.into_inner())
}

#[must_use]
pub fn serialize_clean_dirty_time() -> Vec<u8> {
    let mut packet = PacketWriter::named("PrCheckClubDirtyTimePacket");
    packet.write_bytes(&[0; 32]);
    packet.write_u32(0);
    packet.write_bytes(&[0; 12]);
    packet.into_inner()
}

pub fn serialize_crew_list(
    crew: &[ClubMemberView],
    total: u32,
    me: &ClubMemberView,
) -> Result<Vec<u8>, PacketError> {
    let mut packet = PacketWriter::named("PrGetClubRealCrewListPacket");
    packet.write_u32(count(crew.len()));
    for member in crew {
        write_member(&mut packet, member)?;
    }
    packet.write_u32(total);
    write_member(&mut packet, me)?;
    packet.write_u32(0);
    packet.write_bytes(&[0; 8]);
    Ok(packet.into_inner())
}

#[must_use]
pub fn serialize_empty_club_record() -> Vec<u8> {
    let mut packet = PacketWriter::named("PrGetClubRecordPacket");
    packet.write_bytes(&[0; 16]);
    packet.write_bytes(&[0; 8]);
    packet.into_inner()
}

pub fn serialize_waiting_crew_list(
    waiting: &[ClubWaitingCrewView],
    total: u32,
) -> Result<Vec<u8>, PacketError> {
    let mut packet = PacketWriter::named("PrGetClubWaitingCrewListPacket");
    packet.write_u32(count(waiting.len()));
    for crew in waiting {
        packet.write_u32(crew.user_no);
        packet.write_utf16(&crew.nickname)?;
        packet.write_u32(crew.rp);
    }
    packet.write_u32(total);
    packet.write_u32(0);
    packet.write_bytes(&[0; 8]);
    Ok(packet.into_inner())
}

pub fn serialize_club_house(info: &ClubInfoView, capital: u32) -> Result<Vec<u8>, PacketError> {
    let mut packet = PacketWriter::named("PrInitClubHousePacket");
    packet.write_utf16(&info.name)?;
    packet.write_u8(CLUB_LEVEL);
    packet.write_u8(CLUB_LEVEL);
    packet.write_u32(info.member_count);
    packet.write_u32(CLUB_MAX_MEMBERS);
    packet.write_u32(0);
    packet.write_u32(capital);
    packet.write_u32(info.mark_logo);
    packet.write_u32(info.mark_line);
    packet.write_u32(0);
    packet.write_u32(0);
    Ok(packet.into_inner())
}

#[must_use]
pub fn serialize_club_hq(kind: u8) -> Vec<u8> {
    let mut packet = PacketWriter::named("PrInitClubHQPacket");
    packet.write_u8(kind);
    packet.write_u32(0);
    packet.write_u32(0);
    packet.write_u32(0);
    packet.write_u32(CLUB_MAX_MEMBERS);
    packet.write_u32(10);
    packet.write_u32(0);
    packet.into_inner()
}

pub const CLUB_SUPPORT_OPTIONS: [u32; 4] = [1, 10, 50, 100];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClubSupportRecord {
    pub date: LegacyTime,
    pub nickname: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ClubBankView {
    pub capital: u32,
    pub my_total: u32,
    pub top_supporter: String,
    pub history: Vec<ClubSupportRecord>,
}

fn write_support_history(
    packet: &mut PacketWriter,
    history: &[ClubSupportRecord],
) -> Result<(), PacketError> {
    packet.write_u32(count(history.len()));
    for record in history {
        write_time(packet, record.date);
    }
    packet.write_u32(count(history.len()));
    for record in history {
        packet.write_utf16(&record.nickname)?;
    }
    Ok(())
}

pub fn serialize_club_bank(bank: &ClubBankView) -> Result<Vec<u8>, PacketError> {
    let mut packet = PacketWriter::named("PrInitClubBankPacket");
    packet.write_u32(bank.capital);
    packet.write_u32(bank.my_total);
    packet.write_utf16(&bank.top_supporter)?;
    write_support_history(&mut packet, &bank.history)?;
    packet.write_u32(count(CLUB_SUPPORT_OPTIONS.len()));
    for option in CLUB_SUPPORT_OPTIONS {
        packet.write_u32(option);
    }
    packet.write_u32(0);
    Ok(packet.into_inner())
}

pub const CLUB_SUPPORT_OK: u32 = 0;
pub const CLUB_SUPPORT_FAILED: u32 = 0x0D;

pub fn serialize_club_support(
    lucci_after: u32,
    amount: u32,
    bank: &ClubBankView,
    result: u32,
) -> Result<Vec<u8>, PacketError> {
    let mut packet = PacketWriter::named("PrClubSupportPacket");
    packet.write_u32(lucci_after);
    packet.write_u32(bank.capital);
    packet.write_u32(amount);
    packet.write_u32(bank.my_total);
    packet.write_utf16(&bank.top_supporter)?;
    write_support_history(&mut packet, &bank.history)?;
    packet.write_u32(result);
    Ok(packet.into_inner())
}

#[must_use]
pub fn serialize_rider_center(kind: u8) -> Vec<u8> {
    let mut packet = PacketWriter::named("PrInitRiderCenterPacket");
    packet.write_u8(kind);
    packet.write_u32(0);
    packet.write_u32(0);
    packet.write_u32(0);
    packet.write_u32(0);
    packet.into_inner()
}

#[must_use]
pub fn serialize_leave_club(result: u32) -> Vec<u8> {
    let mut packet = PacketWriter::named("PrLeaveClubPacket");
    packet.write_u8(0);
    packet.write_u32(result);
    packet.into_inner()
}

#[must_use]
pub fn serialize_break_up_club(date: LegacyTime, result: u32) -> Vec<u8> {
    let mut packet = PacketWriter::named("PrBreakUpClubPacket");
    write_time(&mut packet, date);
    packet.write_u32(result);
    packet.into_inner()
}

#[must_use]
pub fn serialize_change_auto_join(auto_join: bool, result: u32) -> Vec<u8> {
    let mut packet = PacketWriter::named("PrChangeClubAutoJoinStatePacket");
    packet.write_u8(u8::from(auto_join));
    packet.write_u32(result);
    packet.into_inner()
}

#[must_use]
pub fn serialize_crew_result(reply_name: &str, user_no: u32, result: u32) -> Vec<u8> {
    serialize_u32_pair_reply(reply_name, user_no, result)
}

#[must_use]
pub fn serialize_update_crew_grade(user_no: u32, grade: u16, result: u32) -> Vec<u8> {
    let mut packet = PacketWriter::named("PrUpdateCrewGradePacket");
    packet.write_u32(user_no);
    packet.write_u16(grade);
    packet.write_u32(result);
    packet.into_inner()
}

pub fn serialize_change_club_name(
    name: &str,
    capital: u32,
    result: u32,
) -> Result<Vec<u8>, PacketError> {
    let mut packet = PacketWriter::named("PrChangeClubNamePacket");
    packet.write_utf16(name)?;
    packet.write_u32(capital);
    packet.write_u32(result);
    Ok(packet.into_inner())
}

#[must_use]
pub fn serialize_empty_racing_center() -> Vec<u8> {
    let mut packet = PacketWriter::named("PrInitRacingCenterPacket");
    packet.write_u8(0);
    packet.write_u8(0);
    packet.write_u32(0);
    packet.write_u32(0);
    packet.into_inner()
}

#[must_use]
pub fn serialize_change_club_mark(
    mark_logo: u32,
    mark_line: u32,
    capital: u32,
    result: u32,
) -> Vec<u8> {
    let mut packet = PacketWriter::named("PrChangeClubMarkPacket");
    packet.write_u32(mark_logo);
    packet.write_u32(mark_line);
    packet.write_u32(capital);
    packet.write_u32(result);
    packet.into_inner()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adler32::packet_hash;

    fn time() -> LegacyTime {
        LegacyTime {
            days_since_1900: 0x2D31,
            quarter_seconds: 7,
        }
    }

    fn info() -> ClubInfoView {
        ClubInfoView {
            club_code: 10_001,
            name: "Club".to_owned(),
            created: time(),
            master_user_no: 3,
            master_nickname: "Master".to_owned(),
            intro: "Hi".to_owned(),
            mark_logo: 22,
            mark_line: 0,
            member_count: 1,
        }
    }

    fn member() -> ClubMemberView {
        ClubMemberView {
            club_code: 10_001,
            user_no: 3,
            nickname: "Master".to_owned(),
            rp: 100,
            grade: CLUB_GRADE_MASTER,
            joined: time(),
        }
    }

    fn utf16_len(text: &str) -> usize {
        4 + 2 * text.encode_utf16().count()
    }

    #[test]
    fn every_request_name_classifies_to_itself() {
        for request in ClubRequest::ALL {
            assert_eq!(
                classify_club_request(packet_hash(request.request_name())),
                Some(request)
            );
        }
        assert_eq!(
            packet_hash("PqCheckMyLeaveDatePacket"),
            packet_hash(ClubRequest::CheckMyLeaveDate.request_name())
        );
    }

    #[test]
    fn create_request_reads_name_intro_and_mark_exactly() {
        let mut packet = PacketWriter::named("PqCreateClubPacket");
        packet.write_utf16("Club").unwrap();
        packet.write_utf16("Intro").unwrap();
        packet.write_u32(22);
        packet.write_u32(1);
        let parsed = parse_club_request(&packet.into_inner()).unwrap();
        assert!(matches!(
            parsed,
            ParsedClubRequest::CreateClub { ref name, ref intro, mark_logo: 22, mark_line: 1 }
                if name == "Club" && intro == "Intro"
        ));

        let mut trailing = PacketWriter::named("PqJoinClubPacket");
        trailing.write_u32(5);
        trailing.write_u8(0);
        assert!(matches!(
            parse_club_request(&trailing.into_inner()),
            Err(ClubProtocolError::TrailingBytes { .. })
        ));
    }

    #[test]
    fn change_club_mark_reads_the_selected_logo_and_line() {
        let mut packet = PacketWriter::named("PqChangeClubMarkPacket");
        for value in [30, 282, 1, 10] {
            packet.write_u32(value);
        }
        assert!(matches!(
            parse_club_request(&packet.into_inner()).unwrap(),
            ParsedClubRequest::ChangeClubMark {
                mark_logo: 282,
                mark_line: 10
            }
        ));
        let reply = serialize_change_club_mark(282, 10, 100, CLUB_RESULT_OK);
        assert_eq!(&reply[4..8], &282_u32.to_le_bytes());
        assert_eq!(&reply[8..12], &10_u32.to_le_bytes());
        assert_eq!(&reply[12..16], &100_u32.to_le_bytes());
    }

    #[test]
    fn waiting_crew_process_reads_the_raw_decision_dword() {
        let mut packet = PacketWriter::named("PqWaitingCrewProcessPacket");
        packet.write_u32(9);
        packet.write_u32(1);
        assert!(matches!(
            parse_club_request(&packet.into_inner()).unwrap(),
            ParsedClubRequest::WaitingCrewProcess {
                user_no: 9,
                accept: true
            }
        ));
    }

    #[test]
    fn club_info_matches_the_recovered_reader_layout() {
        let info = info();
        let packet = serialize_club_info("PrInitClubInfoPacket", Some(&info)).unwrap();
        let expected = 4
            + 4
            + utf16_len("Club")
            + 4
            + 2
            + 4
            + utf16_len("Master")
            + 12
            + 1
            + 4
            + utf16_len("Hi")
            + 28
            + 4
            + 8;
        assert_eq!(packet.len(), expected);
        assert_eq!(
            &packet[..4],
            &packet_hash("PrInitClubInfoPacket").to_le_bytes()
        );
        assert_eq!(&packet[4..8], &10_001_u32.to_le_bytes());
    }

    #[test]
    fn crew_member_rows_are_fifty_bytes_plus_the_nickname() {
        let me = member();
        let packet = serialize_crew_list(std::slice::from_ref(&me), 1, &me).unwrap();
        let row = 4 + 4 + utf16_len("Master") + 4 + 2 + 4 + 4 + 16 + 4;
        assert_eq!(packet.len(), 4 + 4 + row + 4 + row + 4 + 8);
    }

    #[test]
    fn club_list_ends_with_the_completion_flag() {
        let packet = serialize_club_list(&[ClubListEntry { info: info() }]).unwrap();
        assert_eq!(packet.last(), Some(&1));
        assert_eq!(&packet[4..8], &1_u32.to_le_bytes());
        assert_eq!(serialize_club_list(&[]).unwrap().len(), 9);
    }

    #[test]
    fn crew_capacity_follows_the_rider_center_levels() {
        assert_eq!(CLUB_MAX_MEMBERS, 150);
        assert_eq!(club_max_members(1), 30);
        assert_eq!(club_max_members(4), 120);
        assert_eq!(club_max_members(0), 30);
        assert_eq!(club_max_members(9), 150);

        let house = serialize_club_house(&info(), 100).unwrap();
        let counts = 4 + utf16_len("Club") + 2;
        assert_eq!(&house[counts..counts + 4], &1_u32.to_le_bytes());
        assert_eq!(&house[counts + 4..counts + 8], &150_u32.to_le_bytes());
    }

    #[test]
    fn fixed_replies_have_the_recovered_lengths() {
        assert_eq!(serialize_create_club(1, 2, 0).len(), 16);
        assert_eq!(
            serialize_join_club(1, "", CLUB_GRADE_MEMBER, 0)
                .unwrap()
                .len(),
            18
        );
        assert_eq!(serialize_leave_club(0).len(), 9);
        assert_eq!(serialize_break_up_club(time(), 0).len(), 12);
        assert_eq!(serialize_clean_dirty_time().len(), 4 + 32 + 4 + 12);
        assert_eq!(serialize_empty_club_record().len(), 4 + 16 + 8);
        assert_eq!(serialize_club_hq(0).len(), 4 + 13 + 12);
        assert_eq!(
            serialize_club_bank(&ClubBankView::default()).unwrap().len(),
            4 + 4 + 4 + 4 + 4 + 4 + 4 + 16 + 4
        );
        assert_eq!(serialize_rider_center(5).len(), 4 + 17);
        let bank = ClubBankView {
            capital: 101,
            my_total: 1,
            top_supporter: "Master".to_owned(),
            history: vec![ClubSupportRecord {
                date: time(),
                nickname: "Master".to_owned(),
            }],
        };
        let support = serialize_club_support(999_999, 1, &bank, CLUB_SUPPORT_OK).unwrap();
        assert_eq!(
            support.len(),
            4 + 16 + utf16_len("Master") + 4 + 4 + 4 + utf16_len("Master") + 4
        );
        assert_eq!(&support[4..8], &999_999_u32.to_le_bytes());
        assert_eq!(serialize_empty_racing_center().len(), 4 + 10);
        assert_eq!(serialize_update_crew_grade(1, 2, 0).len(), 14);
        assert_eq!(serialize_change_auto_join(true, 0).len(), 9);
        let init_first = serialize_club_init_first(&info(), &[member()], &member()).unwrap();
        assert!(init_first.len() > 100);
    }
}
