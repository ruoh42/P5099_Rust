use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, ensure};

mod asset_import;
mod asset_index;
mod bundle;
mod planner;
mod track_bundle;

pub const COMPATIBILITY_ASSERTION: &str = "p5099-static-verified-v1";
const EXPERIMENTAL_NATIVE_ASSERTION: &str = "p5099-xun-sidecar-experimental-v1";

pub use asset_import::{
    AssetCategory, AssetImportOptions, AssetSelection, discover_asset_candidates,
    import_assets_to_dataraw,
};
pub use asset_index::{AssetIndex, AssetOrigin, AssetRecord, AssetRegion, large_legacy_limits};
pub use bundle::stage_compatible;
pub use planner::{PlanOptions, run_plan};
pub use track_bundle::{
    TrackImportOptions, TrackSourceRegion, discover_track_candidates, import_tracks_to_dataraw,
    install_tracks_dataraw, stage_tracks,
};

pub fn ensure_staging_destination(source: &Path, target: &Path, output: &Path) -> Result<()> {
    let source = fs::canonicalize(source)
        .with_context(|| format!("failed to canonicalize {}", source.display()))?;
    let target = fs::canonicalize(target)
        .with_context(|| format!("failed to canonicalize {}", target.display()))?;
    let output_absolute = canonicalize_future_path(output)?;
    ensure!(
        !output_absolute.starts_with(&source),
        "output must not be inside the source Data directory"
    );
    ensure!(
        !output_absolute.starts_with(&target),
        "output must not be inside the live target Data directory"
    );
    Ok(())
}

pub(crate) fn canonicalize_future_path(path: &Path) -> Result<PathBuf> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()?.join(path)
    };
    let mut cursor = absolute.as_path();
    let mut missing = Vec::new();
    while !cursor.exists() {
        let name = cursor
            .file_name()
            .context("staging output has no existing ancestor")?;
        missing.push(name.to_os_string());
        cursor = cursor
            .parent()
            .context("staging output has no existing ancestor")?;
    }
    let mut canonical = fs::canonicalize(cursor)
        .with_context(|| format!("failed to canonicalize {}", cursor.display()))?;
    for component in missing.iter().rev() {
        canonical.push(component);
    }
    Ok(canonical)
}

pub fn verify_sha256(bytes: &[u8], expected: &str, label: &str) -> Result<()> {
    use sha2::{Digest, Sha256};

    ensure!(
        expected.bytes().all(|byte| byte.is_ascii_hexdigit()),
        "invalid SHA-256 hex for {label}"
    );
    let actual = format!("{:x}", Sha256::digest(bytes));
    ensure!(
        actual.eq_ignore_ascii_case(expected),
        "SHA-256 mismatch for {label}: expected {expected}, got {actual}"
    );
    Ok(())
}
