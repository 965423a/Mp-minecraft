//! SMP:通过 LAPIC ICR(INIT+SIPI)经低内存 trampoline(0x7000)唤醒 AP。

use core::arch::asm;
use core::sync::atomic::{AtomicU32, Ordering};

const TRAMP_DST: u64 = 0x7000;
const ICR: u64 = 0x300;
const AP_STACK_SIZE: usize = 16384;
// tramp.S 内偏移(tramp_start 起,编译后 xxd 校准):
const PARAM_OFF: u64 = 0x100; // 参数区起点(与 tramp.S .set PARAM 一致)
const PMODE_OFF: u64 = 0x2B; // pmode_entry(编译后校准)
const LONG_OFF: u64 = 0x6A; // long_entry(编译后校准)
// 参数区字段偏移(见 tramp.S 注释):
const P_GDT_BASE: u64 = 0x02; // base u64
const P_CR3: u64 = 0x0C; // u64
const P_STACK: u64 = 0x14; // u64
const P_READY: u64 = 0x1C; // u64
const P_APID: u64 = 0x24; // u32
const P_PMODE_IP: u64 = 0x28; // u16, +2 = CS
const P_LONG_IP: u64 = 0x2C; // u32(经 32 位 lret 进入)
const P_AP_ENTRY: u64 = 0x34; // u64

unsafe extern "C" {
    static tramp_start: u8;
    static tramp_end: u8;
    static gdt_desc: u8;
    fn enable_sse_ap();
}

static mut AP_STACKS: [[u8; AP_STACK_SIZE]; 64] = [[0; AP_STACK_SIZE]; 64];
static mut AP_READY: [u32; 64] = [0; 64];
static mut AP_COUNT: usize = 1;
static mut TSC_PER_US: u64 = 2500;
/// AP 空闲循环计数(每 ~1ms 递增),验证 AP 确实在跑内核代码。
static AP_TICKS: [AtomicU32; 64] = [const { AtomicU32::new(0) }; 64];

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
pub fn tsc_per_us() -> u64 {
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
    let end = rdtsc() + tpu * us;
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

/// 屏蔽 8259 + IOAPIC 全部中断源,必须在任何 sti 之前调用。
/// 否则 QEMU 的 PIT(8259 IRQ0 / IOAPIC pin 2)会以 INT=0x08(8259 默认向量)投递,
/// 与 #DF 向量 8 混淆,并被 IDT[8] 当作异常 dump。
pub unsafe fn mask_pic_ioapic() {
    init_8259();
    mask_ioapic();
}

/// 完整初始化 8259:主片 IRQ0-7 → 0x30-0x37,从片 IRQ8-15 → 0x38-0x3F,
/// 然后全屏蔽。初始化同时清掉 GRUB 遗留的 pending ISR。
unsafe fn init_8259() {
    outb(0x20, 0x11); // ICW1: edge, cascade, ICW4
    outb(0x21, 0x30); // ICW2: 主片基址 0x30
    outb(0x21, 0x04); // ICW3: 从片挂 IRQ2
    outb(0x21, 0x01); // ICW4: 8086 模式
    outb(0x21, 0xFF); // OCW1: 全屏蔽
    outb(0xA0, 0x11);
    outb(0xA1, 0x38); // ICW2: 从片基址 0x38
    outb(0xA1, 0x02); // ICW3: slave id 2
    outb(0xA1, 0x01);
    outb(0xA1, 0xFF);
}

/// 屏蔽全部 IOAPIC 的 redirection 项(mask bit 16,vector 0xFF 防残余投递),
/// 覆盖双路 X99 的多个 IOAPIC;停掉 PIT 等经 IOAPIC 的中断。
unsafe fn mask_ioapic() {
    for (addr, gsiv, pins) in crate::acpi::madt_ioapics() {
        if addr == 0 {
            continue;
        }
        crate::log!("smp: mask ioapic @ {addr:#x} gsiv={gsiv} pins={pins}");
        mask_ioapic_one(addr, pins);
    }
}

unsafe fn mask_ioapic_one(addr: u64, pins: u32) {
    let sel = addr as *mut u32;
    let win = (addr + 0x10) as *mut u32;
    for i in 0..pins {
        sel.write_volatile(0x10 + 2 * i); // IOREDTBL 低半
        win.write_volatile(0x0001_00FF); // mask + vector 0xFF
        sel.write_volatile(0x11 + 2 * i); // IOREDTBL 高半
        win.write_volatile(0); // dest = 0
    }
}

/// 路由单个 GSIV 到指定 LAPIC(phys 模式):清 mask,边沿触发。
pub unsafe fn ioapic_route(gsiv: u32, vector: u32, lapic: u32) {
    ioapic_route_trig(gsiv, vector, lapic, 0);
}

/// 路由单个 GSIV,trig 为 IOREDTBL 触发位(0=边沿, 0x8000=电平)。
pub unsafe fn ioapic_route_trig(gsiv: u32, vector: u32, lapic: u32, trig: u32) {
    for (addr, base, pins) in crate::acpi::madt_ioapics() {
        if addr == 0 {
            continue;
        }
        if gsiv >= base && gsiv < base + pins {
            let idx = (gsiv - base) as u32;
            let sel = addr as *mut u32;
            let win = (addr + 0x10) as *mut u32;
            sel.write_volatile(0x10 + 2 * idx);
            win.write_volatile(vector | trig); // Fixed, phys, mask 清
            sel.write_volatile(0x11 + 2 * idx);
            win.write_volatile((lapic & 0xFF) << 24); // 高半:dest LAPIC
            let lo = {
                sel.write_volatile(0x10 + 2 * idx);
                win.read_volatile()
            };
            let hi = {
                sel.write_volatile(0x11 + 2 * idx);
                win.read_volatile()
            };
            crate::log!("smp: ioapic route gsiv={gsiv} -> vec={vector:#x} lapic={lapic:#x} @ {addr:#x}+{idx} ioredtbl={hi:#010x}{lo:#010x}");
            return;
        }
    }
    crate::log!("smp: ioapic route gsiv={gsiv}: no ioapic covers it");
}

/// 给 0x7000 的 trampoline 参数区(偏移 PARAM_OFF 起)打补丁。
/// 全部地址字段按 u64 写入(栈/READY/CR3/GDT base/AP 入口),与 tramp.S 布局对应。
unsafe fn patch_tramp(gdt_limit: u32, gdt_base: u64, cr3v: u64, stack: u64, ready: u64, apid: u32) {
    let p = TRAMP_DST as *mut u8;
    (p.add(PARAM_OFF as usize + 0x00) as *mut u16).write_volatile(gdt_limit as u16); // GDT limit
    (p.add(PARAM_OFF as usize + P_GDT_BASE as usize) as *mut u64).write_volatile(gdt_base);
    (p.add(PARAM_OFF as usize + P_CR3 as usize) as *mut u64).write_volatile(cr3v);
    (p.add(PARAM_OFF as usize + P_STACK as usize) as *mut u64).write_volatile(stack);
    (p.add(PARAM_OFF as usize + P_READY as usize) as *mut u64).write_volatile(ready);
    (p.add(PARAM_OFF as usize + P_APID as usize) as *mut u32).write_volatile(apid);
}

/// AP 内核入口:trampoline 在 READY 写完后直接跳到这里。
/// 屏蔽本核 LVT0/LVT1,进入空闲循环(计数递增),证明 AP 真正运行内核代码。
#[unsafe(no_mangle)]
/// 并发压力测试:AP 周期性执行 分配 → 写模式 → 校验 → 释放,
/// 与 BSP/其他 AP 竞争帧链,校验自旋锁与链的完整性。
/// rounds/fails 供 BSP 侧(monitor/gdb)检查;失败置 FLAKY 标志并停机。
pub static AP_STRESS: [core::sync::atomic::AtomicU64; 64] = [const { core::sync::atomic::AtomicU64::new(0) }; 64];
pub static AP_STRESS_FAILS: [core::sync::atomic::AtomicU64; 64] = [const { core::sync::atomic::AtomicU64::new(0) }; 64];

pub extern "C" fn ap_entry(ap_id: u32) -> ! {
    let apic = 0xFEE0_0000u64;
    // 开启 SSE(OSFXSR | OSXMMEXCPT),与 BSP 一致;AP 上允许浮点代码(世界生成)
    unsafe {
        enable_sse_ap();
    }
    unsafe {
        (apic as *mut u32).add(0xF0 / 4).write_volatile(0x1FF); // SVR 使能 LAPIC
        (apic as *mut u32).add(0x350 / 4).write_volatile(0x100FF); // LVT0 屏蔽(vector 0xFF)
        (apic as *mut u32).add(0x360 / 4).write_volatile(0x100FF); // LVT1 屏蔽(vector 0xFF)
    }
    crate::idt::local_init();
    crate::sched::register_idle();
    crate::log!("smp: AP{ap_id} entering idle loop");
    let tpu = unsafe { TSC_PER_US };
    let lapic_id = unsafe { (apic as *mut u32).add(0x20 / 4).read_volatile() >> 24 };
    let node = crate::numa::node_for_lapic(lapic_id).unwrap_or(0);
    loop {
        usleep(1_000, tpu);
        AP_TICKS[ap_id as usize].fetch_add(1, Ordering::Relaxed);
        if AP_TICKS[ap_id as usize].load(Ordering::Relaxed) % 512 == 0 {
            // 压力测试:每轮 64 次 分配+写+校验+释放,优先本节点
            let mut fails = 0u64;
            let mut rounds = 0u64;
            unsafe {
                for _ in 0..64 {
                    if let Some(p) = crate::numa::alloc_local(node) {
                        let w = p as *mut u64;
                        for i in 0..512 {
                            w.add(i).write_volatile(i as u64 ^ 0x3C3C_3C3C_3C3C_3C3C);
                        }
                        for i in 0..512 {
                            if w.add(i).read_volatile() != i as u64 ^ 0x3C3C_3C3C_3C3C_3C3C {
                                fails += 1;
                            }
                        }
                        crate::numa::free(p);
                        rounds += 1;
                    }
                }
            }
            let f = AP_STRESS_FAILS[ap_id as usize].fetch_add(fails, Ordering::Relaxed) + fails;
            AP_STRESS[ap_id as usize].fetch_add(rounds, Ordering::Relaxed);
            if f > 0 {
                crate::log!("smp: AP{ap_id} stress FAILED ({f})");
            }
        }
    }
}

unsafe fn wake_ap(apic_base: u64, id: u32, idx: usize, tpu: u64) -> bool {
    crate::log!(
        "smp: icr0={:#x} icr2={:#x}",
        apic_read(apic_base, 0x300),
        apic_read(apic_base, 0x310)
    );
    // 先写 ICR2(dest field),再写 ICR(低 32)触发投递
    apic_write(apic_base, 0x310, id << 24);
    // INIT assert(level+assert)+ deassert
    apic_write(apic_base, ICR, 0x0000C500);
    wait_icr_ready(apic_base);
    usleep(10_000, tpu);
    apic_write(apic_base, 0x310, id << 24);
    apic_write(apic_base, ICR, 0x00008500);
    wait_icr_ready(apic_base);
    usleep(10_000, tpu); // 等 AP 完成 INIT 重置再发 SIPI,防竞态丢 SIPI
    // SIPI(vector 7 -> 0x7000),两次。delivery mode 必须 = 0b110(Start-Up),
    // 0x607 的 mode=0b000(Fixed)无效,AP 不会启动
    apic_write(apic_base, 0x310, id << 24);
    apic_write(apic_base, ICR, 0x00000607);
    wait_icr_ready(apic_base);
    usleep(1_000, tpu);
    apic_write(apic_base, 0x310, id << 24);
    apic_write(apic_base, ICR, 0x00000607);
    wait_icr_ready(apic_base);
    // 轮询就绪,最多 ~5s
    for _ in 0..50_000 {
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
        crate::log!(
            "smp: bsp={bsp:#x}, madt ids={:#x},{:#x},{:#x},{:#x} n={}",
            ids[0],
            ids[1],
            ids[2],
            ids[3],
            n
        );
        let tpu = tsc_per_us();
        TSC_PER_US = tpu;
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
        let p = TRAMP_DST as *mut u8;
        (p.add(PARAM_OFF as usize + P_PMODE_IP as usize) as *mut u16).write_volatile(0x7000 + PMODE_OFF as u16);
        (p.add(PARAM_OFF as usize + P_PMODE_IP as usize + 2) as *mut u16).write_volatile(0x08);
        (p.add(PARAM_OFF as usize + P_LONG_IP as usize) as *mut u32).write_volatile(0x7000 + LONG_OFF as u32);
        (p.add(PARAM_OFF as usize + P_AP_ENTRY as usize) as *mut u64).write_volatile(ap_entry as usize as u64);
        let gdp = &gdt_desc as *const u8;
        let gdt_limit = (gdp as *const u16).read_volatile() as u32;
        let gdt_base = (gdp.add(2) as *const u32).read_volatile() as u64
            | ((gdp.add(6) as *const u16).read_volatile() as u64) << 32;
        let cr3v = cr3_value();
        // 屏蔽 8259 全部 IRQ(PIT 等),否则 QEMU 会把 IRQ0 以 INT=0x08 持续投递,
        // 干扰 wait-for-SIPI 状态的 AP(错过 SIPI);此处重复执行(无副作用)
        mask_pic_ioapic();
        crate::log!(
            "smp: imr0={:#x} imr1={:#x} lvt0={:#x} lvt1={:#x}",
            inb(0x21),
            inb(0xA1),
            apic_read(apic_base, 0x350),
            apic_read(apic_base, 0x360)
        );
        outb(0x21, 0xFF);
        outb(0xA1, 0xFF);
        mask_ioapic(); // QEMU 的 PIT(IRQ0)经 IOAPIC 以 INT=0x08 投递,8259 mask 挡不住
        // 启用本核 LAPIC(SVR),屏蔽 LVT0/LVT1,避免 QEMU 把 PIT 中断以 NMI 形式注入
        (apic_base as *mut u32).add(0xF0 / 4).write_volatile(0x1FF);
        (apic_base as *mut u32).add(0x350 / 4).write_volatile(0x10000);
        (apic_base as *mut u32).add(0x360 / 4).write_volatile(0x10000);
        let mut online = 1usize;
        let mut ap = 0usize;
        // AP 冷启动(进入 wait-for-SIPI)比 BSP 慢,过早发 INIT/SIPI 会丢,先等它就绪
        usleep(500_000, tpu);
        for i in 0..n {
            let id = ids[i];
            if id == bsp {
                continue;
            }
            let stack = match crate::numa::alloc_contig(
                crate::numa::node_for_lapic(id).unwrap_or(0),
                AP_STACK_SIZE / 4096,
            ) {
                Some(p) => {
                    crate::log!("smp: AP{} stack from node-local memory", ap);
                    p + AP_STACK_SIZE as u64
                }
                None => {
                    crate::log!("smp: AP{} stack from static fallback", ap);
                    (&AP_STACKS[ap] as *const u8 as u64) + AP_STACKS[ap].len() as u64
                }
            };
            let ready = &AP_READY[ap] as *const u32 as u64;
            AP_READY[ap] = 0;
            patch_tramp(gdt_limit, gdt_base, cr3v, stack, ready, ap as u32);
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
