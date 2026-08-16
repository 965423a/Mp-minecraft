//! 协议常量:目标版本 MC Java 26.1.2 / 协议 775 / data version 4790。

pub const PROTOCOL_VERSION: i32 = 775;

pub const DATA_VERSION: i32 = 4790;

/// 服务器向客户端自报的版本字符串。
pub const VERSION_NAME: &str = "26.1.2";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    Handshake,
    Status,
    Login,
    Configuration,
    Play,
}

impl State {
    pub fn name(&self) -> &'static str {
        match self {
            State::Handshake => "handshake",
            State::Status => "status",
            State::Login => "login",
            State::Configuration => "configuration",
            State::Play => "play",
        }
    }
}

pub mod intent {
    pub const STATUS: i32 = 1;
    pub const LOGIN: i32 = 2;
}

/// 压缩阈值;包体(不含长度前缀)超过它才压缩。
pub const COMPRESSION_THRESHOLD: i32 = 256;

/// 世界高度参数(超平坦 overworld)。
pub const WORLD_MIN_Y: i32 = -64;
pub const WORLD_HEIGHT: i32 = 384;
pub const WORLD_SECTIONS: usize = 24; // 384 / 16
pub const SECTION_ENTRIES: usize = 4096;
pub const BIOME_ENTRIES: usize = 64;
