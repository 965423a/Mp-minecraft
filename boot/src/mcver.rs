//! Minecraft 服务器版本注册表与切换:switch 命令。
//!
//! 从 1.0 到当前最新(26.2),每版本记录协议号与区块格式特征。
//! 切换影响:status 响应(版本名/协议号)、区块打包(高度上限/位数)。
//! 协议号取自官方 protocol version(1.0=4 ... 1.21.4=769),
//! 26.x 为延续版本(26.1=774, 26.2=776)。

use core::sync::atomic::{AtomicUsize, Ordering};

/// 区块格式特征:按版本变化的世界高度与打包位数。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VerFeatures {
    /// 世界最大 Y(高度上限,区块打包只含此下 section)。
    pub world_max_y: i32,
    /// 默认打包位数(1.13+ 扁平化后 8 位足够)。
    pub pack_bits: u32,
}

pub const fn feat(world_max_y: i32, pack_bits: u32) -> VerFeatures {
    VerFeatures {
        world_max_y,
        pack_bits,
    }
}

/// 版本注册表:从 1.0 到 26.2。
pub const VERSIONS: &[(&str, i32, i32, VerFeatures)] = &[
    ("1.0", 4, 405, feat(128, 4)),
    ("1.1", 23, 405, feat(128, 4)),
    ("1.2.5", 29, 405, feat(128, 4)),
    ("1.3.2", 39, 405, feat(128, 4)),
    ("1.4.7", 49, 405, feat(128, 4)),
    ("1.5.2", 61, 405, feat(128, 4)),
    ("1.6.4", 78, 405, feat(128, 4)),
    ("1.7.10", 5, 405, feat(128, 4)),
    ("1.8.9", 47, 405, feat(128, 4)),
    ("1.9.4", 110, 405, feat(128, 4)),
    ("1.10.2", 210, 405, feat(128, 4)),
    ("1.11.2", 316, 405, feat(128, 4)),
    ("1.12.2", 340, 405, feat(128, 4)),
    ("1.13.2", 404, 1631, feat(256, 8)),
    ("1.14.4", 498, 1963, feat(256, 8)),
    ("1.15.2", 578, 2230, feat(256, 8)),
    ("1.16.5", 754, 2586, feat(256, 8)),
    ("1.17.1", 756, 2730, feat(256, 8)),
    ("1.18.2", 758, 2975, feat(384, 8)),
    ("1.19.4", 762, 3337, feat(384, 8)),
    ("1.20.1", 763, 3465, feat(384, 8)),
    ("1.20.4", 765, 3700, feat(384, 8)),
    ("1.21.1", 767, 3953, feat(384, 8)),
    ("1.21.4", 769, 4189, feat(384, 8)),
    ("26.1", 774, 4770, feat(384, 8)),
    ("26.2", 776, 4800, feat(384, 8)),
];

static CUR: AtomicUsize = AtomicUsize::new(VERSIONS.len() - 1);

/// 当前版本索引。
pub fn cur_idx() -> usize {
    CUR.load(Ordering::Relaxed)
}

/// 当前版本名。
pub fn cur_name() -> &'static str {
    VERSIONS[cur_idx()].0
}

/// 当前协议号。
pub fn cur_protocol() -> i32 {
    VERSIONS[cur_idx()].1
}

/// 当前数据版本。
pub fn cur_data_version() -> i32 {
    VERSIONS[cur_idx()].2
}

/// 当前区块格式特征。
pub fn cur_features() -> VerFeatures {
    VERSIONS[cur_idx()].3
}

/// 按名字切换。返回是否成功。
pub fn switch(name: &str) -> bool {
    for (i, v) in VERSIONS.iter().enumerate() {
        if v.0 == name {
            CUR.store(i, Ordering::Relaxed);
            return true;
        }
    }
    false
}