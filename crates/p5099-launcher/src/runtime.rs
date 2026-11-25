use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

use anyhow::{Context, Result, bail};
use p5099_connector::SikarugirWrapper;

const BOTTLE_TEMPLATES: [&str; 4] = ["win11_64", "win10_64", "win7_64", "win7"];

pub fn default_crossover_wine() -> PathBuf {
    if cfg!(target_os = "macos") {
        PathBuf::from("/Applications/CrossOver.app/Contents/SharedSupport/CrossOver/bin/wine")
    } else if cfg!(target_os = "linux") {
        PathBuf::from("/opt/cxoffice/bin/wine")
    } else {
        PathBuf::new()
    }
}

fn home() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .filter(|home| !home.is_empty())
        .map(PathBuf::from)
}

fn bottles_directory() -> Option<PathBuf> {
    let home = home()?;
    Some(if cfg!(target_os = "macos") {
        home.join("Library/Application Support/CrossOver/Bottles")
    } else {
        home.join(".cxoffice")
    })
}

pub fn crossover_bottles() -> Vec<String> {
    let Some(directory) = bottles_directory() else {
        return Vec::new();
    };
    let mut bottles = fs::read_dir(directory)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|entry| entry.path().join("cxbottle.conf").is_file())
        .filter_map(|entry| entry.file_name().into_string().ok())
        .collect::<Vec<_>>();
    bottles.sort();
    bottles
}

pub fn ensure_crossover_bottle(wine: &Path, bottle: &str) -> Result<bool> {
    let bin = wine
        .parent()
        .context("CrossOver 실행 파일 경로가 올바르지 않습니다")?;
    let cxbottle = bin.join("cxbottle");
    if !cxbottle.is_file() {
        bail!(
            "CrossOver를 찾지 못했습니다 ({}). CrossOver를 설치하거나 설정(⚙)에서 경로를 지정하세요.",
            cxbottle.display()
        );
    }
    let exists = Command::new(&cxbottle)
        .args(["--bottle", bottle, "--status"])
        .output()
        .context("CrossOver를 실행하지 못했습니다")?
        .status
        .success();
    if exists {
        return Ok(false);
    }
    let templates = bin.join("../share/crossover/bottle_templates");
    let template = BOTTLE_TEMPLATES
        .into_iter()
        .find(|template| templates.join(template).is_dir())
        .unwrap_or(BOTTLE_TEMPLATES[1]);
    let output = Command::new(&cxbottle)
        .args([
            "--bottle",
            bottle,
            "--create",
            "--template",
            template,
            "--description",
            "KartRider P5099",
        ])
        .output()
        .context("CrossOver 보틀을 만들지 못했습니다")?;
    if !output.status.success() {
        bail!(
            "CrossOver 보틀 '{bottle}'을(를) 만들지 못했습니다: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(true)
}

pub fn sikarugir_wrappers() -> Vec<PathBuf> {
    let mut folders = vec![PathBuf::from("/Applications/Sikarugir")];
    if let Some(home) = home() {
        folders.insert(0, home.join("Applications/Sikarugir"));
    }
    let mut wrappers = folders
        .iter()
        .flat_map(|folder| fs::read_dir(folder).into_iter().flatten().flatten())
        .map(|entry| entry.path())
        .filter(|path| SikarugirWrapper::new(path).is_valid())
        .collect::<Vec<_>>();
    wrappers.sort();
    wrappers
}

pub fn wrapper_label(path: &Path) -> String {
    path.file_stem().map_or_else(
        || path.display().to_string(),
        |stem| stem.to_string_lossy().into_owned(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wrappers_are_labelled_by_their_app_name() {
        assert_eq!(
            wrapper_label(Path::new("/Users/a/Applications/Sikarugir/KartRider.app")),
            "KartRider"
        );
    }

    #[test]
    fn a_missing_crossover_is_reported() {
        let error = ensure_crossover_bottle(Path::new("/nonexistent/bin/wine"), "P5099")
            .unwrap_err()
            .to_string();
        assert!(error.contains("CrossOver를 찾지 못했습니다"), "{error}");
    }

    #[test]
    #[ignore = "needs CrossOver with at least one bottle"]
    fn existing_crossover_bottles_are_found_and_kept() {
        let bottles = crossover_bottles();
        let bottle = bottles.first().expect("a CrossOver bottle");
        assert!(!ensure_crossover_bottle(&default_crossover_wine(), bottle).unwrap());
    }
}
