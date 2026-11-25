use std::{
    fs::OpenOptions,
    io,
    net::{IpAddr, Ipv4Addr, SocketAddr},
    path::{Path, PathBuf},
    time::Duration,
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result, bail};
use clap::Parser;
use p5099_core::ports::{DEFAULT_CONFIGURED_PORT, PortTopology};
use p5099_server::{
    AdminConfig, BoundServer, DEFAULT_MAX_LOGIN_SESSIONS, DownloadServer,
    ItemProbabilityRankPolicy, MIN_ADMIN_TOKEN_CHARS, OperatorConsole,
    RewardPersistenceRuntimeError, ServerConfig, ServerError, ServerHandle,
    load_item_probability_xml,
};
use tracing_appender::non_blocking::{NonBlockingBuilder, WorkerGuard};
use tracing_subscriber::{EnvFilter, Layer, layer::SubscriberExt, util::SubscriberInitExt};

mod client_paths;
#[cfg(feature = "discord")]
mod discord;

/// P5099 game server. Players connect through the launcher.
///
/// Every option can also be set with the environment variable
/// `P5099_<OPTION>` (for `--client-dir`, `P5099_CLIENT_DIR`); a flag wins
/// over the variable.
#[derive(Debug, Parser)]
#[allow(clippy::struct_excessive_bools, reason = "command-line switches")]
#[command(name = "p5099-server", version)]
struct ServerArgs {
    /// Increase runtime logging.
    #[arg(short, long, env = "P5099_VERBOSE")]
    verbose: bool,

    #[arg(long, default_value = "127.0.0.1", env = "P5099_BIND")]
    bind: IpAddr,

    #[arg(long, default_value = "127.0.0.1", env = "P5099_ADVERTISE")]
    advertise: Ipv4Addr,

    #[arg(long, default_value_t = DEFAULT_CONFIGURED_PORT, env = "P5099_CONFIGURED_PORT")]
    configured_port: u16,

    #[arg(
        long,
        default_value = "Profile",
        value_name = "PATH",
        env = "P5099_PROFILE_ROOT"
    )]
    profile_root: PathBuf,

    /// Stock-client root, Profile directory, or Data directory. The server
    /// parses the client's RHO archives directly.
    #[arg(
        long = "client-dir",
        alias = "catalog",
        value_name = "CLIENT_DIR",
        env = "P5099_CLIENT_DIR"
    )]
    client_dir: Option<PathBuf>,

    /// Stock-client Data directory containing KR archives and item.rho.
    #[arg(long, value_name = "DATA_DIR", env = "P5099_CLIENT_DATA_DIR")]
    client_data_dir: Option<PathBuf>,

    /// Enable experimental DataRaw/XUN support and require `DataRaw` clients to
    /// match this installation's normalized file list.
    #[arg(long, env = "P5099_DATA_RAW_SUPPORT")]
    data_raw_support: bool,

    /// Portable item-probability XML override. Without this, client
    /// item.rho/RHO5 data is loaded automatically when Data is configured.
    #[arg(long, value_name = "PATH", env = "P5099_ITEM_PROBABILITY_XML")]
    item_probability_xml: Option<PathBuf>,

    /// Trust the live rank carried by item-pickup packets. Set false for the
    /// Combined fallback when clients are outside the LAN/friends trust boundary.
    #[arg(long, default_value_t = true, action = clap::ArgAction::Set, env = "P5099_TRUST_CLIENT_ITEM_RANK")]
    trust_client_item_rank: bool,

    #[arg(long, default_value_t = 250, env = "P5099_FIRST_MESSAGE_DELAY_MS")]
    first_message_delay_ms: u64,

    #[arg(long, default_value_t = 12, env = "P5099_LOGIN_TIMEOUT_SECONDS")]
    login_timeout_seconds: u64,

    #[arg(
        long,
        default_value_t = 300,
        env = "P5099_SESSION_IDLE_TIMEOUT_SECONDS"
    )]
    session_idle_timeout_seconds: u64,

    #[arg(
        long,
        default_value_t = 15,
        env = "P5099_SESSION_WRITE_TIMEOUT_SECONDS"
    )]
    session_write_timeout_seconds: u64,

    #[arg(long, default_value_t = DEFAULT_MAX_LOGIN_SESSIONS, env = "P5099_MAX_LOGIN_SESSIONS")]
    max_login_sessions: usize,

    /// Allow non-loopback clients to create profiles for new nicknames.
    #[arg(long, env = "P5099_ALLOW_REMOTE_PROFILE_CREATION")]
    allow_remote_profile_creation: bool,

    /// Require a launcher account (nickname + password): the game login
    /// must carry the ticket issued by the account service (base port + 5).
    #[arg(long, env = "P5099_REQUIRE_ACCOUNT")]
    require_account: bool,

    /// Stop the launcher from registering new accounts.
    #[arg(long, env = "P5099_DISABLE_ACCOUNT_REGISTRATION")]
    disable_account_registration: bool,

    /// Folder the launcher downloads from over HTTP (base port + 6):
    /// `windows/` and `macos/` clients with signed manifests, and `launcher/`
    /// releases.
    #[arg(long, value_name = "DIR", env = "P5099_DISTRIBUTION_DIR")]
    distribution_dir: Option<PathBuf>,

    /// Operator console address (Source RCON on base port + 7). The
    /// console starts only when a token is set through `P5099_ADMIN_TOKEN`
    /// or `--admin-token-file`.
    #[arg(long, default_value = "127.0.0.1", env = "P5099_ADMIN_BIND")]
    admin_bind: IpAddr,

    /// File holding the operator console token, which is also the RCON
    /// password (at least 16 characters).
    #[arg(long, value_name = "PATH", env = "P5099_ADMIN_TOKEN_FILE")]
    admin_token_file: Option<PathBuf>,

    /// Write packet contents to the log file for protocol debugging. Off by
    /// default: the contents hold chat, and messenger chat is logged when on.
    /// Login tickets and room passwords stay out either way.
    #[arg(long, env = "P5099_LOG_PACKET_PAYLOADS")]
    log_packet_payloads: bool,
}

const ADMIN_TOKEN_ENV: &str = "P5099_ADMIN_TOKEN";

fn admin_config(args: &ServerArgs, ports: PortTopology) -> Result<Option<AdminConfig>> {
    let token = match &args.admin_token_file {
        Some(path) => Some(std::fs::read_to_string(path).with_context(|| {
            format!("failed to read the operator token file {}", path.display())
        })?),
        None => std::env::var(ADMIN_TOKEN_ENV)
            .ok()
            .filter(|token| !token.trim().is_empty()),
    };
    let Some(token) = token else {
        return Ok(None);
    };
    let address = SocketAddr::new(args.admin_bind, ports.admin_tcp());
    AdminConfig::new(address, &token)
        .map(Some)
        .with_context(|| {
            format!("the operator token must be at least {MIN_ADMIN_TOKEN_CHARS} characters")
        })
}

fn main() -> Result<()> {
    let args = ServerArgs::parse();
    let _logging = init_tracing(args.verbose)?;
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("failed to create the server runtime")?;
    runtime.block_on(run_server(args))
}

fn init_tracing(verbose: bool) -> Result<WorkerGuard> {
    let default_level = if verbose {
        "debug,serenity=warn"
    } else {
        "info,serenity=warn"
    };
    let console_filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(default_level));
    let log_path = create_log_file()?;
    let file = OpenOptions::new()
        .append(true)
        .open(&log_path)
        .with_context(|| format!("failed to open log file {}", log_path.display()))?;

    let console = tracing_subscriber::fmt::layer()
        .with_writer(io::stderr)
        .with_filter(console_filter);
    let (file_writer, file_worker) = NonBlockingBuilder::default()
        .buffered_lines_limit(4_096)
        .lossy(true)
        .finish(file);
    let packet_file = tracing_subscriber::fmt::layer()
        .with_ansi(false)
        .with_writer(file_writer)
        .with_filter(EnvFilter::new("info,p5099_packet=debug,serenity=warn"));
    tracing_subscriber::registry()
        .with(console)
        .with(packet_file)
        .init();
    tracing::info!(log_file = %log_path.display(), "file logging enabled");
    Ok(file_worker)
}

fn create_log_file() -> Result<PathBuf> {
    let directory = match std::env::var_os("P5099_LOG_DIR") {
        Some(directory) if !directory.is_empty() => PathBuf::from(directory),
        _ => default_log_directory()?,
    };
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("system clock is before the Unix epoch")?
        .as_millis();
    create_log_file_in(&directory, timestamp, std::process::id())
}

fn create_log_file_in(directory: &Path, timestamp: u128, process: u32) -> Result<PathBuf> {
    std::fs::create_dir_all(directory)
        .with_context(|| format!("failed to create log directory {}", directory.display()))?;
    for sequence in 0_u8..=99 {
        let suffix = if sequence == 0 {
            String::new()
        } else {
            format!("-{sequence}")
        };
        let path = directory.join(format!("p5099-server-{timestamp}-{process}{suffix}.log"));
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(_) => return Ok(path),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("failed to create log file {}", path.display()));
            }
        }
    }
    bail!(
        "could not reserve a unique P5099 log file in {}",
        directory.display()
    )
}

fn default_log_directory() -> Result<PathBuf> {
    let executable =
        std::env::current_exe().context("failed to locate the p5099-server executable")?;
    let parent = executable
        .parent()
        .ok_or_else(|| anyhow::anyhow!("p5099-server executable has no parent directory"))?;
    Ok(parent.join("logs"))
}

async fn run_server(args: ServerArgs) -> Result<()> {
    let ports = PortTopology::new(args.configured_port)
        .context("configured port cannot provide all P5099 service offsets")?;
    let client_paths = client_paths::resolve_client_runtime_paths(
        args.client_dir.as_deref(),
        args.client_data_dir.as_deref(),
    )?;
    let item_probabilities = args
        .item_probability_xml
        .as_deref()
        .map(load_item_probability_xml)
        .transpose()
        .context("failed to load the item-probability XML override")?;
    let data_raw_directory = if args.data_raw_support {
        let data = client_paths
            .client_data_dir
            .as_deref()
            .context("--data-raw-support requires --client-dir or --client-data-dir")?;
        let directory = data
            .parent()
            .context("client Data directory has no installation root")?
            .join("DataRaw");
        Some(std::fs::canonicalize(&directory).with_context(|| {
            format!("failed to locate DataRaw directory {}", directory.display())
        })?)
    } else {
        None
    };
    let admin = admin_config(&args, ports)?;
    if admin.is_none() {
        tracing::info!("RCON operator console is off; set {ADMIN_TOKEN_ENV} to enable it");
    }
    #[cfg(feature = "discord")]
    let discord = discord::configure(&args.profile_root)?;
    let config = ServerConfig {
        bind_address: args.bind,
        advertised_address: args.advertise,
        ports,
        profile_root: args.profile_root,
        catalog_path: None,
        client_data_dir: client_paths.client_data_dir,
        data_raw_directory,
        item_probabilities,
        item_probability_rank_policy: if args.trust_client_item_rank {
            ItemProbabilityRankPolicy::TrustClientReported
        } else {
            ItemProbabilityRankPolicy::CombinedFallback
        },
        first_message_delay: Duration::from_millis(args.first_message_delay_ms),
        login_timeout: Duration::from_secs(args.login_timeout_seconds),
        session_idle_timeout: Duration::from_secs(args.session_idle_timeout_seconds),
        session_write_timeout: Duration::from_secs(args.session_write_timeout_seconds),
        max_login_sessions: args.max_login_sessions,
        allow_remote_profile_creation: args.allow_remote_profile_creation,
        require_account: args.require_account,
        allow_account_registration: !args.disable_account_registration,
        admin: admin.clone(),
        log_packet_payloads: args.log_packet_payloads,
        ..ServerConfig::default()
    };
    let downloads = start_downloads(args.distribution_dir.as_deref(), args.bind, ports).await?;
    let catalog_configured = config.client_data_dir.is_some();
    let emblem_catalog_configured = config.client_data_dir.is_some();

    let server = BoundServer::bind(config)
        .await
        .context("failed to bind the P5099 transport set")?
        .start()
        .context("failed to start the P5099 supervisor")?;
    #[cfg(feature = "discord")]
    if let Some(discord) = discord {
        discord::start(discord, server.console());
    }
    let endpoints = server.endpoints();
    tracing::info!(
        game_udp = %endpoints.game_udp,
        login_tcp = %endpoints.login_tcp,
        p2p_udp = %endpoints.p2p_udp,
        messenger_tcp = %endpoints.messenger_tcp,
        xun_sidecar_tcp = %endpoints.xun_sidecar_tcp,
        "P5099 foundation server is running"
    );
    log_client_data(catalog_configured, emblem_catalog_configured);

    let result = wait_for_server_exit(&server).await;
    if let Some(downloads) = downloads {
        downloads.abort();
    }
    result
}

fn log_client_data(catalog_configured: bool, emblem_catalog_configured: bool) {
    if catalog_configured {
        tracing::warn!("race settlement, MyRoom, and progression handling remain in progress");
    } else {
        tracing::warn!(
            "no client Data directory is configured, so PqGetRider is unavailable; \
             race settlement, MyRoom, and progression handling remain in progress"
        );
    }
    if emblem_catalog_configured {
        tracing::info!("stock KR client emblem definitions are configured");
    } else {
        tracing::warn!(
            "no client data directory is configured, so RequestEmblems has no stock emblem catalog"
        );
    }
}

async fn start_downloads(
    directory: Option<&Path>,
    bind: IpAddr,
    ports: PortTopology,
) -> Result<Option<tokio::task::JoinHandle<()>>> {
    let Some(directory) = directory else {
        tracing::warn!("no --distribution-dir, so launchers cannot download the client");
        return Ok(None);
    };
    let address = SocketAddr::new(bind, ports.download_tcp());
    let server = DownloadServer::bind(address, directory)
        .await
        .with_context(|| format!("failed to serve {} on {address}", directory.display()))?;
    tracing::info!(%address, directory = %directory.display(), "serving launcher downloads");
    Ok(Some(tokio::spawn(server.run())))
}

async fn wait_for_server_exit(server: &ServerHandle) -> Result<()> {
    let console = server.console();
    tokio::select! {
        result = server.wait() => result.context("P5099 server runtime stopped"),
        signal = stop_request(&console) => {
            signal.context("failed to install the process shutdown-signal handler")?;
            shutdown_server_after_signal(server, &console).await
        }
    }
}

async fn stop_request(console: &OperatorConsole) -> std::io::Result<()> {
    tokio::select! {
        signal = shutdown_signal() => signal,
        () = console.shutdown_requested() => Ok(()),
    }
}

async fn shutdown_server_after_signal(
    server: &ServerHandle,
    console: &OperatorConsole,
) -> Result<()> {
    match server.shutdown().await {
        Ok(()) => Ok(()),
        Err(
            error @ ServerError::RewardPersistence(RewardPersistenceRuntimeError::DeadLetter {
                ..
            }),
        ) => {
            let status = server
                .reward_status()
                .await
                .context("failed to inspect retained reward recovery state")?;
            tracing::error!(
                %error,
                outstanding_lanes = status.outstanding_lanes().len(),
                dead_letters = status.dead_letters().len(),
                "graceful shutdown is paused on retained reward recovery state"
            );
            tracing::warn!(
                "send the shutdown signal (or console shutdown) again to explicitly discard in-memory reward recovery and force shutdown"
            );
            tokio::select! {
                result = server.wait() => {
                    result.context(
                        "P5099 server runtime stopped while awaiting force-shutdown confirmation",
                    )
                }
                signal = stop_request(console) => {
                    signal.context("failed to wait for force-shutdown confirmation")?;
                    server
                        .force_shutdown()
                        .await
                        .context("forced server shutdown failed")
                }
            }
        }
        Err(error) => Err(error).context("server shutdown failed"),
    }
}

#[cfg(unix)]
async fn shutdown_signal() -> std::io::Result<()> {
    use tokio::signal::unix::{SignalKind, signal};

    let mut interrupt = signal(SignalKind::interrupt())?;
    let mut terminate = signal(SignalKind::terminate())?;
    tokio::select! {
        _ = interrupt.recv() => Ok(()),
        _ = terminate.recv() => Ok(()),
    }
}

#[cfg(not(unix))]
async fn shutdown_signal() -> std::io::Result<()> {
    tokio::signal::ctrl_c().await
}

#[cfg(test)]
mod tests {
    use std::{net::Ipv4Addr, path::Path};

    use clap::Parser;
    use tempfile::tempdir;

    use super::ServerArgs;

    #[test]
    fn server_uses_legacy_profile_directory_and_no_catalog_by_default() {
        let args = ServerArgs::try_parse_from(["p5099-server"]).unwrap();

        assert_eq!(args.profile_root, Path::new("Profile"));
        assert_eq!(args.client_dir, None);
        assert_eq!(args.client_data_dir, None);
        assert_eq!(args.bind, std::net::IpAddr::V4(Ipv4Addr::LOCALHOST));
        assert_eq!(
            args.max_login_sessions,
            p5099_server::DEFAULT_MAX_LOGIN_SESSIONS
        );
        assert!(args.trust_client_item_rank);
        assert!(!args.allow_remote_profile_creation);

        let args =
            ServerArgs::try_parse_from(["p5099-server", "--trust-client-item-rank", "false"])
                .unwrap();
        assert!(!args.trust_client_item_rank);
    }

    #[test]
    fn server_accepts_explicit_profile_catalog_and_client_data_paths() {
        let args = ServerArgs::try_parse_from([
            "p5099-server",
            "--profile-root",
            "profiles",
            "--client-dir",
            "ClientRoot",
            "--client-data-dir",
            "client/Data",
        ])
        .unwrap();

        assert_eq!(args.profile_root, Path::new("profiles"));
        assert_eq!(args.client_dir.as_deref(), Some(Path::new("ClientRoot")));
        assert_eq!(
            args.client_data_dir.as_deref(),
            Some(Path::new("client/Data"))
        );
    }

    #[test]
    fn file_log_reservation_creates_distinct_appendable_files() {
        let temporary = tempdir().unwrap();
        let directory = temporary.path().join("nested").join("logs");
        let first = super::create_log_file_in(&directory, 123, 456).unwrap();
        let second = super::create_log_file_in(&directory, 123, 456).unwrap();

        assert_eq!(first.file_name().unwrap(), "p5099-server-123-456.log");
        assert_eq!(second.file_name().unwrap(), "p5099-server-123-456-1.log");
        assert!(first.is_file());
        assert!(second.is_file());
    }
}
