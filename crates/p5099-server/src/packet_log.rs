use std::{
    fmt,
    net::SocketAddr,
    sync::{
        LazyLock, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Instant,
};

use p5099_core::adler32::packet_hash;

pub(crate) const MAX_PACKET_LOG_BYTES: usize = 4 * 1_024;
const MAX_PACKET_LOG_RECORDS_PER_SECOND: usize = 512;
const MAX_SESSION_FAILURE_LOG_RECORDS_PER_SECOND: usize = 64;

static PAYLOADS: AtomicBool = AtomicBool::new(false);

const SECRET_PACKETS: [&str; 10] = [
    "PqCnAuthenLogin",
    "PqLogin",
    "PqChangeRoomInfoPacket",
    "ChCreateRoomRequestPacket",
    "ChJoinRoomRequestPacket",
    "ChRqEnterMyRoomPacket",
    "ChReRqEnterMyRoomPacket",
    "ChCmdPwEnterMyRoomPacket",
    "ChRqMyroomCheckPassEtcPacket",
    "RmNotiMyRoomInfoPacket",
];

static SECRET_HASHES: LazyLock<Vec<u32>> = LazyLock::new(|| {
    SECRET_PACKETS
        .iter()
        .map(|name| packet_hash(name))
        .collect()
});

pub(crate) fn set_payload_logging(enabled: bool) {
    PAYLOADS.store(enabled, Ordering::Relaxed);
}

fn payload_visible(
    transport: &str,
    kind: &str,
    direction: PacketDirection,
    first_word: Option<u32>,
) -> bool {
    if !PAYLOADS.load(Ordering::Relaxed) {
        return false;
    }
    if transport == "login-tcp" && kind != "logical" && direction == PacketDirection::Received {
        return false;
    }
    !(kind == "logical" && first_word.is_some_and(|word| SECRET_HASHES.contains(&word)))
}

static PACKET_LOG_BUDGET: LazyLock<Mutex<PacketLogBudget>> =
    LazyLock::new(|| Mutex::new(PacketLogBudget::new(Instant::now())));
static SESSION_FAILURE_LOG_BUDGET: LazyLock<Mutex<PacketLogBudget>> =
    LazyLock::new(|| Mutex::new(PacketLogBudget::new(Instant::now())));

struct PacketLogBudget {
    window_started: Instant,
    records: usize,
    suppressed: usize,
}

impl PacketLogBudget {
    const fn new(window_started: Instant) -> Self {
        Self {
            window_started,
            records: 0,
            suppressed: 0,
        }
    }

    fn take_slot(&mut self, now: Instant, maximum_records: usize) -> (bool, usize) {
        let mut prior_suppressed = 0;
        if now.duration_since(self.window_started).as_secs() >= 1 {
            self.window_started = now;
            self.records = 0;
            prior_suppressed = std::mem::take(&mut self.suppressed);
        }
        if self.records < maximum_records {
            self.records += 1;
            (true, prior_suppressed)
        } else {
            self.suppressed = self.suppressed.saturating_add(1);
            (false, prior_suppressed)
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum PacketDirection {
    Received,
    Sent,
}

impl PacketDirection {
    const fn label(self) -> &'static str {
        match self {
            Self::Received => "received",
            Self::Sent => "sent",
        }
    }
}

pub(crate) fn trace_packet(
    transport: &'static str,
    kind: &'static str,
    direction: PacketDirection,
    peer: Option<SocketAddr>,
    bytes: &[u8],
) {
    let (permitted, prior_suppressed) = PACKET_LOG_BUDGET
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .take_slot(Instant::now(), MAX_PACKET_LOG_RECORDS_PER_SECOND);
    if prior_suppressed != 0 {
        tracing::warn!(
            target: "p5099_packet",
            dropped_records = prior_suppressed,
            maximum_records_per_second = MAX_PACKET_LOG_RECORDS_PER_SECOND,
            "packet diagnostics were rate-limited"
        );
    }
    if !permitted {
        return;
    }
    let captured = bytes.len().min(MAX_PACKET_LOG_BYTES);
    let first_word_le = bytes
        .get(..4)
        .map(|header| u32::from_le_bytes([header[0], header[1], header[2], header[3]]));
    tracing::debug!(
        target: "p5099_packet",
        transport,
        kind,
        direction = direction.label(),
        peer = ?peer,
        bytes = bytes.len(),
        captured_bytes = captured,
        truncated = bytes.len() > captured,
        first_word_le = ?first_word_le.map(Hash),
        raw_hex = %if payload_visible(transport, kind, direction, first_word_le) {
            HexPreview(&bytes[..captured]).to_string()
        } else {
            "<omitted>".to_owned()
        },
        "P5099 packet"
    );
}

#[derive(Clone, Copy)]
pub(crate) struct SessionFailure<'a> {
    pub(crate) transport: &'static str,
    pub(crate) peer: Option<SocketAddr>,
    pub(crate) stage: &'static str,
    pub(crate) request_hash: Option<&'a str>,
    pub(crate) request_bytes: Option<usize>,
    pub(crate) response_count: Option<usize>,
    pub(crate) authenticated: Option<bool>,
}

impl SessionFailure<'_> {
    pub(crate) const fn terminal(transport: &'static str, peer: Option<SocketAddr>) -> Self {
        Self {
            transport,
            peer,
            stage: "session terminal error",
            request_hash: None,
            request_bytes: None,
            response_count: None,
            authenticated: None,
        }
    }
}

pub(crate) fn trace_session_failure<E>(failure: SessionFailure<'_>, error: &E)
where
    E: std::error::Error + ?Sized,
{
    let (permitted, prior_suppressed) = SESSION_FAILURE_LOG_BUDGET
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .take_slot(Instant::now(), MAX_SESSION_FAILURE_LOG_RECORDS_PER_SECOND);
    if prior_suppressed != 0 {
        tracing::debug!(
            target: "p5099_packet",
            kind = "session-failure",
            dropped_records = prior_suppressed,
            maximum_records_per_second = MAX_SESSION_FAILURE_LOG_RECORDS_PER_SECOND,
            "session-failure diagnostics were rate-limited"
        );
    }
    if !permitted {
        return;
    }
    tracing::debug!(
        target: "p5099_packet",
        transport = failure.transport,
        kind = "session-failure",
        peer = ?failure.peer,
        stage = failure.stage,
        request_hash = ?failure.request_hash,
        request_bytes = ?failure.request_bytes,
        response_count = ?failure.response_count,
        authenticated = ?failure.authenticated,
        error = %error,
        error_debug = ?error,
        "P5099 TCP session failure"
    );
}

struct Hash(u32);

impl fmt::Debug for Hash {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "0x{:08X}", self.0)
    }
}

struct HexPreview<'a>(&'a [u8]);

impl fmt::Display for HexPreview<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        const HEX: &[u8; 16] = b"0123456789ABCDEF";
        let mut encoded = String::with_capacity(self.0.len() * 2);
        for byte in self.0 {
            encoded.push(char::from(HEX[usize::from(byte >> 4)]));
            encoded.push(char::from(HEX[usize::from(byte & 0x0f)]));
        }
        formatter.write_str(&encoded)
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use p5099_core::adler32::packet_hash;

    use super::{
        HexPreview, MAX_PACKET_LOG_RECORDS_PER_SECOND, PacketDirection, PacketLogBudget,
        payload_visible, set_payload_logging,
    };

    #[test]
    fn payloads_stay_out_unless_enabled_and_secrets_stay_out_always() {
        let login = Some(packet_hash("PqLogin"));
        let room_list = Some(packet_hash("ChGetRoomListRequestPacket"));
        let received = PacketDirection::Received;
        set_payload_logging(false);
        assert!(!payload_visible(
            "login-tcp",
            "logical",
            received,
            room_list
        ));
        assert!(!payload_visible("messenger-tcp", "wire", received, None));
        set_payload_logging(true);
        assert!(payload_visible("login-tcp", "logical", received, room_list));
        assert!(!payload_visible("login-tcp", "logical", received, login));
        assert!(!payload_visible(
            "login-tcp",
            "logical",
            PacketDirection::Sent,
            login
        ));
        assert!(!payload_visible("login-tcp", "wire", received, None));
        assert!(payload_visible(
            "login-tcp",
            "wire",
            PacketDirection::Sent,
            None
        ));
        assert!(payload_visible("messenger-tcp", "wire", received, None));
        set_payload_logging(false);
    }

    #[test]
    fn hex_preview_uses_uppercase_compact_hex() {
        assert_eq!(HexPreview(&[0, 10, 255]).to_string(), "000AFF");
    }

    #[test]
    fn rate_budget_limits_one_window_and_reports_prior_drops() {
        let start = Instant::now();
        let mut budget = PacketLogBudget::new(start);
        for _ in 0..MAX_PACKET_LOG_RECORDS_PER_SECOND {
            assert_eq!(
                budget.take_slot(start, MAX_PACKET_LOG_RECORDS_PER_SECOND),
                (true, 0)
            );
        }
        assert_eq!(
            budget.take_slot(start, MAX_PACKET_LOG_RECORDS_PER_SECOND),
            (false, 0)
        );
        assert_eq!(
            budget.take_slot(
                start + Duration::from_secs(1),
                MAX_PACKET_LOG_RECORDS_PER_SECOND
            ),
            (true, 1)
        );
    }
}
