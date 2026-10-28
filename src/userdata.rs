use std::path::{Path, PathBuf};
use std::process::Command;

const STEAM_APP_ID: &str = "3493540";

pub fn find_userdata() -> Option<PathBuf> {
    if let Ok(appdata) = std::env::var("APPDATA") {
        let native = Path::new(&appdata).join("Transport Fever 3");
        if native.is_dir() {
            return Some(native);
        }
    }

    if let Some(steam) = steam_path() {
        if let Some(userdata) = latest_user_local(&steam.join("userdata")) {
            return Some(userdata);
        }
    }
    None
}

pub fn find_from_game_dir(game_dir: &Path) -> Option<PathBuf> {
    let mut path = game_dir.to_path_buf();
    for _ in 0..3 {
        path = path.parent()?.to_path_buf();
    }
    let userdata = latest_user_local(&path.join("userdata"));
    if userdata.is_some() {
        return userdata;
    }
    find_userdata()
}

fn latest_user_local(userdata: &Path) -> Option<PathBuf> {
    let entries = std::fs::read_dir(userdata).ok()?;
    let mut users: Vec<PathBuf> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .collect();
    if users.is_empty() {
        return None;
    }
    users.sort_by_key(|path| path.metadata().and_then(|m| m.modified()).ok());

    let mut existing: Vec<PathBuf> = users
        .iter()
        .map(|user| user.join(STEAM_APP_ID).join("local"))
        .filter(|path| path.is_dir())
        .collect();
    if let Some(latest) = existing.pop() {
        return Some(latest);
    }

    users
        .pop()
        .map(|user| user.join(STEAM_APP_ID).join("local"))
}

fn steam_path() -> Option<PathBuf> {
    if let Ok(output) = Command::new("reg")
        .args(["query", "HKCU\\Software\\Valve\\Steam", "/v", "SteamPath"])
        .output()
    {
        if output.status.success() {
            let text = String::from_utf8_lossy(&output.stdout);
            if let Some(line) = text.lines().find(|line| line.contains("SteamPath")) {
                if let Some(index) = line.find("REG_SZ") {
                    let raw = line[index + "REG_SZ".len()..].trim();
                    let path = PathBuf::from(raw.replace('/', "\\"));
                    if path.is_dir() {
                        return Some(path);
                    }
                }
            }
        }
    }
    for guess in ["C:\\Program Files (x86)\\Steam", "C:\\Program Files\\Steam"] {
        let path = PathBuf::from(guess);
        if path.is_dir() {
            return Some(path);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derives_userdata_from_steam_game_dir() {
        let root = std::env::temp_dir().join(format!("tf3-mapgen-test-{}", std::process::id()));
        let game_dir = root.join("steamapps/common/Transport Fever 3");
        let local = root.join("userdata/42/3493540/local");
        std::fs::create_dir_all(&game_dir).unwrap();
        std::fs::create_dir_all(&local).unwrap();
        let found = find_from_game_dir(&game_dir);
        assert_eq!(found.as_deref(), Some(local.as_path()));
        let _ = std::fs::remove_dir_all(&root);
    }
}
