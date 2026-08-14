//! 网络层:TCP 监听、连接管理、帧读取(长度前缀 + 可选压缩)。

use std::io::{self, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::Arc;

use flate2::read::ZlibDecoder;
use flate2::write::ZlibEncoder;
use flate2::Compression;
use mc_protocol::buf::MAX_BYTES;
use mc_protocol::varint;

/// 每个连接的读取状态:缓冲未消费的字节流,按长度前缀切分帧。
pub struct ConnReader {
    buf: Vec<u8>,
    compression: bool,
}

impl ConnReader {
    pub fn new() -> Self {
        ConnReader { buf: Vec::with_capacity(4096), compression: false }
    }

    pub fn set_compression(&mut self, on: bool) {
        self.compression = on;
    }

    /// 从流读入更多字节,返回本帧(长度前缀 + 包体)。
    pub fn next_frame(&mut self, stream: &mut TcpStream) -> io::Result<Option<Vec<u8>>> {
        loop {
            if let Some(frame) = self.try_frame()? {
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

    fn try_frame(&mut self) -> io::Result<Option<Vec<u8>>> {
        let mut pos = 0usize;
        let Some(len) = varint::decode_varint_i32(&self.buf, &mut pos) else {
            return Ok(None);
        };
        if len < 0 || len as usize > MAX_BYTES {
            return Ok(None);
        }
        let Some(end) = pos.checked_add(len as usize) else {
            return Ok(None);
        };
        if end > self.buf.len() {
            return Ok(None);
        }
        let frame = self.buf[pos..end].to_vec();
        self.buf.drain(..end);
        if !self.compression {
            return Ok(Some(frame));
        }
        // 压缩模式:包体 = 未压缩长度 varint + (若 >0)zlib 数据
        let mut rp = 0usize;
        let Some(uncompressed_len) = varint::decode_varint_i32(&frame, &mut rp) else {
            return Ok(Some(frame));
        };
        if uncompressed_len == 0 {
            return Ok(Some(frame[rp..].to_vec()));
        }
        let mut dec = ZlibDecoder::new(&frame[rp..]);
        let mut out = Vec::with_capacity(uncompressed_len as usize);
        dec.read_to_end(&mut out)?;
        Ok(Some(out))
    }

    /// 非阻塞轮询:有完整帧返回,无数据返回 None(不阻塞)。
    pub fn try_poll(&mut self, stream: &mut TcpStream) -> io::Result<Option<Vec<u8>>> {
        if let Some(frame) = self.try_frame()? {
            return Ok(Some(frame));
        }
        let mut chunk = [0u8; 4096];
        match stream.read(&mut chunk) {
            Ok(0) => Ok(None),
            Ok(n) => {
                self.buf.extend_from_slice(&chunk[..n]);
                if self.buf.len() > MAX_BYTES {
                    return Err(io::Error::new(io::ErrorKind::InvalidData, "frame too large"));
                }
                self.try_frame()
            }
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => Ok(None),
            Err(e) => Err(e),
        }
    }
}

/// 发送一个包:长度前缀 + 包体(压缩模式时按压缩格式)。
pub fn send_packet_compressed(
    stream: &mut TcpStream,
    packet: &[u8],
    threshold: i32,
    compressed: bool,
) -> io::Result<()> {
    let mut body = Vec::with_capacity(packet.len() + 8);
    if compressed {
        if packet.len() >= threshold as usize {
            let mut enc = ZlibEncoder::new(Vec::new(), Compression::default());
            enc.write_all(packet)?;
            let z = enc.finish()?;
            varint::write_varint(packet.len() as u32, &mut body);
            body.extend_from_slice(&z);
        } else {
            varint::write_varint(0, &mut body);
            body.extend_from_slice(packet);
        }
    } else {
        body.extend_from_slice(packet);
    }
    let mut frame = Vec::with_capacity(body.len() + 5);
    varint::write_varint(body.len() as u32, &mut frame);
    frame.extend_from_slice(&body);
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
