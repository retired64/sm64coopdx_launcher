use std::collections::HashMap;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};

use crate::config::{GAME_STDERR_LOG, ROM_FILENAME, ROM_MD5};

/// Raised by the monitor thread when the game process exits.
/// The main loop polls this flag each frame.
pub static GAME_EXITED: AtomicBool = AtomicBool::new(false);

/// Resolve the game binary path using 4‑tier + optional profile override priority:
///
/// - CLI argument `--game-path`
/// - Environment variable `SM64COOPDX_PATH`
/// - Per‑profile `game_path` from profile.json (only if `profile_override` is Some)
/// - `launcher.toml` → `[game].path`
/// - Default: `<launcher_dir>/../games/mario64/sm64coopdx`
///
/// Each level is validated — if the path doesn't point to an existing file,
/// we fall through to the next level.
///
/// Every returned path is canonicalized (absolute, symlinks resolved) so
/// that `spawn_game`'s `current_dir(binary.parent())` cannot interfere with
/// relative‑path resolution in `Command::new()`.
pub fn resolve_game_path(cli_arg: Option<&str>, profile_override: Option<&str>) -> PathBuf {
    resolve_game_path_inner(cli_arg, profile_override, None)
}

fn resolve_game_path_inner(
    cli_arg: Option<&str>,
    profile_override: Option<&str>,
    config_base: Option<&Path>,
) -> PathBuf {
    // 1. CLI arg
    if let Some(p) = cli_arg {
        let path = PathBuf::from(p);
        if path.is_file()
            && let Ok(abs) = canonicalize(&path)
        {
            log::info!("Using game path from CLI: {}", abs.display());
            return abs;
        }
        log::warn!("CLI --game-path points to non-existent file: {p}");
    }

    // 2. Env var
    if let Ok(p) = std::env::var("SM64COOPDX_PATH") {
        let path = PathBuf::from(&p);
        if path.is_file()
            && let Ok(abs) = canonicalize(&path)
        {
            log::info!("Using game path from SM64COOPDX_PATH: {}", abs.display());
            return abs;
        }
        log::warn!("SM64COOPDX_PATH points to non-existent file: {p}");
    }

    // 2.5. Per‑profile binary path override (§9B)
    if let Some(pp) = profile_override {
        let path = PathBuf::from(pp);
        if path.is_file()
            && let Ok(abs) = canonicalize(&path)
        {
            log::info!("Using game path from profile override: {}", abs.display());
            return abs;
        }
        log::warn!("Profile override points to non-existent file: {pp}");
    }

    // 3. launcher.toml
    {
        let toml_path = if let Some(base) = config_base {
            read_config_game_path_at(base)
        } else {
            read_config_game_path()
        };
        if let Some(path) = toml_path {
            if path.is_file()
                && let Ok(abs) = canonicalize(&path)
            {
                log::info!("Using game path from launcher.toml: {}", abs.display());
                return abs;
            }
            log::warn!(
                "launcher.toml [game].path points to non-existent file: {}",
                path.display()
            );
        }
    }

    // 4. Default relative path
    let default = default_game_path();
    if let Ok(abs) = canonicalize(&default) {
        log::info!("Using default game path: {}", abs.display());
        abs
    } else {
        log::warn!("Default game path not found: {}", default.display());
        default
    }
}

/// Canonicalize a path to absolute form, or fall back to joining with CWD.
fn canonicalize(path: &Path) -> std::io::Result<PathBuf> {
    match path.canonicalize() {
        Ok(abs) => Ok(abs),
        Err(_) => {
            // Canonicalize may fail on some filesystems; fall back to
            // manually resolving relative to the current working directory.
            let cwd = std::env::current_dir()?;
            Ok(cwd.join(path))
        }
    }
}

/// Read `[game].path` from `launcher.toml` in the XDG config dir.
fn read_config_game_path() -> Option<PathBuf> {
    let config_dir = dirs::config_dir()?;
    read_config_game_path_at(&config_dir)
}

fn read_config_game_path_at(base: &Path) -> Option<PathBuf> {
    let config_file = base.join("sm64coopdx").join("launcher.toml");
    let content = std::fs::read_to_string(&config_file).ok()?;
    let parsed: toml::Table = toml::from_str(&content).ok()?;
    let game = parsed.get("game")?.as_table()?;
    let path_str = game.get("path")?.as_str()?;
    Some(PathBuf::from(path_str))
}

/// Read `[game].rom_path` from `launcher.toml` in the XDG config dir.
fn read_config_rom_path() -> Option<PathBuf> {
    let config_dir = dirs::config_dir()?;
    read_config_rom_path_at(&config_dir)
}

fn read_config_rom_path_at(base: &Path) -> Option<PathBuf> {
    let config_file = base.join("sm64coopdx").join("launcher.toml");
    let content = std::fs::read_to_string(&config_file).ok()?;
    let parsed: toml::Table = toml::from_str(&content).ok()?;
    let game = parsed.get("game")?.as_table()?;
    let path_str = game.get("rom_path")?.as_str()?;
    Some(PathBuf::from(path_str))
}

/// Compute MD5 hash of a file. Returns lowercase hex string.
fn compute_md5(path: &Path) -> Option<String> {
    let mut file = fs::File::open(path).ok()?;
    let mut ctx = md5::Context::new();
    let mut buf = [0u8; 8192];
    loop {
        let n = file.read(&mut buf).ok()?;
        if n == 0 {
            break;
        }
        ctx.consume(&buf[..n]);
    }
    Some(format!("{:x}", ctx.compute()))
}

/// Verify a ROM file matches the expected MD5.
fn is_rom_valid(path: &Path) -> bool {
    compute_md5(path).as_deref() == Some(ROM_MD5)
}

/// Scan a directory for a valid SM64 US ROM (any *.z64 file with correct MD5).
/// Returns the path to the first valid ROM found.
fn scan_dir_for_rom(dir: &Path) -> Option<PathBuf> {
    let entries = fs::read_dir(dir).ok()?;
    for entry in entries.filter_map(|e| e.ok()) {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) == Some("z64")
            && is_rom_valid(&path)
        {
            return Some(path);
        }
    }
    None
}

/// Try to find a valid SM64 US ROM by searching common locations.
/// Returns the path to the first valid ROM found, or None.
pub fn find_rom(game_dir: &Path, data_dir: &Path) -> Option<PathBuf> {
    // 1. Scan game binary directory
    if let Some(rom) = scan_dir_for_rom(game_dir) {
        log::info!("ROM found in game directory: {}", rom.display());
        return Some(rom);
    }

    // 2. Scan launcher data directory
    if let Some(rom) = scan_dir_for_rom(data_dir) {
        log::info!("ROM found in launcher data dir: {}", rom.display());
        return Some(rom);
    }

    // 3. Check launcher.toml [game].rom_path
    if let Some(rom_path) = read_config_rom_path() {
        if rom_path.is_file() && is_rom_valid(&rom_path) {
            log::info!("ROM found via launcher.toml: {}", rom_path.display());
            return Some(rom_path);
        }
        log::warn!(
            "launcher.toml rom_path invalid or not found: {}",
            rom_path.display()
        );
    }

    // 4. Search ~/sm64coopdx_Linux-* directories
    if let Some(home) = dirs::home_dir()
        && let Ok(entries) = fs::read_dir(&home)
    {
        for entry in entries.filter_map(|e| e.ok()) {
            let name = entry.file_name();
            let name_str = name.to_string_lossy();
            if name_str.starts_with("sm64coopdx_Linux-") && entry.path().is_dir()
                && let Some(rom) = scan_dir_for_rom(&entry.path())
            {
                log::info!(
                    "ROM found in {}: {}",
                    entry.path().display(),
                    rom.display()
                );
                return Some(rom);
            }
        }
    }

    log::error!("No valid SM64 US ROM found. Expected MD5: {}", ROM_MD5);
    None
}

/// Ensure a valid ROM (`baserom.us.z64`) exists in the game's savepath.
///
/// Searches for a ROM using `find_rom()`, then copies it to
/// `{savepath}/{ROM_FILENAME}`. Returns the path to the copied ROM.
pub fn ensure_rom(savepath: &Path, game_dir: &Path, data_dir: &Path) -> Result<PathBuf, String> {
    let dest = savepath.join(ROM_FILENAME);

    // Already present? Verify it's valid.
    if dest.is_file() && is_rom_valid(&dest) {
        log::info!("ROM already present and valid: {}", dest.display());
        return Ok(dest);
    }

    // Find the ROM
    let source = find_rom(game_dir, data_dir).ok_or_else(|| {
        format!(
            "No valid Super Mario 64 US ROM found.\n\
             Expected MD5: {ROM_MD5}\n\
             Place 'baserom.us.z64' in one of these locations:\n\
             - {}\n\
             - {}\n\
             Or set [game].rom_path in ~/.config/sm64coopdx/launcher.toml",
            game_dir.display(),
            data_dir.display(),
        )
    })?;

    // Copy to savepath
    fs::create_dir_all(savepath)
        .map_err(|e| format!("Cannot create savepath: {e}"))?;
    fs::copy(&source, &dest)
        .map_err(|e| format!("Cannot copy ROM to {}: {e}", dest.display()))?;

    log::info!("ROM copied: {} → {}", source.display(), dest.display());
    Ok(dest)
}

/// Validate that the game installation has all required files.
///
/// Checks:
/// - `{game_dir}/lang/English.ini` exists
/// - `{game_dir}/dynos/` directory exists
/// - Savepath is writable
/// - ROM exists at `{savepath}/{ROM_FILENAME}`
pub fn validate_game_installation(game_dir: &Path, savepath: &Path) -> Result<(), String> {
    // Check language files
    let lang_file = game_dir.join("lang").join("English.ini");
    if !lang_file.is_file() {
        return Err(format!(
            "Language file not found: {}\n\
             The game installation seems incomplete. Make sure the 'lang/' directory\n\
             is present next to the game binary.",
            lang_file.display()
        ));
    }

    // Check dynos directory
    let dynos_dir = game_dir.join("dynos");
    if !dynos_dir.is_dir() {
        log::warn!(
            "DynOS directory not found: {}. DynOS packs may not load.",
            dynos_dir.display()
        );
    }

    // Ensure savepath exists and is writable
    if !savepath.exists() {
        fs::create_dir_all(savepath)
            .map_err(|e| format!("Cannot create savepath {}: {e}", savepath.display()))?;
    }

    let test_file = savepath.join(".launcher_write_test");
    fs::write(&test_file, b"test")
        .map_err(|e| format!("Savepath {} is not writable: {e}", savepath.display()))?;
    let _ = fs::remove_file(&test_file);

    Ok(())
}

/// Default path: `<launcher_dir>/../games/mario64/sm64coopdx`, with fallbacks.
fn default_game_path() -> PathBuf {
    let exe = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("."));
    let parent = exe.parent().unwrap_or(Path::new("."));
    let grandparent = parent.parent().unwrap_or(Path::new("."));

    // 1. <launcher>/../games/mario64/sm64coopdx (existing default)
    let candidate = grandparent.join("games").join("mario64").join("sm64coopdx");
    if candidate.is_file() {
        return candidate;
    }

    // 2. ./sm64coopdx (same directory as launcher)
    let cwd_candidate = parent.join("sm64coopdx");
    if cwd_candidate.is_file() {
        return cwd_candidate;
    }

    // 3. ../sm64coopdx (parent directory)
    let parent_candidate = grandparent.join("sm64coopdx");
    if parent_candidate.is_file() {
        return parent_candidate;
    }

    // 4. Search ~/sm64coopdx_Linux-*/sm64coopdx
    if let Some(home) = dirs::home_dir()
        && let Ok(entries) = fs::read_dir(&home)
    {
        for entry in entries.filter_map(|e| e.ok()) {
            let name = entry.file_name();
            let name_str = name.to_string_lossy();
            if name_str.starts_with("sm64coopdx_Linux-") && entry.path().is_dir() {
                let bin = entry.path().join("sm64coopdx");
                if bin.is_file() {
                    log::info!(
                        "Found game binary in release directory: {}",
                        bin.display()
                    );
                    return bin;
                }
            }
        }
    }

    // Fallback: original default path (for informational purposes)
    candidate
}

/// Build a safe environment for the game process.
///
/// Uses `Command::env_clear()` + whitelist. Variables NOT in the whitelist
/// — especially LD_LIBRARY_PATH, LD_PRELOAD, PYTHONPATH — are stripped to
/// prevent segfaults from inheriting launcher/Flatpak/AppImage libraries.
pub fn build_game_env() -> HashMap<String, String> {
    let whitelist = [
        "DISPLAY",
        "WAYLAND_DISPLAY",
        "XAUTHORITY",
        "HOME",
        "XDG_RUNTIME_DIR",
        "XDG_DATA_HOME",
        "XDG_CONFIG_HOME",
        "XDG_CACHE_HOME",
        "XDG_DATA_DIRS",
        "PULSE_SERVER",
        "PULSE_COOKIE",
        "ALSA_CARD",
        "DBUS_SESSION_BUS_ADDRESS",
        "LANG",
        "LC_ALL",
        "LC_MESSAGES",
        "PATH",
        "USER",
        "SHELL",
        "TERM",
    ];

    let mut env = HashMap::new();
    for key in &whitelist {
        if let Ok(val) = std::env::var(key) {
            env.insert(key.to_string(), val);
        }
    }
    env
}

/// Spawn the game process with a clean environment.
///
/// On error, the game's stderr is captured to `{log_dir}/{GAME_STDERR_LOG}`
/// for post-mortem diagnosis.
pub fn spawn_game(binary: &Path, args: &[String], log_dir: &Path) -> Result<Child, String> {
    // Validate binary exists
    if !binary.is_file() {
        return Err(format!("Game binary not found: {}", binary.display()));
    }

    // Ensure executable permissions (Unix only)
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(meta) = std::fs::metadata(binary) {
            let mut perms = meta.permissions();
            if perms.mode() & 0o111 == 0 {
                perms.set_mode(0o755);
                std::fs::set_permissions(binary, perms)
                    .map_err(|e| format!("chmod failed: {e}"))?;
            }
        }
    }

    let cwd = binary.parent().unwrap_or(Path::new("."));
    let env = build_game_env();

    log::info!(
        "Spawning: {:?} with {} args, cwd={:?}",
        binary,
        args.len(),
        cwd
    );

    // Create stderr log file for post-mortem diagnosis
    let stderr_log = log_dir.join(GAME_STDERR_LOG);
    let stderr_file = fs::File::create(&stderr_log)
        .map_err(|e| format!("Cannot create stderr log {}: {e}", stderr_log.display()))?;

    let child = Command::new(binary)
        .args(args)
        .current_dir(cwd)
        .env_clear()
        .envs(&env)
        .stdout(Stdio::null())
        .stderr(Stdio::from(stderr_file))
        .spawn()
        .map_err(|e| format!("Failed to spawn game: {e}"))?;

    Ok(child)
}

/// Spawn a monitor thread that waits for the game process to exit and sets
/// the GAME_EXITED flag. Per AGENTS.md §'Modelo de concurrencia', this
/// thread MUST NOT mutate UI state directly — it only sets the atomic flag.
pub fn spawn_monitor(child: Child) {
    GAME_EXITED.store(false, Ordering::SeqCst);
    std::thread::spawn(move || {
        let mut c = child;
        match c.wait() {
            Ok(status) => {
                log::info!("Game process exited with status: {status}");
            }
            Err(e) => {
                log::error!("Failed to wait for game process: {e}");
            }
        }
        GAME_EXITED.store(true, Ordering::Release);
    });
}

/// Build CLI args for enabled mods (Mod Manager).
///
/// - If any mods are enabled → `--enable-mod <name>` for each
/// - If no mods are enabled → `--disable-mods`
pub fn build_mod_args(enabled_names: &[String]) -> Vec<String> {
    if enabled_names.is_empty() {
        vec!["--disable-mods".to_string()]
    } else {
        enabled_names
            .iter()
            .flat_map(|n| ["--enable-mod".to_string(), n.clone()])
            .collect()
    }
}

/// Build CLI args from NetworkConfig.
///
/// Per the game binary `--help`: --server, --client, --coopnet, --playername, --playercount.
pub fn build_network_args(config: &crate::managers::network_manager::NetworkConfig) -> Vec<String> {
    let mut args = Vec::new();

    match config.mode {
        crate::managers::network_manager::NetworkMode::Local => {}
        crate::managers::network_manager::NetworkMode::Server => {
            args.push("--server".into());
            args.push(config.host_port.to_string());
        }
        crate::managers::network_manager::NetworkMode::Client => {
            if !config.join_ip.is_empty() {
                args.push("--client".into());
                args.push(config.join_ip.clone());
                args.push(config.join_port.to_string());
            }
        }
        crate::managers::network_manager::NetworkMode::CoopNet => {
            if !config.coopnet_password.is_empty() {
                args.push("--coopnet".into());
                args.push(config.coopnet_password.clone());
            }
        }
    }

    if !config.player_name.is_empty() {
        args.push("--playername".into());
        args.push(config.player_name.clone());
    }

    if config.max_players > 0 {
        args.push("--playercount".into());
        args.push(config.max_players.to_string());
    }

    args
}

/// Build the complete game CLI args from all active sub‑screen states.
///
/// Concatenates mod args + network args + profile args. Order follows
/// the spec §9A: mods → network → profiles.
pub fn build_all_game_args(
    enabled_mods: &[String],
    network_config: &crate::managers::network_manager::NetworkConfig,
    profile_config: Option<&crate::managers::profile_manager::ProfileConfig>,
    profile_dir: Option<&Path>,
    data_dir: &Path,
) -> Vec<String> {
    let mut args = build_mod_args(enabled_mods);
    args.extend(build_network_args(network_config));
    if let (Some(config), Some(dir)) = (profile_config, profile_dir) {
        args.extend(crate::managers::profile_manager::build_profile_args(
            config, dir, data_dir,
        ));
    }
    args
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn resolve_cli_priority() {
        let dir = std::env::temp_dir().join("sm64launcher_test_resolve_cli");
        fs::create_dir_all(&dir).unwrap();
        let real = dir.join("real_game");
        fs::write(&real, "fake").unwrap();

        let path = resolve_game_path(Some(real.to_str().unwrap()), None);
        assert_eq!(path, real);
    }

    #[test]
    fn resolve_falls_through_when_cli_missing() {
        let path = resolve_game_path(Some("/nonexistent/foo/bar/game"), None);
        // Should fall through all layers to default (which may not exist either)
        assert!(!path.to_string_lossy().contains("nonexistent"));
    }

    #[test]
    fn resolve_falls_through_when_env_missing() {
        // CLI not set, env not set → fall to toml, then default
        let path = resolve_game_path(None, None);
        // Just verify it doesn't panic and returns some path
        let _ = path;
    }

    #[test]
    fn build_env_strips_ld_library_path() {
        // This test verifies that even if LD_LIBRARY_PATH is set in the
        // current process, build_game_env does NOT include it.
        let env = build_game_env();
        assert!(!env.contains_key("LD_LIBRARY_PATH"));
        assert!(!env.contains_key("LD_PRELOAD"));
        assert!(!env.contains_key("PYTHONPATH"));
    }

    #[test]
    fn build_env_includes_display_if_set() {
        let env = build_game_env();
        // DISPLAY should be set on any X11/Wayland system
        if std::env::var("DISPLAY").is_ok() {
            assert!(env.contains_key("DISPLAY"));
        }
    }

    #[test]
    fn mod_args_enabled() {
        let names = vec!["mod_a".to_string(), "mod_b".to_string()];
        let args = build_mod_args(&names);
        assert_eq!(args, vec!["--enable-mod", "mod_a", "--enable-mod", "mod_b"]);
    }

    #[test]
    fn mod_args_disable_when_empty() {
        let args = build_mod_args(&[]);
        assert_eq!(args, vec!["--disable-mods"]);
    }

    #[test]
    fn spawn_with_missing_binary() {
        let result = spawn_game(Path::new("/nonexistent/game"), &[], Path::new("/tmp"));
        assert!(result.is_err());
    }

    #[test]
    fn spawn_resolves_relative_path_correctly() {
        // Reproduces the bug where resolve_game_path returned a relative path
        // and spawn_game's current_dir(binary.parent()) caused double-prefix.
        let dir = std::env::temp_dir().join("sm64launcher_test_rel_canon");
        fs::create_dir_all(&dir).unwrap();
        let bin = dir.join("my_game");
        fs::write(&bin, "fake").unwrap();

        // Resolve via CLI with an absolute path so we control the input
        let abs = canonicalize(&bin).unwrap();
        let resolved = resolve_game_path(Some(abs.to_str().unwrap()), None);
        assert!(resolved.is_absolute(), "resolved path must be absolute");
        assert_eq!(resolved, abs);
    }

    #[test]
    fn resolve_toml_priority() {
        let dir = std::env::temp_dir().join("sm64launcher_test_toml_prio");
        fs::create_dir_all(&dir).unwrap();
        let game = dir.join("real_from_toml");
        fs::write(&game, "fake").unwrap();
        let cfg_dir = dir.join("config");
        let sm64_dir = cfg_dir.join("sm64coopdx");
        fs::create_dir_all(&sm64_dir).unwrap();
        let toml_file = sm64_dir.join("launcher.toml");
        fs::write(
            &toml_file,
            format!("[game]\npath = \"{}\"\n", game.to_str().unwrap()),
        )
        .unwrap();

        // Pass config_base directly — no env var hacks needed
        let resolved = resolve_game_path_inner(None, None, Some(&cfg_dir));
        assert_eq!(resolved, canonicalize(&game).unwrap());
    }

    #[test]
    fn resolve_toml_falls_through_when_file_missing() {
        let dir = std::env::temp_dir().join("sm64launcher_test_toml_fall");
        fs::create_dir_all(&dir).unwrap();
        let cfg_dir = dir.join("config");
        let sm64_dir = cfg_dir.join("sm64coopdx");
        fs::create_dir_all(&sm64_dir).unwrap();
        let toml_file = sm64_dir.join("launcher.toml");
        fs::write(&toml_file, "[game]\npath = \"/nonexistent/game\"\n").unwrap();

        // Should fall through to default since the toml path doesn't exist
        let resolved = resolve_game_path_inner(None, None, Some(&cfg_dir));
        assert!(!resolved.to_string_lossy().contains("nonexistent"));
    }

    // ── Step 25: profile override (tier 2.5) tests ──

    #[test]
    fn resolve_profile_override_beats_toml() {
        let dir = std::env::temp_dir().join("sm64launcher_test_profile_beats_toml");
        fs::create_dir_all(&dir).unwrap();
        let game_profile = dir.join("game_from_profile");
        fs::write(&game_profile, "fake").unwrap();
        let game_toml = dir.join("game_from_toml");
        fs::write(&game_toml, "fake").unwrap();

        let cfg_dir = dir.join("config");
        let sm64_dir = cfg_dir.join("sm64coopdx");
        fs::create_dir_all(&sm64_dir).unwrap();
        fs::write(
            sm64_dir.join("launcher.toml"),
            format!("[game]\npath = \"{}\"\n", game_toml.to_str().unwrap()),
        )
        .unwrap();

        let resolved =
            resolve_game_path_inner(None, Some(game_profile.to_str().unwrap()), Some(&cfg_dir));
        assert_eq!(resolved, canonicalize(&game_profile).unwrap());
    }

    #[test]
    fn resolve_profile_override_loses_to_cli_and_env() {
        let dir = std::env::temp_dir().join("sm64launcher_test_profile_loses_to_cli_env");
        fs::create_dir_all(&dir).unwrap();
        let game_cli = dir.join("game_from_cli");
        fs::write(&game_cli, "fake").unwrap();
        let game_env = dir.join("game_from_env");
        fs::write(&game_env, "fake").unwrap();
        let game_profile = dir.join("game_from_profile");
        fs::write(&game_profile, "fake").unwrap();

        // CLI wins over profile_override
        let resolved_cli = resolve_game_path(
            Some(game_cli.to_str().unwrap()),
            Some(game_profile.to_str().unwrap()),
        );
        assert_eq!(resolved_cli, canonicalize(&game_cli).unwrap());

        // Env wins over profile_override (when CLI is None)
        // SAFETY: we restore the env var immediately after the test
        unsafe {
            std::env::set_var("SM64COOPDX_PATH", game_env.to_str().unwrap());
        }
        let resolved_env = resolve_game_path(None, Some(game_profile.to_str().unwrap()));
        unsafe {
            std::env::remove_var("SM64COOPDX_PATH");
        }
        assert_eq!(resolved_env, canonicalize(&game_env).unwrap());
    }

    // ── Step 28 tests ──

    #[test]
    fn build_all_game_args_concatenates_all_sources() {
        use crate::managers::network_manager::{NetworkConfig, NetworkMode};
        use crate::managers::profile_manager::ProfileConfig;

        let mods = vec!["mod_a".to_string(), "mod_b".to_string()];
        let net = NetworkConfig {
            mode: NetworkMode::Server,
            host_port: 7777,
            player_name: "PlayerNet".into(),
            max_players: 8,
            ..Default::default()
        };

        let tmp = std::env::temp_dir().join("sm64launcher_step28_test");
        std::fs::create_dir_all(&tmp).unwrap();
        std::fs::create_dir_all(tmp.join("saves")).unwrap();
        let _ = std::fs::File::create(tmp.join("sm64config.txt"));

        let profile = ProfileConfig {
            playername: "PlayerProfile".into(),
            skip_intro: true,
            headless: true,
            ..Default::default()
        };

        let args = build_all_game_args(&mods, &net, Some(&profile), Some(&tmp), &tmp);

        // Mod args present
        assert!(args.contains(&"--enable-mod".into()));
        assert!(args.contains(&"mod_a".into()));
        // Network args present
        assert!(args.contains(&"--server".into()));
        assert!(args.contains(&"7777".into()));
        // Profile args present (playername from profile overrides network)
        assert!(args.contains(&"--playername".into()));
        // Both playername values could be present; profile wins (last)
        // Profile boolean flags
        assert!(args.contains(&"--skip-intro".into()));
        assert!(args.contains(&"--headless".into()));

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn build_all_game_args_no_profile_is_noop() {
        use crate::managers::network_manager::NetworkConfig;

        let mods: Vec<String> = vec![];
        let net = NetworkConfig::default();
        let args = build_all_game_args(&mods, &net, None, None, Path::new("."));

        // Only mod + network args, no profile
        assert!(args.contains(&"--disable-mods".into()));
        // No profile args should be present
        assert!(!args.contains(&"--savepath".into()));
    }
}
