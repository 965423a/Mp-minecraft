//! mc-server:标准 MC 服务器目录模式的服务器核心。
//!
//! 目录布局(与原版/Paper 一致):
//!   server/            ← 运行目录
//!   ├── bin/           服务器程序与库
//!   ├── config/        server.properties 等
//!   ├── logs/          latest.log
//!   ├── world/         世界数据(region/*.mcr)
//!   └── start.sh       启动脚本

mod config;
mod logger;
mod network;
mod protocol;
mod world;

use config::ServerConfig;
use logger::Logger;
use std::env;
use std::net::TcpStream;
use std::path::PathBuf;
use std::process::ExitCode;

/// Status JSON 里的版本名(与协议常量一致)。
pub const VERSION_NAME: &str = mc_protocol::consts::VERSION_NAME;

fn main() -> ExitCode {
    let log = Logger::new();

    // ---- 运行目录:默认取环境变量 MCS_HOME 或当前目录 ----
    let root: PathBuf = env::var("MCS_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            // 若当前目录名是 bin,则向上取一层
            if env::current_dir()
                .map(|p| p.file_name().map(|n| n == "bin").unwrap_or(false))
                .unwrap_or(false)
            {
                env::current_dir().unwrap().parent().unwrap().to_path_buf()
            } else {
                env::current_dir().unwrap()
            }
        });
    let bin_dir = root.join("bin");
    let config_dir = root.join("config");
    let log_dir = root.join("logs");
    let world_dir = root.join("world");

    ServerConfig::init_vanilla_files(&root);
    let _ = std::fs::create_dir_all(&bin_dir);
    let _ = std::fs::create_dir_all(&config_dir);
    let _ = std::fs::create_dir_all(&world_dir);

    if log.open(&log_dir).is_err() {
        eprintln!("[ERROR] cannot open log dir {}", log_dir.display());
    }
    log.info("Mp-minecraft server starting");
    log.info(&format!("root dir: {}", root.display()));

    // ---- 配置 ----
    let mut cfg = ServerConfig::load_or_create(&config_dir);
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
    log.info(&format!(
        "server listening on 0.0.0.0:{} (protocol 775, MC 26.1.2)",
        port
    ));
    match network::NetworkServer::bind(port, move |stream: TcpStream| {
        let _ = protocol::handle_connection(stream, port, &motd, max_players);
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