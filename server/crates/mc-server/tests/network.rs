//! 网络集成测试:真实 TCP 起服,模拟客户端完成握手 + Status 全流程。

use mc_protocol::buf::{ReadBuf, WriteBuf};
use mc_protocol::consts::PROTOCOL_VERSION;
use std::io::{Read, Write};
use std::net::TcpStream;

fn write_frame(stream: &mut TcpStream, packet: &[u8]) {
    let mut frame = Vec::with_capacity(packet.len() + 5);
    mc_protocol::varint::write_varint(packet.len() as u32, &mut frame);
    frame.extend_from_slice(packet);
    stream.write_all(&frame).unwrap();
}

fn read_frame(stream: &mut TcpStream) -> Vec<u8> {
    let mut len_bytes = [0u8; 5];
    let mut pos = 0;
    loop {
        stream.read_exact(&mut len_bytes[pos..pos + 1]).unwrap();
        pos += 1;
        let mut used = 0;
        let len = match mc_protocol::varint::decode_varint_i32(&len_bytes[..pos], &mut used) {
            Some(v) => v,
            None => {
                if pos >= 5 {
                    panic!("bad length varint");
                }
                continue;
            }
        };
        let mut body = vec![0u8; len as usize];
        stream.read_exact(&mut body).unwrap();
        return body;
    }
}

fn start_server() -> (u16, std::process::Child) {
    static NEXT: std::sync::atomic::AtomicU16 = std::sync::atomic::AtomicU16::new(28000);
    let port = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let home = std::env::temp_dir().join(format!("mcs-net-test-{port}"));
    let _ = std::fs::remove_dir_all(&home);
    let child = std::process::Command::new(env!("CARGO_BIN_EXE_mc-server"))
        .env("MCS_HOME", &home)
        .env("MCS_THREADS", "2")
        .env("MCS_PORT", port.to_string())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    std::thread::sleep(std::time::Duration::from_millis(800));
    (port, child)
}

#[test]
fn status_ping_roundtrip() {
    let (port, mut child) = start_server();
    let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();

    let mut hs = WriteBuf::new();
    hs.write_varint(0x00);
    hs.write_varint(PROTOCOL_VERSION);
    hs.write_string("127.0.0.1");
    hs.write_u16(port);
    hs.write_varint(1);
    write_frame(&mut stream, &hs.into_bytes());

    let mut req = WriteBuf::new();
    req.write_varint(0x00);
    write_frame(&mut stream, &req.into_bytes());

    let resp = read_frame(&mut stream);
    let mut r = ReadBuf::new(&resp);
    assert_eq!(r.read_varint().unwrap(), 0x00);
    let json = r.read_string().unwrap();
    assert!(json.contains("\"protocol\":") && json.contains("A Mp-minecraft Server"), "bad status json: {json}");
    assert!(json.contains(PROTOCOL_VERSION.to_string().as_str()));

    let now = 1234567890i64;
    let mut ping = WriteBuf::new();
    ping.write_varint(0x01);
    ping.write_i64(now);
    write_frame(&mut stream, &ping.into_bytes());

    let pong = read_frame(&mut stream);
    let mut r = ReadBuf::new(&pong);
    assert_eq!(r.read_varint().unwrap(), 0x01);
    assert_eq!(r.read_i64().unwrap(), now);
    let _ = child.kill();
}

#[test]
fn login_intent_accepted() {
    let (port, mut child) = start_server();
    let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
    let mut hs = WriteBuf::new();
    hs.write_varint(0x00);
    hs.write_varint(PROTOCOL_VERSION);
    hs.write_string("127.0.0.1");
    hs.write_u16(port);
    hs.write_varint(2);
    write_frame(&mut stream, &hs.into_bytes());

    let mut start = WriteBuf::new();
    start.write_varint(0x00);
    start.write_string("Tester");
    start.write_uuid([0u8; 16]);
    write_frame(&mut stream, &start.into_bytes());

    std::thread::sleep(std::time::Duration::from_millis(200));
    let mut buf = [0u8; 1];
    stream.set_nonblocking(true).unwrap();
    let n = stream.read(&mut buf).unwrap_or(0);
    assert_eq!(n, 0, "login 未实现时应静默断开或等待,不应崩溃");
    let _ = child.kill();
}