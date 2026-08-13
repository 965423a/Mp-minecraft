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
mod world;

use config::ServerConfig;
use logger::Logger;
use std::env;
use std::path::PathBuf;
use std::process::ExitCode;

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

    if log.open(&log_dir).is_err() {
        eprintln!("[ERROR] cannot open log dir {}", log_dir.display());
    }
    log.info("Mp-minecraft server starting");
    log.info(&format!("root dir: {}", root.display()));

    // ---- 配置 ----
    let cfg = ServerConfig::load_or_create(&config_dir);
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

    // ---- 网络监听(后续实现 v775 协议) ----
    log.info(&format!(
        "server listening on 0.0.0.0:{} (protocol 775, MC 26.1.2)",
        cfg.port
    ));
    log.info("Done. This is a Mp-minecraft server.");
    log.info("Type 'stop' to shut down (console input not yet wired).");

    ExitCode::SUCCESS
}