//! 连接状态机:Handshake → Status / Login → Play。
//! 当前实现:Handshake 识别意图,Status 完整(响应列表与 Ping/Pong)。

use crate::network::{send_packet, ConnReader};
use mc_protocol::buf::ReadBuf;
use mc_protocol::consts::{intent, PROTOCOL_VERSION, State};
use std::io;
use std::net::TcpStream;

pub struct ConnInfo {
    pub state: State,
    pub protocol_version: i32,
    pub host: String,
    pub port: u16,
}

impl ConnInfo {
    pub fn new() -> Self {
        ConnInfo {
            state: State::Handshake,
            protocol_version: 0,
            host: String::new(),
            port: 25565,
        }
    }
}

/// 处理一个客户端连接,直到断开。
pub fn handle_connection(
    mut stream: TcpStream,
    port: u16,
    motd: &str,
    max_players: i32,
) -> io::Result<()> {
    stream.set_nodelay(true)?;
    let mut conn = ConnInfo::new();
    let mut reader = ConnReader::new();

    loop {
        let Some(frame) = reader.next_frame(&mut stream)? else {
            return Ok(());
        };
        let mut r = ReadBuf::new(&frame);
        let Ok(packet_id) = r.read_varint() else {
            return Ok(());
        };
        match conn.state {
            State::Handshake => {
                if !handle_handshake(&mut conn, packet_id, &mut r) {
                    return Ok(());
                }
            }
            State::Status => {
                handle_status(&mut stream, packet_id, &mut r, motd, max_players, port)?;
                if packet_id == 0x01 {
                    return Ok(()); // Ping 后断开
                }
            }
            State::Login => {
                let _ = &mut r;
                return Ok(()); // 后续实现
            }
            State::Play | State::Configuration => {
                let _ = &mut r;
                return Ok(());
            }
        }
    }
}

/// Handshake (0x00):协议版本 + 地址 + 端口 + 意图。
fn handle_handshake(conn: &mut ConnInfo, packet_id: i32, r: &mut ReadBuf) -> bool {
    if packet_id != 0x00 {
        return false;
    }
    let Ok(protocol_version) = r.read_varint() else {
        return false;
    };
    let Ok(host) = r.read_string() else {
        return false;
    };
    let Ok(port) = r.read_u16() else {
        return false;
    };
    let Ok(next_state) = r.read_varint() else {
        return false;
    };
    conn.protocol_version = protocol_version;
    conn.host = host;
    conn.port = port;
    conn.state = match next_state {
        intent::STATUS => State::Status,
        intent::LOGIN => State::Login,
        _ => return false,
    };
    true
}

/// Status:Request (0x00) → Response;Ping (0x01) → Pong。
fn handle_status(
    stream: &mut TcpStream,
    packet_id: i32,
    r: &mut ReadBuf,
    motd: &str,
    max_players: i32,
    port: u16,
) -> io::Result<()> {
    match packet_id {
        0x00 => {
            if !r.finished() {
                return Ok(());
            }
            let json = format!(
                "{{\"version\":{{\"name\":\"{}\",\"protocol\":{}}},\
                 \"players\":{{\"max\":{},\"online\":0,\"sample\":[]}},\
                 \"description\":{{\"text\":\"{}\"}},\
                 \"favicon\":null}}",
                crate::VERSION_NAME, PROTOCOL_VERSION, max_players, motd
            );
            let mut p = mc_protocol::buf::WriteBuf::new();
            mc_protocol::packets::status::clientbound::write_response(&mut p, &json);
            send_packet(stream, &p.into_bytes())
        }
        0x01 => {
            let Ok(timestamp) = r.read_i64() else {
                return Ok(());
            };
            let mut p = mc_protocol::buf::WriteBuf::new();
            mc_protocol::packets::status::clientbound::write_pong(&mut p, timestamp);
            send_packet(stream, &p.into_bytes())
        }
        _ => Ok(()),
    }
}
