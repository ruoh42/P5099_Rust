use thiserror::Error;

pub const NM_FUNC_CLASS: u16 = 0x666F;

pub const PLAIN_CODEC_MAGIC: u32 = 0x001F_CA34;
pub const PLAIN_CODEC_TRAILER: u32 = 0x008A_119E;
pub const SCRAMBLE_CODEC_MAGIC: u32 = 0x0A98_C32A;
pub const SCRAMBLE_CODEC_TRAILER: u32 = 0x820A_9E12;
const SCRAMBLE_CODEC_ID: u8 = 1;
const SCRAMBLE_OVERHEAD: usize = 10;

pub const MAX_NM_OBJECT_BYTES: usize = 0x10_0000;

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum NmError {
    #[error("NM data is truncated")]
    Truncated,
    #[error("NM frame has unknown codec magic 0x{0:08X}")]
    UnknownCodec(u32),
    #[error("NM frame trailer does not match its codec")]
    BadTrailer,
    #[error("NM scramble checksum mismatch")]
    BadChecksum,
    #[error("NM scramble codec id {0} is not supported")]
    BadCodecId(u8),
    #[error("NM object of {0} bytes exceeds the bound")]
    TooLarge(usize),
    #[error("NM object tag class 0x{actual:04X} does not match 0x{expected:04X}")]
    WrongClass { expected: u16, actual: u16 },
    #[error("NM string is not valid UTF-16")]
    InvalidString,
}

#[must_use]
pub fn length_u32(length: usize) -> u32 {
    u32::try_from(length).unwrap_or(u32::MAX)
}

fn checksum(bytes: &[u8]) -> u16 {
    let (sum, xor) = bytes.iter().fold((0_u8, 0_u8), |(sum, xor), &byte| {
        (sum.wrapping_add(byte), xor ^ byte)
    });
    (u16::from(!sum) << 1) | u16::from(xor)
}

fn read_u32_at(bytes: &[u8], offset: usize) -> Result<u32, NmError> {
    bytes
        .get(offset..offset + 4)
        .map(|slice| u32::from_le_bytes(slice.try_into().expect("four bytes")))
        .ok_or(NmError::Truncated)
}

pub fn scramble_decode(payload: &[u8]) -> Result<Vec<u8>, NmError> {
    let n = payload.len();
    if n < SCRAMBLE_OVERHEAD {
        return Err(NmError::Truncated);
    }
    let stored = u16::from_le_bytes([payload[n - 2], payload[n - 1]]);
    if stored != checksum(&payload[..n - 2]) {
        return Err(NmError::BadChecksum);
    }
    let reversed = payload[..n - 2].iter().rev().copied().collect::<Vec<_>>();
    if reversed[1] != SCRAMBLE_CODEC_ID {
        return Err(NmError::BadCodecId(reversed[1]));
    }
    let key = read_u32_at(&reversed, 3)?.to_le_bytes()[0];
    let mut previous = 0_u8;
    Ok(reversed[8..]
        .iter()
        .map(|&cipher| {
            let plain = cipher ^ (previous ^ key);
            previous = cipher;
            plain
        })
        .collect())
}

#[must_use]
pub fn scramble_encode(plain: &[u8], key: u32) -> Vec<u8> {
    let key_byte = key.to_le_bytes()[0];
    let mut reversed = vec![0, SCRAMBLE_CODEC_ID, 0];
    reversed.extend_from_slice(&key.to_le_bytes());
    reversed.push(0);
    let mut previous = 0_u8;
    for &byte in plain {
        let cipher = byte ^ (previous ^ key_byte);
        reversed.push(cipher);
        previous = cipher;
    }
    let mut payload = reversed.into_iter().rev().collect::<Vec<_>>();
    let check = checksum(&payload);
    payload.extend_from_slice(&check.to_le_bytes());
    payload
}

pub fn decode_frame(frame: &[u8]) -> Result<(Vec<u8>, usize), NmError> {
    let magic = read_u32_at(frame, 0)?;
    let length = read_u32_at(frame, 4)? as usize;
    if length > MAX_NM_OBJECT_BYTES + SCRAMBLE_OVERHEAD {
        return Err(NmError::TooLarge(length));
    }
    let payload = frame.get(8..8 + length).ok_or(NmError::Truncated)?;
    let trailer = read_u32_at(frame, 8 + length)?;
    let object = match magic {
        PLAIN_CODEC_MAGIC if trailer == PLAIN_CODEC_TRAILER => payload.to_vec(),
        SCRAMBLE_CODEC_MAGIC if trailer == SCRAMBLE_CODEC_TRAILER => scramble_decode(payload)?,
        PLAIN_CODEC_MAGIC | SCRAMBLE_CODEC_MAGIC => return Err(NmError::BadTrailer),
        other => return Err(NmError::UnknownCodec(other)),
    };
    Ok((object, 12 + length))
}

#[must_use]
pub fn encode_plain_frame(object: &[u8]) -> Vec<u8> {
    let mut frame = Vec::with_capacity(object.len() + 12);
    frame.extend_from_slice(&PLAIN_CODEC_MAGIC.to_le_bytes());
    frame.extend_from_slice(&length_u32(object.len()).to_le_bytes());
    frame.extend_from_slice(object);
    frame.extend_from_slice(&PLAIN_CODEC_TRAILER.to_le_bytes());
    frame
}

#[derive(Debug, Default, Clone)]
pub struct NmWriter {
    bytes: Vec<u8>,
    open: Vec<usize>,
}

impl NmWriter {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn u8(&mut self, value: u8) -> &mut Self {
        self.bytes.push(value);
        self
    }

    pub fn u16(&mut self, value: u16) -> &mut Self {
        self.bytes.extend_from_slice(&value.to_le_bytes());
        self
    }

    pub fn u32(&mut self, value: u32) -> &mut Self {
        self.bytes.extend_from_slice(&value.to_le_bytes());
        self
    }

    pub fn u64(&mut self, value: u64) -> &mut Self {
        self.bytes.extend_from_slice(&value.to_le_bytes());
        self
    }

    pub fn compact(&mut self, value: u32) -> &mut Self {
        if value < 0x40 {
            self.u8((value << 2).to_le_bytes()[0])
        } else if value < 0x4000 {
            let [low, high, ..] = ((value << 2) | 1).to_le_bytes();
            self.u16(u16::from_le_bytes([low, high]))
        } else if value < 0x4000_0000 {
            self.u32((value << 2) | 2)
        } else {
            self.u8(3).u32(value)
        }
    }

    pub fn wstr(&mut self, value: &str) -> &mut Self {
        let units = value.encode_utf16().collect::<Vec<_>>();
        self.compact(length_u32(units.len()));
        for unit in units {
            self.u16(unit);
        }
        self
    }

    pub fn bytes(&mut self, value: &[u8]) -> &mut Self {
        self.compact(length_u32(value.len()));
        self.bytes.extend_from_slice(value);
        self
    }

    pub fn begin(&mut self, class: u16, version: u8, flag: u8) -> &mut Self {
        self.u32(u32::from(class) << 16 | u32::from(version) << 8 | u32::from(flag));
        self.open.push(self.bytes.len());
        self.u32(0)
    }

    pub fn end(&mut self) -> &mut Self {
        if let Some(at) = self.open.pop() {
            let length = length_u32(self.bytes.len() - at - 4);
            self.bytes[at..at + 4].copy_from_slice(&length.to_le_bytes());
        }
        self
    }

    #[must_use]
    pub fn into_bytes(self) -> Vec<u8> {
        self.bytes
    }
}

#[derive(Debug, Clone)]
pub struct NmReader<'a> {
    bytes: &'a [u8],
    position: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NmObjectHeader {
    pub class: u16,
    pub version: u8,
    pub flag: u8,
    pub length: u32,
}

impl<'a> NmReader<'a> {
    #[must_use]
    pub const fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, position: 0 }
    }

    fn take(&mut self, count: usize) -> Result<&'a [u8], NmError> {
        let slice = self
            .bytes
            .get(self.position..self.position + count)
            .ok_or(NmError::Truncated)?;
        self.position += count;
        Ok(slice)
    }

    pub fn u8(&mut self) -> Result<u8, NmError> {
        Ok(self.take(1)?[0])
    }

    pub fn u16(&mut self) -> Result<u16, NmError> {
        Ok(u16::from_le_bytes(self.take(2)?.try_into().expect("two")))
    }

    pub fn u32(&mut self) -> Result<u32, NmError> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().expect("four")))
    }

    pub fn u64(&mut self) -> Result<u64, NmError> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into().expect("eight")))
    }

    pub fn compact(&mut self) -> Result<u32, NmError> {
        let first = self.u8()?;
        Ok(match first & 3 {
            0 => u32::from(first >> 2),
            1 => {
                let second = self.u8()?;
                u32::from(u16::from_le_bytes([first, second]) >> 2)
            }
            2 => {
                let rest = self.take(3)?;
                u32::from_le_bytes([first, rest[0], rest[1], rest[2]]) >> 2
            }
            _ => self.u32()?,
        })
    }

    pub fn wstr(&mut self) -> Result<String, NmError> {
        let units = self.compact()? as usize;
        if units > MAX_NM_OBJECT_BYTES / 2 {
            return Err(NmError::TooLarge(units * 2));
        }
        let raw = self.take(units * 2)?;
        let units = raw
            .as_chunks::<2>()
            .0
            .iter()
            .map(|pair| u16::from_le_bytes(*pair))
            .collect::<Vec<_>>();
        String::from_utf16(&units).map_err(|_| NmError::InvalidString)
    }

    pub fn bytes(&mut self) -> Result<&'a [u8], NmError> {
        let length = self.compact()? as usize;
        if length > MAX_NM_OBJECT_BYTES {
            return Err(NmError::TooLarge(length));
        }
        self.take(length)
    }

    pub fn header(&mut self) -> Result<NmObjectHeader, NmError> {
        let tag = self.u32()?;
        let length = self.u32()?;
        Ok(NmObjectHeader {
            class: (tag >> 16) as u16,
            version: tag.to_le_bytes()[1],
            flag: tag.to_le_bytes()[0],
            length,
        })
    }

    #[must_use]
    pub fn remaining(&self) -> &'a [u8] {
        &self.bytes[self.position.min(self.bytes.len())..]
    }

    #[must_use]
    pub const fn position(&self) -> usize {
        self.position
    }
}

#[must_use]
pub fn nm_func_name(code: u32) -> Option<&'static str> {
    NM_FUNC_NAMES
        .binary_search_by_key(&code, |(candidate, _)| *candidate)
        .ok()
        .map(|index| NM_FUNC_NAMES[index].1)
}

#[must_use]
pub fn nm_event_name(code: u32) -> Option<&'static str> {
    NM_EVENT_NAMES
        .binary_search_by_key(&code, |(candidate, _)| *candidate)
        .ok()
        .map(|index| NM_EVENT_NAMES[index].1)
}

pub const NM_FUNC_NAMES: &[(u32, &str)] = &[
    (0x1002, "RegisterCallbackFunc"),
    (0x1003, "ResetCallbackFunc"),
    (0x1004, "AttachToNMCOServerFunc"),
    (0x1005, "DetachFromNMCOServerFunc"),
    (0x1006, "BringForwardStandAloneMsgFunc"),
    (0x1007, "StartStandAloneMsgFunc"),
    (0x1103, "LoginVirtualFunc"),
    (0x1104, "LogoutVirtualFunc"),
    (0x1105, "GetMyVirtualUserListFunc"),
    (0x1106, "ChangeMyPositionFunc"),
    (0x1107, "SendRefreshMessageFunc"),
    (0x1108, "ChangeMyLevelFunc"),
    (0x1109, "RemoveMyVirtualUserFunc"),
    (0x110A, "LoginPassportFunc"),
    (0x110C, "LoginNexonPassportFunc"),
    (0x110D, "LoginMessengerFunc"),
    (0x110E, "LogoutMessengerFunc"),
    (0x110F, "LoginAuthFunc"),
    (0x1110, "LogoutAuthFunc"),
    (0x1111, "InitializeFunc"),
    (0x1112, "SecurityAuthFunc"),
    (0x1113, "GetGameTokenFunc"),
    (0x1114, "FinalizeFunc"),
    (0x1118, "SetAuthClosedCallbackFunc"),
    (0x1119, "TPATokenLoginFunc"),
    (0x111A, "PlugTokenLoginFunc"),
    (0x2001, "GetMyInfoFunc"),
    (0x2002, "GetUserDataListFunc"),
    (0x2003, "ChangeMyInfoFunc"),
    (0x2004, "GetConfigFunc"),
    (0x2005, "ChangeConfigFunc"),
    (0x2006, "GetLocalConfigFunc"),
    (0x2007, "ChangeLocalConfigFunc"),
    (0x2008, "GetCountFunc"),
    (0x2009, "SetCountFunc"),
    (0x200A, "ChangeNicknameFunc"),
    (0x200B, "ChangeMetaDataFunc"),
    (0x200C, "LoginNexonSocialFunc"),
    (0x200E, "GetSocialFriendListFunc"),
    (0x200F, "GetGameFriendListFunc"),
    (0x2010, "DeleteSocialFriendFunc"),
    (0x2011, "DeleteGameFriendFunc"),
    (0x2012, "GetReceivedFriendRequestsFunc"),
    (0x2013, "AcceptFriendRequestFunc"),
    (0x2014, "RejectFriendRequestFunc"),
    (0x2015, "GetSentFriendRequestsFunc"),
    (0x2016, "CancelFriendRequestFunc"),
    (0x2017, "SendFriendRequestFunc"),
    (0x2018, "GetBlockedUserListFunc"),
    (0x2019, "BlockUserFunc"),
    (0x201A, "CancelBlockedUserFunc"),
    (0x201B, "GetSocialChatListFunc"),
    (0x201C, "CreateChatChannelFunc"),
    (0x201D, "InviteChatChannelFunc"),
    (0x201E, "CloseChatChannelFunc"),
    (0x201F, "GetPreviousMessageListFunc"),
    (0x2020, "GetNextMessageListFunc"),
    (0x2021, "SendTalkFunc"),
    (0x2022, "SearchSocialUserFunc"),
    (0x2023, "GetGameTagListFunc"),
    (0x2024, "GetDefaultProfileImageListFunc"),
    (0x2025, "ToggleFavoriteUserFunc"),
    (0x2026, "GetUserProfileFunc"),
    (0x2027, "SendFileFunc"),
    (0x2028, "GetConnectNotificationFunc"),
    (0x2029, "GetSocialMyInfoFunc"),
    (0x202A, "GetChatChannelUserListFunc"),
    (0x202B, "SetReadMarkFunc"),
    (0x202C, "SetSocialNicknameFunc"),
    (0x202D, "SetPlugConnectNotificationFunc"),
    (0x202E, "SetGameConnectNotificationFunc"),
    (0x202F, "SetSocialProfileUrlFunc"),
    (0x2030, "GetUnreadSocialCountFunc"),
    (0x2031, "GetChatChannelInfoFunc"),
    (0x2032, "GetLastSeenTimestampFunc"),
    (0x2033, "SetCharacterNameFunc"),
    (0x2034, "RequestNewsFunc"),
    (0x2035, "CheckNewsFunc"),
    (0x2101, "GetDomainFunc"),
    (0x2103, "SetStatInfoFunc"),
    (0x2104, "SetLocaleFunc"),
    (0x2105, "GetNexonPassportFunc"),
    (0x2106, "GetSecurityQuestionFunc"),
    (0x2107, "GetNxHWIDFunc"),
    (0x2109, "SetCodePageFunc"),
    (0x4001, "GetFriendListFunc"),
    (0x4002, "GetFriendInfoFunc"),
    (0x4003, "RequestNewFriendFunc"),
    (0x4005, "BlockFriendFunc"),
    (0x4006, "ChangeFriendMemoFunc"),
    (0x4007, "AddFriendToCategoryFunc"),
    (0x4008, "DeleteFriendFromCategoryFunc"),
    (0x4009, "MoveFriendCategoryFunc"),
    (0x400A, "ChangeFriendMemoExFunc"),
    (0x4101, "AddCategoryFunc"),
    (0x4102, "DeleteCategoryFunc"),
    (0x4103, "ChangeCategoryNameFunc"),
    (0x4104, "ChangeCategoryPropertyFunc"),
    (0x4105, "ChangeCategoryAllowTypeFunc"),
    (0x5201, "GetRejectedUserListFunc"),
    (0x5202, "AppendRejectedUserFunc"),
    (0x5203, "RemoveRejectedUserFunc"),
    (0x6001, "GetNoteBoxFunc"),
    (0x6002, "SendNoteMsgFunc"),
    (0x6003, "ProcessNoteFunc"),
    (0x6004, "SendNoteInfoFunc"),
    (0x6005, "GetNoteBox2Func"),
    (0x6006, "SendAccuseNoteMsgFunc"),
    (0x7002, "MonitorGuildOnlineInfoFunc"),
    (0x7003, "GetMyGuildListExFunc"),
    (0x7004, "MonitorGuildOnlineInfoExFunc"),
    (0x8001, "ExecutePatcherFunc"),
    (0x8002, "ExecuteLauncherFunc"),
    (0x8003, "ExecuteCommonFunc"),
    (0x8004, "GetUrlFunc"),
    (0x8005, "GetPathFunc"),
    (0x8006, "SetSessionValueFunc"),
    (0x8007, "GetSessionValueFunc"),
    (0x8008, "GetGameListFunc"),
    (0x8009, "GetUrlExFunc"),
    (0x800A, "DownloadFileFunc"),
    (0x800B, "UploadFileFunc"),
    (0x800C, "GetSupportGameListFunc"),
    (0x800D, "GetGameServerListFunc"),
    (0x800E, "GetGameFullNameFunc"),
    (0x800F, "GetGameShortNameFunc"),
    (0x8010, "GetGameFriendTitleFunc"),
    (0x8011, "GetGameServerNameFunc"),
    (0x8012, "GetConnConfigFunc"),
    (0x8013, "SetConnConfigFunc"),
    (0x8015, "ExecuteNGMPatcherFunc"),
    (0x8016, "ExecuteNGMLauncherFunc"),
    (0x8017, "IsNGMInstalledFunc"),
    (0x8018, "RestrictedWordsFunc"),
    (0x8019, "MinimizeAllFunc"),
    (0x801A, "LogReportSyncFunc"),
    (0x801C, "SetServerInfoFunc"),
    (0x801E, "GetServerInfoFunc"),
    (0x801F, "WriteToWiselogFunc"),
    (0x8020, "ExecuteNGMInstallerFunc"),
    (0x8021, "OpenGameSummaryFunc"),
    (0x8022, "ExecuteNGMPatcher2Func"),
    (0x9001, "GetUserInfoFunc"),
    (0x9002, "FindUserFunc"),
    (0x9003, "GetFindUserResultFunc"),
    (0x9004, "SendNoteExMsgFunc"),
    (0x9006, "CustomNotifyFunc"),
    (0xA001, "RequestChatSessionFunc"),
    (0xA002, "RequestMultiChatSessionFunc"),
    (0xA003, "RequestFileUploadSessionFunc"),
    (0xA004, "RequestWBSessionFunc"),
    (0xA006, "CreateChatRoomFunc"),
    (0xA008, "RequestSessionToOtherFunc"),
    (0xA009, "CreateChatRoomExFunc"),
    (0xA00A, "JoinToChatRoomExFunc"),
    (0xA00B, "RequestChatSessionExFunc"),
    (0xA00C, "RequestMultiChatSessionExFunc"),
    (0xA00D, "RequestFileUploadSessionExFunc"),
    (0xA00E, "RequestWBSessionExFunc"),
    (0xA010, "CreateChannelFunc"),
    (0xA011, "JoinChannelFunc"),
    (0xB001, "CRRegisterCallbackFunc"),
    (0xB002, "CRCloseFunc"),
    (0xB003, "CRGetRoomInfoFunc"),
    (0xB004, "CRChangeRoomInfoFunc"),
    (0xB005, "CRChangeMyInfoFunc"),
    (0xB006, "CRGetMemberListFunc"),
    (0xB007, "CRInviteUserFunc"),
    (0xB008, "CRBanUserFunc"),
    (0xB009, "CRSendChatMessageFunc"),
    (0xB00A, "CRGetMemberInfoFunc"),
    (0xB00B, "CRGetRoomInfoExFunc"),
    (0xC001, "GSRegisterCallbackFunc"),
    (0xC002, "GSWantCloseFunc"),
    (0xC003, "GSGetSessionInfoFunc"),
    (0xC004, "GSSetServingProcessIDFunc"),
    (0xC005, "GSGetMemberListFunc"),
    (0xC006, "GSInviteUserFunc"),
    (0xC007, "GSGetInviteCandidateListFunc"),
    (0xC008, "GSInviteUserExFunc"),
    (0xC101, "CSSendChatMessageFunc"),
    (0xC201, "FUDSGetFileEventFunc"),
    (0xC202, "FUSSendFileFunc"),
    (0xC203, "FUSControlFunc"),
    (0xC301, "FDSControlFunc"),
    (0xC302, "FDSGetDownloadDirFunc"),
    (0xC303, "FDSSetDownloadDirFunc"),
    (0xC401, "WSSendWBMessageFunc"),
    (0xC402, "WSGetAssocSerialKeyFunc"),
    (0xD001, "CHCloseFunc"),
    (0xD002, "CHGetChannelInfoFunc"),
    (0xD003, "CHChangeChannelInfoFunc"),
    (0xD004, "CHGetMemberInfoListFunc"),
    (0xD005, "CHChangeMyInfoFunc"),
    (0xD006, "CHInviteUserFunc"),
    (0xD007, "CHBanUserFunc"),
    (0xD008, "CHSendChannelMessageFunc"),
    (0xF000, "CharacterLoginFunc"),
    (0xF001, "CharacterLogoutFunc"),
    (0xF002, "CharacterRemoveFunc"),
    (0xF003, "CharacterChangeNameFunc"),
    (0xF004, "CharacterLogin64Func"),
    (0xF005, "CharacterLogout64Func"),
    (0xF006, "CharacterRemove64Func"),
    (0xF007, "CharacterChangeName64Func"),
    (0xF008, "CharacterSyncFunc"),
    (0xF009, "CharacterSync64Func"),
    (0xF010, "GameLogInitializeFunc"),
    (0xF011, "GameLogFinalizeFunc"),
    (0xF012, "GameLogWriteStageLogFunc"),
    (0xF013, "GameLogWriteErrorLogFunc"),
    (0xF014, "GameLogGetSessionIDFunc"),
];

pub const NM_EVENT_NAMES: &[(u32, &str)] = &[
    (0x1001, "MessengerReplyEvent"),
    (0x1002, "MsgConnectionClosedEvent"),
    (0x1003, "RefreshEvent"),
    (0x1004, "SpecialEvent"),
    (0x1005, "RequestNewFriendEvent"),
    (0x1006, "ServerMessageEvent"),
    (0x1007, "CustomMessageEvent"),
    (0x1008, "NoteInstantMsgEvent"),
    (0x1009, "RefreshMessageEvent"),
    (0x100A, "FindUserReplyEvent"),
    (0x100B, "InviteVirtualUserEvent"),
    (0x100C, "UserInfoEvent"),
    (0x100D, "GuildOnlineInfoEvent"),
    (0x100E, "RealFriendInfoChangedEvent"),
    (0x100F, "VirtualFriendInfoChangedEvent"),
    (0x1010, "FriendInfoChangedEvent"),
    (0x1011, "NotifyEvent"),
    (0x1012, "GuildOnlineInfoExEvent"),
    (0x1013, "GuildChatMessageEvent"),
    (0x1014, "CustomNotifyEvent"),
    (0x1015, "RejectedUserListChangedEvent"),
    (0x1016, "NoteInfoEvent"),
    (0x1017, "AuthConnectionClosedEvent"),
    (0x1018, "AuthSecondaryConnectionClosedEvent"),
    (0x8001, "SocialFriendListEvent"),
    (0x8002, "SocialFriendChangedEvent"),
    (0x8003, "SentFriendRequestEvent"),
    (0x8004, "ReceivedFriendRequestEvent"),
    (0x8005, "SocialBlockEvent"),
    (0x8008, "SocialChatMessageEvent"),
    (0x8009, "SocialChatMemberEvent"),
    (0x800A, "SocialPushPacketEvent"),
    (0x800B, "SocialFriendPacketEvent"),
    (0x800C, "SocialChatClosedEvent"),
    (0x800D, "SocialChatInfoChangedEvent"),
    (0x9001, "CRChatRoomCreatedEvent"),
    (0x9003, "CRChatRoomErrorEvent"),
    (0x9004, "CRChatRoomMemberEvent"),
    (0x9005, "CRChatRoomInfoEvent"),
    (0x9006, "CRChatRoomMessageReceivedEvent"),
    (0x9007, "CRChatRoomCreatedExEvent"),
    (0x9008, "CRChatRoomEstablishedExEvent"),
    (0x9009, "CRChatRoomInfoExEvent"),
    (0xA001, "GSSessionCreatedEvent"),
    (0xA002, "GSSessionEstablishedEvent"),
    (0xA003, "GSSessionFailedEvent"),
    (0xA004, "GSSessionClosedEvent"),
    (0xA005, "GSSessionInfoChangedEvent"),
    (0xA006, "GSNewMemberJoinedEvent"),
    (0xA007, "GSMemberLeavedEvent"),
    (0xA008, "GSMemberInfoChangedEvent"),
    (0xA009, "GSSessionInfoEvent"),
    (0xA00A, "GSInviteRejectedEvent"),
    (0xA101, "CSChatMessageReceivedEvent"),
    (0xA102, "CSMultiChatCreatedEvent"),
    (0xA201, "FUSFileUploadEventReceivedEvent"),
    (0xA301, "FDSFileDownloadEventReceivedEvent"),
    (0xA401, "WSWhiteBoardMsgReceivedEvent"),
    (0xA402, "WSWhiteBoardAckReceivedEvent"),
    (0xA403, "WSAssocSerialKeyEvent"),
    (0xB001, "AttendanceEventEvent"),
    (0xC001, "ChannelErrorEvent"),
    (0xC002, "ChannelCreatedEvent"),
    (0xC003, "ChannelEstablishedEvent"),
    (0xC004, "ChannelInfoEvent"),
    (0xC005, "ChannelMemberInfoEvent"),
    (0xC006, "ChannelMemberInfoListEvent"),
    (0xC007, "ChannelMessageEvent"),
    (0xD001, "LogReportSyncEvent"),
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BridgeMessage {
    Hello {
        protocol: u32,
        nickname: String,
        ticket: String,
    },
    Call {
        sequence: u32,
        code: u32,
        object: Vec<u8>,
    },
    Reply {
        sequence: u32,
        result: u32,
        object: Vec<u8>,
    },
    Event {
        code: u32,
        object: Vec<u8>,
    },
}

pub const BRIDGE_PROTOCOL: u32 = 2;
const BRIDGE_HELLO: u8 = 1;
const BRIDGE_CALL: u8 = 2;
const BRIDGE_REPLY: u8 = 3;
const BRIDGE_EVENT: u8 = 4;

impl BridgeMessage {
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let mut writer = NmWriter::new();
        match self {
            Self::Hello {
                protocol,
                nickname,
                ticket,
            } => {
                writer.u8(BRIDGE_HELLO).u32(*protocol).wstr(nickname);
                if !ticket.is_empty() {
                    writer.wstr(ticket);
                }
            }
            Self::Call {
                sequence,
                code,
                object,
            } => {
                writer.u8(BRIDGE_CALL).u32(*sequence).u32(*code);
                writer.bytes.extend_from_slice(object);
            }
            Self::Reply {
                sequence,
                result,
                object,
            } => {
                writer.u8(BRIDGE_REPLY).u32(*sequence).u32(*result);
                writer.bytes.extend_from_slice(object);
            }
            Self::Event { code, object } => {
                writer.u8(BRIDGE_EVENT).u32(*code);
                writer.bytes.extend_from_slice(object);
            }
        }
        writer.into_bytes()
    }

    pub fn decode(body: &[u8]) -> Result<Self, NmError> {
        if body.len() > MAX_NM_OBJECT_BYTES + 64 {
            return Err(NmError::TooLarge(body.len()));
        }
        let mut reader = NmReader::new(body);
        Ok(match reader.u8()? {
            BRIDGE_HELLO => {
                let protocol = reader.u32()?;
                let nickname = reader.wstr()?;
                let ticket = if reader.remaining().is_empty() {
                    String::new()
                } else {
                    reader.wstr()?
                };
                Self::Hello {
                    protocol,
                    nickname,
                    ticket,
                }
            }
            BRIDGE_CALL => Self::Call {
                sequence: reader.u32()?,
                code: reader.u32()?,
                object: reader.remaining().to_vec(),
            },
            BRIDGE_REPLY => Self::Reply {
                sequence: reader.u32()?,
                result: reader.u32()?,
                object: reader.remaining().to_vec(),
            },
            BRIDGE_EVENT => Self::Event {
                code: reader.u32()?,
                object: reader.remaining().to_vec(),
            },
            other => return Err(NmError::BadCodecId(other)),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compact_lengths_roundtrip_at_every_width() {
        for value in [
            0,
            1,
            0x3F,
            0x40,
            0x3FFF,
            0x4000,
            0x3FFF_FFFF,
            0x4000_0000,
            u32::MAX,
        ] {
            let mut writer = NmWriter::new();
            writer.compact(value);
            let bytes = writer.into_bytes();
            let expected_width = match value {
                0..0x40 => 1,
                0x40..0x4000 => 2,
                0x4000..0x4000_0000 => 4,
                _ => 5,
            };
            assert_eq!(bytes.len(), expected_width, "{value:#x}");
            assert_eq!(NmReader::new(&bytes).compact().unwrap(), value);
        }
    }

    #[test]
    fn objects_strings_and_frames_roundtrip() {
        let mut writer = NmWriter::new();
        writer
            .begin(NM_FUNC_CLASS, 1, 0)
            .u32(1)
            .wstr("친구")
            .u64(7)
            .end();
        let object = writer.into_bytes();
        let mut reader = NmReader::new(&object);
        let header = reader.header().unwrap();
        assert_eq!(
            (header.class, header.version, header.flag),
            (NM_FUNC_CLASS, 1, 0)
        );
        assert_eq!(header.length as usize, object.len() - 8);
        assert_eq!(reader.u32().unwrap(), 1);
        assert_eq!(reader.wstr().unwrap(), "친구");
        assert_eq!(reader.u64().unwrap(), 7);

        let frame = encode_plain_frame(&object);
        assert_eq!(decode_frame(&frame).unwrap(), (object.clone(), frame.len()));

        let mut scrambled = SCRAMBLE_CODEC_MAGIC.to_le_bytes().to_vec();
        let payload = scramble_encode(&object, 0x1234_56A7);
        scrambled.extend_from_slice(&length_u32(payload.len()).to_le_bytes());
        scrambled.extend_from_slice(&payload);
        scrambled.extend_from_slice(&SCRAMBLE_CODEC_TRAILER.to_le_bytes());
        assert_eq!(payload.len(), object.len() + 10);
        assert_eq!(decode_frame(&scrambled).unwrap().0, object);

        let mut corrupt = payload.clone();
        corrupt[4] ^= 1;
        assert_eq!(scramble_decode(&corrupt), Err(NmError::BadChecksum));
    }

    #[test]
    fn class_name_tables_are_sorted_and_resolve() {
        assert!(NM_FUNC_NAMES.windows(2).all(|pair| pair[0].0 < pair[1].0));
        assert!(NM_EVENT_NAMES.windows(2).all(|pair| pair[0].0 < pair[1].0));
        assert_eq!(nm_func_name(0x4001), Some("GetFriendListFunc"));
        assert_eq!(nm_event_name(0x1010), Some("FriendInfoChangedEvent"));
        assert_eq!(nm_func_name(0x7777), None);
    }

    #[test]
    fn bridge_messages_roundtrip() {
        for message in [
            BridgeMessage::Hello {
                protocol: BRIDGE_PROTOCOL,
                nickname: "rider".to_owned(),
                ticket: "00ff".to_owned(),
            },
            BridgeMessage::Hello {
                protocol: 1,
                nickname: "legacy".to_owned(),
                ticket: String::new(),
            },
            BridgeMessage::Call {
                sequence: 9,
                code: 0x4001,
                object: vec![1, 2, 3],
            },
            BridgeMessage::Reply {
                sequence: 9,
                result: 1,
                object: vec![4],
            },
            BridgeMessage::Event {
                code: 0x1003,
                object: vec![],
            },
        ] {
            assert_eq!(BridgeMessage::decode(&message.encode()).unwrap(), message);
        }
    }
}
