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

    /// 打开日志文件(追加)。与原版一致:启动时旧日志轮转为 <date>-<n>.log.gz。
    pub fn open(&self, log_dir: &Path) -> std::io::Result<()> {
        std::fs::create_dir_all(log_dir)?;
        let path = log_dir.join("latest.log");
        if path.exists() {
            Self::rotate(log_dir, &path);
        }
        let f = OpenOptions::new().create(true).append(true).open(path)?;
        *self.file.lock().unwrap() = Some(f);
        Ok(())
    }

    fn rotate(log_dir: &Path, latest: &Path) {
        let date = {
            let d = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|x| x.as_secs())
                .unwrap_or(0);
            let days = (d / 86400) as i64;
            let rem = d % 86400;
            let (h, m, s) = (rem / 3600, (rem % 3600) / 60, rem % 60);
            let (y, mo, dd) = days_from_epoch(days);
            format!("{y:04}-{mo:02}-{dd:02}-{h}-{m}-{s}")
        };
        let mut n = 1usize;
        let mut target = log_dir.join(format!("{date}-{n}.log.gz"));
        while target.exists() {
            n += 1;
            target = log_dir.join(format!("{date}-{n}.log.gz"));
        }
        let Ok(src) = std::fs::File::open(latest) else {
            return;
        };
        let Ok(out) = std::fs::File::create(&target) else {
            return;
        };
        let mut enc = flate2::write::GzEncoder::new(out, flate2::Compression::default());
        let mut buf = std::io::BufReader::new(src);
        if std::io::copy(&mut buf, &mut enc).is_ok() {
            let _ = enc.finish();
            let _ = std::fs::remove_file(latest);
        }
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

/// 天数 → (年, 月, 日),公历(0 = 1970-01-01)。
fn days_from_epoch(days: i64) -> (i64, u32, u32) {
    let mut d = days + 719468;
    let era = if d >= 0 { d } else { d - 146096 } / 146097;
    let doe = d - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let dd = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let mo = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let yy = if mo <= 2 { y + 1 } else { y };
    (yy, mo, dd)
}