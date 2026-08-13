//! 连接状态机:Handshake → Status / Login → Configuration → Play。

use crate::network::{send_packet, ConnReader};
use mc_protocol::buf::{ReadBuf, WriteBuf};
use mc_protocol::consts::{intent, COMPRESSION_THRESHOLD, PROTOCOL_VERSION, State};
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
                handle_status(&mut stream, packet_id, &mut r, motd, max_players)?;
                if packet_id == 0x01 {
                    return Ok(());
                }
            }
            State::Login => {
                if !handle_login(&mut stream, &mut conn, packet_id, &mut r)? {
                    return Ok(());
                }
            }
            State::Configuration => {
                if !handle_configuration(&mut stream, &mut conn, packet_id, &mut r)? {
                    return Ok(());
                }
            }
            State::Play => {
                handle_play(&mut stream, packet_id, &mut r)?;
            }
        }
    }
}

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

fn handle_status(
    stream: &mut TcpStream,
    packet_id: i32,
    r: &mut ReadBuf,
    motd: &str,
    max_players: i32,
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
            let mut p = WriteBuf::new();
            mc_protocol::packets::status::clientbound::write_response(&mut p, &json);
            send_packet(stream, &p.into_bytes())
        }
        0x01 => {
            let Ok(timestamp) = r.read_i64() else {
                return Ok(());
            };
            let mut p = WriteBuf::new();
            mc_protocol::packets::status::clientbound::write_pong(&mut p, timestamp);
            send_packet(stream, &p.into_bytes())
        }
        _ => Ok(()),
    }
}

fn handle_login(
    stream: &mut TcpStream,
    conn: &mut ConnInfo,
    packet_id: i32,
    r: &mut ReadBuf,
) -> io::Result<bool> {
    match packet_id {
        0x00 => {
            let Ok(start) = mc_protocol::packets::login::serverbound::read_login_start(r) else {
                return Ok(false);
            };
            if conn.protocol_version != PROTOCOL_VERSION {
                send_disconnect(
                    stream,
                    &format!(
                        "{{\"text\":\"Unsupported protocol version {}. Expected {}\"}}",
                        conn.protocol_version, PROTOCOL_VERSION
                    ),
                )?;
                return Ok(false);
            }
            let mut p = WriteBuf::new();
            mc_protocol::packets::login::clientbound::write_set_compression(
                &mut p,
                COMPRESSION_THRESHOLD,
            );
            send_packet(stream, &p.into_bytes())?;

            let mut uuid = start.uuid;
            if uuid == [0u8; 16] {
                uuid = crate::offline_uuid(&start.name);
            }
            let mut p = WriteBuf::new();
            mc_protocol::packets::login::clientbound::write_success(&mut p, uuid, &start.name, &[]);
            send_packet(stream, &p.into_bytes())?;
            log_join(conn, &start.name, uuid);
            Ok(true)
        }
        0x03 => {
            conn.state = State::Configuration;
            Ok(true)
        }
        _ => Ok(false),
    }
}

fn handle_configuration(
    stream: &mut TcpStream,
    conn: &mut ConnInfo,
    packet_id: i32,
    r: &mut ReadBuf,
) -> io::Result<bool> {
    use mc_protocol::packets::configuration;
    match packet_id {
        configuration::serverbound::ID_CLIENT_INFORMATION => {
            let _ = configuration::serverbound::read_client_information(r);
            conn.state = State::Configuration;
            Ok(true)
        }
        configuration::serverbound::ID_KNOWN_PACKS => {
            let mut p = WriteBuf::new();
            configuration::clientbound::write_known_packs(&mut p, &[]);
            send_packet(stream, &p.into_bytes())?;
            let mut p = WriteBuf::new();
            configuration::clientbound::write_registry_data(
                &mut p,
                "minecraft:worldgen/biome",
                &[],
            );
            send_packet(stream, &p.into_bytes())?;
            let mut p = WriteBuf::new();
            configuration::clientbound::write_feature_flags(&mut p, &[]);
            send_packet(stream, &p.into_bytes())?;
            let mut p = WriteBuf::new();
            configuration::clientbound::write_update_tags_empty(&mut p);
            send_packet(stream, &p.into_bytes())?;
            let mut p = WriteBuf::new();
            configuration::clientbound::write_finish_configuration(&mut p);
            send_packet(stream, &p.into_bytes())?;
            Ok(true)
        }
        configuration::serverbound::ID_ACK_FINISH_CONFIGURATION => {
            conn.state = State::Play;
            Ok(true)
        }
        _ => Ok(false),
    }
}

fn handle_play(stream: &mut TcpStream, packet_id: i32, r: &mut ReadBuf) -> io::Result<bool> {
    let _ = stream;
    match packet_id {
        mc_protocol::packets::play::serverbound::ID_CONFIRM_TELEPORTATION => {
            let _ = mc_protocol::packets::play::serverbound::read_confirm_teleportation(r);
            Ok(true)
        }
        _ => Ok(true),
    }
}

fn send_disconnect(stream: &mut TcpStream, reason_json: &str) -> io::Result<()> {
    let mut p = WriteBuf::new();
    mc_protocol::packets::login::clientbound::write_disconnect(&mut p, reason_json);
    send_packet(stream, &p.into_bytes())
}

fn log_join(conn: &ConnInfo, name: &str, uuid: [u8; 16]) {
    let hex = uuid
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<Vec<_>>()
        .join("");
    eprintln!("[player] {name} ({hex}) logged in from {}({})", conn.host, conn.port);
}
