#![cfg_attr(windows, windows_subsystem = "windows")]

mod credentials;
mod discord;
mod legacy;
mod runtime;
mod self_update;
mod server;
mod theme;
mod updater;

use std::{
    net::Ipv4Addr,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, Sender},
    },
    thread,
    time::Duration,
};

use anyhow::{Context as _, Result, anyhow, bail};
use eframe::egui;
use p5099_connector::{
    AccountError, ConnectorCancellation, ConnectorPlan, ConnectorRequest, ConnectorStage,
    DEFAULT_ACCOUNT_TIMEOUT, DEFAULT_PROBE_TIMEOUT, InstallationOptions, LaunchTicket, Runner,
    SikarugirWrapper, account_key, account_login_with_key, account_register, change_password,
    execute_connector_with_progress_and_cancellation,
};
use p5099_core::{
    account_protocol::{AccountFailure, AccountKey, MAX_PASSWORD_CHARS, MIN_PASSWORD_CHARS},
    nickname::normalize_nickname,
    ports::PortTopology,
};
use serde::{Deserialize, Serialize};
use server::ServerAddress;

const BUILT_IN_SERVER: Option<&str> = match option_env!("P5099_SERVER_HOST") {
    Some(host) if !host.is_empty() => Some(host),
    _ => None,
};

const WINDOW_TITLE: &str = "P5099";
const WINDOW_SIZE: [f32; 2] = [440.0, 640.0];
const STORAGE_ID: &str = "p5099";
const ICON_PNG: &[u8] = include_bytes!("../../../assets/icon-256.png");
const WINDOW_ICON_PNG: &[u8] = if cfg!(target_os = "macos") {
    include_bytes!("../../../assets/icon-macos.png")
} else {
    ICON_PNG
};
const APP_DATA_NAME: &str = "p5099";
const SETTINGS_KEY: &str = "p5099-launcher-settings";
const SERVER_FIELD_ID: &str = "p5099-server-field";
const AVATAR_SIZE: f32 = 44.0;
const MAX_SETTINGS_BYTES: usize = 64 * 1024;

fn main() -> Result<()> {
    self_update::clean_up();
    legacy::migrate_settings(STORAGE_ID);
    let relaunch = Arc::new(Mutex::new(None));
    let mut viewport = egui::ViewportBuilder::default()
        .with_title(WINDOW_TITLE)
        .with_inner_size(WINDOW_SIZE)
        .with_min_inner_size(WINDOW_SIZE)
        .with_resizable(false);
    if let Ok(icon) = eframe::icon_data::from_png_bytes(WINDOW_ICON_PNG) {
        viewport = viewport.with_icon(icon);
    }
    let options = eframe::NativeOptions {
        viewport,
        centered: true,
        ..Default::default()
    };
    eframe::run_native(
        STORAGE_ID,
        options,
        Box::new(|creation_context| {
            configure_platform_fonts(&creation_context.egui_ctx);
            theme::apply(&creation_context.egui_ctx);
            Ok(Box::new(Launcher::new(
                creation_context.storage,
                Arc::clone(&relaunch),
                logo_texture(&creation_context.egui_ctx),
            )))
        }),
    )
    .map_err(|error| anyhow!("접속기 창을 열지 못했습니다: {error}"))?;
    let target = relaunch.lock().ok().and_then(|mut slot| slot.take());
    if let Some(target) = target {
        self_update::relaunch(&target)?;
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum RunnerChoice {
    Auto,
    Native,
    NativeElevated,
    Wine,
    CrossOver,
    Sikarugir,
}

impl RunnerChoice {
    const AVAILABLE: &'static [Self] = if cfg!(windows) {
        &[Self::Auto, Self::Native, Self::NativeElevated]
    } else if cfg!(target_os = "macos") {
        &[Self::Auto, Self::CrossOver, Self::Sikarugir]
    } else {
        &[Self::Auto, Self::Wine, Self::CrossOver]
    };

    const fn label(self) -> &'static str {
        match self {
            Self::Auto if cfg!(target_os = "macos") => "자동 (CrossOver, 없으면 Sikarugir)",
            Self::Auto => "자동",
            Self::Native => "직접 실행 (관리자 권한 없음)",
            Self::NativeElevated => "직접 실행 (Windows UAC)",
            Self::Wine => "Wine",
            Self::CrossOver => "CrossOver",
            Self::Sikarugir => "Sikarugir",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
struct Settings {
    server: String,
    game_directory: String,
    game_executable: String,
    nickname: String,
    runner: RunnerChoice,
    wine_binary: String,
    wine_prefix: String,
    crossover_binary: String,
    crossover_bottle: String,
    sikarugir_app: String,
    discord_presence: bool,
    remember_login: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            server: BUILT_IN_SERVER.unwrap_or_default().to_owned(),
            game_directory: String::new(),
            game_executable: String::new(),
            nickname: String::new(),
            runner: RunnerChoice::Auto,
            wine_binary: "wine".to_owned(),
            wine_prefix: String::new(),
            crossover_binary: runtime::default_crossover_wine().display().to_string(),
            crossover_bottle: "p5099".to_owned(),
            sikarugir_app: String::new(),
            discord_presence: true,
            remember_login: false,
        }
    }
}

impl Settings {
    fn load(storage: Option<&dyn eframe::Storage>) -> Self {
        storage
            .and_then(|storage| storage.get_string(SETTINGS_KEY))
            .filter(|encoded| encoded.len() <= MAX_SETTINGS_BYTES)
            .and_then(|encoded| serde_json::from_str(&encoded).ok())
            .unwrap_or_default()
    }

    fn server(&self) -> Result<ServerAddress> {
        ServerAddress::parse(&self.server)
    }

    fn game_directory(&self) -> Result<PathBuf, String> {
        match optional_path(&self.game_directory) {
            Some(directory) if directory.is_absolute() => Ok(directory),
            Some(directory) => Err(format!(
                "설치 위치는 전체 경로여야 합니다: {}",
                directory.display()
            )),
            None => default_game_directory(),
        }
    }

    fn runner(&self) -> Result<Runner> {
        Ok(match self.runner {
            RunnerChoice::Auto if cfg!(target_os = "macos") => self.macos_runner()?,
            RunnerChoice::Auto => Runner::Auto,
            RunnerChoice::Native => Runner::Native,
            RunnerChoice::NativeElevated => Runner::NativeElevated,
            RunnerChoice::Wine => Runner::Wine {
                binary: required_path(&self.wine_binary, "Wine 실행 파일")?,
                prefix: optional_path(&self.wine_prefix),
            },
            RunnerChoice::CrossOver => self.crossover_runner()?,
            RunnerChoice::Sikarugir => Runner::Sikarugir {
                app: self.sikarugir_wrapper().ok_or_else(|| {
                    anyhow!(
                        "Sikarugir wrapper를 찾지 못했습니다. Sikarugir Creator로 wrapper를 \
                         만든 뒤 설정(⚙)에서 선택하세요."
                    )
                })?,
            },
        })
    }

    fn crossover_runner(&self) -> Result<Runner> {
        Ok(Runner::CrossOver {
            wine_binary: required_path(&self.crossover_binary, "CrossOver 실행 파일")?,
            bottle: required_text(&self.crossover_bottle, "CrossOver 보틀")?.to_owned(),
        })
    }

    fn sikarugir_wrapper(&self) -> Option<PathBuf> {
        optional_path(&self.sikarugir_app)
            .or_else(|| runtime::sikarugir_wrappers().into_iter().next())
    }

    fn macos_runner(&self) -> Result<Runner> {
        if Path::new(self.crossover_binary.trim()).is_file() {
            return self.crossover_runner();
        }
        if let Some(app) = self.sikarugir_wrapper() {
            return Ok(Runner::Sikarugir { app });
        }
        bail!(
            "macOS에서는 CrossOver 또는 Sikarugir가 있어야 게임을 실행할 수 있습니다. \
             CrossOver를 설치하거나, Sikarugir Creator로 wrapper를 만드세요."
        )
    }

    fn runtime_summary(&self) -> Result<String, String> {
        match self.runner().map_err(|error| format!("{error:#}"))? {
            Runner::CrossOver { bottle, .. } => Ok(format!("CrossOver · 보틀 {bottle}")),
            Runner::Sikarugir { app } if SikarugirWrapper::new(&app).is_valid() => {
                Ok(format!("Sikarugir · {}", runtime::wrapper_label(&app)))
            }
            Runner::Sikarugir { app } => Err(format!(
                "Sikarugir wrapper가 올바르지 않습니다: {}",
                app.display()
            )),
            Runner::Wine { .. } => Ok("Wine".to_owned()),
            Runner::Auto | Runner::Native | Runner::NativeElevated => Ok(String::new()),
        }
    }

    fn connector_plan(
        &self,
        game_directory: PathBuf,
        nickname: String,
        ticket: String,
        server_address: Ipv4Addr,
        ports: PortTopology,
    ) -> Result<ConnectorPlan> {
        ConnectorPlan::new(ConnectorRequest {
            game_directory,
            game_executable: optional_path(&self.game_executable),
            nickname,
            login_ticket: Some(ticket),
            server_address,
            ports,
            runner: self.runner()?,
            probe_timeout: DEFAULT_PROBE_TIMEOUT,
            installation_options: InstallationOptions::default(),
        })
        .context("접속 설정이 올바르지 않습니다")
    }
}

fn required_text<'a>(value: &'a str, label: &str) -> Result<&'a str> {
    let value = value.trim();
    if value.is_empty() {
        Err(anyhow!("{label}을(를) 입력하세요"))
    } else {
        Ok(value)
    }
}

fn required_path(value: &str, label: &str) -> Result<PathBuf> {
    required_text(value, label).map(PathBuf::from)
}

fn optional_path(value: &str) -> Option<PathBuf> {
    let value = value.trim();
    (!value.is_empty()).then(|| PathBuf::from(value))
}

fn default_game_directory() -> Result<PathBuf, String> {
    game_directory_named(APP_DATA_NAME)
}

fn game_directory_named(name: &str) -> Result<PathBuf, String> {
    let base = if cfg!(target_os = "macos") {
        std::env::var_os("HOME")
            .map(|home| PathBuf::from(home).join("Library").join(name))
            .ok_or_else(|| "홈 폴더 위치를 알 수 없습니다.".to_owned())?
    } else if cfg!(windows) {
        std::env::var_os("ProgramData")
            .map_or_else(|| PathBuf::from(r"C:\ProgramData"), PathBuf::from)
            .join(name)
    } else {
        std::env::current_exe()
            .map_err(|error| format!("실행기 위치를 알 수 없습니다: {error}"))?
            .parent()
            .map(Path::to_owned)
            .ok_or_else(|| "실행기 위치를 알 수 없습니다.".to_owned())?
    };
    Ok(base.join("Game"))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Page {
    Login,
    Register,
    ChangePassword,
}

#[derive(Debug, Clone, PartialEq)]
enum Status {
    Idle,
    Busy(String, Option<f32>),
    Done(String),
    Failed(String),
}

enum WorkerEvent {
    Progress(String, Option<f32>),
    Finished(Result<Outcome, String>),
}

enum Outcome {
    LoggedIn(credentials::SavedLogin, Option<String>),
    Launched(Session),
    LoggedOut(String),
    LauncherUpdated(PathBuf),
    Registered(String),
    PasswordChanged(credentials::SavedLogin, Option<String>),
}

#[derive(Debug, Clone)]
struct Session {
    server: ServerAddress,
    ticket: LaunchTicket,
}

#[derive(Default)]
struct Passwords {
    password: String,
    confirm: String,
    new_password: String,
}

impl Passwords {
    fn clear(&mut self) {
        *self = Self::default();
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ClientState {
    Unknown,
    Checking,
    Known(updater::UpdateCheck),
    Failed(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum LauncherState {
    Checking,
    Current,
    Outdated(self_update::Update),
}

struct Launcher {
    logo: Option<egui::TextureHandle>,
    settings: Settings,
    self_update: LauncherState,
    self_update_events: Option<Receiver<Option<self_update::Update>>>,
    relaunch: Arc<Mutex<Option<PathBuf>>>,
    client: ClientState,
    client_events: Option<Receiver<Result<updater::UpdateCheck, String>>>,
    passwords: Passwords,
    page: Page,
    show_settings: bool,
    status: Status,
    events: Option<Receiver<WorkerEvent>>,
    cancel: Option<Arc<AtomicBool>>,
    session: Option<Session>,
    checked_server: Option<ServerAddress>,
    discord: Option<discord::DiscordPresence>,
    bottles: Vec<String>,
    account: Option<credentials::SavedLogin>,
    wrappers: Vec<PathBuf>,
}

impl Launcher {
    fn new(
        storage: Option<&dyn eframe::Storage>,
        relaunch: Arc<Mutex<Option<PathBuf>>>,
        logo: Option<egui::TextureHandle>,
    ) -> Self {
        let mut settings = Settings::load(storage);
        if settings.game_directory.trim().is_empty()
            && let (Ok(old), Ok(new)) = (
                game_directory_named(legacy::OLD_APP_DATA_NAME),
                default_game_directory(),
            )
            && !legacy::migrate_game(&old, &new)
        {
            settings.game_directory = old.display().to_string();
        }
        let account = (credentials::SUPPORTED && settings.remember_login)
            .then(|| credentials::load(BUILT_IN_SERVER))
            .flatten();
        if let Some(account) = &account {
            settings.server.clone_from(&account.server);
            settings.nickname.clone_from(&account.nickname);
        }
        let mut launcher = Self {
            logo,
            settings,
            account,
            self_update: LauncherState::Current,
            self_update_events: None,
            relaunch,
            client: ClientState::Unknown,
            client_events: None,
            passwords: Passwords::default(),
            page: Page::Login,
            show_settings: false,
            status: Status::Idle,
            events: None,
            cancel: None,
            session: None,
            checked_server: None,
            discord: None,
            bottles: runtime::crossover_bottles(),
            wrappers: runtime::sikarugir_wrappers(),
        };
        launcher.checked_server = launcher.active_server().ok();
        launcher.start_launcher_check();
        launcher
    }

    fn active_server(&self) -> Result<ServerAddress> {
        match &self.account {
            Some(account) => ServerAddress::parse(&account.server),
            None => self.settings.server(),
        }
    }

    /// Launcher updates come from GitHub releases, not the game server, so this
    /// runs once at startup whatever server is selected.
    fn start_launcher_check(&mut self) {
        self.self_update = LauncherState::Current;
        self.self_update_events = None;
        let (sender, receiver) = mpsc::channel();
        let spawned = thread::Builder::new()
            .name("p5099-launcher-self-check".to_owned())
            .spawn(move || {
                let _ = sender.send(self_update::check().ok().flatten());
            });
        if spawned.is_ok() {
            self.self_update = LauncherState::Checking;
            self.self_update_events = Some(receiver);
        }
    }

    fn follow_server(&mut self, context: &egui::Context) {
        let server = self.active_server().ok();
        let typing = context.memory(|memory| memory.has_focus(egui::Id::new(SERVER_FIELD_ID)));
        if server == self.checked_server || typing || self.busy() {
            return;
        }
        self.checked_server = server;
        self.client_events = None;
        self.client = ClientState::Unknown;
    }

    fn refresh_runtimes(&mut self) {
        self.bottles = runtime::crossover_bottles();
        self.wrappers = runtime::sikarugir_wrappers();
    }

    const fn busy(&self) -> bool {
        matches!(self.status, Status::Busy(..))
    }

    const fn update_required(&self) -> bool {
        matches!(&self.self_update, LauncherState::Outdated(update) if update.required)
    }

    fn drain_launcher_events(&mut self) {
        if let Some(events) = &self.self_update_events
            && let Ok(update) = events.try_recv()
        {
            self.self_update = update.map_or(LauncherState::Current, LauncherState::Outdated);
            self.self_update_events = None;
        }
    }

    fn start_launcher_update(&mut self, context: &egui::Context) {
        let LauncherState::Outdated(update) = &self.self_update else {
            return;
        };
        let update = update.clone();
        self.spawn(
            context,
            "새 실행기를 받는 중…",
            move |progress, cancel| {
                #[allow(clippy::cast_precision_loss, reason = "display only")]
                let target = self_update::install(&update, cancel, &|done, total| {
                    progress(
                        format!("새 실행기를 받는 중… ({})", update.version),
                        Some(done as f32 / total.max(1) as f32),
                    );
                })
                .context("실행기를 업데이트하지 못했습니다")?;
                Ok(Outcome::LauncherUpdated(target))
            },
        );
    }

    fn launcher_banner(&mut self, ui: &mut egui::Ui, context: &egui::Context) {
        let LauncherState::Outdated(update) = &self.self_update else {
            return;
        };
        let (version, required, installable) =
            (update.version, update.required, update.asset.is_some());
        let busy = self.busy();
        let color = if required {
            theme::ERROR
        } else {
            theme::ACCENT
        };
        theme::banner(ui, color, |ui| {
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.label(
                        egui::RichText::new(format!("새 실행기 {version}이(가) 있습니다"))
                            .color(color),
                    );
                    if required {
                        ui.weak("이 버전으로는 접속할 수 없습니다. 실행기를 업데이트하세요.");
                    } else if !installable {
                        ui.weak("새 버전을 받아 설치하세요.");
                    }
                });
                if installable {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui
                            .add_enabled_ui(!busy, |ui| {
                                theme::secondary_button(ui, "업데이트 후 재시작")
                            })
                            .inner
                            .clicked()
                        {
                            self.start_launcher_update(context);
                        }
                    });
                }
            });
        });
        ui.add_space(8.0);
    }

    fn start_client_check(&mut self, context: &egui::Context) {
        let game_directory = match self.settings.game_directory() {
            Ok(directory) => directory,
            Err(message) => {
                self.client = ClientState::Failed(message);
                return;
            }
        };
        let server = match self.active_server() {
            Ok(server) => server,
            Err(error) => {
                self.client = ClientState::Failed(format!("{error:#}"));
                return;
            }
        };
        let (sender, receiver) = mpsc::channel();
        self.client_events = Some(receiver);
        self.client = ClientState::Checking;
        let context = context.clone();
        let spawned = thread::Builder::new()
            .name("p5099-launcher-check".to_owned())
            .spawn(move || {
                let result = self_update::release_key()
                    .and_then(|key| {
                        updater::check_client(
                            &game_directory,
                            &server.download_url(),
                            updater::PLATFORM_FOLDER,
                            &key,
                            &AtomicBool::new(false),
                            &|_| {},
                        )
                    })
                    .map_err(|error| format!("{error:#}"));
                let _ = sender.send(result);
                context.request_repaint();
            });
        if spawned.is_err() {
            self.client_events = None;
            self.client = ClientState::Failed("업데이트를 확인하지 못했습니다.".to_owned());
        }
    }

    fn drain_client_events(&mut self) {
        if let Some(events) = &self.client_events
            && let Ok(result) = events.try_recv()
        {
            self.client = match result {
                Ok(check) => ClientState::Known(check),
                Err(message) => ClientState::Failed(message),
            };
            self.client_events = None;
        }
    }

    fn client_status(&mut self, ui: &mut egui::Ui, context: &egui::Context) {
        const MIB: f64 = 1024.0 * 1024.0;
        #[allow(clippy::cast_precision_loss, reason = "display only")]
        let size = |bytes: u64| {
            if bytes as f64 >= 1024.0 * MIB {
                format!("{:.1} GB", bytes as f64 / (1024.0 * MIB))
            } else {
                format!("{:.0} MB", (bytes as f64 / MIB).ceil())
            }
        };
        let busy = self.busy();
        let idle = !matches!(self.client, ClientState::Unknown | ClientState::Checking);
        ui.horizontal(|ui| {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .add_enabled_ui(idle && !busy, |ui| {
                        theme::secondary_button(ui, "⟳ 다시 확인")
                    })
                    .inner
                    .clicked()
                {
                    self.start_client_check(context);
                }
                ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                    let (color, text) = match &self.client {
                        ClientState::Unknown | ClientState::Checking => {
                            ui.spinner();
                            ui.label("업데이트 확인 중…");
                            return;
                        }
                        ClientState::Known(check) if !check.installed => {
                            (theme::WARNING, format!("설치 필요 · {}", size(check.bytes)))
                        }
                        ClientState::Known(check) if check.files > 0 => (
                            theme::WARNING,
                            format!(
                                "업데이트 있음 · 파일 {}개, {}",
                                check.files,
                                size(check.bytes)
                            ),
                        ),
                        ClientState::Known(_) => (theme::SUCCESS, "최신 버전".to_owned()),
                        ClientState::Failed(message) => {
                            (theme::ERROR, format!("확인 실패: {message}"))
                        }
                    };
                    theme::status_dot(ui, color);
                    ui.add(egui::Label::new(text).truncate());
                });
            });
        });
    }

    fn start_label(&self) -> &'static str {
        match &self.client {
            ClientState::Known(check) if !check.installed => "설치 후 게임 시작",
            ClientState::Known(check) if check.files > 0 => "업데이트 후 게임 시작",
            _ => "게임 시작",
        }
    }

    fn drain_events(&mut self, context: &egui::Context) {
        let Some(events) = &self.events else {
            return;
        };
        while let Ok(event) = events.try_recv() {
            match event {
                WorkerEvent::Progress(message, fraction) => {
                    self.status = Status::Busy(message, fraction);
                }
                WorkerEvent::Finished(result) => {
                    self.status = match result {
                        Ok(Outcome::LoggedIn(account, save_error)) => {
                            self.log_in_as(account);
                            Status::Done(with_note("로그인했습니다.", save_error))
                        }
                        Ok(Outcome::Launched(session)) => {
                            self.client = ClientState::Unknown;
                            self.discord = self.settings.discord_presence.then(|| {
                                discord::spawn(session.server.clone(), session.ticket.clone())
                            });
                            self.session = Some(session);
                            Status::Done("게임을 실행했습니다.".to_owned())
                        }
                        Ok(Outcome::LoggedOut(message)) => {
                            self.log_out();
                            Status::Failed(message)
                        }
                        Ok(Outcome::LauncherUpdated(target)) => {
                            if let Ok(mut slot) = self.relaunch.lock() {
                                *slot = Some(target);
                            }
                            context.send_viewport_cmd(egui::ViewportCommand::Close);
                            Status::Done("새 실행기를 시작합니다…".to_owned())
                        }
                        Ok(Outcome::Registered(nickname)) => {
                            self.settings.nickname = nickname;
                            self.page = Page::Login;
                            Status::Done("가입했습니다. 이제 로그인하세요.".to_owned())
                        }
                        Ok(Outcome::PasswordChanged(account, save_error)) => {
                            self.log_in_as(account);
                            Status::Done(with_note("비밀번호를 바꿨습니다.", save_error))
                        }
                        Err(message) => Status::Failed(message),
                    };
                    self.passwords.clear();
                    self.events = None;
                    self.cancel = None;
                    return;
                }
            }
        }
    }

    fn spawn(
        &mut self,
        context: &egui::Context,
        first_message: &str,
        job: impl FnOnce(&dyn Fn(String, Option<f32>), &AtomicBool) -> Result<Outcome> + Send + 'static,
    ) {
        let (sender, receiver) = mpsc::channel();
        self.events = Some(receiver);
        self.status = Status::Busy(first_message.to_owned(), None);
        let cancel = Arc::new(AtomicBool::new(false));
        self.cancel = Some(Arc::clone(&cancel));
        let context = context.clone();
        let spawned = thread::Builder::new()
            .name("p5099-launcher-worker".to_owned())
            .spawn(move || {
                let notify = |sender: &Sender<WorkerEvent>, event| {
                    let _ = sender.send(event);
                    context.request_repaint();
                };
                let progress = |message: String, fraction: Option<f32>| {
                    notify(&sender, WorkerEvent::Progress(message, fraction));
                };
                let result = job(&progress, &cancel).map_err(|error| format!("{error:#}"));
                notify(&sender, WorkerEvent::Finished(result));
            });
        if let Err(error) = spawned {
            self.events = None;
            self.status = Status::Failed(format!("작업을 시작하지 못했습니다: {error}"));
        }
    }

    fn log_in_as(&mut self, account: credentials::SavedLogin) {
        self.settings.server.clone_from(&account.server);
        self.settings.nickname.clone_from(&account.nickname);
        self.account = Some(account);
        self.page = Page::Login;
    }

    fn log_out(&mut self) {
        credentials::clear();
        self.account = None;
        self.session = None;
        self.discord = None;
        self.page = Page::Login;
        self.passwords.clear();
        self.status = Status::Idle;
    }

    fn start_login(&mut self, context: &egui::Context) {
        let server = match self.settings.server() {
            Ok(server) => server,
            Err(error) => {
                self.status = Status::Failed(format!("{error:#}"));
                return;
            }
        };
        let nickname = self.settings.nickname.clone();
        let password = self.passwords.password.clone();
        let remember = credentials::SUPPORTED && self.settings.remember_login;
        self.spawn(context, "로그인하는 중…", move |_, _| {
            block_on(async {
                let key = account_key(&nickname, &password)
                    .await
                    .map_err(|error| account_message(&error))?;
                let ticket = log_in(&server, server.resolve().await?, &nickname, &key)
                    .await
                    .map_err(|error| account_message(&error))?;
                let account = credentials::SavedLogin {
                    server: server.to_string(),
                    nickname: ticket.nickname,
                    key,
                };
                let save_error = remember.then(|| keep(&account)).flatten();
                Ok(Outcome::LoggedIn(account, save_error))
            })
        });
    }

    fn start_game(&mut self, context: &egui::Context) {
        let Some(account) = self.account.clone() else {
            return;
        };
        let settings = self.settings.clone();
        let server = match ServerAddress::parse(&account.server) {
            Ok(server) => server,
            Err(error) => {
                self.status = Status::Failed(format!("{error:#}"));
                return;
            }
        };
        let game_directory = match (settings.game_directory(), settings.runner()) {
            (Err(message), _) => {
                self.status = Status::Failed(message);
                return;
            }
            (_, Err(error)) => {
                self.status = Status::Failed(format!("{error:#}"));
                return;
            }
            (Ok(directory), Ok(_)) => directory,
        };
        self.spawn(
            context,
            "로그인하는 중…",
            move |progress, cancel| {
                let address = block_on(server.resolve())?;
                let ticket = match block_on(async {
                    Ok(log_in(&server, address, &account.nickname, &account.key).await)
                })? {
                    Ok(ticket) => ticket,
                    Err(error) if error.failure() == Some(AccountFailure::BadCredentials) => {
                        credentials::clear();
                        return Ok(Outcome::LoggedOut(
                            "로그인 정보가 더 이상 맞지 않습니다. 다시 로그인하세요.".to_owned(),
                        ));
                    }
                    Err(error) => return Err(account_message(&error)),
                };
                updater::sync_client(
                    &game_directory,
                    &server.download_url(),
                    updater::PLATFORM_FOLDER,
                    &self_update::release_key()?,
                    cancel,
                    &|step| {
                        let (message, fraction) = sync_message(&step);
                        progress(message, fraction);
                    },
                )
                .context("클라이언트를 업데이트하지 못했습니다")?;
                if let Runner::CrossOver {
                    wine_binary,
                    bottle,
                } = settings.runner()?
                {
                    progress(
                        format!(
                            "CrossOver 보틀 '{bottle}'을(를) 준비하는 중… \
                             (처음에는 1~2분 걸릴 수 있습니다)"
                        ),
                        None,
                    );
                    runtime::ensure_crossover_bottle(&wine_binary, &bottle)?;
                }
                let plan = settings.connector_plan(
                    game_directory,
                    ticket.nickname.clone(),
                    ticket.ticket.clone(),
                    address,
                    server.ports(),
                )?;
                block_on(async {
                    let mut execution = execute_connector_with_progress_and_cancellation(
                        &plan,
                        &ConnectorCancellation::new(),
                        |stage| progress(stage_message(stage).to_owned(), None),
                    )
                    .await
                    .context("게임을 실행하지 못했습니다")?;
                    let _ = execution.launched_process.try_status();
                    Ok(Outcome::Launched(Session { server, ticket }))
                })
            },
        );
    }

    fn start_register(&mut self, context: &egui::Context) {
        if self.passwords.password != self.passwords.confirm {
            self.status = Status::Failed("비밀번호 확인이 일치하지 않습니다.".to_owned());
            return;
        }
        let server = match self.settings.server() {
            Ok(server) => server,
            Err(error) => {
                self.status = Status::Failed(format!("{error:#}"));
                return;
            }
        };
        let nickname = self.settings.nickname.clone();
        let password = self.passwords.password.clone();
        self.spawn(context, "가입하는 중…", move |_, _| {
            block_on(async {
                account_register(
                    server.resolve().await?,
                    server.ports(),
                    &nickname,
                    &password,
                    DEFAULT_ACCOUNT_TIMEOUT,
                )
                .await
                .map(Outcome::Registered)
                .map_err(|error| account_message(&error))
            })
        });
    }

    fn start_change_password(&mut self, context: &egui::Context) {
        if self.passwords.new_password != self.passwords.confirm {
            self.status = Status::Failed("새 비밀번호 확인이 일치하지 않습니다.".to_owned());
            return;
        }
        let server = match self.settings.server() {
            Ok(server) => server,
            Err(error) => {
                self.status = Status::Failed(format!("{error:#}"));
                return;
            }
        };
        let nickname = self.settings.nickname.clone();
        let password = self.passwords.password.clone();
        let new_password = self.passwords.new_password.clone();
        let remember = credentials::SUPPORTED && self.settings.remember_login;
        self.spawn(context, "비밀번호를 바꾸는 중…", move |_, _| {
            block_on(async {
                change_password(
                    server.resolve().await?,
                    server.ports(),
                    &nickname,
                    &password,
                    &new_password,
                    DEFAULT_ACCOUNT_TIMEOUT,
                )
                .await
                .map_err(|error| account_message(&error))?;
                let account = credentials::SavedLogin {
                    server: server.to_string(),
                    nickname: normalize_nickname(&nickname)?,
                    key: account_key(&nickname, &new_password)
                        .await
                        .map_err(|error| account_message(&error))?,
                };
                let save_error = remember.then(|| keep(&account)).flatten();
                Ok(Outcome::PasswordChanged(account, save_error))
            })
        });
    }

    fn account_form(&mut self, ui: &mut egui::Ui, context: &egui::Context) {
        const PAGES: [(Page, &str); 3] = [
            (Page::Login, "로그인"),
            (Page::Register, "회원가입"),
            (Page::ChangePassword, "비밀번호 변경"),
        ];
        let busy = self.busy();
        ui.add_enabled_ui(!busy, |ui| {
            let labels = PAGES.map(|(_, label)| label);
            let selected = PAGES
                .iter()
                .position(|(page, _)| *page == self.page)
                .unwrap_or(0);
            if let Some(index) = theme::segmented(ui, &labels, selected) {
                self.page = PAGES[index].0;
                self.passwords.clear();
                self.status = Status::Idle;
            }
        });
        ui.add_space(8.0);

        ui.add_enabled_ui(!busy, |ui| {
            theme::field_label(ui, "서버 주소");
            ui.add(
                theme::text_input(&mut self.settings.server, "예: kart.example.com")
                    .id(egui::Id::new(SERVER_FIELD_ID)),
            )
            .on_hover_text("포트가 기본값(39311)이 아니면 주소 뒤에 :포트를 붙이세요.");
            theme::field_label(ui, "닉네임");
            ui.add(theme::text_input(
                &mut self.settings.nickname,
                "닉네임을 입력하세요",
            ));

            let password = self.password_field(ui);
            if self.page == Page::Login && credentials::SUPPORTED {
                self.remember_checkbox(ui);
            }

            if self.page == Page::ChangePassword {
                password_rule_label(ui, "새 비밀번호");
                ui.add(
                    theme::text_input(&mut self.passwords.new_password, "새 비밀번호")
                        .password(true),
                );
            }
            if self.page != Page::Login {
                theme::field_label(ui, "비밀번호 확인");
                ui.add(
                    theme::text_input(&mut self.passwords.confirm, "한 번 더 입력하세요")
                        .password(true),
                );
            }

            let submitted = self.page == Page::Login
                && password.lost_focus()
                && ui.input(|input| input.key_pressed(egui::Key::Enter));
            if submitted && !busy {
                self.start_login(context);
            }
        });

        ui.add_space(10.0);
        ui.add_enabled_ui(!busy && !self.update_required(), |ui| {
            let (label, action): (&str, fn(&mut Self, &egui::Context)) = match self.page {
                Page::Login => ("로그인", Self::start_login),
                Page::Register => ("가입하기", Self::start_register),
                Page::ChangePassword => ("비밀번호 변경", Self::start_change_password),
            };
            if theme::primary_button(ui, label).clicked() {
                action(self, context);
            }
        });
    }

    fn password_field(&mut self, ui: &mut egui::Ui) -> egui::Response {
        let password_label = if self.page == Page::ChangePassword {
            "현재 비밀번호"
        } else {
            "비밀번호"
        };
        if self.page == Page::Register {
            password_rule_label(ui, password_label);
        } else {
            theme::field_label(ui, password_label);
        }
        ui.add(theme::text_input(&mut self.passwords.password, "비밀번호").password(true))
    }

    fn profile(&mut self, ui: &mut egui::Ui, context: &egui::Context) {
        let Some(account) = &self.account else {
            return;
        };
        let (nickname, server) = (account.nickname.clone(), account.server.clone());
        let busy = self.busy();
        ui.horizontal(|ui| {
            let (rect, _) =
                ui.allocate_exact_size(egui::Vec2::splat(AVATAR_SIZE), egui::Sense::hover());
            let painter = ui.painter();
            painter.circle(
                rect.center(),
                AVATAR_SIZE / 2.0,
                theme::INPUT,
                egui::Stroke::new(1.0, theme::BORDER),
            );
            let initial: String = nickname
                .chars()
                .take(1)
                .flat_map(char::to_uppercase)
                .collect();
            painter.text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                initial,
                egui::FontId::proportional(20.0),
                theme::TEXT,
            );
            ui.add_space(4.0);
            ui.vertical(|ui| {
                ui.add_space(3.0);
                ui.add(
                    egui::Label::new(egui::RichText::new(&nickname).size(18.0).strong()).truncate(),
                );
                ui.add(
                    egui::Label::new(egui::RichText::new(&server).size(12.0).color(theme::WEAK))
                        .truncate(),
                );
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .add_enabled_ui(!busy, |ui| theme::secondary_button(ui, "로그아웃"))
                    .inner
                    .clicked()
                {
                    self.log_out();
                }
            });
        });
        if credentials::SUPPORTED {
            ui.add_space(6.0);
            ui.add_enabled_ui(!busy, |ui| self.remember_checkbox(ui));
        }
        ui.add_space(10.0);
        ui.add_enabled_ui(!busy && !self.update_required(), |ui| {
            if theme::primary_button(ui, self.start_label()).clicked() {
                self.start_game(context);
            }
        });
    }

    fn remember_checkbox(&mut self, ui: &mut egui::Ui) {
        let store = if cfg!(windows) {
            "Windows 자격 증명 관리자"
        } else {
            "macOS 키체인"
        };
        let changed = ui
            .checkbox(&mut self.settings.remember_login, "로그인 유지")
            .on_hover_text(format!(
                "실행기를 다시 열어도 로그인한 상태로 시작합니다. 비밀번호 대신 \
                 로그인 키를 {store}에 저장합니다. 여러 사람이 쓰는 PC에서는 켜지 마세요."
            ))
            .changed();
        if !changed {
            return;
        }
        if !self.settings.remember_login {
            credentials::clear();
        } else if let Some(error) = self.account.as_ref().and_then(keep) {
            self.status = Status::Failed(error);
        }
    }

    fn set_game_directory(&mut self, directory: String) {
        if self.settings.game_directory == directory {
            return;
        }
        self.settings.game_directory = directory;
        self.client_events = None;
        self.client = ClientState::Unknown;
    }

    fn game_folder(&mut self, ui: &mut egui::Ui) {
        let directory = self.settings.game_directory();
        ui.label("설치 위치");
        match &directory {
            Ok(directory) => {
                let path = directory.display().to_string();
                ui.add(egui::Label::new(egui::RichText::new(&path).monospace()).truncate())
                    .on_hover_text(path);
            }
            Err(message) => {
                ui.add(egui::Label::new(egui::RichText::new(message).color(theme::ERROR)).wrap());
            }
        }
        ui.end_row();

        ui.label("");
        let custom = !self.settings.game_directory.trim().is_empty();
        ui.horizontal(|ui| {
            if ui
                .button("변경…")
                .on_hover_text(
                    "위치를 바꾸면 새 위치에 게임을 다시 받습니다. \
                     이전 위치의 파일은 지워지지 않습니다.",
                )
                .clicked()
                && let Some(folder) = rfd::FileDialog::new()
                    .set_title("게임을 설치할 폴더 선택")
                    .pick_folder()
            {
                self.set_game_directory(install_folder(&folder).display().to_string());
            }
            if ui
                .add_enabled(custom, egui::Button::new("기본 위치로"))
                .clicked()
            {
                self.set_game_directory(String::new());
            }
            if let Ok(directory) = &directory
                && directory.is_dir()
                && ui.button("열기").clicked()
            {
                open_folder(directory);
            }
        });
        ui.end_row();
    }

    fn bottle_field(&mut self, ui: &mut egui::Ui) {
        ui.label("CrossOver 보틀");
        ui.horizontal(|ui| {
            egui::ComboBox::from_id_salt("bottle")
                .selected_text("목록")
                .width(64.0)
                .show_ui(ui, |ui| {
                    for bottle in &self.bottles {
                        ui.selectable_value(
                            &mut self.settings.crossover_bottle,
                            bottle.clone(),
                            bottle,
                        );
                    }
                });
            ui.add(theme::text_input(
                &mut self.settings.crossover_bottle,
                "보틀 이름",
            ))
            .on_hover_text("없는 보틀이면 처음 게임을 시작할 때 만듭니다.");
        });
        ui.end_row();
    }

    fn wrapper_field(&mut self, ui: &mut egui::Ui) {
        ui.label("Sikarugir");
        ui.horizontal(|ui| {
            let selected = optional_path(&self.settings.sikarugir_app).map_or_else(
                || "자동 선택".to_owned(),
                |path| runtime::wrapper_label(&path),
            );
            if ui.button("⟳").on_hover_text("새로 고침").clicked() {
                self.refresh_runtimes();
            }
            if ui.button("찾아보기…").clicked()
                && let Some(path) = rfd::FileDialog::new()
                    .add_filter("Sikarugir wrapper", &["app"])
                    .pick_file()
            {
                self.settings.sikarugir_app = path.display().to_string();
            }
            egui::ComboBox::from_id_salt("wrapper")
                .selected_text(selected)
                .width(ui.available_width())
                .show_ui(ui, |ui| {
                    ui.selectable_value(
                        &mut self.settings.sikarugir_app,
                        String::new(),
                        "자동 선택",
                    );
                    for wrapper in &self.wrappers {
                        ui.selectable_value(
                            &mut self.settings.sikarugir_app,
                            wrapper.display().to_string(),
                            runtime::wrapper_label(wrapper),
                        );
                    }
                });
        });
        ui.end_row();
    }

    fn runtime_line(&self, ui: &mut egui::Ui) {
        if cfg!(windows) {
            return;
        }
        match self.settings.runtime_summary() {
            Ok(summary) if summary.is_empty() => {}
            Ok(summary) => {
                ui.horizontal(|ui| {
                    theme::status_dot(ui, theme::WEAK);
                    ui.weak(format!("실행 환경 · {summary}"));
                });
            }
            Err(message) => theme::message(ui, theme::ERROR, message),
        }
    }

    fn settings_page(&mut self, ui: &mut egui::Ui) {
        theme::card(ui, |ui| {
            egui::Grid::new("settings")
                .num_columns(2)
                .spacing([12.0, 8.0])
                .min_col_width(84.0)
                .show(ui, |ui| self.settings_rows(ui));
            if let Ok(directory) = self.settings.game_directory()
                && directory.to_string_lossy().contains(char::is_whitespace)
            {
                ui.add_space(4.0);
                theme::message(
                    ui,
                    theme::WARNING,
                    "설치 경로에 띄어쓰기가 있으면 게임이 제대로 실행되지 않을 수 있습니다. \
                     띄어쓰기가 없는 폴더에 다시 설치하세요.",
                );
            }
        });
    }

    fn settings_rows(&mut self, ui: &mut egui::Ui) {
        self.game_folder(ui);

        ui.label("실행 방식");
        egui::ComboBox::from_id_salt("runner")
            .selected_text(self.settings.runner.label())
            .width(ui.available_width())
            .show_ui(ui, |ui| {
                for &runner in RunnerChoice::AVAILABLE {
                    ui.selectable_value(&mut self.settings.runner, runner, runner.label());
                }
            });
        ui.end_row();

        let field = |ui: &mut egui::Ui, label: &str, hint: &str, value: &mut String| {
            ui.label(label);
            ui.add(theme::text_input(value, hint));
            ui.end_row();
        };
        match self.settings.runner {
            RunnerChoice::Wine => {
                field(ui, "Wine", "wine", &mut self.settings.wine_binary);
                field(
                    ui,
                    "Wine prefix",
                    "기본 prefix 사용",
                    &mut self.settings.wine_prefix,
                );
            }
            RunnerChoice::CrossOver => {
                field(
                    ui,
                    "CrossOver",
                    "CrossOver wine 경로",
                    &mut self.settings.crossover_binary,
                );
                self.bottle_field(ui);
            }
            RunnerChoice::Sikarugir => self.wrapper_field(ui),
            RunnerChoice::Auto if cfg!(target_os = "macos") => {
                self.bottle_field(ui);
                self.wrapper_field(ui);
            }
            RunnerChoice::Auto | RunnerChoice::Native | RunnerChoice::NativeElevated => {}
        }
        field(
            ui,
            "게임 실행 파일",
            "자동으로 찾기",
            &mut self.settings.game_executable,
        );

        if !discord::SUPPORTED {
            return;
        }
        ui.label("Discord");
        if ui
            .checkbox(&mut self.settings.discord_presence, "게임 상태 표시")
            .changed()
        {
            self.discord = if self.settings.discord_presence {
                self.session
                    .clone()
                    .map(|session| discord::spawn(session.server, session.ticket))
            } else {
                None
            };
        }
        ui.end_row();
    }

    fn status_line(&self, ui: &mut egui::Ui) {
        match &self.status {
            Status::Idle => {}
            Status::Busy(message, fraction) => {
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.add(egui::Label::new(message.as_str()).truncate());
                });
                if let Some(fraction) = fraction {
                    ui.horizontal(|ui| {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if let Some(cancel) = &self.cancel
                                && theme::secondary_button(ui, "취소").clicked()
                            {
                                cancel.store(true, Ordering::Relaxed);
                            }
                            ui.label(
                                egui::RichText::new(format!("{:.0}%", fraction * 100.0))
                                    .color(theme::WEAK),
                            );
                            ui.add(
                                egui::ProgressBar::new(*fraction)
                                    .desired_width(ui.available_width())
                                    .desired_height(8.0)
                                    .fill(theme::ACCENT)
                                    .corner_radius(4),
                            );
                        });
                    });
                }
            }
            Status::Done(message) => {
                ui.add_space(8.0);
                theme::message(ui, theme::SUCCESS, message);
            }
            Status::Failed(message) => {
                ui.add_space(8.0);
                theme::message(ui, theme::ERROR, message);
            }
        }
    }

    fn header(&mut self, ui: &mut egui::Ui) {
        egui::Panel::top("header")
            .frame(
                egui::Frame::new()
                    .fill(theme::HEADER)
                    .stroke(egui::Stroke::new(1.0, theme::BORDER))
                    .inner_margin(egui::Margin::symmetric(16, 10)),
            )
            .show_separator_line(false)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    if let Some(logo) = &self.logo {
                        ui.image((logo.id(), egui::vec2(28.0, 28.0)));
                    }
                    ui.label(egui::RichText::new(WINDOW_TITLE).size(20.0).strong());
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let (icon, tip) = if self.show_settings {
                            ("⬅", "돌아가기")
                        } else {
                            ("⚙", "설정")
                        };
                        let button = egui::Button::new(egui::RichText::new(icon).size(18.0))
                            .fill(egui::Color32::TRANSPARENT)
                            .stroke(egui::Stroke::NONE)
                            .min_size(egui::vec2(32.0, 32.0));
                        if ui
                            .add_enabled(!self.busy(), button)
                            .on_hover_text(tip)
                            .clicked()
                        {
                            self.show_settings = !self.show_settings;
                        }
                    });
                });
            });
    }

    fn footer(&self, ui: &mut egui::Ui) {
        egui::Panel::bottom("footer")
            .frame(
                egui::Frame::new()
                    .fill(theme::BACKGROUND)
                    .inner_margin(egui::Margin::symmetric(16, 6)),
            )
            .show_separator_line(false)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    let small =
                        |text: String| egui::RichText::new(text).size(11.0).color(theme::WEAK);
                    let server = self
                        .active_server()
                        .map_or_else(|_| "서버 미설정".to_owned(), |server| server.to_string());
                    ui.label(small(server));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(small(format!("v{}", env!("CARGO_PKG_VERSION"))));
                    });
                });
            });
    }
}

impl eframe::App for Launcher {
    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        if let Ok(encoded) = serde_json::to_string(&self.settings) {
            storage.set_string(SETTINGS_KEY, encoded);
        }
    }

    fn logic(&mut self, context: &egui::Context, _frame: &mut eframe::Frame) {
        self.drain_events(context);
        self.drain_client_events();
        self.drain_launcher_events();
        self.follow_server(context);
        if self.client == ClientState::Unknown && !self.busy() {
            self.start_client_check(context);
        }
        if self.busy()
            || self.client == ClientState::Checking
            || self.self_update == LauncherState::Checking
        {
            context.request_repaint_after(Duration::from_millis(100));
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let context = ui.ctx().clone();
        if self.busy() {
            self.show_settings = false;
        }
        self.header(ui);
        self.footer(ui);
        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(theme::BACKGROUND)
                    .inner_margin(egui::Margin::symmetric(16, 12)),
            )
            .show(ui, |ui| {
                self.launcher_banner(ui, &context);
                if self.show_settings {
                    self.settings_page(ui);
                    return;
                }
                theme::card(ui, |ui| {
                    if self.account.is_some() {
                        self.profile(ui, &context);
                    } else {
                        self.account_form(ui, &context);
                    }
                    self.status_line(ui);
                });
                if self.account.is_some() {
                    ui.add_space(8.0);
                    self.client_status(ui, &context);
                    self.runtime_line(ui);
                }
            });
    }
}

fn password_rule_label(ui: &mut egui::Ui, text: &str) {
    ui.scope(|ui| {
        ui.spacing_mut().interact_size.y = 0.0;
        ui.horizontal(|ui| {
            theme::field_label(ui, text);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(
                    egui::RichText::new(format!("{MIN_PASSWORD_CHARS}~{MAX_PASSWORD_CHARS}자"))
                        .size(12.0)
                        .color(theme::WEAK),
                )
                .on_hover_text("비밀번호는 이 PC에서만 변환되며 서버로 전송되지 않습니다.");
            });
        });
    });
}

fn install_folder(picked: &Path) -> PathBuf {
    let empty = std::fs::read_dir(picked).is_ok_and(|mut entries| entries.next().is_none());
    if empty || picked.join("KartRider.exe").is_file() {
        picked.to_owned()
    } else {
        picked.join(APP_DATA_NAME)
    }
}

fn open_folder(directory: &Path) {
    let program = if cfg!(windows) {
        "explorer"
    } else if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    let _ = std::process::Command::new(program).arg(directory).spawn();
}

async fn log_in(
    server: &ServerAddress,
    address: Ipv4Addr,
    nickname: &str,
    key: &AccountKey,
) -> Result<LaunchTicket, AccountError> {
    account_login_with_key(
        address,
        server.ports(),
        nickname,
        key,
        DEFAULT_ACCOUNT_TIMEOUT,
    )
    .await
}

fn keep(account: &credentials::SavedLogin) -> Option<String> {
    credentials::save(account)
        .err()
        .map(|error| format!("{error:#}"))
}

fn with_note(message: &str, note: Option<String>) -> String {
    note.map_or_else(|| message.to_owned(), |note| format!("{message} ({note})"))
}

fn logo_texture(context: &egui::Context) -> Option<egui::TextureHandle> {
    let icon = eframe::icon_data::from_png_bytes(ICON_PNG).ok()?;
    let size = [
        usize::try_from(icon.width).ok()?,
        usize::try_from(icon.height).ok()?,
    ];
    Some(context.load_texture(
        "p5099-logo",
        egui::ColorImage::from_rgba_unmultiplied(size, &icon.rgba),
        egui::TextureOptions::LINEAR,
    ))
}

fn block_on<T>(future: impl Future<Output = Result<T>>) -> Result<T> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("작업 런타임을 만들지 못했습니다")?
        .block_on(future)
}

#[allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    reason = "progress fractions only need display precision"
)]
fn sync_message(step: &updater::SyncProgress) -> (String, Option<f32>) {
    match step {
        updater::SyncProgress::FetchingManifest => {
            ("클라이언트 파일 목록을 받는 중…".to_owned(), None)
        }
        updater::SyncProgress::Checking { done, total } => (
            format!("클라이언트 파일 확인 중 ({done}/{total})"),
            Some(*done as f32 / (*total).max(1) as f32),
        ),
        updater::SyncProgress::Downloading {
            file,
            done_bytes,
            total_bytes,
            files_left,
        } => {
            const MIB: f64 = 1024.0 * 1024.0;
            (
                format!(
                    "다운로드 중 {:.0}/{:.0} MB · 남은 파일 {files_left}개 · {file}",
                    *done_bytes as f64 / MIB,
                    *total_bytes as f64 / MIB
                ),
                Some((*done_bytes as f64 / (*total_bytes).max(1) as f64) as f32),
            )
        }
    }
}

const fn stage_message(stage: ConnectorStage) -> &'static str {
    match stage {
        ConnectorStage::PreparingInstallation => "클라이언트를 준비하는 중…",
        ConnectorStage::CheckingDataRaw => "클라이언트 데이터를 확인하는 중…",
        ConnectorStage::ProbingMessenger => "서버 연결을 확인하는 중…",
        ConnectorStage::LaunchingGame => "게임을 실행하는 중…",
    }
}

fn account_message(error: &AccountError) -> anyhow::Error {
    let message = match error {
        AccountError::Nickname(_) => {
            "닉네임을 사용할 수 없습니다. (1~32자, < > : \" / \\ | ? * 및 제어 문자 제외)"
        }
        AccountError::Password(_) => "비밀번호 길이가 맞지 않습니다.",
        AccountError::Timeout { .. } | AccountError::Connect { .. } => {
            "서버에 연결하지 못했습니다. 잠시 후 다시 시도하세요."
        }
        AccountError::Io(_) | AccountError::Protocol => "서버와 통신하는 중 오류가 났습니다.",
        AccountError::Refused(failure) => match failure {
            AccountFailure::Exists => "이미 가입된 닉네임입니다.",
            AccountFailure::InvalidNickname => "닉네임을 사용할 수 없습니다.",
            AccountFailure::BadCredentials => "닉네임 또는 비밀번호가 틀렸습니다.",
            AccountFailure::RateLimited => "시도가 너무 많습니다. 잠시 후 다시 시도하세요.",
            AccountFailure::RegistrationClosed => "지금은 회원가입을 받지 않습니다.",
            AccountFailure::BadRequest => "서버가 요청을 거부했습니다.",
            AccountFailure::Unavailable => "서버에서 계정을 저장하지 못했습니다.",
            AccountFailure::Banned => "운영자가 이용을 정지한 계정입니다.",
            AccountFailure::Maintenance => "서버 점검 중입니다. 잠시 후 다시 시도하세요.",
        },
    };
    anyhow!("{message}")
}

fn configure_platform_fonts(context: &egui::Context) {
    let loaded = platform_cjk_font_candidates()
        .into_iter()
        .filter_map(|path| std::fs::read(path).ok())
        .collect::<Vec<_>>();
    if loaded.is_empty() {
        return;
    }
    let mut fonts = egui::FontDefinitions::default();
    let mut names = Vec::with_capacity(loaded.len());
    for (index, bytes) in loaded.into_iter().enumerate() {
        let name = format!("platform-cjk-{index}");
        fonts.font_data.insert(
            name.clone(),
            std::sync::Arc::new(egui::FontData::from_owned(bytes)),
        );
        names.push(name);
    }
    for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
        if let Some(family_fonts) = fonts.families.get_mut(&family) {
            for (index, name) in names.iter().enumerate() {
                family_fonts.insert(index, name.clone());
            }
        }
    }
    context.set_fonts(fonts);
}

#[cfg(target_os = "windows")]
fn platform_cjk_font_candidates() -> Vec<PathBuf> {
    let fonts = std::env::var_os("WINDIR")
        .map_or_else(|| PathBuf::from(r"C:\Windows"), PathBuf::from)
        .join("Fonts");
    vec![fonts.join("malgun.ttf"), fonts.join("gulim.ttc")]
}

#[cfg(target_os = "macos")]
fn platform_cjk_font_candidates() -> Vec<PathBuf> {
    [
        "/System/Library/Fonts/AppleSDGothicNeo.ttc",
        "/System/Library/Fonts/Supplemental/AppleGothic.ttf",
    ]
    .into_iter()
    .map(PathBuf::from)
    .collect()
}

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
fn platform_cjk_font_candidates() -> Vec<PathBuf> {
    [
        "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
        "/usr/share/fonts/truetype/noto/NotoSansCJK-Regular.ttc",
        "/usr/share/fonts/truetype/nanum/NanumGothic.ttf",
    ]
    .into_iter()
    .map(PathBuf::from)
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_plan_targets_the_entered_server_with_the_ticket() {
        let settings = Settings {
            server: "kart.example.com:40000".to_owned(),
            runner: RunnerChoice::Native,
            ..Settings::default()
        };
        let plan = settings
            .connector_plan(
                PathBuf::from("/games/kart"),
                "Rider".to_owned(),
                "ab".repeat(32),
                Ipv4Addr::new(192, 0, 2, 20),
                settings.server().unwrap().ports(),
            )
            .unwrap();
        assert_eq!(plan.server_address, Ipv4Addr::new(192, 0, 2, 20));
        assert_eq!(plan.login_endpoint.port(), 40_001);
        assert_eq!(plan.nickname, "Rider");
        assert_eq!(plan.login_ticket.as_deref(), Some("ab".repeat(32).as_str()));
    }

    #[test]
    fn a_chosen_game_folder_replaces_the_default() {
        let folder = std::env::temp_dir().join("p5099-game");
        let settings = Settings {
            game_directory: folder.display().to_string(),
            ..Settings::default()
        };
        assert_eq!(settings.game_directory(), Ok(folder));
        assert_eq!(
            Settings::default().game_directory(),
            default_game_directory()
        );
        let relative = Settings {
            game_directory: "Game".to_owned(),
            ..Settings::default()
        };
        assert!(relative.game_directory().is_err());
    }

    #[test]
    fn the_game_never_spills_into_a_busy_folder() {
        let picked = tempfile::tempdir().unwrap();
        assert_eq!(install_folder(picked.path()), picked.path());
        std::fs::write(picked.path().join("notes.txt"), "").unwrap();
        assert_eq!(
            install_folder(picked.path()),
            picked.path().join(APP_DATA_NAME)
        );
        std::fs::write(picked.path().join("KartRider.exe"), "").unwrap();
        assert_eq!(install_folder(picked.path()), picked.path());
    }

    #[test]
    fn settings_never_carry_a_password() {
        let encoded = serde_json::to_string(&Settings::default()).unwrap();
        assert!(!encoded.to_lowercase().contains("password"));
    }
}
