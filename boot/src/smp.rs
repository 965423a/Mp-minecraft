//! SMP:BSP 通过 LAPIC ICR(INIT + SIPI)唤醒 AP,
//! AP 经低内存 trampoline(0x7000)进入 64 位并标记就绪。

use core::arch::asm;

const TRAMP_DST: u64 = 0x7000;
const ICR: u64 = 0x300;
const AP_STACK_SIZE: usize = 16384;
// tramp.S 内偏移(tramp_start 起,hexdump 校准):
const PMODE_OFF: u64 = 0x3F; // pmode_entry
const LONG_OFF: u64 = 0x68; // long_entry

unsafe extern "C" {
    static tramp_start: u8;
    static tramp_end: u8;
    static gdt_desc: u8;
}

static mut AP_STACKS: [[u8; AP_STACK_SIZE]; 64] = [[0; AP_STACK_SIZE]; 64];
static mut AP_READY: [u32; 64] = [0; 64];
static mut AP_COUNT: usize = 1;

fn outb(port: u16, val: u8) {
    unsafe {
        asm!("out dx, al", in("dx") port, in("al") val, options(nomem, nostack));
    }
}

fn inb(port: u16) -> u8 {
    let v: u8;
    unsafe {
        asm!("in al, dx", in("dx") port, out("al") v, options(nomem, nostack));
    }
    v
}

fn rdtsc() -> u64 {
    let lo: u32;
    let hi: u32;
    unsafe {
        asm!("rdtsc", out("eax") lo, out("edx") hi, options(nomem, nostack));
    }
    ((hi as u64) << 32) | lo as u64
}

fn cr3_value() -> u64 {
    let v: u64;
    unsafe {
        asm!("mov rax, cr3", out("rax") v, options(nomem, nostack));
    }
    v
}

/// 用 PIT 通道 0 校准 TSC 频率,返回 ticks/us。
fn tsc_per_us() -> u64 {
    unsafe {
        // mode 2,初值 65536(~54.9ms 满周期)
        outb(0x43, 0x34);
        outb(0x40, 0x00);
        outb(0x40, 0x00);
        let t0 = rdtsc();
        let mut cnt = 0u32;
        for _ in 0..200_000 {
            outb(0x43, 0x00); // latch
            let lo = inb(0x40);
            let hi = inb(0x40);
            cnt = ((hi as u32) << 8) | lo as u32;
            if cnt < 32768 {
                break;
            }
        }
        let t1 = rdtsc();
        let half_us = 27_500u64; // 半计数窗口 ≈ 27.5ms
        let per_us = (t1 - t0) / half_us;
        if per_us < 100 || per_us > 100_000 {
            2500
        } else {
            per_us
        }
    }
}

fn usleep(us: u64, tpu: u64) {
    let t = rdtsc();
    let end = t + tpu * us;
    while rdtsc() < end {
        core::hint::spin_loop();
    }
}

unsafe fn apic_write(apic_base: u64, off: u64, v: u32) {
    ((apic_base + off) as *mut u32).write_volatile(v);
}

unsafe fn apic_read(apic_base: u64, off: u64) -> u32 {
    ((apic_base + off) as *const u32).read_volatile()
}

fn wait_icr_ready(apic_base: u64) {
    for _ in 0..10_000 {
        if unsafe { apic_read(apic_base, ICR) } & 0x1000 == 0 {
            return;
        }
        core::hint::spin_loop();
    }
}

/// 给 0x7000 的 trampoline 参数区打补丁。
unsafe fn patch_tramp(gdt: u64, cr3v: u64, stack: u64, ready: u64, apid: u32) {
    let p = TRAMP_DST as *mut u8;
    (p.add(0x00) as *mut u32).write_volatile((23u32 << 16) | (gdt & 0xFFFF) as u32);
    (p.add(0x04) as *mut u32).write_volatile((gdt >> 16) as u32);
    (p.add(0x08) as *mut u32).write_volatile(cr3v as u32);
    (p.add(0x0C) as *mut u32).write_volatile(stack as u32);
    (p.add(0x10) as *mut u32).write_volatile(ready as u32);
    (p.add(0x14) as *mut u32).write_volatile(apid);
}

unsafe fn wake_ap(apic_base: u64, id: u32, idx: usize, tpu: u64) -> bool {
    // INIT(level 触发)
    apic_write(apic_base, ICR, (id << 24) | 0x000C0500);
    wait_icr_ready(apic_base);
    usleep(10_000, tpu);
    // SIPI(vector 7 -> 0x7000),两次
    apic_write(apic_base, ICR, (id << 24) | 0x00060407);
    wait_icr_ready(apic_base);
    usleep(200, tpu);
    apic_write(apic_base, ICR, (id << 24) | 0x00060407);
    wait_icr_ready(apic_base);
    // 轮询就绪,最多 ~100ms
    for _ in 0..1000 {
        if AP_READY[idx] != 0 {
            return true;
        }
        usleep(100, tpu);
    }
    false
}

/// 唤醒全部 AP。返回在线 CPU 数(含 BSP)。
pub fn init() -> usize {
    unsafe {
        let Some((apic_base, ids, n)) = crate::acpi::madt_parse() else {
            crate::log!("smp: no MADT, 1 cpu");
            return 1;
        };
        let bsp = crate::acpi::lapic_id();
        let tpu = tsc_per_us();
        // 复制 trampoline 到低内存
        let sz = &tramp_end as *const u8 as usize - &tramp_start as *const u8 as usize;
        if sz > 0x800 {
            crate::log!("smp: trampoline too large ({sz} bytes), 1 cpu");
            return 1;
        }
        let src = &tramp_start as *const u8;
        for i in 0..sz {
            *((TRAMP_DST + i as u64) as *mut u8) = *src.add(i);
        }
        // 打补丁:跳转目标 = 0x7000 + tramp 内偏移
        let pmode_off = PMODE_OFF;
        let long_off = LONG_OFF;
        *((TRAMP_DST + 0x18) as *mut u16) = (0x7000 + pmode_off) as u16;
        *((TRAMP_DST + 0x1A) as *mut u16) = 0x08;
        *((TRAMP_DST + 0x1C) as *mut u32) = (0x7000 + long_off) as u32;
        let gdt = &gdt_desc as *const u8 as u64;
        let cr3v = cr3_value();
        let mut online = 1usize;
        let mut ap = 0usize;
        for i in 0..n {
            let id = ids[i];
            if id == bsp {
                continue;
            }
            let stack =
                (&AP_STACKS[ap] as *const u8 as u64) + AP_STACKS[ap].len() as u64;
            let ready = &AP_READY[ap] as *const u32 as u64;
            AP_READY[ap] = 0;
            patch_tramp(gdt, cr3v, stack, ready, ap as u32);
            crate::log!("smp: waking lapic {:#x}, stack {:#x}", id, stack);
            if wake_ap(apic_base, id, ap, tpu) {
                let node = crate::numa::node_for_lapic(id).unwrap_or(0);
                crate::log!(
                    "smp: AP{} lapic {:#x} online, node {}",
                    ap,
                    id,
                    node
                );
                online += 1;
                ap += 1;
            } else {
                crate::log!("smp: AP{} lapic {:#x} failed to start", ap, id);
            }
        }
        AP_COUNT = online;
        crate::log!("smp: total {} cpus online", online);
        online
    }
}

pub fn cpu_count() -> usize {
    unsafe { AP_COUNT }
}
