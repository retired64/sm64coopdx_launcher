use std::collections::HashSet;
use std::fs;
use std::io;
use std::path::Path;
#[cfg(test)]
use std::path::PathBuf;

use crate::ui::common::{ItemType, UiItem};

/// Scan the dynos/packs directory and build a Vec<UiItem>.
///
/// Only immediate subdirectories are considered as DynOS packs (the game
/// loads packs by directory name). Files and directories without a valid
/// pack structure are silently ignored.
pub fn scan_packs(packs_dir: &Path) -> Result<Vec<UiItem>, String> {
    if !packs_dir.exists() {
        fs::create_dir_all(packs_dir)
            .map_err(|e| format!("Failed to create dynos/packs dir {:?}: {e}", packs_dir))?;
        return Ok(Vec::new());
    }

    let mut items: Vec<UiItem> = Vec::new();
    let entries = fs::read_dir(packs_dir)
        .map_err(|e| format!("Failed to read dynos/packs dir {:?}: {e}", packs_dir))?;

    for entry in entries {
        let entry = entry.map_err(|e| format!("Dir entry error: {e}"))?;
        let path = entry.path();
        if path.is_dir() {
            let name = entry.file_name().to_string_lossy().to_string();
            items.push(UiItem {
                name: name.clone(),
                rel_path: name,
                enabled: false,
                item_type: ItemType::Toggle,
                value: String::new(),
            });
        }
    }

    items.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(items)
}

/// Parse `dynos-pack:` lines from sm64config.txt.
pub fn parse_enabled_packs(config_path: &Path) -> Result<HashSet<String>, String> {
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
        if let Some(name) = trimmed.strip_prefix("dynos-pack: ") {
            enabled.insert(name.to_string());
        }
        if let Some(name) = trimmed.strip_prefix("dynos-pack:") {
            let name = name.trim().to_string();
            if !name.is_empty() {
                enabled.insert(name);
            }
        }
    }
    Ok(enabled)
}

/// Apply enabled/disabled state from config.
pub fn apply_enabled_state(items: &mut [UiItem], enabled: &HashSet<String>) {
    for item in items.iter_mut() {
        item.enabled = enabled.contains(&item.name) || enabled.contains(&item.rel_path);
    }
}

/// Write the `dynos-pack:` section atomically.
pub fn write_enabled_packs(config_path: &Path, enabled_names: &[String]) -> Result<(), String> {
    let prefix = "dynos-pack:";

    let mut non_prefix_lines: Vec<String> = Vec::new();
    if config_path.exists() {
        let content =
            fs::read_to_string(config_path).map_err(|e| format!("Failed to read config: {e}"))?;
        for line in content.lines() {
            let trimmed = line.trim();
            if !trimmed.starts_with(prefix) && !trimmed.is_empty() {
                non_prefix_lines.push(line.to_string());
            }
            if trimmed.is_empty() {
                non_prefix_lines.push(String::new());
            }
        }
    }

    if let Some(parent) = config_path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("Failed to create config dir: {e}"))?;
    }

    let mut output = String::new();
    for line in &non_prefix_lines {
        output.push_str(line);
        output.push('\n');
    }
    for name in enabled_names {
        output.push_str(&format!("{prefix} {name}\n"));
    }

    let tmp = config_path.with_extension("tmp");
    fs::write(&tmp, &output).map_err(|e| format!("Failed to write tmp config: {e}"))?;
    fs::rename(&tmp, config_path).map_err(|e| format!("Failed to rename config: {e}"))?;

    Ok(())
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
    fn scan_detects_subdirs() {
        let dir = tmp_dir("scan_dynos");
        fs::create_dir(dir.join("pack_a")).unwrap();
        fs::create_dir(dir.join("pack_b")).unwrap();
        fs::write(dir.join("junk.txt"), "x").unwrap(); // should be ignored

        let items = scan_packs(&dir).unwrap();
        assert_eq!(items.len(), 2);
        assert!(items.iter().any(|i| i.name == "pack_a"));
        assert!(items.iter().any(|i| i.name == "pack_b"));
    }

    #[test]
    fn scan_creates_missing_dir() {
        let parent = tmp_dir("scan_dynos_missing");
        let missing = parent.join("does_not_exist");
        let items = scan_packs(&missing).unwrap();
        assert!(items.is_empty());
        assert!(missing.exists());
    }

    #[test]
    fn parse_enabled_handles_mixed_lines() {
        let dir = tmp_dir("parse_dynos_mixed");
        let config = dir.join("sm64config.txt");
        fs::write(
            &config,
            "enable-mod: some_mod\n\
             dynos-pack: pack_one\n\
             coop_player_name Player1\n\
             dynos-pack: pack_two\n",
        )
        .unwrap();

        let enabled = parse_enabled_packs(&config).unwrap();
        assert_eq!(enabled.len(), 2);
        assert!(enabled.contains("pack_one"));
        assert!(enabled.contains("pack_two"));
    }

    #[test]
    fn write_preserves_other_lines() {
        let dir = tmp_dir("write_dynos_preserve");
        let config = dir.join("sm64config.txt");
        fs::write(
            &config,
            "enable-mod: my_mod\n\
             coop_player_name Bob\n\
             dynos-pack: old_pack\n",
        )
        .unwrap();

        write_enabled_packs(&config, &["new_pack".to_string()]).unwrap();
        let content = fs::read_to_string(&config).unwrap();
        assert!(content.contains("enable-mod: my_mod"));
        assert!(content.contains("coop_player_name Bob"));
        assert!(!content.contains("old_pack"));
        assert!(content.contains("dynos-pack: new_pack"));
    }

    #[test]
    fn write_is_atomic_no_leftover_tmp() {
        let dir = tmp_dir("write_dynos_atomic");
        let config = dir.join("sm64config.txt");
        fs::write(&config, "dynos-pack: old\n").unwrap();
        let tmp = dir.join("sm64config.tmp");

        write_enabled_packs(&config, &["a".to_string()]).unwrap();
        assert!(!tmp.exists(), ".tmp should not survive a successful write");
        assert!(config.exists());
    }
}
