use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::Path;

const PREFIXES: &[&str] = &[
    "coop_player_name",
    "coop_join_ip",
    "coop_join_port",
    "coop_host_port",
    "amount_of_players",
    "coop_network_system",
    "coopnet_password",
];

/// Network connection mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetworkMode {
    Local,
    Client,
    Server,
    CoopNet,
}

impl NetworkMode {
    pub fn from_i32(v: i32) -> Self {
        match v {
            1 => NetworkMode::CoopNet,
            _ => NetworkMode::Local,
        }
    }

    pub fn to_i32(self) -> i32 {
        match self {
            NetworkMode::Local => 0,
            NetworkMode::CoopNet => 1,
            // Client and Server are not represented by coop_network_system
            // directly; they determine other args. For now return 0.
            NetworkMode::Client => 0,
            NetworkMode::Server => 0,
        }
    }

    pub fn next(self) -> Self {
        match self {
            NetworkMode::Local => NetworkMode::Client,
            NetworkMode::Client => NetworkMode::Server,
            NetworkMode::Server => NetworkMode::CoopNet,
            NetworkMode::CoopNet => NetworkMode::Local,
        }
    }

    pub fn prev(self) -> Self {
        match self {
            NetworkMode::Local => NetworkMode::CoopNet,
            NetworkMode::Client => NetworkMode::Local,
            NetworkMode::Server => NetworkMode::Client,
            NetworkMode::CoopNet => NetworkMode::Server,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            NetworkMode::Local => "Local",
            NetworkMode::Client => "Client",
            NetworkMode::Server => "Server",
            NetworkMode::CoopNet => "CoopNet",
        }
    }
}

/// Network configuration read from sm64config.txt.
#[derive(Debug, Clone)]
pub struct NetworkConfig {
    pub player_name: String,
    pub mode: NetworkMode,
    pub join_ip: String,
    pub join_port: u16,
    pub host_port: u16,
    pub max_players: u16,
    pub coopnet_password: String,
}

impl Default for NetworkConfig {
    fn default() -> Self {
        Self {
            player_name: String::new(),
            mode: NetworkMode::Local,
            join_ip: String::new(),
            join_port: 7777,
            host_port: 7777,
            max_players: 16,
            coopnet_password: String::new(),
        }
    }
}

/// Parse network keys from sm64config.txt into a NetworkConfig.
pub fn parse_network_config(config_path: &Path) -> Result<NetworkConfig, String> {
    let content = match fs::read_to_string(config_path) {
        Ok(c) => c,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(NetworkConfig::default()),
        Err(e) => return Err(format!("Failed to read config: {e}")),
    };

    let mut map: HashMap<String, String> = HashMap::new();
    for line in content.lines() {
        let trimmed = line.trim();
        for &pfx in PREFIXES {
            if let Some(val) = trimmed.strip_prefix(&format!("{pfx} ")) {
                map.insert(pfx.to_string(), val.to_string());
            }
            if let Some(val) = trimmed.strip_prefix(pfx) {
                let val = val.trim().to_string();
                if !val.is_empty() {
                    map.insert(pfx.to_string(), val);
                }
            }
        }
    }

    Ok(NetworkConfig {
        player_name: map.get("coop_player_name").cloned().unwrap_or_default(),
        mode: map
            .get("coop_network_system")
            .and_then(|s| s.parse().ok())
            .map(NetworkMode::from_i32)
            .unwrap_or(NetworkMode::Local),
        join_ip: map.get("coop_join_ip").cloned().unwrap_or_default(),
        join_port: map
            .get("coop_join_port")
            .and_then(|s| s.parse().ok())
            .unwrap_or(7777),
        host_port: map
            .get("coop_host_port")
            .and_then(|s| s.parse().ok())
            .unwrap_or(7777),
        max_players: map
            .get("amount_of_players")
            .and_then(|s| s.parse().ok())
            .unwrap_or(16),
        coopnet_password: map.get("coopnet_password").cloned().unwrap_or_default(),
    })
}

/// Write network config to sm64config.txt atomically, preserving non‑network
/// lines (enable-mod:, dynos-pack:, etc.).
pub fn write_network_config(config_path: &Path, config: &NetworkConfig) -> Result<(), String> {
    let mut non_net_lines: Vec<String> = Vec::new();
    if config_path.exists() {
        let content =
            fs::read_to_string(config_path).map_err(|e| format!("Failed to read config: {e}"))?;
        for line in content.lines() {
            let trimmed = line.trim();
            let is_net = PREFIXES.iter().any(|p| trimmed.starts_with(p));
            if !is_net {
                non_net_lines.push(line.to_string());
            }
        }
    }

    if let Some(parent) = config_path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("Failed to create config dir: {e}"))?;
    }

    let mut out = String::new();
    for line in &non_net_lines {
        out.push_str(line);
        out.push('\n');
    }
    // Write network lines
    let mode_num = config.mode.to_i32();
    out.push_str(&format!("coop_player_name {}\n", config.player_name));
    out.push_str(&format!("coop_join_ip {}\n", config.join_ip));
    out.push_str(&format!("coop_join_port {}\n", config.join_port));
    out.push_str(&format!("coop_host_port {}\n", config.host_port));
    out.push_str(&format!("amount_of_players {}\n", config.max_players));
    out.push_str(&format!("coop_network_system {mode_num}\n"));
    out.push_str(&format!("coopnet_password {}\n", config.coopnet_password));

    let tmp = config_path.with_extension("tmp");
    fs::write(&tmp, &out).map_err(|e| format!("Failed to write tmp config: {e}"))?;
    fs::rename(&tmp, config_path).map_err(|e| format!("Failed to rename config: {e}"))?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn tmp_dir(prefix: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("sm64launcher_test_{prefix}"));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn parse_returns_defaults_for_missing_file() {
        let cfg = parse_network_config(Path::new("/nonexistent/config.txt")).unwrap();
        assert_eq!(cfg.join_port, 7777);
        assert_eq!(cfg.max_players, 16);
    }

    #[test]
    fn parse_reads_all_keys() {
        let dir = tmp_dir("net_parse");
        let config = dir.join("sm64config.txt");
        fs::write(
            &config,
            "coop_player_name Alice\n\
             enable-mod: some_mod\n\
             coop_join_ip 192.168.1.1\n\
             dynos-pack: pack_a\n\
             coop_host_port 8888\n\
             amount_of_players 8\n\
             coop_network_system 1\n\
             coopnet_password secret\n",
        )
        .unwrap();
        let cfg = parse_network_config(&config).unwrap();
        assert_eq!(cfg.player_name, "Alice");
        assert_eq!(cfg.join_ip, "192.168.1.1");
        assert_eq!(cfg.host_port, 8888);
        assert_eq!(cfg.max_players, 8);
        assert!(matches!(cfg.mode, NetworkMode::CoopNet));
        assert_eq!(cfg.coopnet_password, "secret");
    }

    #[test]
    fn write_preserves_other_lines() {
        let dir = tmp_dir("net_write");
        let config = dir.join("sm64config.txt");
        fs::write(
            &config,
            "enable-mod: my_mod\n\
             dynos-pack: my_pack\n\
             coop_player_name Old\n",
        )
        .unwrap();

        let mut cfg = NetworkConfig::default();
        cfg.player_name = "New".to_string();
        write_network_config(&config, &cfg).unwrap();

        let content = fs::read_to_string(&config).unwrap();
        assert!(content.contains("enable-mod: my_mod"));
        assert!(content.contains("dynos-pack: my_pack"));
        assert!(!content.contains("Old"));
        assert!(content.contains("coop_player_name New"));
    }

    #[test]
    fn mode_cycle_wraps() {
        let mut m = NetworkMode::Local;
        m = m.next(); // Client
        m = m.next(); // Server
        m = m.next(); // CoopNet
        m = m.next(); // Local
        assert_eq!(m, NetworkMode::Local);

        m = m.prev(); // CoopNet
        assert_eq!(m, NetworkMode::CoopNet);
        m = m.prev(); // Server
        assert_eq!(m, NetworkMode::Server);
    }
}
