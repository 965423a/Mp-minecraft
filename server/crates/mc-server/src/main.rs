//! mc-server:与原版 jar 布局一致的服务器核心。
//!
//! 运行目录结构(与原版 jar 启动后生成的一致,无 server.jar):
//!   server.properties  eula.txt  ops.json  whitelist.json ...
//!   logs/              latest.log 与历史轮转 .log.gz
//!   world/             level.dat、region/、entities/、poi/、session.lock
//!   crash-reports/

mod config;
mod logger;
mod network;
mod protocol;
mod registry;
mod world;

use config::ServerConfig;
use logger::Logger;
use std::env;
use std::net::TcpStream;
use std::path::PathBuf;
use std::process::ExitCode;

/// Status JSON 里的版本名(与协议常量一致)。
pub const VERSION_NAME: &str = mc_protocol::consts::VERSION_NAME;

/// 离线模式 UUID:用户名转 UUID v3(Mojang 规则 "OfflinePlayer:" + name)。
pub fn offline_uuid(name: &str) -> [u8; 16] {
    use std::hash::Hasher;
    let mut h = std::collections::hash_map::DefaultHasher::new();
    h.write(("OfflinePlayer:".to_owned() + name).as_bytes());
    let v = h.finish() as u64;
    let mut u = [0u8; 16];
    u[..8].copy_from_slice(&v.to_be_bytes());
    u[8..].copy_from_slice(&v.to_be_bytes());
    u[6] = (u[6] & 0x0f) | 0x30; // v3
    u[8] = (u[8] & 0x3f) | 0x80;
    u
}

fn main() -> ExitCode {
    let log = Logger::new();

    // ---- 运行目录:默认取环境变量 MCS_HOME 或当前目录 ----
    let root: PathBuf = env::var("MCS_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));

    ServerConfig::init_vanilla_files(&root);
    let _ = std::fs::create_dir_all(root.join("crash-reports"));

    if log.open(&root.join("logs")).is_err() {
        eprintln!("[ERROR] cannot open log dir {}", root.join("logs").display());
    }
    log.info("Mp-minecraft server starting");
    log.info(&format!("root dir: {}", root.display()));

    // ---- 配置 ----
    let mut cfg = ServerConfig::load_or_create(&root);
    if let Ok(p) = env::var("MCS_PORT") {
        if let Ok(port) = p.parse::<u16>() {
            cfg.port = port;
        }
    }
    log.info(&format!(
        "config: seed={} type={} port={} motd=\"{}\"",
        cfg.seed, cfg.world_type, cfg.port, cfg.motd
    ));

    // ---- 世界:预生成出生点周围区块 ----
    let wtype = cfg.world_type();
    let world_dir = root.join(&cfg.level_name);
    let result = world::generate_spawn(&world_dir, cfg.seed, wtype, cfg.view_distance);
    match result {
        Ok(stats) => {
            log.info(&format!(
                "world generated: {} chunks, {} blocks, {} surface mean, saved {} files",
                stats.chunks, stats.blocks, stats.mean_surface, stats.files
            ));
        }
        Err(e) => {
            log.error(&format!("world generation failed: {e}"));
            return ExitCode::FAILURE;
        }
    }

    // ---- 网络监听:真实 TCP,处理 Handshake/Status/Login ----
    let port = cfg.port;
    let motd = cfg.motd.clone();
    let max_players = cfg.max_players;
    let view_distance = cfg.view_distance;
    let seed = cfg.seed;
    let flat = cfg.world_type() == mc_world::generator::WorldType::Superflat;
    log.info(&format!(
        "server listening on 0.0.0.0:{} (protocol 775, MC 26.1.2)",
        port
    ));
    let registry = std::sync::Arc::new(registry::load_registry());
    log.info(&format!(
        "registry loaded: {} entries",
        registry.entries.len()
    ));
    match network::NetworkServer::bind(port, move |stream: TcpStream| {
        let _ = protocol::handle_connection(
            stream,
            port,
            &motd,
            max_players,
            view_distance,
            (0.5, 65.0, 0.5),
            seed,
            flat,
            &world_dir,
            wtype,
            &registry,
        );
    }) {
        Ok(net) => {
            log.info("Done. This is a Mp-minecraft server.");
            log.info("Type 'stop' to shut down (console input not yet wired).");
            if let Err(e) = net.run() {
                log.error(&format!("network error: {e}"));
                return ExitCode::FAILURE;
            }
        }
        Err(e) => {
            log.error(&format!("cannot bind port {port}: {e}"));
            return ExitCode::FAILURE;
        }
    }

    ExitCode::SUCCESS
}