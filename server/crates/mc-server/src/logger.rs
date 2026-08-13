//! 日志:写入 logs/latest.log,同时输出到 stdout(原版行为)。

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::Path;
use std::sync::Mutex;

pub struct Logger {
    file: Mutex<Option<File>>,
}

impl Logger {
    pub fn new() -> Self {
        Logger { file: Mutex::new(None) }
    }

    /// 打开日志文件(追加)。
    pub fn open(&self, log_dir: &Path) -> std::io::Result<()> {
        std::fs::create_dir_all(log_dir)?;
        let path = log_dir.join("latest.log");
        let f = OpenOptions::new().create(true).append(true).open(path)?;
        *self.file.lock().unwrap() = Some(f);
        Ok(())
    }

    pub fn info(&self, msg: &str) {
        self.log("INFO", msg);
    }

    pub fn warn(&self, msg: &str) {
        self.log("WARN", msg);
    }

    pub fn error(&self, msg: &str) {
        self.log("ERROR", msg);
    }

    fn log(&self, level: &str, msg: &str) {
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let line = format!("[{ts}] [{level}] {msg}");
        println!("{line}");
        if let Some(f) = self.file.lock().unwrap().as_mut() {
            let _ = writeln!(f, "{line}");
            let _ = f.flush();
        }
    }
}

impl Default for Logger {
    fn default() -> Self {
        Self::new()
    }
}