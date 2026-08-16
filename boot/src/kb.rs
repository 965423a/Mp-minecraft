//! PS/2 键盘:中断驱动 + 扫描码环形缓冲。

use core::sync::atomic::{AtomicUsize, Ordering};

pub const IRQ1_VEC: u32 = 0x31;

static BUF: [AtomicUsize; 256] = [const { AtomicUsize::new(0) }; 256];
static HEAD: AtomicUsize = AtomicUsize::new(0);
static TAIL: AtomicUsize = AtomicUsize::new(0);

/// 键盘 IRQ 处理:数据就绪则读 0x60 入缓冲。
#[unsafe(no_mangle)]
pub extern "C" fn kb_irq(_f: *mut crate::idt::Frame) {
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

/// 初始化:注册 0x31 向量 + IOAPIC 路由 IRQ1 -> BSP。
pub fn init() {
    let (gsiv, _flags) = crate::acpi::madt_irq_override(1);
    crate::idt::irq_register(IRQ1_VEC, kb_irq);
    unsafe {
        crate::smp::ioapic_route(gsiv, IRQ1_VEC, crate::idt::lapic_id()); // edge
    }
    crate::log!("kb: IRQ1 -> vec {IRQ1_VEC:#x} gsiv={gsiv}");
}