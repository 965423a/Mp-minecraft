//! 服务器配置:server.properties 解析(对齐原版属性名)。

use std::collections::HashMap;
use std::fs;
use std::path::Path;

pub struct ServerConfig {
    pub seed: u64,
    pub level_name: String,
    pub world_type: String,
    pub port: u16,
    pub motd: String,
    pub max_players: i32,
    #[allow(dead_code)]
    pub spawn_radius: i32,
    pub view_distance: i32,
    pub online_mode: bool,
}

impl Default for ServerConfig {
    fn default() -> Self {
        ServerConfig {
            seed: 0, // 0 = 随机
            level_name: "world".to_string(),
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
    /// 默认配置内容(首次运行时写入,与原版 jar 生成的属性键完全一致)。
    pub const fn default_properties() -> &'static str {
        "#Minecraft server properties\n\
         enable-jmx-monitoring=false\n\
         rcon.port=25575\n\
         level-seed=\n\
         gamemode=survival\n\
         enable-command-block=false\n\
         enable-query=false\n\
         generator-settings={}\n\
         enforce-secure-profile=false\n\
         level-name=world\n\
         motd=A Mp-minecraft Server\n\
         query.port=25565\n\
         pvp=true\n\
         generate-structures=true\n\
         max-chained-neighbor-updates=1000000\n\
         difficulty=easy\n\
         network-compression-threshold=256\n\
         max-tick-time=60000\n\
         require-resource-pack=false\n\
         use-native-transport=true\n\
         max-players=20\n\
         online-mode=false\n\
         enable-status=true\n\
         allow-flight=false\n\
         initial-disabled-packs=\n\
         broadcast-rcon-to-ops=true\n\
         view-distance=8\n\
         server-ip=\n\
         resource-pack-prompt=\n\
         allow-nether=true\n\
         server-port=25565\n\
         enable-rcon=false\n\
         sync-chunk-writes=true\n\
         op-permission-level=4\n\
         prevent-proxy-connections=false\n\
         hide-online-players=false\n\
         resource-pack=\n\
         entity-broadcast-range-percentage=100\n\
         simulation-distance=8\n\
         rcon.password=\n\
         player-idle-timeout=0\n\
         force-gamemode=false\n\
         rate-limit=0\n\
         hardcore=false\n\
         white-list=false\n\
         broadcast-console-to-ops=true\n\
         spawn-npcs=true\n\
         spawn-animals=true\n\
         log-ips=true\n\
         function-permission-level=2\n\
         level-type=minecraft\\:normal\n\
         text-filtering-config=\n\
         spawn-monsters=true\n\
         enforce-whitelist=false\n\
         spawn-protection=0\n\
         max-world-size=29999984\n"
    }

    /// 首次运行建立与原版 jar 布局一致的文件(eula、白名单、缓存等)。
    pub fn init_vanilla_files(root: &Path) {
        let _ = fs::create_dir_all(root);
        let eula = root.join("eula.txt");
        if !eula.exists() {
            let _ = fs::write(
                &eula,
                "#By changing the setting below to TRUE you are indicating your agreement to our EULA\n\
                 eula=true\n",
            );
        }
        for f in [
            "ops.json",
            "whitelist.json",
            "banned-players.json",
            "banned-ips.json",
            "usercache.json",
            "usernamecache.json",
        ] {
            let p = root.join(f);
            if !p.exists() {
                let _ = fs::write(&p, "[]\n");
            }
        }
    }

    pub fn load_or_create(root: &Path) -> Self {
        let path = root.join("server.properties");
        if !path.exists() {
            let _ = fs::create_dir_all(root);
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
                    "level-seed" | "seed" => {
                        cfg.seed = v.parse::<u64>().unwrap_or(0);
                    }
                    "level-name" => cfg.level_name = v.to_string(),
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
            "superflat" | "minecraft\\:superflat" => mc_world::generator::WorldType::Superflat,
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
        assert_eq!(cfg.world_type, "minecraft\\:normal");
        assert_ne!(cfg.seed, 0); // 随机种子
        let _ = fs::remove_dir_all(&dir);
    }
}