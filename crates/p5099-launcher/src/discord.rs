use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use discord_rich_presence::{
    DiscordIpc, DiscordIpcClient,
    activity::{Activity, Assets, Party, Timestamps},
};
use p5099_connector::{DEFAULT_ACCOUNT_TIMEOUT, LaunchTicket, account_presence};
use p5099_core::account_protocol::PlayerPresence;

use crate::server::ServerAddress;

const DISCORD_APPLICATION_ID: Option<&str> = match option_env!("P5099_DISCORD_APPLICATION_ID") {
    Some(id) if !id.is_empty() => Some(id),
    _ => None,
};
pub const SUPPORTED: bool = DISCORD_APPLICATION_ID.is_some();
const LARGE_IMAGE_KEY: &str = "icon";
const POLL_INTERVAL: Duration = Duration::from_secs(15);

#[derive(Debug, Clone, PartialEq, Eq)]
struct Status {
    details: String,
    state: Option<String>,
    party: Option<[i32; 2]>,
}

fn describe(presence: &PlayerPresence) -> Option<Status> {
    match presence {
        PlayerPresence::Offline => None,
        PlayerPresence::Lobby => Some(Status {
            details: "로비".to_owned(),
            state: None,
            party: None,
        }),
        PlayerPresence::Room(room) => Some(Status {
            details: format!(
                "{} · {}",
                room.mode_label(),
                if room.racing {
                    "주행 중"
                } else {
                    "대기 중"
                }
            ),
            state: Some(if room.name.trim().is_empty() {
                "방".to_owned()
            } else {
                room.name.clone()
            }),
            party: Some([
                i32::from(room.players.max(1)),
                i32::from(room.capacity.max(room.players).max(1)),
            ]),
        }),
    }
}

fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|elapsed| i64::try_from(elapsed.as_secs()).ok())
        .unwrap_or(0)
}

pub struct DiscordPresence {
    stop: Arc<AtomicBool>,
}

impl Drop for DiscordPresence {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

pub fn spawn(server: ServerAddress, ticket: LaunchTicket) -> DiscordPresence {
    let stop = Arc::new(AtomicBool::new(false));
    let worker_stop = Arc::clone(&stop);
    if let Some(application_id) = DISCORD_APPLICATION_ID {
        let _ = thread::Builder::new()
            .name("p5099-discord".to_owned())
            .spawn(move || run(application_id, &server, &ticket, &worker_stop));
    }
    DiscordPresence { stop }
}

struct Publisher {
    application_id: &'static str,
    client: Option<DiscordIpcClient>,
    shown: Option<Status>,
    since: i64,
}

impl Publisher {
    fn client(&mut self) -> Option<&mut DiscordIpcClient> {
        if self.client.is_none() {
            let mut client = DiscordIpcClient::new(self.application_id);
            if client.connect().is_ok() {
                self.client = Some(client);
                self.shown = None;
            }
        }
        self.client.as_mut()
    }

    fn show(&mut self, status: Option<Status>, nickname: &str) {
        if status == self.shown && self.client.is_some() {
            return;
        }
        let changed_place = status
            .as_ref()
            .map(|status| (&status.details, &status.state))
            != self
                .shown
                .as_ref()
                .map(|shown| (&shown.details, &shown.state));
        if changed_place {
            self.since = unix_now();
        }
        let since = self.since;
        let Some(client) = self.client() else {
            return;
        };
        let result = match &status {
            None => client.clear_activity(),
            Some(status) => {
                let mut activity = Activity::new()
                    .details(status.details.as_str())
                    .timestamps(Timestamps::new().start(since))
                    .assets(
                        Assets::new()
                            .large_image(LARGE_IMAGE_KEY)
                            .large_text(nickname),
                    );
                if let Some(state) = &status.state {
                    activity = activity.state(state.as_str());
                }
                if let Some(size) = status.party {
                    activity = activity.party(Party::new().id("p5099-room").size(size));
                }
                client.set_activity(activity)
            }
        };
        if result.is_ok() {
            self.shown = status;
        } else {
            self.client = None;
        }
    }

    fn close(&mut self) {
        if let Some(mut client) = self.client.take() {
            let _ = client.clear_activity();
            let _ = client.close();
        }
    }
}

fn run(
    application_id: &'static str,
    server: &ServerAddress,
    ticket: &LaunchTicket,
    stop: &AtomicBool,
) {
    let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    else {
        return;
    };
    let mut publisher = Publisher {
        application_id,
        client: None,
        shown: None,
        since: unix_now(),
    };
    while !stop.load(Ordering::Relaxed) {
        let presence = runtime.block_on(async {
            account_presence(
                server.resolve().await?,
                server.ports(),
                ticket,
                DEFAULT_ACCOUNT_TIMEOUT,
            )
            .await
            .map_err(anyhow::Error::from)
        });
        if let Ok(presence) = presence {
            publisher.show(describe(&presence), &ticket.nickname);
        }
        let mut waited = Duration::ZERO;
        while waited < POLL_INTERVAL && !stop.load(Ordering::Relaxed) {
            thread::sleep(Duration::from_millis(250));
            waited += Duration::from_millis(250);
        }
    }
    publisher.close();
}

#[cfg(test)]
mod tests {
    use p5099_core::account_protocol::RoomPresenceInfo;

    use super::*;

    #[test]
    fn rooms_show_mode_name_and_party() {
        let status = describe(&PlayerPresence::Room(RoomPresenceInfo {
            name: "초보만".to_owned(),
            game_type: 4,
            speed_type: 7,
            racing: true,
            players: 5,
            capacity: 8,
        }))
        .unwrap();
        assert_eq!(status.details, "아이템 팀전 · 주행 중");
        assert_eq!(status.state.as_deref(), Some("초보만"));
        assert_eq!(status.party, Some([5, 8]));
        assert_eq!(describe(&PlayerPresence::Lobby).unwrap().details, "로비");
        assert_eq!(describe(&PlayerPresence::Offline), None);
    }
}

#[cfg(test)]
mod live {
    use super::*;

    #[test]
    #[ignore = "needs a running Discord app and P5099_DISCORD_APPLICATION_ID"]
    fn discord_accepts_the_application_id() {
        let application_id =
            DISCORD_APPLICATION_ID.expect("build with P5099_DISCORD_APPLICATION_ID");
        let mut client = DiscordIpcClient::new(application_id);
        client.connect().expect("Discord handshake");
        client.close().unwrap();
    }
}
