//! 服务器配置:server.properties 解析(对齐原版属性名)。

use std::collections::HashMap;
use std::fs;
use std::path::Path;

pub struct ServerConfig {
    pub seed: u64,
    pub world_type: String,
    pub port: u16,
    pub motd: String,
    pub max_players: i32,
    pub spawn_radius: i32,
    pub view_distance: i32,
    pub online_mode: bool,
}

impl Default for ServerConfig {
    fn default() -> Self {
        ServerConfig {
            seed: 0, // 0 = 随机
            world_type: "normal".to_string(),
            port: 25565,
            motd: "A Mp-minecraft Server".to_string(),
            max_players: 20,
            spawn_radius: 2,
            view_distance: 8,
            online_mode: false,
        }
    }
}

impl ServerConfig {
    /// 默认配置内容(首次运行时写入)。
    pub const fn default_properties() -> &'static str {
        "# Mp-minecraft server.properties\n\
         seed=0\n\
         level-type=normal\n\
         server-port=25565\n\
         motd=A Mp-minecraft Server\n\
         max-players=20\n\
         spawn-protection=0\n\
         view-distance=8\n\
         online-mode=false\n"
    }

    pub fn load_or_create(config_dir: &Path) -> Self {
        let path = config_dir.join("server.properties");
        if !path.exists() {
            if let Some(p) = path.parent() {
                let _ = fs::create_dir_all(p);
            }
            let _ = fs::write(&path, Self::default_properties());
        }
        let mut cfg = ServerConfig::default();
        if let Ok(text) = fs::read_to_string(&path) {
            for line in text.lines() {
                let line = line.trim();
                if line.is_empty() || line.starts_with('#') || line.starts_with('!') {
                    continue;
                }
                let (k, v) = match line.split_once('=') {
                    Some(kv) => (kv.0.trim(), kv.1.trim()),
                    None => continue,
                };
                match k {
                    "seed" => {
                        cfg.seed = v.parse::<u64>().unwrap_or(0);
                    }
                    "level-type" => cfg.world_type = v.to_string(),
                    "server-port" => {
                        cfg.port = v.parse::<u16>().unwrap_or(25565);
                    }
                    "motd" => cfg.motd = v.to_string(),
                    "max-players" => {
                        cfg.max_players = v.parse::<i32>().unwrap_or(20);
                    }
                    "view-distance" => {
                        cfg.view_distance = v.parse::<i32>().unwrap_or(8).clamp(2, 32);
                    }
                    "online-mode" => cfg.online_mode = v == "true",
                    _ => {}
                }
            }
        }
        // seed=0 表示随机
        if cfg.seed == 0 {
            cfg.seed = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos() as u64)
                .unwrap_or(2026);
        }
        cfg
    }

    pub fn world_type(&self) -> mc_world::generator::WorldType {
        match self.world_type.as_str() {
            "superflat" => mc_world::generator::WorldType::Superflat,
            _ => mc_world::generator::WorldType::Normal,
        }
    }
}

#[allow(dead_code)]
pub fn _parse_bench() -> HashMap<String, String> {
    HashMap::new()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn parse_properties() {
        let dir = std::env::temp_dir().join("mcs-cfg-test");
        let _ = fs::create_dir_all(&dir);
        let mut f = fs::File::create(dir.join("server.properties")).unwrap();
        writeln!(f, "# comment").unwrap();
        writeln!(f, "seed=777").unwrap();
        writeln!(f, "level-type=superflat").unwrap();
        writeln!(f, "server-port=12345").unwrap();
        writeln!(f, "max-players=5").unwrap();
        writeln!(f, "online-mode=false").unwrap();
        drop(f);
        let cfg = ServerConfig::load_or_create(&dir);
        assert_eq!(cfg.seed, 777);
        assert_eq!(cfg.world_type, "superflat");
        assert_eq!(cfg.port, 12345);
        assert_eq!(cfg.max_players, 5);
        assert!(!cfg.online_mode);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn creates_default_file() {
        let dir = std::env::temp_dir().join("mcs-cfg-default");
        let _ = fs::remove_dir_all(&dir);
        let cfg = ServerConfig::load_or_create(&dir);
        assert!(dir.join("server.properties").exists());
        assert_eq!(cfg.world_type, "normal");
        assert_ne!(cfg.seed, 0); // 随机种子
        let _ = fs::remove_dir_all(&dir);
    }
}