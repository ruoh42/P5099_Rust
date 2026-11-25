use std::{fs, path::Path};

const OLD_STORAGE_ID: &str = "카트라이더 P5099";
pub const OLD_APP_DATA_NAME: &str = "kartrider-p5099";
const SETTINGS_FILE: &str = "app.ron";

pub fn migrate_settings(storage_id: &str) {
    let (Some(old_folder), Some(new_folder)) = (
        eframe::storage_dir(OLD_STORAGE_ID),
        eframe::storage_dir(storage_id),
    ) else {
        return;
    };
    let (old, new) = (
        old_folder.join(SETTINGS_FILE),
        new_folder.join(SETTINGS_FILE),
    );
    if old == new || new.exists() || !old.is_file() {
        return;
    }
    let copied = fs::create_dir_all(&new_folder).and_then(|()| fs::copy(&old, &new));
    if copied.is_ok() {
        let _ = fs::remove_file(&old);
        let _ = fs::remove_dir(&old_folder);
        if let Some(parent) = old_folder.parent()
            && parent.ends_with(OLD_STORAGE_ID)
        {
            let _ = fs::remove_dir(parent);
        }
    }
}

pub fn migrate_game(old: &Path, new: &Path) -> bool {
    if old == new || !has_entries(old) || has_entries(new) {
        return true;
    }
    if let Some(parent) = new.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let _ = fs::remove_dir(new);
    if fs::rename(old, new).is_err() {
        return false;
    }
    if let Some(parent) = old.parent() {
        let _ = fs::remove_dir(parent);
    }
    true
}

fn has_entries(folder: &Path) -> bool {
    fs::read_dir(folder).is_ok_and(|mut entries| entries.next().is_some())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_game_moves_to_the_new_folder_once() {
        let root = tempfile::tempdir().unwrap();
        let old = root.path().join(OLD_APP_DATA_NAME).join("Game");
        let new = root.path().join("p5099").join("Game");
        fs::create_dir_all(&old).unwrap();
        fs::write(old.join("KartRider.exe"), "game").unwrap();
        fs::create_dir_all(&new).unwrap();

        assert!(migrate_game(&old, &new));
        assert_eq!(fs::read(new.join("KartRider.exe")).unwrap(), b"game");
        assert!(!root.path().join(OLD_APP_DATA_NAME).exists());

        assert!(migrate_game(&old, &new));
        assert!(new.join("KartRider.exe").is_file());
    }

    #[test]
    fn a_game_in_the_new_folder_is_never_replaced() {
        let root = tempfile::tempdir().unwrap();
        let (old, new) = (root.path().join("old"), root.path().join("new"));
        fs::create_dir_all(&old).unwrap();
        fs::create_dir_all(&new).unwrap();
        fs::write(old.join("KartRider.exe"), "old").unwrap();
        fs::write(new.join("KartRider.exe"), "new").unwrap();

        assert!(migrate_game(&old, &new));
        assert_eq!(fs::read(new.join("KartRider.exe")).unwrap(), b"new");
        assert!(old.join("KartRider.exe").is_file());
    }
}
