//! 服务器逻辑嵌入演示:mc-world 世界生成 + mc-protocol 协议打包 +
//! mc-hotpath C/Rust 热路径,全部作为内核任务运行。

use alloc::format;
use alloc::string::String;
use core::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};

use mc_protocol::buf::{ReadBuf, WriteBuf};
use mc_protocol::consts::{PROTOCOL_VERSION, VERSION_NAME};
use mc_protocol::packets::play::clientbound::write_chunk_data;
use mc_protocol::packets::status::clientbound::write_response;
use mc_world::chunk::SECTIONS;
use mc_world::generator::{WorldGenerator, WorldType};

static GEN_SEED: AtomicU64 = AtomicU64::new(0);
static GEN_JOBS: AtomicUsize = AtomicUsize::new(0); // 每任务区块数
static GEN_TOTAL: AtomicUsize = AtomicUsize::new(0); // 已完成区块总数
static GEN_DONE: AtomicUsize = AtomicUsize::new(0); // 已完成任务数
static GEN_START: AtomicU64 = AtomicU64::new(0); // 开始 TSC

fn rdtsc() -> u64 {
    let lo: u32;
    let hi: u32;
    unsafe {
        core::arch::asm!("rdtsc", out("eax") lo, out("edx") hi, options(nomem, nostack));
    }
    ((hi as u64) << 32) | lo as u64
}

/// 生成任务:本核负责一批区块(确定性:seed + 坐标 → 同一世界)。
fn gen_task() -> ! {
    let cpu = crate::idt::lapic_id() as usize;
    let seed = GEN_SEED.load(Ordering::Relaxed);
    let jobs = GEN_JOBS.load(Ordering::Relaxed);
    let wg = WorldGenerator::new(seed, WorldType::Normal);
    let mut made = 0usize;
    for i in 0..jobs {
        let cx = (cpu * jobs + i) as i32;
        let chunk = wg.generate(cx, cpu as i32);
        core::hint::black_box(chunk);
        made += 1;
    }
    GEN_TOTAL.fetch_add(made, Ordering::Relaxed);
    GEN_DONE.fetch_add(1, Ordering::Relaxed);
    crate::sched::exit();
}

/// 服务化世界生成:mc-server.service 的真实内核任务。
/// start 后持续生成区块(坐标轮转),stop 置标志后停止生成、空转。
static SERVER_STOP: AtomicBool = AtomicBool::new(true);
static SERVER_TASK: AtomicUsize = AtomicUsize::new(usize::MAX);
static SERVER_CHUNKS: AtomicUsize = AtomicUsize::new(0);
static SERVER_SEED: AtomicU64 = AtomicU64::new(0xC0FFEE);

fn mc_server_task() -> ! {
    let seed = SERVER_SEED.load(Ordering::Relaxed);
    let wg = WorldGenerator::new(seed, WorldType::Normal);
    let mut i = 0u64;
    loop {
        if SERVER_STOP.load(Ordering::Relaxed) {
            core::hint::spin_loop();
            continue;
        }
        let cx = (i % 64) as i32 - 32;
        let cz = ((i / 64) % 64) as i32 - 32;
        let chunk = wg.generate(cx, cz);
        SERVER_PACKED.fetch_add(pack_chunk_data(&chunk, cx, cz), Ordering::Relaxed);
        core::hint::black_box(chunk);
        SERVER_CHUNKS.fetch_add(1, Ordering::Relaxed);
        i += 1;
    }
}

/// systemctl start mc-server.service:spawn 服务任务(幂等)。
pub fn server_start(seed: u64) -> bool {
    SERVER_SEED.store(seed, Ordering::Relaxed);
    SERVER_STOP.store(false, Ordering::Relaxed);
    if SERVER_TASK.load(Ordering::Relaxed) != usize::MAX {
        return true;
    }
    match crate::sched::spawn(mc_server_task) {
        Some(id) => {
            SERVER_TASK.store(id as usize, Ordering::Relaxed);
            true
        }
        None => false,
    }
}

/// systemctl stop mc-server.service:停止生成(任务保留空转)。
pub fn server_stop() {
    SERVER_STOP.store(true, Ordering::Relaxed);
}

/// 服务状态与生成计数(systemctl status / tasks)。
pub fn server_stats() -> (bool, usize) {
    (
        !SERVER_STOP.load(Ordering::Relaxed),
        SERVER_CHUNKS.load(Ordering::Relaxed),
    )
}

/// shell 命令:genworld [seed] [chunks_per_core]
/// 每核一个生成任务,完成后打印总耗时与区块数。
pub fn cmd_genworld(vga: &mut crate::Vga, seed: u64, jobs: usize) {
    let ncores = crate::smp::cpu_count();
    GEN_SEED.store(seed, Ordering::Relaxed);
    GEN_JOBS.store(jobs, Ordering::Relaxed);
    GEN_TOTAL.store(0, Ordering::Relaxed);
    GEN_DONE.store(0, Ordering::Relaxed);
    GEN_START.store(rdtsc(), Ordering::Relaxed);

    let mut spawned = 0usize;
    for _ in 0..ncores {
        if crate::sched::spawn(gen_task).is_some() {
            spawned += 1;
        }
    }
    if spawned == 0 {
        let _ = core::fmt::write(
            &mut *vga,
            format_args!("  genworld: no tasks spawned (table full?)\n"),
        );
        return;
    }
    while GEN_DONE.load(Ordering::Relaxed) < spawned {
        crate::sleep_short();
    }
    let us = (rdtsc() - GEN_START.load(Ordering::Relaxed)) / crate::smp::tsc_per_us();
    let total = GEN_TOTAL.load(Ordering::Relaxed);
    let _ = core::fmt::write(
        &mut *vga,
        format_args!(
            "  genworld: seed={seed} {total} chunks on {spawned} cores in {us} us ({:} us/chunk)\n",
            us / total.max(1) as u64
        ),
    );
    crate::log!(
        "genworld: seed={seed} {total} chunks on {spawned} cores in {us} us, allocs={} frees={}",
        crate::kalloc::stats().0,
        crate::kalloc::stats().1
    );
}
/// 区块协议打包:非空 section 位打包 → chunk data 帧。返回打包 section 数。
/// 这是服务器核心链路(生成 → 位打包 → 协议包)的内核验证路径。
fn pack_chunk_data(chunk: &mc_world::Chunk, cx: i32, cz: i32) -> usize {
    let mut w = WriteBuf::with_capacity(4096);
    let mut packed: alloc::vec::Vec<u8> = alloc::vec::Vec::new();
    let mut nsections = 0usize;
    let bits = 8u32;
    for i in 0..SECTIONS {
        let sec = chunk.section(i);
        if sec.is_empty() {
            continue;
        }
        nsections += 1;
        for v in sec.pack(bits) {
            packed.extend_from_slice(&v.to_le_bytes());
        }
    }
    write_chunk_data(&mut w, cx, cz, &[], &packed, &[], &[]);
    core::hint::black_box(w.len());
    nsections
}

static SERVER_PACKED: AtomicUsize = AtomicUsize::new(0);

/// shell 命令:pkt —— 服务器核心协议链路验证。
/// 1) C 热路径 varint 与 Rust 参考交叉验证(0..=1000 + 边界);
/// 2) Status Response 帧封装 + ReadBuf 解码 round-trip;
/// 3) 真实区块生成 → 位打包 → chunk data 帧。
pub fn cmd_pkt(vga: &mut crate::Vga, seed: u64) {
    // 1) varint 交叉验证
    let mut ok = 0usize;
    let mut total = 0usize;
    for v in 0..=1000u32 {
        let c = mc_hotpath::c_encode_varint(v);
        let r = mc_hotpath::r_encode_varint(v);
        total += 1;
        if c == r {
            if let Some((dv, n)) = mc_hotpath::c_decode_varint(&c) {
                if dv == v && n == c.len() {
                    ok += 1;
                }
            }
        }
    }
    for v in [0u32, 127, 128, 255, 16_384, 0x7FFF_FFFF, u32::MAX] {
        let c = mc_hotpath::c_encode_varint(v);
        total += 1;
        if c == mc_hotpath::r_encode_varint(v)
            && mc_hotpath::c_decode_varint(&c).map(|(d, _)| d) == Some(v)
        {
            ok += 1;
        }
    }
    let _ = core::fmt::write(
        &mut *vga,
        format_args!("  pkt: varint cross-check {ok}/{total}\n"),
    );
    crate::log!("pkt: varint cross-check {ok}/{total}");

    // 2) Status Response 帧封装 + 解码 round-trip
    let json = format!(
        r#"{{"version":{{"name":"{VERSION_NAME}","protocol":{PROTOCOL_VERSION}}},"players":{{"max":16,"online":1}},"description":{{"text":"mcs-kernel"}}}}"#
    );
    let mut body = WriteBuf::with_capacity(256);
    write_response(&mut body, &json);
    let mut frame = WriteBuf::with_capacity(256);
    frame.write_varint(body.len() as i32);
    frame.write_raw(&body.data);
    let mut r = ReadBuf::new(&frame.data);
    let len = r.read_varint().unwrap_or(-1);
    let pid = r.read_varint().unwrap_or(-1);
    let back = r.read_string().unwrap_or_default();
    let ok2 = len == body.len() as i32 && pid == 0x00 && back == json;
    let _ = core::fmt::write(
        &mut *vga,
        format_args!(
            "  pkt: status frame {}B roundtrip {} ({VERSION_NAME} proto={PROTOCOL_VERSION})\n",
            frame.len(),
            if ok2 { "ok" } else { "FAIL" }
        ),
    );
    crate::log!(
        "pkt: status frame {}B roundtrip {} ({VERSION_NAME} proto={PROTOCOL_VERSION})",
        frame.len(),
        if ok2 { "ok" } else { "FAIL" }
    );

    // 3) 真实区块:生成 → 位打包 → chunk data 帧
    let wg = WorldGenerator::new(seed, WorldType::Normal);
    let chunk = wg.generate(0, 0);
    let n = pack_chunk_data(&chunk, 0, 0);
    let _ = core::fmt::write(
        &mut *vga,
        format_args!("  pkt: chunk-data pack {n} sections ok\n"),
    );
    crate::log!("pkt: chunk-data pack {n} sections ok");
}
