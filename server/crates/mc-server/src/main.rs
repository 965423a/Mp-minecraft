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

    let root = env::var("MCS_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));

    ServerConfig::init_vanilla_files(&root);
    let _ = std::fs::create_dir_all(root.join("crash-reports"));

    let log_dir = root.join("logs");
    if log.open(&log_dir).is_err() {
        eprintln!("[ERROR] cannot open log dir {}", log_dir.display());
    }
    log.info("Mp-minecraft server starting");
    log.info(&format!("root dir: {}", root.display()));

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

    let wtype = cfg.world_type();
    let world_dir = root.join(&cfg.level_name);
    match world::generate_spawn(&world_dir, cfg.seed, wtype, cfg.view_distance) {
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

    let port = cfg.port;
    let motd = cfg.motd.clone();
    let max_players = cfg.max_players;
    let view_distance = cfg.view_distance;
    let seed = cfg.seed;
    let flat = wtype == mc_world::generator::WorldType::Superflat;
    log.info(&format!(
        "server listening on 0.0.0.0:{} (protocol 775, MC 26.1.2)",
        port
    ));
    let registry = std::sync::Arc::new(registry::load_registry());
    log.info(&format!(
        "registry loaded: {} entries",
        registry.entries.len()
    ));
    let world = std::sync::Arc::new(std::sync::Mutex::new(world::World::open(&world_dir, seed, wtype)));
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
            world.clone(),
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