mod account;
mod bml;
mod client_files;
mod codec_error;
mod dataraw_preflight;
mod detection;
mod encoded_block;
mod execution;
mod file_safety;
mod identity;
mod installation;
mod launch;
mod limits;
mod pin;
mod probe;
mod special_tracks;
#[cfg(test)]
mod test_fixture;
mod wire;
mod xml;

pub use account::{
    AccountError, DEFAULT_ACCOUNT_TIMEOUT, LaunchTicket, account_key, change_password,
    login_with_key as account_login_with_key, presence as account_presence,
    register as account_register,
};
pub use bml::BmlObject;
pub use client_files::{build_client_manifest, hash_file};
pub use codec_error::PinCodecError;
pub use dataraw_preflight::{DataRawPreflightError, verify_dataraw_preflight};
pub use detection::{
    BuildDetectionError, BuildEvidence, P5099_CLIENT_LOCATION, P5099_LOCALE_ID, PinDetectionSource,
    detect_p5099,
};
pub use encoded_block::{
    BlockEncoding, DEFAULT_KART_CRYPTO_KEY, DecodedBlock, EncodedBlockError, FLAG_KART_CRYPTO,
    FLAG_ZLIB,
};
pub use execution::{
    ConnectorCancellation, ConnectorExecution, ConnectorExecutionError, ConnectorPlan,
    ConnectorPlanError, ConnectorRequest, ConnectorStage,
    execute_connector_with_progress_and_cancellation,
};
pub use file_safety::{
    ConnectorFileError, PersistentFilePreparation, PristineAction, PristineState,
};
pub use identity::{IdentityError, MAXIMUM_NICKNAME_LENGTH, normalize_nickname};
pub use installation::{
    DEFAULT_INSTALLATION_LOCK_TIMEOUT, DEFAULT_MAXIMUM_PERSISTENT_FILE_BYTES, InstallationError,
    InstallationOptions, PreparedInstallation, XUN_SIDECAR_SESSION_FILE, prepare_installation,
};
pub use launch::{
    LaunchError, LaunchRequest, LaunchSpec, LaunchStatus, LaunchedProcess, Runner, RunnerBackend,
    SikarugirWrapper,
};
pub use limits::CodecLimits;
pub use pin::{
    AuthMethod, P5099_MINOR_VERSION, P5099_PIN_MAGIC, P5099_RIDER_DATA_DIRECTORY,
    P5099_SCREENSHOT_DIRECTORY, P5099_STORAGE_ROOT, PinDocument, PinHeader, PinPatchOptions,
    PinPatchReport, ShallowPinHeader, decode_shallow_pin_header_with_limits,
    patch_p5099_pin_with_limits,
};
pub use probe::{DEFAULT_PROBE_TIMEOUT, ProbeError, probe_messenger, probe_tcp};
pub use special_tracks::{
    SAFE_BLOCKED_TRACK_IDS, SPECIAL_TRACK_OVERLAY_FILE, SpecialTrackPatchError,
    SpecialTrackPatchReport,
};
pub use xml::{LauncherProfileRole, launcher_profile_xml, server_config_xml};
