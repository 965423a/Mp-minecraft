//! 网络层:TCP 监听、连接管理、帧读取(长度前缀 + 包体)。

use std::io::{self, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::Arc;

use mc_protocol::buf::MAX_BYTES;
use mc_protocol::varint;

/// 每个连接的读取状态:缓冲未消费的字节流,按长度前缀切分帧。
pub struct ConnReader {
    buf: Vec<u8>,
}

impl ConnReader {
    pub fn new() -> Self {
        ConnReader { buf: Vec::with_capacity(4096) }
    }

    /// 从流读入更多字节,返回本帧(长度前缀 + 包体)。
    pub fn next_frame(&mut self, stream: &mut TcpStream) -> io::Result<Option<Vec<u8>>> {
        loop {
            if let Some(frame) = self.try_frame() {
                return Ok(Some(frame));
            }
            let mut chunk = [0u8; 4096];
            let n = stream.read(&mut chunk)?;
            if n == 0 {
                return Ok(None);
            }
            self.buf.extend_from_slice(&chunk[..n]);
            if self.buf.len() > MAX_BYTES {
                return Err(io::Error::new(io::ErrorKind::InvalidData, "frame too large"));
            }
        }
    }

    fn try_frame(&mut self) -> Option<Vec<u8>> {
        let mut pos = 0usize;
        let len = varint::decode_varint_i32(&self.buf, &mut pos)?;
        if len < 0 || len as usize > MAX_BYTES {
            return None;
        }
        let end = pos.checked_add(len as usize)?;
        if end > self.buf.len() {
            return None;
        }
        let frame = self.buf[pos..end].to_vec();
        self.buf.drain(..end);
        Some(frame)
    }
}

/// 发送一个包:长度前缀 + 包体。
pub fn send_packet(stream: &mut TcpStream, packet: &[u8]) -> io::Result<()> {
    let mut frame = Vec::with_capacity(packet.len() + 5);
    varint::write_varint(packet.len() as u32, &mut frame);
    frame.extend_from_slice(packet);
    stream.write_all(&frame)?;
    stream.flush()
}

/// 服务器句柄。
pub struct NetworkServer {
    pub port: u16,
    listener: TcpListener,
    handler: Arc<dyn Fn(TcpStream) + Send + Sync>,
}

impl NetworkServer {
    pub fn bind(port: u16, handler: impl Fn(TcpStream) + Send + Sync + 'static) -> io::Result<Self> {
        let listener = TcpListener::bind(("0.0.0.0", port))?;
        Ok(NetworkServer {
            port,
            listener,
            handler: Arc::new(handler),
        })
    }

    /// 阻塞接受连接,每个连接一个线程。
    pub fn run(&self) -> io::Result<()> {
        for stream in self.listener.incoming() {
            match stream {
                Ok(stream) => {
                    let handler = Arc::clone(&self.handler);
                    std::thread::spawn(move || {
                        let _ = handler(stream);
                    });
                }
                Err(e) => {
                    eprintln!("[network] accept error: {e}");
                }
            }
        }
        Ok(())
    }
}
