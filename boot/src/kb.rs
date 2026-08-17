//! PS/2 键盘:中断驱动 + 扫描码环形缓冲。
//! 含 i8042 存在性自检:无 PS/2 控制器的平台(纯 UEFI/Gen2)静默停用。

use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

pub const IRQ1_VEC: u32 = 0x31;

/// i8042 自检结果(0x64 0xAA 回 0x55 = 存在)。
pub static PS2_OK: AtomicBool = AtomicBool::new(false);

fn outb(port: u16, val: u8) {
    unsafe {
        core::arch::asm!("out dx, al", in("dx") port, in("al") val, options(nomem, nostack));
    }
}

fn inb(port: u16) -> u8 {
    let v: u8;
    unsafe {
        core::arch::asm!("in al, dx", in("dx") port, out("al") v, options(nomem, nostack));
    }
    v
}

/// 轮询读 0x64 状态直至输出缓冲就绪(带上限防无控制器挂死)。
fn wait_out(limit: u32) -> bool {
    for _ in 0..limit {
        let st = inb(0x64);
        if st & 0x01 != 0 {
            return true;
        }
        if st == 0xFF {
            return false;
        }
        core::hint::spin_loop();
    }
    false
}

/// 0x64 自检:写 0xAA 后 0x60 应回 0x55。失败(回 0xFF 或超时)视为无 i8042。
fn probe() -> bool {
    outb(0x64, 0xAA);
    if !wait_out(50_000) {
        return false;
    }
    inb(0x60) == 0x55
}

static BUF: [AtomicUsize; 256] = [const { AtomicUsize::new(0) }; 256];
static HEAD: AtomicUsize = AtomicUsize::new(0);
static TAIL: AtomicUsize = AtomicUsize::new(0);

/// 键盘 IRQ 处理:数据就绪则读 0x60 入缓冲。
#[unsafe(no_mangle)]
pub extern "C" fn kb_irq(_f: *mut crate::idt::Frame) {
    if !PS2_OK.load(Ordering::Relaxed) {
        return;
    }
    let status: u8;
    unsafe {
        core::arch::asm!(
            "in al, dx",
            in("dx") 0x64u16,
            out("al") status,
            options(nomem, nostack)
        );
    }
    if status & 0x01 != 0 {
        let sc: u8;
        unsafe {
            core::arch::asm!(
                "in al, dx",
                in("dx") 0x60u16,
                out("al") sc,
                options(nomem, nostack)
            );
        }
        let h = HEAD.load(Ordering::Relaxed);
        let n = (h + 1) % BUF.len();
        if n != TAIL.load(Ordering::Relaxed) {
            BUF[h].store(sc as usize, Ordering::Relaxed);
            HEAD.store(n, Ordering::Relaxed);
        }
    }
}

pub fn pop() -> Option<u8> {
    let t = TAIL.load(Ordering::Relaxed);
    if HEAD.load(Ordering::Relaxed) == t {
        return None;
    }
    let sc = BUF[t].load(Ordering::Relaxed) as u8;
    TAIL.store((t + 1) % BUF.len(), Ordering::Relaxed);
    Some(sc)
}

/// 初始化:检测 i8042,存在则注册 0x31 向量 + IOAPIC 路由 IRQ1 -> BSP。
pub fn init() -> bool {
    let ok = probe();
    PS2_OK.store(ok, Ordering::Relaxed);
    if !ok {
        crate::log!("kb: no i8042 (PS/2 keyboard absent), serial input only");
        return false;
    }
    // 自测后键盘复位:启用扫描(发送 0xF4,读 ACK 0xFA 前先清输出缓冲)
    let (gsiv, _flags) = crate::acpi::madt_irq_override(1);
    crate::idt::irq_register(IRQ1_VEC, kb_irq);
    unsafe {
        crate::smp::ioapic_route(gsiv, IRQ1_VEC, crate::idt::lapic_id()); // edge
    }
    crate::log!("kb: PS/2 present, IRQ1 -> vec {IRQ1_VEC:#x} gsiv={gsiv}");
    true
}