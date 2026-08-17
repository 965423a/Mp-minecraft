//! IDT:256 中断门 + 异常 dump + 每核 APIC 定时器(1ms tick)。

use core::arch::asm;
use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};

/// per-CPU 原子,64B 缓存行对齐(防伪共享)。
#[repr(align(64))]
struct AlU64(AtomicU64);
#[repr(align(64))]
struct AlBool(AtomicBool);

const APIC: u64 = 0xFEE0_0000;
const TMR_LVT: u32 = 0x320;
const TMR_DIV: u32 = 0x3E0;
const TMR_INIT: u32 = 0x380;
const TMR_CUR: u32 = 0x390;
const EOI: u32 = 0xB0;
const TICK_VEC: u32 = 0x20;
const SPURIOUS_VEC: u32 = 0xFF;

#[repr(C)]
pub struct Frame {
    pub rax: u64,
    pub rcx: u64,
    pub rdx: u64,
    pub rbx: u64,
    pub rbp: u64,
    pub rsi: u64,
    pub rdi: u64,
    pub r8: u64,
    pub r9: u64,
    pub r10: u64,
    pub r11: u64,
    pub r12: u64,
    pub r13: u64,
    pub r14: u64,
    pub r15: u64,
    pub err: u64,
    pub vec: u64,
    pub rip: u64,
    pub cs: u64,
    pub rflags: u64,
    pub rsp: u64,
    pub ss: u64,
}

unsafe extern "C" {
    static stub_table: [u64; 256];
}

fn stub_base() -> u64 {
    let base: u64;
    unsafe {
        asm!("lea {0}, [rip + {1}]", out(reg) base, sym stub_table, options(nostack, readonly));
    }
    base
}

static mut IDT: [u64; 512] = [0; 512];
static mut IDT_DESC: [u8; 10] = [0; 10];
static mut TSC_PER_TICK: [u64; 64] = [0; 64];
/// 每核 tick 计数(stats/uptime)。
static TICK_COUNT: [AlU64; 64] = [const { AlU64(AtomicU64::new(0)) }; 64];
static TICK_TOTAL: AtomicU64 = AtomicU64::new(0);
static TICKS: [AlU64; 64] = [const { AlU64(AtomicU64::new(0)) }; 64];
static IDT_READY: AtomicBool = AtomicBool::new(false);

/// x2APIC 已启用(APIC_BASE MSR bit10)。启用时所有 LAPIC 寄存器走 MSR
/// (0x800 + MMIO 偏移/4),ICR 为 64 位 MSR 0x830。
pub static X2APIC: AtomicBool = AtomicBool::new(false);

/// 检测 x2APIC 状态(APIC_BASE MSR bit10)。不主动启用:
/// QEMU TCG 的 x2APIC MSR 支持不完整(EOI 读会 #GP),无法靠 CPUID 可靠识别;
/// 固件/管理程序已启用(如 Hyper-V Gen2)则走 MSR 路径,否则 MMIO 路径,
/// 两者本内核均支持。
pub fn apic_detect() {
    let lo: u32;
    let hi: u32;
    unsafe {
        asm!(
            "rdmsr",
            in("ecx") 0x1Bu32,
            out("eax") lo,
            out("edx") hi,
            options(nomem, nostack)
        );
    }
    let msr = ((hi as u64) << 32) | lo as u64;
    let enabled = msr & 0x400 != 0;
    X2APIC.store(enabled, Ordering::Relaxed);
    crate::log!(
        "apic: x2APIC {}",
        if enabled { "enabled by firmware" } else { "off, using MMIO" }
    );
}

/// LAPIC 寄存器写:MMIO 或 x2APIC MSR(0x800 + off/4)。
/// ICR(0x300)在 x2APIC 下必须用 64 位组合写,由 apic_icr 处理。
pub fn apic_w(off: u32, v: u32) {
    if X2APIC.load(Ordering::Relaxed) && off != 0x300 {
        let idx: u64 = (0x800 + off / 4) as u64;
        unsafe {
            asm!(
                "wrmsr",
                in("ecx") idx as u32,
                in("eax") v,
                in("edx") 0u32,
                options(nomem, nostack)
            );
        }
    } else {
        unsafe {
            ((APIC + off as u64) as *mut u32).write_volatile(v);
        }
    }
}

/// LAPIC 寄存器读:MMIO 或 x2APIC MSR。
pub fn apic_r(off: u32) -> u32 {
    if X2APIC.load(Ordering::Relaxed) {
        let idx: u64 = (0x800 + off / 4) as u64;
        let lo: u32;
        let hi: u32;
        unsafe {
            asm!(
                "rdmsr",
                in("ecx") idx as u32,
                out("eax") lo,
                out("edx") hi,
                options(nomem, nostack)
            );
        }
        lo
    } else {
        unsafe { ((APIC + off as u64) as *const u32).read_volatile() }
    }
}

/// ICR 组合写(64 位目标):x2APIC 一次 wrmsr 0x830;MMIO 分 ICR2/ICR 两步。
pub fn apic_icr(dest: u32, cmd: u32) {
    if X2APIC.load(Ordering::Relaxed) {
        let v: u64 = ((dest as u64) << 32) | cmd as u64;
        unsafe {
            asm!(
                "wrmsr",
                in("ecx") 0x830u32,
                in("eax") v as u32,
                in("edx") (v >> 32) as u32,
                options(nomem, nostack)
            );
        }
    } else {
        unsafe {
            ((APIC + 0x310u64) as *mut u32).write_volatile(dest << 24);
            ((APIC + 0x300u64) as *mut u32).write_volatile(cmd);
        }
    }
}

/// ICR 忙等待(发送完成)。x2APIC 下读 MSR 0x830 的 bit12。
pub fn apic_icr_busy() -> bool {
    if X2APIC.load(Ordering::Relaxed) {
        let lo: u32;
        let hi: u32;
        unsafe {
            asm!(
                "rdmsr",
                in("ecx") 0x830u32,
                out("eax") lo,
                out("edx") hi,
                options(nomem, nostack)
            );
        }
        lo & 0x1000 != 0
    } else {
        unsafe { ((APIC + 0x300u64) as *const u32).read_volatile() & 0x1000 != 0 }
    }
}

pub fn lapic_id() -> u32 {
    apic_r(0x20) >> 24
}

fn rdtsc() -> u64 {
    let lo: u32;
    let hi: u32;
    unsafe {
        asm!("rdtsc", out("eax") lo, out("edx") hi, options(nomem, nostack));
    }
    ((hi as u64) << 32) | lo as u64
}

fn calibrate(idx: usize) -> u64 {
    const INIT: u32 = 0x10_0000; // 16M 计数:0.3GHz 下 ~50ms,精度足够
    unsafe {
        apic_w(TMR_DIV, 0x0B); // divide by 1
        apic_w(TMR_LVT, 0x1_0000); // one-shot, masked
        apic_w(TMR_INIT, INIT);
        let t0 = rdtsc();
        let mut spins = 0u64;
        while apic_r(TMR_CUR) != 0 {
            spins += 1;
            if spins & 0xFFFF == 0 && rdtsc() - t0 > 5_000_000_000 {
                crate::log!("calib: cpu{} CUR stuck cur={:#x}", idx, apic_r(TMR_CUR));
                return 0;
            }
            core::hint::spin_loop();
        }
        let dt = rdtsc() - t0;
        let per = dt / INIT as u64;
        crate::log!("calib: cpu{} dt={} per={}", idx, dt, per);
        TSC_PER_TICK[idx] = per;
        per
    }
}

fn arm(idx: usize, tpu: u64) {
    unsafe {
        let per = calibrate(idx);
        if per == 0 {
            return;
        }
        apic_w(TMR_DIV, 0x0B);
        apic_w(TMR_LVT, TICK_VEC | 0x2_0000); // periodic
        apic_w(TMR_INIT, (tpu * 1000 / per) as u32); // 1ms
    }
}

/// 系统累计 tick(1ms/次)。
pub fn tick_total() -> u64 {
    TICK_TOTAL.load(Ordering::Relaxed)
}

/// 每核 tick 数。
pub fn tick_cpu(cpu: usize) -> u64 {
    TICK_COUNT[cpu].0.load(Ordering::Relaxed)
}

pub fn ticks(cpu: usize) -> u64 {
    TICKS[cpu].0.load(Ordering::Relaxed)
}

fn dump(fr: &Frame) {
    unsafe {
        crate::kerr(0x10, b"unhandled interrupt\0".as_ptr(), fr.vec, fr.err, fr.rip);
    }
    crate::log!(
        "intr: vec={} err={:#x} rax={:#x} rdx={:#x} rip={:#x} cs={:#x} rflags={:#x} rsp={:#x} ss={:#x} lvt={:#x}",
        fr.vec,
        fr.err,
        fr.rax,
        fr.rdx,
        fr.rip,
        fr.cs,
        fr.rflags,
        fr.rsp,
        fr.ss,
        apic_r(TMR_LVT)
    );
    unsafe {
        let base = fr.rsp as *const u64;
        crate::log!(
            "intr: iret-frame: rip={:#x} cs={:#x} rflags={:#x} rsp={:#x} ss={:#x}",
            base.read_volatile(),
            base.add(1).read_volatile(),
            base.add(2).read_volatile(),
            base.add(3).read_volatile(),
            base.add(4).read_volatile()
        );
    }
    unsafe {
        crate::sched::dump_tasks();
    }
    unsafe {
        asm!("cli");
    }
}

pub type IrqHandler = unsafe extern "C" fn(*mut Frame);
static mut HANDLERS: [Option<IrqHandler>; 256] = [const { None }; 256];

/// 注册向量处理函数(非 tick/spurious 中断分发)。
pub fn irq_register(vec: u32, h: IrqHandler) {
    unsafe {
        HANDLERS[vec as usize] = Some(h);
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn int_handler(f: *mut Frame) {
    let fr = unsafe { &*f };
    if fr.vec == TICK_VEC as u64 {
        let id = lapic_id() as usize;
        TICKS[id].0.fetch_add(1, Ordering::Relaxed);
        apic_w(EOI, 0); // 先 EOI:on_tick 可能切栈不再返回
        TICK_COUNT[id].0.fetch_add(1, Ordering::Relaxed);
        TICK_TOTAL.fetch_add(1, Ordering::Relaxed);
        crate::sched::on_tick(f);
        return;
    }
    if fr.vec == SPURIOUS_VEC as u64 {
        apic_w(EOI, 0);
        return;
    }
    if let Some(h) = unsafe { HANDLERS[fr.vec as usize] } {
        unsafe { h(f) };
        apic_w(EOI, 0);
        return;
    }
    dump(fr);
}

/// BSP:填 256 项中断门(0x18 段,0xE 门,P),lidt,配本核 timer,开中断。
pub fn init() {
    unsafe {
        let base = stub_base();
        for i in 0..256 {
            let off = unsafe { core::ptr::read((base as *const u64).add(i)) };
            let lo = (off & 0xFFFF)
                | (0x18u64 << 16)
                | (0xEu64 << 40)
                | (1u64 << 47)
                | ((off >> 16 & 0xFFFF) << 48);
            IDT[i * 2] = lo;
            IDT[i * 2 + 1] = off >> 32;
        }
        let base = core::ptr::addr_of!(IDT) as u64;
        IDT_DESC[0] = (256 * 16 - 1) as u8;
        IDT_DESC[1] = ((256 * 16 - 1) >> 8) as u8;
        IDT_DESC[2..10].copy_from_slice(&base.to_le_bytes());
        asm!("lidt [{}]", in(reg) core::ptr::addr_of!(IDT_DESC) as u64, options(nostack));
        crate::smp::mask_pic_ioapic(); // 必须在 sti 前屏蔽 8259/IOAPIC,否则 PIT 以 vec=8 投递
        let id = lapic_id() as usize;
        arm(id, crate::smp::tsc_per_us());
        asm!("sti");
        IDT_READY.store(true, Ordering::Release);
        crate::log!(
            "idt: 256 gates idt={:#x} stub={:#x} ticks={:#x}, BSP{} timer 1ms",
            core::ptr::addr_of!(IDT) as u64,
            stub_base(),
            core::ptr::addr_of!(TICKS) as u64,
            id
        );
    }
}

/// AP:lidt(复位后 IDTR 无效)+ 本核 timer + 开中断。
/// 等待 BSP 填好 IDT 并 lidt 之后才加载(否则 IDT limit=0,中断即 #GP)。
pub fn local_init() {
    unsafe {
        while !IDT_READY.load(Ordering::Acquire) {
            core::hint::spin_loop();
        }
        let base = core::ptr::addr_of!(IDT) as u64;
        IDT_DESC[2..10].copy_from_slice(&base.to_le_bytes());
        asm!("lidt [{}]", in(reg) core::ptr::addr_of!(IDT_DESC) as u64, options(nostack));
        let id = lapic_id() as usize;
        arm(id, crate::smp::tsc_per_us());
        asm!("sti");
        crate::log!("smp: AP{} timer 1ms", id);
    }
}