use std::{
    collections::BTreeSet,
    fmt::Write as _,
    path::{Path, PathBuf},
    sync::{Arc, PoisonError, RwLock},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};
use serenity::{
    builder::{CreateAllowedMentions, CreateMessage, CreateThread, EditMessage},
    gateway::ActivityData,
    http::{Http, StatusCode},
    model::{
        channel::{AutoArchiveDuration, MessageFlags},
        id::{ChannelId, GenericChannelId, MessageId},
        user::OnlineStatus,
    },
    prelude::{Context, SerenityError},
};
use tokio::sync::{Mutex, Notify};
use tracing::{info, warn};

use crate::{
    console::{Console, ConsoleError, Player, Room, Status},
    format::{elapsed, escape},
    views::{self, Components, Tone},
};

const FAILURES_BEFORE_OFFLINE: u32 = 2;
const ALERT_THREAD_NAME: &str = "서버 알림";
const MAX_ALERT_NAMES: usize = 30;
const REFRESH_COOLDOWN: Duration = Duration::from_secs(5);
pub const POLL_CHOICES: [u64; 6] = [10, 15, 30, 60, 120, 300];
const DEFAULT_POLL_SECONDS: u64 = 30;

pub fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs())
}

#[derive(Debug, Clone)]
pub struct Snapshot {
    pub status: Status,
    pub players: Vec<Player>,
    pub rooms: Vec<Room>,
    pub started: u64,
}

impl Snapshot {
    pub async fn fetch(console: &Console) -> Result<Self, ConsoleError> {
        let (status, players, rooms) =
            tokio::try_join!(console.status(), console.players(), console.rooms())?;
        Ok(Self {
            started: status.started(),
            status,
            players,
            rooms,
        })
    }
}

#[derive(Debug, Clone)]
pub enum Health {
    Starting,
    Online(Arc<Snapshot>),
    Offline { since: u64, error: String },
}

#[derive(Debug, Clone)]
pub struct Board {
    pub health: Health,
    pub changed: u64,
    key: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Dashboard {
    channel_id: GenericChannelId,
    message_id: MessageId,
    #[serde(default)]
    thread_id: Option<GenericChannelId>,
    #[serde(skip)]
    shown: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub alert_channel: Option<GenericChannelId>,
    pub audit_channel: Option<GenericChannelId>,
    pub join_alerts: bool,
    pub poll_seconds: u64,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            alert_channel: None,
            audit_channel: None,
            join_alerts: false,
            poll_seconds: DEFAULT_POLL_SECONDS,
        }
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct StateFile {
    #[serde(default)]
    dashboards: Vec<Dashboard>,
    #[serde(default)]
    settings: Settings,
}

#[derive(Debug, Default)]
struct Cycle {
    failures: u32,
    first_failure: u64,
    last: Option<Instant>,
}

pub struct Monitor {
    console: Console,
    state_file: PathBuf,
    board: RwLock<Board>,
    cycle: Mutex<Cycle>,
    dashboards: Mutex<Vec<Dashboard>>,
    settings: RwLock<Settings>,
    wake: Notify,
    context: RwLock<Option<Context>>,
    presence: std::sync::Mutex<Option<String>>,
}

impl Monitor {
    pub async fn new(console: Console, state_file: PathBuf) -> Self {
        let StateFile {
            dashboards,
            mut settings,
        } = load_state(&state_file).await;
        settings.poll_seconds = settings.poll_seconds.max(POLL_CHOICES[0]);
        info!(count = dashboards.len(), ?settings, "loaded state");
        let board = Board {
            key: board_key(&Health::Starting),
            health: Health::Starting,
            changed: unix_now(),
        };
        Self {
            console,
            state_file,
            board: RwLock::new(board),
            cycle: Mutex::new(Cycle::default()),
            dashboards: Mutex::new(dashboards),
            settings: RwLock::new(settings),
            wake: Notify::new(),
            context: RwLock::new(None),
            presence: std::sync::Mutex::new(None),
        }
    }

    pub fn console(&self) -> &Console {
        &self.console
    }

    pub fn settings(&self) -> Settings {
        self.settings
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub fn poll_seconds(&self) -> u64 {
        self.settings().poll_seconds
    }

    pub async fn update_settings(&self, change: impl FnOnce(&mut Settings)) {
        let dashboards = self.dashboards.lock().await;
        let poll_before = self.poll_seconds();
        change(
            &mut self
                .settings
                .write()
                .unwrap_or_else(PoisonError::into_inner),
        );
        self.save(&dashboards).await;
        drop(dashboards);
        if self.poll_seconds() != poll_before {
            self.wake.notify_one();
        }
    }

    async fn save(&self, dashboards: &[Dashboard]) {
        save_state(&self.state_file, dashboards, self.settings()).await;
    }

    pub fn board(&self) -> Board {
        self.board
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub fn set_context(&self, context: Context) {
        *self.context.write().unwrap_or_else(PoisonError::into_inner) = Some(context);
        *self.presence.lock().unwrap_or_else(PoisonError::into_inner) = None;
        self.update_presence(&self.board().health);
    }

    pub async fn run(self: Arc<Self>, http: Arc<Http>) {
        loop {
            self.poll(&http).await;
            tokio::select! {
                () = tokio::time::sleep(Duration::from_secs(self.poll_seconds())) => {}
                () = self.wake.notified() => {}
            }
        }
    }

    pub async fn poll(&self, http: &Http) {
        let mut cycle = self.cycle.lock().await;
        cycle.last = Some(Instant::now());
        let previous = self.board().health;
        let fetched = Snapshot::fetch(&self.console).await;
        let health = match fetched {
            Ok(mut snapshot) => {
                cycle.failures = 0;
                if let Health::Online(old) = &previous
                    && snapshot.started.abs_diff(old.started) <= 2
                {
                    snapshot.started = old.started;
                }
                Health::Online(Arc::new(snapshot))
            }
            Err(error) => {
                if cycle.failures == 0 {
                    cycle.first_failure = unix_now();
                }
                cycle.failures += 1;
                warn!(%error, failures = cycle.failures, "console poll failed");
                match &previous {
                    Health::Online(_) if cycle.failures < FAILURES_BEFORE_OFFLINE => {
                        previous.clone()
                    }
                    _ => Health::Offline {
                        since: cycle.first_failure,
                        error: error.to_string(),
                    },
                }
            }
        };
        drop(cycle);

        let key = board_key(&health);
        let board = {
            let mut board = self.board.write().unwrap_or_else(PoisonError::into_inner);
            if board.key != key {
                board.key = key;
                board.changed = unix_now();
            }
            board.health = health.clone();
            board.clone()
        };

        self.update_presence(&health);
        for alert in alerts(&previous, &health, self.settings().join_alerts) {
            self.alert(http, alert).await;
        }
        self.update_dashboards(http, &board).await;
    }

    pub async fn refresh(&self, http: &Http) {
        let recent = self
            .cycle
            .lock()
            .await
            .last
            .is_some_and(|last| last.elapsed() < REFRESH_COOLDOWN);
        if !recent {
            self.poll(http).await;
        }
    }

    pub async fn dashboard_messages(&self) -> Vec<MessageId> {
        self.dashboards
            .lock()
            .await
            .iter()
            .map(|dashboard| dashboard.message_id)
            .collect()
    }

    fn update_presence(&self, health: &Health) {
        let (text, status) = match health {
            Health::Starting => return,
            Health::Offline { .. } => ("서버 응답 없음".to_owned(), OnlineStatus::DoNotDisturb),
            Health::Online(snapshot) if snapshot.status.maintenance => (
                format!("점검 중 · 접속자 {}", snapshot.status.players_online),
                OnlineStatus::Idle,
            ),
            Health::Online(snapshot) => (
                format!(
                    "접속자 {} · 방 {}",
                    snapshot.status.players_online, snapshot.status.rooms
                ),
                OnlineStatus::Online,
            ),
        };
        let mut shown = self.presence.lock().unwrap_or_else(PoisonError::into_inner);
        if shown.as_deref() == Some(text.as_str()) {
            return;
        }
        if let Some(context) = self
            .context
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()
        {
            context.set_presence(Some(ActivityData::custom(text.clone())), status);
            *shown = Some(text);
        }
    }

    async fn update_dashboards(&self, http: &Http, board: &Board) {
        let mut dashboards = self.dashboards.lock().await;
        let mut removed = false;
        let mut index = 0;
        while index < dashboards.len() {
            let dashboard = &mut dashboards[index];
            if dashboard.shown.as_deref() == Some(board.key.as_str()) {
                index += 1;
                continue;
            }
            let edit = EditMessage::new()
                .components(views::dashboard(&board.health, board.changed))
                .flags(MessageFlags::IS_COMPONENTS_V2);
            match dashboard
                .channel_id
                .edit_message(http, dashboard.message_id, edit)
                .await
            {
                Ok(_) => {
                    dashboard.shown = Some(board.key.clone());
                    index += 1;
                }
                Err(error) if is_gone(&error) => {
                    info!(
                        channel = %dashboard.channel_id,
                        message = %dashboard.message_id,
                        "dashboard message is gone; forgetting it"
                    );
                    dashboards.remove(index);
                    removed = true;
                }
                Err(error) => {
                    warn!(%error, channel = %dashboard.channel_id, "failed to update a dashboard");
                    index += 1;
                }
            }
        }
        if removed {
            self.save(&dashboards).await;
        }
    }

    pub async fn create_dashboard(
        &self,
        http: &Http,
        channel_id: GenericChannelId,
    ) -> Result<Option<GenericChannelId>, SerenityError> {
        if matches!(self.board().health, Health::Starting) {
            self.poll(http).await;
        }
        let mut dashboards = self.dashboards.lock().await;
        let board = self.board();
        let message = channel_id
            .send_message(
                http,
                CreateMessage::new()
                    .components(views::dashboard(&board.health, board.changed))
                    .flags(MessageFlags::IS_COMPONENTS_V2)
                    .allowed_mentions(CreateAllowedMentions::new()),
            )
            .await?;
        let thread = ChannelId::new(channel_id.get())
            .create_thread_from_message(
                http,
                message.id,
                CreateThread::new(ALERT_THREAD_NAME)
                    .auto_archive_duration(AutoArchiveDuration::OneWeek),
            )
            .await
            .map(|thread| thread.id.widen())
            .inspect_err(|error| warn!(%error, %channel_id, "failed to open the alert thread"))
            .ok();
        dashboards.push(Dashboard {
            channel_id,
            message_id: message.id,
            thread_id: thread,
            shown: Some(board.key),
        });
        if thread.is_some() {
            self.settings
                .write()
                .unwrap_or_else(PoisonError::into_inner)
                .alert_channel = thread;
        }
        self.save(&dashboards).await;
        Ok(thread)
    }

    pub async fn remove_dashboards(&self, http: &Http, channel_id: GenericChannelId) -> usize {
        let mut dashboards = self.dashboards.lock().await;
        let (gone, kept): (Vec<_>, Vec<_>) = dashboards
            .drain(..)
            .partition(|dashboard| dashboard.channel_id == channel_id);
        *dashboards = kept;
        {
            let mut settings = self
                .settings
                .write()
                .unwrap_or_else(PoisonError::into_inner);
            if gone.iter().any(|dashboard| {
                dashboard.thread_id.is_some() && dashboard.thread_id == settings.alert_channel
            }) {
                settings.alert_channel = None;
            }
        }
        self.save(&dashboards).await;
        drop(dashboards);
        for dashboard in &gone {
            if let Err(error) = channel_id
                .delete_message(http, dashboard.message_id, Some("대시보드 제거"))
                .await
                && !is_gone(&error)
            {
                warn!(%error, "failed to delete a dashboard message");
            }
            if let Some(thread) = dashboard.thread_id
                && let Err(error) = thread.delete(http, Some("대시보드 제거")).await
                && !is_gone(&error)
            {
                warn!(%error, "failed to delete an alert thread");
            }
        }
        gone.len()
    }

    pub async fn alert(&self, http: &Http, components: Components) {
        let Some(channel) = self.settings().alert_channel else {
            return;
        };
        match try_post(http, channel, components).await {
            Ok(()) => {}
            Err(error) if is_gone(&error) => {
                info!(%channel, "alert channel is gone; turning alerts off");
                self.update_settings(|settings| {
                    if settings.alert_channel == Some(channel) {
                        settings.alert_channel = None;
                    }
                })
                .await;
            }
            Err(error) => warn!(%error, %channel, "failed to post an alert"),
        }
    }
}

pub async fn try_post(
    http: &Http,
    channel: GenericChannelId,
    components: Components,
) -> Result<(), SerenityError> {
    let message = CreateMessage::new()
        .components(components)
        .flags(MessageFlags::IS_COMPONENTS_V2)
        .allowed_mentions(CreateAllowedMentions::new());
    channel.send_message(http, message).await.map(drop)
}

pub async fn post(http: &Http, channel: GenericChannelId, components: Components) {
    if let Err(error) = try_post(http, channel, components).await {
        warn!(%error, %channel, "failed to post a message");
    }
}

fn is_gone(error: &SerenityError) -> bool {
    matches!(error, SerenityError::Http(http) if http.status_code() == Some(StatusCode::NOT_FOUND))
}

fn board_key(health: &Health) -> String {
    serde_json::to_string(&views::dashboard(health, 0)).unwrap_or_default()
}

async fn load_state(path: &Path) -> StateFile {
    match tokio::fs::read(path).await {
        Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_else(|error| {
            warn!(%error, path = %path.display(), "ignoring an unreadable state file");
            StateFile::default()
        }),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => StateFile::default(),
        Err(error) => {
            warn!(%error, path = %path.display(), "failed to read the state file");
            StateFile::default()
        }
    }
}

async fn save_state(path: &Path, dashboards: &[Dashboard], settings: Settings) {
    let state = StateFile {
        dashboards: dashboards.to_vec(),
        settings,
    };
    let Ok(bytes) = serde_json::to_vec_pretty(&state) else {
        return;
    };
    let temporary = path.with_extension("json.tmp");
    let result = async {
        tokio::fs::write(&temporary, bytes).await?;
        tokio::fs::rename(&temporary, path).await
    }
    .await;
    if let Err(error) = result {
        warn!(%error, path = %path.display(), "failed to save the state file");
    }
}

fn names(names: &BTreeSet<&str>) -> String {
    let mut text = names
        .iter()
        .take(MAX_ALERT_NAMES)
        .map(|name| escape(name))
        .collect::<Vec<_>>()
        .join(", ");
    if names.len() > MAX_ALERT_NAMES {
        let _ = write!(text, " 외 {}명", names.len() - MAX_ALERT_NAMES);
    }
    text
}

fn alerts(previous: &Health, current: &Health, join_alerts: bool) -> Vec<Components> {
    match (previous, current) {
        (Health::Online(_), Health::Offline { error, .. }) => vec![views::message(
            Tone::Error,
            format!("**서버 응답 없음**\n-# {}", escape(error)),
        )],
        (Health::Offline { since, .. }, Health::Online(_)) => vec![views::message(
            Tone::Success,
            format!(
                "**서버 복구** · 중단 {}",
                elapsed(unix_now().saturating_sub(*since))
            ),
        )],
        (Health::Online(old), Health::Online(new)) => {
            let mut cards = Vec::new();
            let (before, after) = (&old.status, &new.status);
            if new.started > old.started + 60 || before.version != after.version {
                cards.push(views::message(
                    Tone::Warning,
                    format!("**서버 재시작** · v{}", escape(&after.version)),
                ));
            }
            if before.maintenance != after.maintenance {
                cards.push(if after.maintenance {
                    views::message(Tone::Warning, "**점검 모드 켜짐**")
                } else {
                    views::message(Tone::Success, "**점검 모드 꺼짐**")
                });
            }
            if before.registration_open != after.registration_open {
                cards.push(if after.registration_open {
                    views::message(Tone::Success, "**가입 열림**")
                } else {
                    views::message(Tone::Warning, "**가입 닫힘**")
                });
            }
            if after.dead_letters() > before.dead_letters() {
                cards.push(views::message(
                    Tone::Error,
                    format!(
                        "**보상 저장 실패** {}건",
                        after.dead_letters() - before.dead_letters()
                    ),
                ));
            }
            if join_alerts {
                let was: BTreeSet<&str> = old.players.iter().map(|p| p.nickname.as_str()).collect();
                let now: BTreeSet<&str> = new.players.iter().map(|p| p.nickname.as_str()).collect();
                let joined: BTreeSet<&str> = now.difference(&was).copied().collect();
                let left: BTreeSet<&str> = was.difference(&now).copied().collect();
                let mut lines = Vec::new();
                if !joined.is_empty() {
                    lines.push(format!("접속: {}", names(&joined)));
                }
                if !left.is_empty() {
                    lines.push(format!("퇴장: {}", names(&left)));
                }
                if !lines.is_empty() {
                    cards.push(views::message(Tone::Info, lines.join("\n")));
                }
            }
            cards
        }
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot(version: &str, started: u64, maintenance: bool, players: &[&str]) -> Health {
        let status: Status = serde_json::from_value(serde_json::json!({
            "version": version, "server_time": started + 100, "uptime_seconds": 100,
            "players_online": players.len(), "rooms": 0, "accounts": 0,
            "maintenance": maintenance,
        }))
        .unwrap();
        Health::Online(Arc::new(Snapshot {
            started: status.started(),
            status,
            players: players
                .iter()
                .map(|name| Player {
                    nickname: (*name).to_owned(),
                    user_no: 1,
                    address: "127.0.0.1".to_owned(),
                    room: None,
                    admin: false,
                })
                .collect(),
            rooms: Vec::new(),
        }))
    }

    #[test]
    fn reads_state_files() {
        let old: StateFile =
            serde_json::from_str(r#"{"dashboards":[{"channel_id":"1","message_id":"2"}]}"#)
                .unwrap();
        assert_eq!(old.dashboards.len(), 1);
        assert_eq!(old.settings, Settings::default());
        let settings = Settings {
            alert_channel: Some(GenericChannelId::new(5)),
            audit_channel: None,
            join_alerts: true,
            poll_seconds: 60,
        };
        let text = serde_json::to_string(&StateFile {
            dashboards: Vec::new(),
            settings: settings.clone(),
        })
        .unwrap();
        let back: StateFile = serde_json::from_str(&text).unwrap();
        assert_eq!(back.settings, settings);
    }

    #[test]
    fn announces_changes() {
        let before = snapshot("1.0", 1_000, false, &["a", "b"]);
        assert!(alerts(&before, &before, true).is_empty());
        assert_eq!(
            alerts(&before, &snapshot("1.0", 1_000, true, &["a", "b"]), false).len(),
            1
        );
        assert_eq!(
            alerts(&before, &snapshot("1.1", 9_000, false, &["a", "b"]), false).len(),
            1
        );
        assert_eq!(
            alerts(&before, &snapshot("1.0", 1_000, false, &["a", "c"]), true).len(),
            1
        );
        assert!(alerts(&before, &snapshot("1.0", 1_000, false, &["a", "c"]), false).is_empty());
        let down = Health::Offline {
            since: 1,
            error: "x".to_owned(),
        };
        assert_eq!(alerts(&before, &down, false).len(), 1);
        assert_eq!(alerts(&down, &before, false).len(), 1);
        assert!(alerts(&Health::Starting, &down, false).is_empty());
    }
}
