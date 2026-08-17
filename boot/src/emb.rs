//! 服务器逻辑嵌入演示:mc-world 世界生成作为内核任务,多核并行。

use core::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

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
    loop {
        core::hint::spin_loop();
    }
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