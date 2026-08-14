//! 连接状态机:Handshake → Status / Login → Configuration → Play。

use crate::network::{send_packet_compressed, ConnReader};
use mc_protocol::buf::{ReadBuf, WriteBuf};
use mc_protocol::consts::{intent, COMPRESSION_THRESHOLD, PROTOCOL_VERSION, State};
use std::io;
use std::net::TcpStream;

pub struct ConnInfo {
    pub state: State,
    pub protocol_version: i32,
    pub host: String,
    pub port: u16,
    pub name: String,
    pub uuid: [u8; 16],
    pub hashed_seed: i64,
    pub is_flat: bool,
    pub entity_id: i32,
    pub teleport_id: i32,
    pub keep_alive_id: i64,
}

impl ConnInfo {
    pub fn new() -> Self {
        ConnInfo {
            state: State::Handshake,
            protocol_version: 0,
            host: String::new(),
            port: 25565,
            name: String::new(),
            uuid: [0u8; 16],
            hashed_seed: 0,
            is_flat: false,
            entity_id: 0,
            teleport_id: 0,
            keep_alive_id: 0,
        }
    }
}

pub fn handle_connection(
    mut stream: TcpStream,
    _port: u16,
    motd: &str,
    max_players: i32,
    view_distance: i32,
    spawn: (f64, f64, f64),
    seed: u64,
    flat: bool,
    world_dir: &std::path::Path,
    wtype: mc_world::generator::WorldType,
    registry: &crate::registry::Registry,
) -> io::Result<()> {
    stream.set_nodelay(true)?;
    let mut conn = ConnInfo::new();
    conn.hashed_seed = seed as i64;
    conn.is_flat = flat;
    let mut reader = ConnReader::new();
    let mut compressed = false;
    let mut last_tick = std::time::Instant::now();

    loop {
        // Play 状态下每 50ms 发送一次 Keep Alive(模拟 20TPS 节拍)。
        if conn.state == State::Play {
            let now = std::time::Instant::now();
            if now.duration_since(last_tick).as_millis() >= 50 {
                last_tick = now;
                send_keep_alive(&mut stream, &mut conn, compressed)?;
            }
        }

        let frame = if conn.state == State::Play {
            stream.set_nonblocking(true)?;
            let frame = reader.try_poll(&mut stream)?;
            stream.set_nonblocking(false)?;
            match frame {
                Some(f) => Some(f),
                None => {
                    std::thread::sleep(std::time::Duration::from_millis(2));
                    continue;
                }
            }
        } else {
            reader.next_frame(&mut stream)?
        };
        let Some(frame) = frame else {
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
                if !handle_login(&mut stream, &mut reader, &mut conn, packet_id, &mut r, &mut compressed)? {
                    return Ok(());
                }
            }
            State::Configuration => {
                if !handle_configuration(
                    &mut stream,
                    &mut conn,
                    packet_id,
                    &mut r,
                    &mut compressed,
                    max_players,
                    view_distance,
                    spawn,
                    world_dir,
                    wtype,
                    registry,
                )? {
                    return Ok(());
                }
            }
            State::Play => {
                if !handle_play(&mut stream, packet_id, &mut r, &mut conn, compressed)? {
                    return Ok(());
                }
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
            send_packet_compressed(stream, &p.into_bytes(), COMPRESSION_THRESHOLD, false)
        }
        0x01 => {
            let Ok(timestamp) = r.read_i64() else {
                return Ok(());
            };
            let mut p = WriteBuf::new();
            mc_protocol::packets::status::clientbound::write_pong(&mut p, timestamp);
            send_packet_compressed(stream, &p.into_bytes(), COMPRESSION_THRESHOLD, false)
        }
        _ => Ok(()),
    }
}

fn handle_login(
    stream: &mut TcpStream,
    reader: &mut ConnReader,
    conn: &mut ConnInfo,
    packet_id: i32,
    r: &mut ReadBuf,
    compressed: &mut bool,
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
            send_packet_compressed(stream, &p.into_bytes(), COMPRESSION_THRESHOLD, *compressed)?;
            *compressed = true;
            reader.set_compression(true);

            let mut uuid = start.uuid;
            if uuid == [0u8; 16] {
                uuid = crate::offline_uuid(&start.name);
            }
            conn.name = start.name.clone();
            conn.uuid = uuid;
            let mut p = WriteBuf::new();
            mc_protocol::packets::login::clientbound::write_success(&mut p, uuid, &start.name, &[]);
            send_packet_compressed(stream, &p.into_bytes(), COMPRESSION_THRESHOLD, *compressed)?;
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

#[allow(clippy::too_many_arguments)]
fn handle_configuration(
    stream: &mut TcpStream,
    conn: &mut ConnInfo,
    packet_id: i32,
    r: &mut ReadBuf,
    compressed: &mut bool,
    max_players: i32,
    view_distance: i32,
    spawn: (f64, f64, f64),
    world_dir: &std::path::Path,
    wtype: mc_world::generator::WorldType,
    registry: &crate::registry::Registry,
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
            send_packet_compressed(stream, &p.into_bytes(), COMPRESSION_THRESHOLD, *compressed)?;
            for (reg, entries) in registry.groups() {
                let refs: Vec<(&str, Option<&[u8]>)> = entries
                    .iter()
                    .map(|(k, n)| (k.as_str(), Some(n.as_slice())))
                    .collect();
                let mut p = WriteBuf::new();
                configuration::clientbound::write_registry_data(&mut p, &reg, &refs);
                send_packet_compressed(stream, &p.into_bytes(), COMPRESSION_THRESHOLD, *compressed)?;
            }
            let mut p = WriteBuf::new();
            configuration::clientbound::write_feature_flags(&mut p, &[]);
            send_packet_compressed(stream, &p.into_bytes(), COMPRESSION_THRESHOLD, *compressed)?;
            let mut p = WriteBuf::new();
            configuration::clientbound::write_update_tags_empty(&mut p);
            send_packet_compressed(stream, &p.into_bytes(), COMPRESSION_THRESHOLD, *compressed)?;
            let mut p = WriteBuf::new();
            configuration::clientbound::write_finish_configuration(&mut p);
            send_packet_compressed(stream, &p.into_bytes(), COMPRESSION_THRESHOLD, *compressed)?;
            Ok(true)
        }
        configuration::serverbound::ID_ACK_FINISH_CONFIGURATION => {
            conn.state = State::Play;
            let ok = send_play_init(
                stream, conn, max_players, view_distance, spawn, world_dir, wtype, *compressed,
            );
            ok.map(|_| true)
        }
        _ => Ok(false),
    }
}

#[allow(clippy::too_many_arguments)]
fn handle_play(
    _stream: &mut TcpStream,
    packet_id: i32,
    r: &mut ReadBuf,
    conn: &mut ConnInfo,
    _compressed: bool,
) -> io::Result<bool> {
    use mc_protocol::packets::play::serverbound as sb;
    match packet_id {
        sb::ID_CONFIRM_TELEPORTATION => {
            let _ = sb::read_confirm_teleportation(r);
            Ok(true)
        }
        sb::ID_SET_POSITION => {
            let _ = sb::read_player_position(r, false).ok();
            Ok(true)
        }
        sb::ID_SET_POSITION_ROTATION => {
            let _ = sb::read_player_position(r, true).ok();
            Ok(true)
        }
        sb::ID_KEEP_ALIVE => {
            let Ok(id) = sb::read_keep_alive(r) else {
                return Ok(false);
            };
            if id != conn.keep_alive_id {
                return Ok(false);
            }
            Ok(true)
        }
        sb::ID_PLAYER_LOADED => Ok(true),
        _ => Ok(true),
    }
}

/// 进入 Play 后发送初始包:Join Game + Player Info + 区块 + 位置同步。
pub fn send_play_init(    stream: &mut TcpStream,
    conn: &mut ConnInfo,
    max_players: i32,
    view_distance: i32,
    spawn: (f64, f64, f64),
    world_dir: &std::path::Path,
    wtype: mc_world::generator::WorldType,
    compressed: bool,
) -> io::Result<()> {
    use mc_protocol::packets::play::clientbound as cb;
    conn.entity_id = 1;
    conn.keep_alive_id = 1;

    let mut p = WriteBuf::new();
    cb::write_join_game(
        &mut p,
        &cb::JoinGame {
            entity_id: conn.entity_id,
            hardcore: false,
            dimension_names: vec![
                "minecraft:overworld".to_string(),
                "minecraft:the_nether".to_string(),
                "minecraft:the_end".to_string(),
            ],
            max_players,
            view_distance,
            simulation_distance: view_distance,
            reduced_debug_info: false,
            enable_respawn_screen: true,
            do_limited_crafting: false,
            dimension_type: 0,
            dimension_name: "minecraft:overworld".to_string(),
            hashed_seed: conn.hashed_seed,
            gamemode: 0,
            previous_gamemode: -1,
            is_debug: false,
            is_flat: conn.is_flat,
            portal_cooldown: 0,
            sea_level: 63,
            enforces_secure_chat: false,
        },
    );
    send_packet_compressed(stream, &p.into_bytes(), COMPRESSION_THRESHOLD, compressed)?;

    let mut p = WriteBuf::new();
    cb::write_player_info_add(&mut p, conn.uuid, &conn.name, &[]);
    send_packet_compressed(stream, &p.into_bytes(), COMPRESSION_THRESHOLD, compressed)?;

    // 区块:出生点 3x3 网格
    let spawn_chunk_x = (spawn.0.floor() as i32).div_euclid(16);
    let spawn_chunk_z = (spawn.2.floor() as i32).div_euclid(16);
    let light = mc_world::network::light_full();

    let mut p = WriteBuf::new();
    cb::write_set_chunk_cache_center(&mut p, spawn_chunk_x, spawn_chunk_z);
    send_packet_compressed(stream, &p.into_bytes(), COMPRESSION_THRESHOLD, compressed)?;

    let mut p = WriteBuf::new();
    cb::write_set_default_spawn_position(&mut p, 0, 65, 0, 0.0);
    send_packet_compressed(stream, &p.into_bytes(), COMPRESSION_THRESHOLD, compressed)?;

    let mut p = WriteBuf::new();
    cb::write_chunk_batch_start(&mut p);
    send_packet_compressed(stream, &p.into_bytes(), COMPRESSION_THRESHOLD, compressed)?;

    let mut sent = 0;
    for cx in (spawn_chunk_x - 1)..=(spawn_chunk_x + 1) {
        for cz in (spawn_chunk_z - 1)..=(spawn_chunk_z + 1) {
            let chunk = crate::world::load_or_generate(world_dir, conn.hashed_seed as u64, wtype, cx, cz);
            let hms = mc_world::network::chunk_heightmaps(&chunk);
            let heightmaps: Vec<(&[u64], u32)> = hms.iter().map(|(ty, d)| (d.as_slice(), *ty)).collect();
            let mut data = Vec::new();
            mc_world::network::write_sections(&chunk, PLAINS_BIOME, &mut data);
            let mut p = WriteBuf::new();
            cb::write_chunk_data(&mut p, cx, cz, &heightmaps, &data, &[], &light);
            send_packet_compressed(stream, &p.into_bytes(), COMPRESSION_THRESHOLD, compressed)?;
            sent += 1;
        }
    }

    let mut p = WriteBuf::new();
    cb::write_chunk_batch_finished(&mut p, sent);
    send_packet_compressed(stream, &p.into_bytes(), COMPRESSION_THRESHOLD, compressed)?;

    conn.teleport_id += 1;
    let mut p = WriteBuf::new();
    cb::write_sync_player_position(
        &mut p,
        conn.teleport_id,
        spawn.0,
        spawn.1,
        spawn.2,
        0.0,
        0.0,
        0.0,
        0.0,
        0.0,
        0,
    );
    send_packet_compressed(stream, &p.into_bytes(), COMPRESSION_THRESHOLD, compressed)
}

/// 原版 26.1 biome 注册表中 minecraft:plains 的 ID。
const PLAINS_BIOME: u16 = 40;

/// 20TPS tick:发送 Keep Alive(原版每 tick 发一次)。
pub fn send_keep_alive(stream: &mut TcpStream, conn: &mut ConnInfo, compressed: bool) -> io::Result<()> {
    use mc_protocol::packets::play::clientbound as cb;
    conn.keep_alive_id += 1;
    let mut p = WriteBuf::new();
    cb::write_keep_alive(&mut p, conn.keep_alive_id);
    send_packet_compressed(stream, &p.into_bytes(), COMPRESSION_THRESHOLD, compressed)
}

fn send_disconnect(stream: &mut TcpStream, reason_json: &str) -> io::Result<()> {
    let mut p = WriteBuf::new();
    mc_protocol::packets::login::clientbound::write_disconnect(&mut p, reason_json);
    send_packet_compressed(stream, &p.into_bytes(), COMPRESSION_THRESHOLD, false)
}

fn log_join(conn: &ConnInfo, name: &str, uuid: [u8; 16]) {
    let hex = uuid
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<Vec<_>>()
        .join("");
    eprintln!("[player] {name} ({hex}) logged in from {}({})", conn.host, conn.port);
}
