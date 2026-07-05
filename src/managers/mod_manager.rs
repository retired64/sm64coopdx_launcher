use std::collections::HashSet;
use std::fs;
use std::io;
use std::path::Path;
#[cfg(test)]
use std::path::PathBuf;

use crate::ui::common::{ItemType, UiItem};

/// Scan the mods directory and build a Vec<UiItem> for the item selector.
///
/// Mod roots are:
/// - `.lua` files (type: Lua)
/// - Directories containing `main.lua` (type: Folder)
///
/// Any other entry (junk files, empty dirs, dirs without main.lua) is
/// silently ignored. If the mods directory does not exist, it is created
/// and an empty list is returned.
pub fn scan_mods(mods_dir: &Path) -> Result<Vec<UiItem>, String> {
    if !mods_dir.exists() {
        fs::create_dir_all(mods_dir)
            .map_err(|e| format!("Failed to create mods dir {:?}: {e}", mods_dir))?;
        return Ok(Vec::new());
    }

    let mut items: Vec<UiItem> = Vec::new();
    let entries = fs::read_dir(mods_dir)
        .map_err(|e| format!("Failed to read mods dir {:?}: {e}", mods_dir))?;

    for entry in entries {
        let entry = entry.map_err(|e| format!("Dir entry error: {e}"))?;
        let path = entry.path();
        let name = entry.file_name();

        if path.is_file() {
            if path.extension().is_some_and(|ext| ext == "lua") {
                let mod_name = path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("unknown")
                    .to_string();
                let rel_path = name.to_string_lossy().to_string();
                items.push(UiItem {
                    name: mod_name,
                    rel_path,
                    enabled: false,
                    item_type: ItemType::Toggle,
                    value: String::new(),
                });
            }
        } else if path.is_dir() && path.join("main.lua").exists() {
            let mod_name = name.to_string_lossy().to_string();
            items.push(UiItem {
                name: mod_name.clone(),
                rel_path: mod_name,
                enabled: false,
                item_type: ItemType::Toggle,
                value: String::new(),
            });
        }
        // All other entries (symlinks, files without .lua, dirs without
        // main.lua) are silently ignored.
    }

    // Stable sort by name for deterministic display
    items.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(items)
}

/// Parse `enable-mod:` lines from sm64config.txt and return the set of
/// enabled mod names.
pub fn parse_enabled_mods(config_path: &Path) -> Result<HashSet<String>, String> {
    let content = match fs::read_to_string(config_path) {
        Ok(c) => c,
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            return Ok(HashSet::new());
        }
        Err(e) => return Err(format!("Failed to read config {:?}: {e}", config_path)),
    };

    let mut enabled = HashSet::new();
    for line in content.lines() {
        let trimmed = line.trim();
        if let Some(name) = trimmed.strip_prefix("enable-mod: ") {
            enabled.insert(name.to_string());
        }
        // Also support "enable-mod:" without space (legacy)
        if let Some(name) = trimmed.strip_prefix("enable-mod:") {
            let name = name.trim().to_string();
            if !name.is_empty() {
                enabled.insert(name);
            }
        }
    }
    Ok(enabled)
}

/// Apply enabled/disabled state from config to a scanned list of UiItems.
pub fn apply_enabled_state(items: &mut [UiItem], enabled: &HashSet<String>) {
    for item in items.iter_mut() {
        item.enabled = enabled.contains(&item.name) || enabled.contains(&item.rel_path);
    }
}

/// Write the `enable-mod:` section of sm64config.txt atomically.
///
/// Only lines starting with `enable-mod:` are touched; all other lines
/// (network, dynos, etc.) are preserved exactly as they were, in their
/// original order.
///
/// Atomicity: writes to a `.tmp` file, then `fs::rename` to the target.
/// On POSIX this is atomic; on Windows it's a best-effort replacement.
pub fn write_enabled_mods(config_path: &Path, enabled_names: &[String]) -> Result<(), String> {
    let prefix = "enable-mod:";

    // Read existing lines (keep non-prefix lines untouched)
    let mut non_mod_lines: Vec<String> = Vec::new();
    if config_path.exists() {
        let content =
            fs::read_to_string(config_path).map_err(|e| format!("Failed to read config: {e}"))?;
        for line in content.lines() {
            let trimmed = line.trim();
            if !trimmed.starts_with(prefix) && !trimmed.is_empty() {
                non_mod_lines.push(line.to_string());
            }
            // Also keep completely empty lines for spacing
            if trimmed.is_empty() {
                non_mod_lines.push(String::new());
            }
        }
    }

    // Ensure parent directory exists
    if let Some(parent) = config_path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("Failed to create config dir: {e}"))?;
    }

    // Build output: non-mod lines first, then enable-mod entries
    let mut output = String::new();
    for line in &non_mod_lines {
        output.push_str(line);
        output.push('\n');
    }
    for name in enabled_names {
        output.push_str(&format!("{prefix} {name}\n"));
    }

    // Atomic write via .tmp + rename
    let tmp = config_path.with_extension("tmp");
    fs::write(&tmp, &output).map_err(|e| format!("Failed to write tmp config: {e}"))?;
    fs::rename(&tmp, config_path).map_err(|e| format!("Failed to rename config: {e}"))?;

    Ok(())
}

/// Toggle a single mod and persist to disk. Returns the new enabled state.
#[allow(dead_code)]
pub fn toggle_mod(
    config_path: &Path,
    mod_rel_path: &str,
    current_enabled: &HashSet<String>,
) -> Result<(bool, HashSet<String>), String> {
    let mut new_set = current_enabled.clone();
    let was_enabled = new_set.contains(mod_rel_path);
    if was_enabled {
        new_set.remove(mod_rel_path);
    } else {
        new_set.insert(mod_rel_path.to_string());
    }
    let names: Vec<String> = new_set.iter().cloned().collect();
    write_enabled_mods(config_path, &names)?;
    Ok((!was_enabled, new_set))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn tmp_dir(prefix: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("sm64launcher_test_{prefix}"));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn scan_detects_lua_files() {
        let dir = tmp_dir("scan_lua");
        fs::write(dir.join("mod_a.lua"), "-- test").unwrap();
        fs::write(dir.join("mod_b.lua"), "").unwrap();
        fs::write(dir.join("readme.txt"), "junk").unwrap(); // should be ignored

        let items = scan_mods(&dir).unwrap();
        assert_eq!(items.len(), 2);
        assert!(items.iter().any(|i| i.name == "mod_a"));
        assert!(items.iter().any(|i| i.name == "mod_b"));
        assert!(!items.iter().any(|i| i.name == "readme"));
    }

    #[test]
    fn scan_detects_folder_mods() {
        let dir = tmp_dir("scan_folder");
        fs::create_dir(dir.join("my_mod")).unwrap();
        fs::write(dir.join("my_mod/main.lua"), "").unwrap();
        fs::create_dir(dir.join("empty_dir")).unwrap(); // no main.lua
        fs::write(dir.join("junk.bin"), "xx").unwrap();

        let items = scan_mods(&dir).unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].name, "my_mod");
        assert_eq!(items[0].rel_path, "my_mod");
    }

    #[test]
    fn scan_creates_missing_dir() {
        let parent = tmp_dir("scan_missing");
        let missing = parent.join("does_not_exist");
        assert!(!missing.exists());
        let items = scan_mods(&missing).unwrap();
        assert!(items.is_empty());
        assert!(missing.exists());
    }

    #[test]
    fn parse_enabled_handles_mixed_lines() {
        let dir = tmp_dir("parse_mixed");
        let config = dir.join("sm64config.txt");
        fs::write(
            &config,
            "enable-mod: mod_one\n\
             dynos-pack: some_pack\n\
             enable-mod: mod_two\n\
             coop_player_name Player1\n\
             enable-mod:\n\
             \n",
        )
        .unwrap();

        let enabled = parse_enabled_mods(&config).unwrap();
        assert_eq!(enabled.len(), 2);
        assert!(enabled.contains("mod_one"));
        assert!(enabled.contains("mod_two"));
        // Empty enable-mod: should NOT be included
        assert!(!enabled.contains(""));
    }

    #[test]
    fn parse_returns_empty_for_missing_file() {
        let enabled = parse_enabled_mods(Path::new("/nonexistent/config.txt")).unwrap();
        assert!(enabled.is_empty());
    }

    #[test]
    fn write_preserves_other_lines() {
        let dir = tmp_dir("write_preserve");
        let config = dir.join("sm64config.txt");
        fs::write(
            &config,
            "dynos-pack: my_pack\n\
             coop_player_name Alice\n\
             enable-mod: old_mod\n",
        )
        .unwrap();

        write_enabled_mods(&config, &["new_mod".to_string()]).unwrap();

        let content = fs::read_to_string(&config).unwrap();
        assert!(content.contains("dynos-pack: my_pack"));
        assert!(content.contains("coop_player_name Alice"));
        assert!(!content.contains("old_mod")); // removed
        assert!(content.contains("enable-mod: new_mod"));
    }

    #[test]
    fn write_is_atomic_no_leftover_tmp() {
        let dir = tmp_dir("write_atomic");
        let config = dir.join("sm64config.txt");
        fs::write(&config, "enable-mod: old\n").unwrap();
        let tmp = dir.join("sm64config.tmp");

        write_enabled_mods(&config, &["a".to_string()]).unwrap();
        assert!(!tmp.exists(), ".tmp should not survive a successful write");
        assert!(config.exists());
    }

    #[test]
    fn toggle_mod_flips_state() {
        let dir = tmp_dir("toggle");
        let config = dir.join("sm64config.txt");
        fs::write(&config, "enable-mod: test_mod\n").unwrap();

        let initial = parse_enabled_mods(&config).unwrap();
        assert!(initial.contains("test_mod"));

        let (new_state, new_set) = toggle_mod(&config, "test_mod", &initial).unwrap();
        assert!(!new_state);
        assert!(!new_set.contains("test_mod"));

        // Toggle back
        let (back, _) = toggle_mod(&config, "test_mod", &new_set).unwrap();
        assert!(back);
    }
}
