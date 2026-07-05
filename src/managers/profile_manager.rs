use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::ui::common::{ItemType, UiItem};

// ═══════════════════════════════════════════════════════════════════════
// ProfileConfig — per‑profile game options
//
// ── Campos §13 (contrato duro de la spec, cita textual) ──
//   playername, skip_intro, no_discord, fullscreen, windowed,
//   skip_update_check, headless
//
// ── Campos inferidos de fuentes secundarias ──
//   created    → stubs/profile_manager.py del launcher Python original
//                (incluye "created" en profile.json como timestamp ISO‑8601)
//   game_path  → prompt‑rust‑launcher.md §9B comentario:
//                "profiles/<name>/profile.json can have 'game_path' field"
// ═══════════════════════════════════════════════════════════════════════
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ProfileConfig {
    pub playername: String,
    #[serde(default)]
    pub skip_intro: bool,
    #[serde(default)]
    pub no_discord: bool,
    #[serde(default)]
    pub fullscreen: bool,
    #[serde(default)]
    pub windowed: bool,
    #[serde(default)]
    pub skip_update_check: bool,
    #[serde(default)]
    pub headless: bool,
    #[serde(default)]
    pub created: Option<String>,
    #[serde(default)]
    pub game_path: Option<String>,
}

/// Ensure the Default profile exists under `profiles_dir`.
/// Creates the directory, profile.json, and saves/ subdir.
pub fn ensure_default_profile(profiles_dir: &Path) -> Result<(), String> {
    let default_dir = profiles_dir.join("Default");
    if !default_dir.exists() {
        fs::create_dir_all(default_dir.join("saves"))
            .map_err(|e| format!("Cannot create Default saves: {e}"))?;
        let config = ProfileConfig {
            playername: "Default".into(),
            created: Some(chrono_like_now()),
            ..Default::default()
        };
        save_profile_config(&default_dir, &config)?;
    }

    // Ensure active.txt exists
    let active_path = profiles_dir.join("active.txt");
    if !active_path.exists() {
        set_active_profile(profiles_dir, "Default")?;
    }

    Ok(())
}

/// Simple ISO‑8601-like timestamp without adding a chrono dependency.
fn chrono_like_now() -> String {
    use std::time::SystemTime;
    let dur = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default();
    let secs = dur.as_secs();
    // YYYY-MM-DDTHH:MM:SSZ approximation
    let days = secs / 86400;
    let time_of_day = secs % 86400;
    let hours = time_of_day / 3600;
    let minutes = (time_of_day % 3600) / 60;
    let seconds = time_of_day % 60;

    // Rough date calculation from epoch (good enough for a display string)
    let (y, m, d) = days_since_epoch(days);
    format!("{y:04}-{m:02}-{d:02}T{hours:02}:{minutes:02}:{seconds:02}Z")
}

fn days_since_epoch(days: u64) -> (u64, u64, u64) {
    let mut remaining = days;
    let mut y = 1970u64;
    loop {
        let days_in_year = if is_leap(y) { 366 } else { 365 };
        if remaining < days_in_year {
            break;
        }
        remaining -= days_in_year;
        y += 1;
    }
    let months_days = if is_leap(y) {
        [31, 29, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    } else {
        [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    };
    let mut m = 1u64;
    for &md in &months_days {
        if remaining < md {
            break;
        }
        remaining -= md;
        m += 1;
    }
    (y, m, remaining + 1)
}

fn is_leap(y: u64) -> bool {
    y.is_multiple_of(4) && (!y.is_multiple_of(100) || y.is_multiple_of(400))
}

/// Load ProfileConfig from profile.json inside a profile directory.
pub fn load_profile_config(profile_dir: &Path) -> Result<ProfileConfig, String> {
    let json_path = profile_dir.join("profile.json");
    let raw = fs::read_to_string(&json_path)
        .map_err(|e| format!("Cannot read {}: {e}", json_path.display()))?;
    serde_json::from_str(&raw)
        .map_err(|e| format!("Invalid profile.json in {}: {e}", profile_dir.display()))
}

/// Save ProfileConfig to profile.json atomically (.tmp → rename).
pub fn save_profile_config(profile_dir: &Path, config: &ProfileConfig) -> Result<(), String> {
    let json_path = profile_dir.join("profile.json");
    let tmp = json_path.with_extension(".tmp");
    let json = serde_json::to_string_pretty(config).map_err(|e| format!("JSON serialize: {e}"))?;
    fs::write(&tmp, &json).map_err(|e| format!("Cannot write {}: {e}", tmp.display()))?;
    fs::rename(&tmp, &json_path).map_err(|e| format!("Atomic rename failed: {e}"))
}

/// Read the name of the active profile from active.txt.
/// Returns None if the file is missing or empty.
pub fn get_active_profile_name(profiles_dir: &Path) -> Option<String> {
    let active_path = profiles_dir.join("active.txt");
    let content = fs::read_to_string(&active_path).ok()?;
    let name = content.trim().to_string();
    if name.is_empty() { None } else { Some(name) }
}

/// Write the active profile name to active.txt atomically.
pub fn set_active_profile(profiles_dir: &Path, name: &str) -> Result<(), String> {
    let active_path = profiles_dir.join("active.txt");
    let tmp = profiles_dir.join(".active.tmp");
    fs::write(&tmp, name.as_bytes()).map_err(|e| format!("Cannot write active.txt: {e}"))?;
    fs::rename(&tmp, &active_path).map_err(|e| format!("Atomic rename failed: {e}"))
}

/// Scan the profiles directory and return a list of UiItems for the
/// item selector, plus the name of the currently active profile.
///
/// The last item in the list is always the synthetic "+ New Profile" action.
pub fn scan_profiles(profiles_dir: &Path, active_name: &str) -> Result<Vec<UiItem>, String> {
    if !profiles_dir.exists() {
        return Ok(vec![synthetic_new_item()]);
    }

    let mut items: Vec<UiItem> = Vec::new();
    let entries =
        fs::read_dir(profiles_dir).map_err(|e| format!("Cannot scan profiles dir: {e}"))?;

    for entry in entries {
        let entry = entry.map_err(|e| format!("Dir entry error: {e}"))?;
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        let name = name.to_string();

        // Load playername from profile.json for display
        let playername = load_profile_config(&path)
            .map(|c| c.playername)
            .unwrap_or_else(|_| name.clone());

        let is_active = name == active_name;

        items.push(UiItem {
            name,
            rel_path: path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default(),
            enabled: is_active,
            item_type: ItemType::Toggle,
            value: playername,
        });
    }

    // Sort alphabetically (Default always first)
    items.sort_by(|a, b| {
        if a.name == "Default" {
            std::cmp::Ordering::Less
        } else if b.name == "Default" {
            std::cmp::Ordering::Greater
        } else {
            a.name.to_lowercase().cmp(&b.name.to_lowercase())
        }
    });

    // Append "+ New Profile" synthetic item
    items.push(synthetic_new_item());

    Ok(items)
}

fn synthetic_new_item() -> UiItem {
    UiItem {
        name: "+ New Profile".into(),
        rel_path: "__new__".into(),
        enabled: false,
        item_type: ItemType::Action,
        value: String::new(),
    }
}

/// Build CLI args from a ProfileConfig for game launch (step 28).
///
/// Per logica-launcher.md §10.2 and verified against the real sm64coopdx
/// binary (--help output).
///
/// `data_dir` is the launcher's XDG data dir (~/.local/share/sm64coopdx/)
/// and becomes the game's --savepath so that launcher and game share the
/// same virtual filesystem root for mods, ROM, dynos, etc.
#[allow(dead_code)]
pub fn build_profile_args(config: &ProfileConfig, profile_dir: &Path, data_dir: &Path) -> Vec<String> {
    let mut args = Vec::new();

    // --savepath = data_dir so game and launcher share the same filesystem root.
    // The game will find ROM, mods, dynos, etc. in the same place the launcher
    // manages them.
    args.push("--savepath".into());
    args.push(data_dir.to_string_lossy().into_owned());

    let config_file = profile_dir.join("sm64config.txt");
    // Only pass --configfile if the file has actual content.
    // An empty file (created by create_profile when no parent config exists)
    // would override the game's own sm64config.txt with nothing, breaking ROM
    // path resolution and other settings.
    let config_non_empty = config_file.metadata().map(|m| m.len() > 0).unwrap_or(false);
    if config_non_empty {
        args.push("--configfile".into());
        args.push(config_file.to_string_lossy().into_owned());
    }

    if !config.playername.is_empty() {
        args.push("--playername".into());
        args.push(config.playername.clone());
    }

    if config.skip_intro {
        args.push("--skip-intro".into());
    }
    if config.no_discord {
        args.push("--no-discord".into());
    }
    if config.fullscreen {
        args.push("--fullscreen".into());
    }
    if config.windowed {
        args.push("--windowed".into());
    }
    if config.skip_update_check {
        args.push("--skip-update-check".into());
    }
    if config.headless {
        args.push("--headless".into());
    }

    args
}

// ═══════════════════════════════════════════════════════════════════════
// Step 26: Profile CRUD
// ═══════════════════════════════════════════════════════════════════════

/// Check if a profile directory exists (case‑insensitive for filesystem safety).
pub fn profile_exists(profiles_dir: &Path, name: &str) -> bool {
    let lower = name.to_lowercase();
    if let Ok(entries) = fs::read_dir(profiles_dir) {
        entries
            .filter_map(|e| e.ok())
            .any(|e| e.file_name().to_string_lossy().to_lowercase() == lower)
    } else {
        false
    }
}

/// Check whether `name` is the currently active profile.
pub fn is_active(profiles_dir: &Path, name: &str) -> bool {
    get_active_profile_name(profiles_dir).as_deref() == Some(name)
}

/// Sanitize a profile name for use as a directory name.
/// Strips leading/trailing whitespace and replaces filesystem-unsafe chars.
fn sanitize_profile_name(name: &str) -> String {
    let trimmed = name.trim();
    trimmed
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' | '\0' => '_',
            c => c,
        })
        .collect::<String>()
        .trim()
        .to_string()
}

/// Create a new profile directory atomically.
///
/// 1. Validates name is non‑empty and not a duplicate.
/// 2. Builds the profile in a temporary location, then renames atomically.
/// 3. Copies the parent `sm64config.txt` if it exists.
/// 4. Writes `profile.json` with playername + timestamp.
pub fn create_profile(
    profiles_dir: &Path,
    name: &str,
    parent_config_path: &Path,
) -> Result<(), String> {
    let name = sanitize_profile_name(name);
    if name.is_empty() {
        return Err("Profile name cannot be empty".into());
    }
    if profile_exists(profiles_dir, &name) {
        return Err(format!("Profile '{name}' already exists"));
    }

    let tmp_dir = profiles_dir.join(format!(".{name}.tmp"));
    let final_dir = profiles_dir.join(&name);

    // Cleanup any leftover temp from a previous failed attempt
    if tmp_dir.exists() {
        let _ = fs::remove_dir_all(&tmp_dir);
    }

    // Build in tmp
    fs::create_dir_all(tmp_dir.join("saves"))
        .map_err(|e| format!("Cannot create saves dir: {e}"))?;

    // Copy parent sm64config.txt if it exists
    if parent_config_path.is_file() {
        fs::copy(parent_config_path, tmp_dir.join("sm64config.txt"))
            .map_err(|e| format!("Cannot copy sm64config.txt: {e}"))?;
    }

    // Write profile.json
    let config = ProfileConfig {
        playername: name.clone(),
        created: Some(chrono_like_now()),
        ..Default::default()
    };
    save_profile_config(&tmp_dir, &config)?;

    // Atomic rename
    if final_dir.exists() {
        return Err(format!("Profile '{name}' already exists (race condition)"));
    }
    fs::rename(&tmp_dir, &final_dir)
        .map_err(|e| format!("Cannot finalize profile '{}': {e}", name))?;

    log::info!("Created profile '{}'", name);
    Ok(())
}

/// Rename a profile directory.
///
/// - Validates old exists, new is non‑empty and not a duplicate.
/// - Uses `std::fs::rename` of the directory.
/// - If the renamed profile was active, updates `active.txt`.
pub fn rename_profile(profiles_dir: &Path, old_name: &str, new_name: &str) -> Result<(), String> {
    let new_name = sanitize_profile_name(new_name);
    if new_name.is_empty() {
        return Err("Profile name cannot be empty".into());
    }
    if !profile_exists(profiles_dir, old_name) {
        return Err(format!("Profile '{old_name}' not found"));
    }
    let lower_new = new_name.to_lowercase();
    let lower_old = old_name.to_lowercase();
    if lower_new != lower_old && profile_exists(profiles_dir, &new_name) {
        return Err(format!("Profile '{new_name}' already exists"));
    }

    let old_dir = profiles_dir.join(old_name);
    let new_dir = profiles_dir.join(&new_name);

    let was_active = is_active(profiles_dir, old_name);

    fs::rename(&old_dir, &new_dir)
        .map_err(|e| format!("Cannot rename '{old_name}' → '{new_name}': {e}"))?;

    // Update active.txt if needed
    if was_active {
        set_active_profile(profiles_dir, &new_name)?;
    }

    log::info!("Renamed profile '{}' → '{}'", old_name, new_name);
    Ok(())
}

/// Delete a profile directory.
///
/// Blocks: cannot delete "Default" or the currently active profile.
/// Returns `Err` with a descriptive message if blocked.
pub fn delete_profile(profiles_dir: &Path, name: &str) -> Result<(), String> {
    if name.eq_ignore_ascii_case("Default") {
        return Err("Cannot delete the Default profile".into());
    }
    if is_active(profiles_dir, name) {
        return Err(format!(
            "Cannot delete active profile '{}'. Activate another first.",
            name
        ));
    }

    let dir = profiles_dir.join(name);
    if !dir.is_dir() {
        return Err(format!("Profile '{name}' not found"));
    }

    fs::remove_dir_all(&dir).map_err(|e| format!("Cannot delete '{name}': {e}"))?;

    log::info!("Deleted profile '{}'", name);
    Ok(())
}

/// Build the list of UiItems for the ProfileDetail sub‑screen (step 27).
///
/// Fields per spec §8F / §10.3:
/// - Player Name (text, ENTER opens keyboard)
/// - Skip Intro (toggle)
/// - Discord Rich Presence (toggle, INVERTED: enabled = !no_discord)
/// - Fullscreen (toggle)
/// - Windowed (toggle)
/// - Skip Update Check (toggle)
/// - Headless (Server) (toggle)
pub fn build_profile_detail_items(config: &ProfileConfig) -> Vec<UiItem> {
    vec![
        UiItem {
            name: "Player Name".into(),
            rel_path: "playername".into(),
            enabled: false,
            item_type: ItemType::Text,
            value: config.playername.clone(),
        },
        UiItem {
            name: "Skip Intro".into(),
            rel_path: "skip_intro".into(),
            enabled: config.skip_intro,
            item_type: ItemType::Toggle,
            value: String::new(),
        },
        // Discord Rich Presence: visual toggle = !no_discord
        UiItem {
            name: "Discord Rich Presence".into(),
            rel_path: "no_discord".into(),
            enabled: !config.no_discord,
            item_type: ItemType::Toggle,
            value: String::new(),
        },
        UiItem {
            name: "Fullscreen".into(),
            rel_path: "fullscreen".into(),
            enabled: config.fullscreen,
            item_type: ItemType::Toggle,
            value: String::new(),
        },
        UiItem {
            name: "Windowed".into(),
            rel_path: "windowed".into(),
            enabled: config.windowed,
            item_type: ItemType::Toggle,
            value: String::new(),
        },
        UiItem {
            name: "Skip Update Check".into(),
            rel_path: "skip_update_check".into(),
            enabled: config.skip_update_check,
            item_type: ItemType::Toggle,
            value: String::new(),
        },
        UiItem {
            name: "Headless (Server)".into(),
            rel_path: "headless".into(),
            enabled: config.headless,
            item_type: ItemType::Toggle,
            value: String::new(),
        },
    ]
}

/// Toggle a boolean field in the profile config and save.
///
/// Handles the Discord inversion: `rel_path = "no_discord"` is flipped
/// directly, but the displayed value is `!no_discord`.
pub fn toggle_profile_config(
    profiles_dir: &Path,
    profile_name: &str,
    rel_path: &str,
) -> Result<ProfileConfig, String> {
    let profile_dir = profiles_dir.join(profile_name);
    let mut config = load_profile_config(&profile_dir)?;

    match rel_path {
        "skip_intro" => config.skip_intro = !config.skip_intro,
        "no_discord" => config.no_discord = !config.no_discord,
        "fullscreen" => config.fullscreen = !config.fullscreen,
        "windowed" => config.windowed = !config.windowed,
        "skip_update_check" => config.skip_update_check = !config.skip_update_check,
        "headless" => config.headless = !config.headless,
        _ => return Err(format!("Unknown config key: {rel_path}")),
    }

    save_profile_config(&profile_dir, &config)?;
    Ok(config)
}

/// Update the player name and save.
pub fn update_profile_playername(
    profiles_dir: &Path,
    profile_name: &str,
    new_name: &str,
) -> Result<ProfileConfig, String> {
    let profile_dir = profiles_dir.join(profile_name);
    let mut config = load_profile_config(&profile_dir)?;
    config.playername = new_name.to_string();
    save_profile_config(&profile_dir, &config)?;
    Ok(config)
}

/// Load the active profile's config and directory path.
///
/// Returns `None` if no profile is configured (shouldn't happen because
/// ensure_default_profile guarantees Default exists). Falls back to
/// "Default" if active.txt is missing or points to a deleted directory.
pub fn load_active_profile(profiles_dir: &Path) -> Option<(ProfileConfig, PathBuf)> {
    let active_name = get_active_profile_name(profiles_dir).unwrap_or_else(|| "Default".into());

    let profile_dir = profiles_dir.join(&active_name);
    if !profile_dir.is_dir() {
        let default_dir = profiles_dir.join("Default");
        if !default_dir.is_dir() {
            return None;
        }
        return load_profile_config(&default_dir)
            .ok()
            .map(|c| (c, default_dir));
    }
    load_profile_config(&profile_dir)
        .ok()
        .map(|c| (c, profile_dir))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn test_profiles_dir() -> PathBuf {
        std::env::temp_dir().join("sm64launcher_profile_test")
    }

    #[test]
    fn default_profile_auto_created() {
        let dir = test_profiles_dir().join("auto_created");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        ensure_default_profile(&dir).unwrap();
        assert!(dir.join("Default").is_dir());
        assert!(dir.join("Default/profile.json").exists());
        assert!(dir.join("Default/saves").is_dir());
        assert!(dir.join("active.txt").exists());

        let active = get_active_profile_name(&dir).unwrap();
        assert_eq!(active, "Default");

        let config = load_profile_config(&dir.join("Default")).unwrap();
        assert_eq!(config.playername, "Default");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn scan_includes_new_item() {
        let dir = test_profiles_dir().join("scan_test");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        ensure_default_profile(&dir).unwrap();

        let items = scan_profiles(&dir, "Default").unwrap();
        // Default + synthetic "+ New Profile"
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].name, "Default");
        assert!(items[0].enabled);
        assert_eq!(items[1].name, "+ New Profile");
        assert_eq!(items[1].item_type, ItemType::Action);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn active_tracking_roundtrip() {
        let dir = test_profiles_dir().join("active_roundtrip");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        set_active_profile(&dir, "SomeProfile").unwrap();
        let name = get_active_profile_name(&dir).unwrap();
        assert_eq!(name, "SomeProfile");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn profile_config_roundtrip() {
        let dir = test_profiles_dir().join("config_roundtrip");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        let config = ProfileConfig {
            playername: "TestPlayer".into(),
            skip_intro: true,
            no_discord: false,
            fullscreen: true,
            windowed: false,
            skip_update_check: true,
            headless: false,
            created: Some("2025-06-01T00:00:00Z".into()),
            game_path: None,
        };

        save_profile_config(&dir, &config).unwrap();
        let loaded = load_profile_config(&dir).unwrap();
        assert_eq!(loaded.playername, "TestPlayer");
        assert!(loaded.skip_intro);
        assert!(!loaded.no_discord);
        assert!(loaded.fullscreen);
        assert!(!loaded.windowed);
        assert!(loaded.skip_update_check);
        assert!(!loaded.headless);

        let _ = fs::remove_dir_all(&dir);
    }

    /// no_discord inversion: toggle ON in UI (Discord enabled) → no_discord=false → no --no-discord arg
    #[test]
    fn no_discord_inversion_correct() {
        let dir = test_profiles_dir().join("no_discord_test");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        fs::create_dir_all(dir.join("saves")).unwrap();
        let _ = fs::File::create(dir.join("sm64config.txt"));

        // Discord ON in UI → no_discord = false → NO --no-discord arg
        let config = ProfileConfig {
            playername: "Test".into(),
            no_discord: false,
            ..Default::default()
        };
        let args = build_profile_args(&config, &dir, &dir);
        assert!(
            !args.contains(&"--no-discord".to_string()),
            "Discord ON should NOT produce --no-discord arg"
        );

        // Discord OFF in UI → no_discord = true → --no-discord IS present
        let config2 = ProfileConfig {
            playername: "Test".into(),
            no_discord: true,
            ..Default::default()
        };
        let args2 = build_profile_args(&config2, &dir, &dir);
        assert!(
            args2.contains(&"--no-discord".to_string()),
            "Discord OFF should produce --no-discord arg"
        );

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn build_profile_args_all_fields() {
        let dir = test_profiles_dir().join("all_fields_test");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        fs::create_dir_all(dir.join("saves")).unwrap();
        // Write actual content so --configfile IS included
        fs::write(dir.join("sm64config.txt"), "enable-mod: test_mod\n").unwrap();

        let config = ProfileConfig {
            playername: "Speedrunner".into(),
            skip_intro: true,
            no_discord: true,
            fullscreen: true,
            windowed: false,
            skip_update_check: true,
            headless: true,
            created: None,
            game_path: None,
        };

        let args = build_profile_args(&config, &dir, &dir);
        assert!(args.contains(&"--savepath".into()));
        assert!(args.contains(&"--configfile".into()));
        assert!(args.contains(&"--playername".into()));
        assert!(args.contains(&"--skip-intro".into()));
        assert!(args.contains(&"--no-discord".into()));
        assert!(args.contains(&"--fullscreen".into()));
        assert!(!args.contains(&"--windowed".into())); // false → not present
        assert!(args.contains(&"--skip-update-check".into()));
        assert!(args.contains(&"--headless".into()));

        let _ = fs::remove_dir_all(&dir);
    }

    /// Empty sm64config.txt must NOT produce --configfile.
    /// Regression test for the bug where create_profile copies an empty
    /// parent config, causing the game to receive --configfile pointing
    /// to an empty file — which overrides the game's own config and
    /// breaks ROM path resolution.
    #[test]
    fn build_profile_args_skips_empty_config() {
        let dir = test_profiles_dir().join("empty_config_test");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        fs::create_dir_all(dir.join("saves")).unwrap();
        fs::write(dir.join("sm64config.txt"), "").unwrap();

        let config = ProfileConfig::default();
        let args = build_profile_args(&config, &dir, &dir);

        assert!(
            !args.contains(&"--configfile".into()),
            "Empty sm64config.txt must NOT produce --configfile"
        );
        assert!(args.contains(&"--savepath".into()));

        let _ = fs::remove_dir_all(&dir);
    }

    // ── Step 26: CRUD tests ──

    fn crud_dir(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("sm64launcher_crud_{label}"));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        ensure_default_profile(&dir).unwrap();
        dir
    }

    #[test]
    fn create_profile_rejects_empty_name() {
        let dir = crud_dir("empty");
        let result = create_profile(&dir, "", &PathBuf::from("/nonexistent"));
        assert!(result.is_err());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn create_profile_rejects_duplicate_case_insensitive() {
        let dir = crud_dir("dup");
        create_profile(&dir, "MyProfile", &PathBuf::from("/nonexistent")).unwrap();
        let result = create_profile(&dir, "myprofile", &PathBuf::from("/nonexistent"));
        assert!(result.is_err());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn create_profile_copies_parent_config() {
        let dir = crud_dir("copycfg");
        let parent_cfg = dir.join("fake_sm64config.txt");
        fs::write(&parent_cfg, "enable-mod: test\n").unwrap();

        create_profile(&dir, "CopiedProfile", &parent_cfg).unwrap();
        let copied = dir.join("CopiedProfile").join("sm64config.txt");
        assert!(copied.exists());
        let content = fs::read_to_string(&copied).unwrap();
        assert_eq!(content, "enable-mod: test\n");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn rename_profile_updates_active_txt_if_was_active() {
        let dir = crud_dir("renactive");
        create_profile(&dir, "OldName", &PathBuf::from("/nonexistent")).unwrap();
        set_active_profile(&dir, "OldName").unwrap();
        assert_eq!(get_active_profile_name(&dir).as_deref(), Some("OldName"));
        rename_profile(&dir, "OldName", "NewName").unwrap();
        assert_eq!(get_active_profile_name(&dir).as_deref(), Some("NewName"));
        assert!(dir.join("NewName").is_dir());
        assert!(!dir.join("OldName").is_dir());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn rename_profile_rejects_duplicate() {
        let dir = crud_dir("rendup");
        create_profile(&dir, "Alpha", &PathBuf::from("/nonexistent")).unwrap();
        create_profile(&dir, "Beta", &PathBuf::from("/nonexistent")).unwrap();
        let result = rename_profile(&dir, "Alpha", "Beta");
        assert!(result.is_err());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn delete_profile_blocks_default() {
        let dir = crud_dir("deldef");
        let result = delete_profile(&dir, "Default");
        assert!(result.is_err());
        assert!(dir.join("Default").is_dir());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn delete_profile_blocks_active() {
        let dir = crud_dir("delactive");
        create_profile(&dir, "ActiveOne", &PathBuf::from("/nonexistent")).unwrap();
        set_active_profile(&dir, "ActiveOne").unwrap();
        let result = delete_profile(&dir, "ActiveOne");
        assert!(result.is_err());
        set_active_profile(&dir, "Default").unwrap();
        assert!(delete_profile(&dir, "ActiveOne").is_ok());
        assert!(!dir.join("ActiveOne").is_dir());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn delete_profile_removes_directory_and_contents() {
        let dir = crud_dir("delrem");
        create_profile(&dir, "ToDelete", &PathBuf::from("/nonexistent")).unwrap();
        let save_file = dir.join("ToDelete/saves/fake.sav");
        fs::create_dir_all(save_file.parent().unwrap()).unwrap();
        fs::write(&save_file, "save data").unwrap();
        assert!(dir.join("ToDelete").is_dir());
        let result = delete_profile(&dir, "ToDelete");
        assert!(result.is_ok());
        assert!(!dir.join("ToDelete").exists());
        let _ = fs::remove_dir_all(&dir);
    }

    /// End‑to‑end verification: full CRUD sequence matching the spec's
    /// 6‑step acceptance checklist for step 26.
    #[test]
    fn e2e_crud_sequence() {
        let dir = crud_dir("e2e");
        let parent_cfg = dir.join("fake_config.txt");
        fs::write(&parent_cfg, "enable-mod: test\ncoop_port: 7777\n").unwrap();

        // Step 1: Create
        create_profile(&dir, "TestProfile", &parent_cfg).unwrap();
        assert!(dir.join("TestProfile").is_dir());
        let copied = dir.join("TestProfile/sm64config.txt");
        assert!(copied.exists());
        assert_eq!(
            fs::read_to_string(&copied).unwrap(),
            "enable-mod: test\ncoop_port: 7777\n"
        );

        // Step 2: Rename while NOT active (Default is active)
        assert_eq!(get_active_profile_name(&dir).as_deref(), Some("Default"));
        rename_profile(&dir, "TestProfile", "TestProfile2").unwrap();
        assert!(!dir.join("TestProfile").is_dir());
        assert!(dir.join("TestProfile2/saves").is_dir());
        assert!(dir.join("TestProfile2/sm64config.txt").exists());
        assert_eq!(get_active_profile_name(&dir).as_deref(), Some("Default"));

        // Step 3: Activate + rename while ACTIVE (most fragile case)
        set_active_profile(&dir, "TestProfile2").unwrap();
        assert_eq!(
            get_active_profile_name(&dir).as_deref(),
            Some("TestProfile2")
        );
        rename_profile(&dir, "TestProfile2", "TestProfile3").unwrap();
        assert!(!dir.join("TestProfile2").is_dir());
        assert!(dir.join("TestProfile3").is_dir());
        // ← THE CRITICAL CHECK: active.txt must point to new name
        assert_eq!(
            get_active_profile_name(&dir).as_deref(),
            Some("TestProfile3"),
            "active.txt must be updated to new name after renaming active profile"
        );

        // Step 4: Block delete of Default
        let r = delete_profile(&dir, "Default");
        assert!(r.is_err());
        assert!(dir.join("Default").is_dir());

        // Step 5: Block delete of active profile
        let r = delete_profile(&dir, "TestProfile3");
        assert!(r.is_err());
        assert!(dir.join("TestProfile3").is_dir());

        // Step 6: Deactivate → delete
        set_active_profile(&dir, "Default").unwrap();
        assert!(delete_profile(&dir, "TestProfile3").is_ok());
        assert!(!dir.join("TestProfile3").exists());
        assert_eq!(get_active_profile_name(&dir).as_deref(), Some("Default"));

        let _ = fs::remove_dir_all(&dir);
    }

    // ── Step 27 tests ──

    #[test]
    fn toggle_flip_persists_immediately() {
        let dir = crud_dir("toggle_persist");
        let config_before = load_profile_config(&dir.join("Default")).unwrap();
        assert!(!config_before.skip_intro);

        // Toggle skip_intro ON
        let config = toggle_profile_config(&dir, "Default", "skip_intro").unwrap();
        assert!(config.skip_intro);

        // Reload from disk — must be persisted
        let config_after = load_profile_config(&dir.join("Default")).unwrap();
        assert!(config_after.skip_intro);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn discord_ui_inversion_in_items() {
        let dir = crud_dir("discord_ui");
        let profile_dir = dir.join("Default");

        // no_discord = false → Discord ON → UI toggle should show enabled=true
        {
            let config = load_profile_config(&profile_dir).unwrap();
            assert!(!config.no_discord);
            let items = build_profile_detail_items(&config);
            let discord_item = items
                .iter()
                .find(|i| i.name == "Discord Rich Presence")
                .unwrap();
            assert!(
                discord_item.enabled,
                "Discord ON (no_discord=false) → UI toggle must be enabled (✓)"
            );
        }

        // Flip: no_discord = true → Discord OFF → UI toggle should show enabled=false
        let config = toggle_profile_config(&dir, "Default", "no_discord").unwrap();
        assert!(config.no_discord);
        let items = build_profile_detail_items(&config);
        let discord_item = items
            .iter()
            .find(|i| i.name == "Discord Rich Presence")
            .unwrap();
        assert!(
            !discord_item.enabled,
            "Discord OFF (no_discord=true) → UI toggle must be disabled (—)"
        );

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn build_detail_items_has_7_fixed_fields() {
        let config = ProfileConfig {
            playername: "Test".into(),
            ..Default::default()
        };
        let items = build_profile_detail_items(&config);
        assert_eq!(items.len(), 7);
        assert_eq!(items[0].name, "Player Name");
        assert_eq!(items[0].item_type, ItemType::Text);
        assert_eq!(items[1].name, "Skip Intro");
        assert_eq!(items[2].name, "Discord Rich Presence");
        assert_eq!(items[6].name, "Headless (Server)");
    }

    // ── Step 28 tests ──

    #[test]
    fn load_active_profile_default_fallback() {
        let dir = crud_dir("load_active");
        // active.txt doesn't exist → should fall to Default
        let active_path = dir.join("active.txt");
        if active_path.exists() {
            std::fs::remove_file(&active_path).unwrap();
        }
        let result = load_active_profile(&dir).unwrap();
        assert_eq!(result.0.playername, "Default");
        assert!(result.1.ends_with("Default"));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn load_active_profile_missing_dir_falls_to_default() {
        let dir = crud_dir("missing_dir");
        // Point active.txt to a non-existent profile
        set_active_profile(&dir, "GhostProfile").unwrap();
        // "GhostProfile" dir doesn't exist → should fall to Default
        let result = load_active_profile(&dir).unwrap();
        assert_eq!(result.0.playername, "Default");

        let _ = fs::remove_dir_all(&dir);
    }
}
