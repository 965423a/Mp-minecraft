//! ACPI 解析:RSDP 扫描 + RSDT/XSDT 遍历,提取 SRAT/SLIT/MADT。

#![allow(dead_code)]

use crate::log;

pub const SRAT_SIG: &[u8; 4] = b"SRAT";
pub const SLIT_SIG: &[u8; 4] = b"SLIT";
pub const MADT_SIG: &[u8; 4] = b"APIC";
pub const FADT_SIG: &[u8; 4] = b"FACP";

/// FADT 的 PM timer 端口(供 PIT 缺失时校准 TSC)。
/// 优先 X_PM_TMR_BLK GAS(FADT offset 206,ACPI 2.0+),否则 legacy PM_TMR_BLK(offset 74)。
pub fn fadt_pm_tmr() -> Option<u32> {
    unsafe {
        let t = find_table(FADT_SIG)?;
        let len = core::ptr::read_volatile((t as *const u8).add(4)) as usize;
        if len >= 218 {
            let space = core::ptr::read_volatile((t as *const u8).add(206));
            let addr = core::ptr::read_volatile((t as *const u8).add(210) as *const u64);
            if space == 1 && addr != 0 && addr < 0x10000 {
                return Some(addr as u32);
            }
        }
        let port = core::ptr::read_volatile((t as *const u8).add(74) as *const u32);
        if port != 0 && port < 0x10000 {
            Some(port)
        } else {
            None
        }
    }
}

/// FADT 的 RESET 寄存器 GAS + reset value(供 reboot 链第一环)。
/// GAS @ offset 114(space_id, bit_width, bit_offset, access, addr u64),
/// reset value @ offset 126。
pub fn fadt_reset() -> Option<(u16, u8)> {
    unsafe {
        let t = find_table(FADT_SIG)?;
        let len = core::ptr::read_volatile((t as *const u8).add(4)) as usize;
        if len < 128 {
            return None;
        }
        let space = core::ptr::read_volatile((t as *const u8).add(114));
        let addr = core::ptr::read_volatile((t as *const u8).add(118) as *const u64);
        let val = core::ptr::read_volatile((t as *const u8).add(126));
        if addr != 0 && addr < 0x10000 {
            Some((addr as u16, val))
        } else {
            None
        }
    }
}

/// multiboot2 info 指针(kernel_main 注入,RSDP 的 EFI 来源用)。
static mut MB2_INFO: *const u8 = core::ptr::null();

pub fn set_mb2(info: *const u8) {
    unsafe {
        MB2_INFO = info;
    }
}

fn mb2_info() -> *const u8 {
    unsafe { core::ptr::addr_of!(MB2_INFO).read() }
}

#[repr(C)]
struct Rsdp {
    sig: [u8; 8],
    checksum: u8,
    oem_id: [u8; 6],
    revision: u8,
    rsdt_addr: u32,
    // 2.0+
    length: u32,
    xsdt_addr: u64,
    ext_checksum: u8,
    reserved: [u8; 3],
}

#[repr(C)]
struct AcpiHeader {
    sig: [u8; 4],
    length: u32,
    revision: u8,
    checksum: u8,
    oem_id: [u8; 6],
    oem_table: [u8; 8],
    oem_rev: u32,
    creator_id: u32,
    creator_rev: u32,
}

fn checksum(data: &[u8]) -> bool {
    let mut sum = 0u8;
    for b in data {
        sum = sum.wrapping_add(*b);
    }
    sum == 0
}

/// 扫描 [0xE0000, 0xFFFFF) 与 EBDA 找 RSDP。返回 xdt 地址与是否为 XSDT。
/// 顺序:① EFI system table 的 ACPI 2.0 config 项(纯 UEFI 平台如 Hyper-V Gen2)
///      ② 传统内存扫描(BIOS/CSM 平台)。
fn rsdp_find() -> Option<(u64, bool)> {
    unsafe {
        if let Some(xsdt) = rsdp_efi() {
            return Some((xsdt, true));
        }
        // EBDA 段地址在 0x40:0x0E(BIOS 数据区)
        let ebda_seg = core::ptr::read_volatile(0x40Eu16 as *const u16);
        let ebda = (ebda_seg as u64) << 4;
        let regions: [(u64, u64); 2] = [(ebda, 0x1000), (0xE0000, 0x20000)];
        for (base, size) in regions {
            let mut addr = base;
            let limit = (base + size).min(0x100000);
            while addr < limit {
                let p = addr as *const u8;
                let sig = core::ptr::read_volatile(p);
                if sig == b'R' {
                    let s = core::slice::from_raw_parts(p, 8);
                    if &s[..] == b"RSD PTR " {
                        let rsdp = &*(p as *const Rsdp);
                        if checksum(core::slice::from_raw_parts(p, 20)) {
                            if rsdp.revision >= 2
                                && rsdp.length >= 36
                                && checksum(core::slice::from_raw_parts(p, rsdp.length as usize))
                            {
                                return Some((rsdp.xsdt_addr, true));
                            }
                            return Some((rsdp.rsdt_addr as u64, false));
                        }
                    }
                }
                addr += 16;
            }
        }
    }
    None
}

/// multiboot2 tag type 9:EFI System Table(64 位)。返回其地址。
/// 从 EFI config table 找 ACPI 2.0 GUID(0x8868E871-...),返回 XSDT 地址。
/// 所有读均带范围检查,地址非法(>16GiB 映射区)直接放弃。
fn rsdp_efi() -> Option<u64> {
    unsafe {
        let info = mb2_info();
        if info.is_null() {
            return None;
        }
        let total = core::ptr::read_volatile(info as *const u32);
        if total < 16 || total > 0x100000 {
            return None;
        }
        let mut off = 16u64; // 跳过 total_size + reserved
        while off + 8 <= total as u64 {
            let tag = (info as u64 + off) as *const u8;
            let ttype = core::ptr::read_volatile(tag as *const u32);
            let tsize = core::ptr::read_volatile((tag as *const u32).add(1));
            if tsize < 8 || tsize as u64 > total as u64 - off {
                return None;
            }
            if ttype == 0 {
                return None; // 结束 tag
            }
            if ttype == 9 {
                let efi_st = core::ptr::read_volatile((tag.add(8)) as *const u64);
                if efi_st == 0 || efi_st >= 0x4_0000_0000 {
                    return None;
                }
                if core::ptr::read_volatile(efi_st as *const u64) != 0x5453595320494249 {
                    return None; // 签名 "IBI SYST" 不对
                }
                let n = core::ptr::read_volatile((efi_st + 80) as *const u64);
                let ct = core::ptr::read_volatile((efi_st + 88) as *const u64);
                if n == 0 || n > 64 || ct == 0 || ct >= 0x4_0000_0000 {
                    return None;
                }
                // EFI_ACPI_20_TABLE_GUID {8868E871-E4F1-4D09-93DA-1C8D3E0E8F4A}
                let guid: [u8; 16] = [
                    0x71, 0xE8, 0x68, 0x88, 0xF1, 0xE4, 0x09, 0x4D, 0x93, 0xDA, 0x1C, 0x8D,
                    0x3E, 0x0E, 0x8F, 0x4A,
                ];
                for i in 0..n {
                    let e = ct + i * 24;
                    if e >= 0x4_0000_0000 {
                        return None;
                    }
                    let g = core::slice::from_raw_parts(e as *const u8, 16);
                    if &g[..] == &guid[..] {
                        let xsdt = core::ptr::read_volatile((e + 16) as *const u64);
                        if xsdt != 0 && xsdt < 0x4_0000_0000 {
                            return Some(xsdt);
                        }
                    }
                }
            }
            off += tsize as u64;
        }
        None
    }
}

/// 在 RSDT/XSDT 中找签名表。返回表指针。
fn find_table(sig: &[u8; 4]) -> Option<*const AcpiHeader> {
    unsafe {
        let (xdt, is_xsdt) = rsdp_find()?;
        if xdt == 0 {
            return None;
        }
        let hdr = &*(xdt as *const AcpiHeader);
        if &hdr.sig != (if is_xsdt { b"XSDT" } else { b"RSDT" }) {
            return None;
        }
        let entries = (hdr.length as usize - 36) / if is_xsdt { 8 } else { 4 };
        for i in 0..entries {
            let entry = xdt + 36 + (i as u64) * if is_xsdt { 8 } else { 4 };
            let addr = if is_xsdt {
                core::ptr::read_volatile(entry as *const u64)
            } else {
                core::ptr::read_volatile(entry as *const u32) as u64
            };
            if addr == 0 {
                continue;
            }
            let thdr = &*(addr as *const AcpiHeader);
            if &thdr.sig == sig
                && checksum(core::slice::from_raw_parts(addr as *const u8, thdr.length as usize))
            {
                return Some(thdr);
            }
        }
    }
    None
}

const MAX_SRAT: usize = 32;

/// SRAT 亲和条目(手工偏移解析,不直接 cast)。
pub struct SratInfo {
    pub mem_aff: [(u32, u64, u64); MAX_SRAT],
    pub mem_cnt: usize,
    pub proc_aff: [(u32, u32); MAX_SRAT],
    pub proc_cnt: usize,
}

fn u32le(p: *const u8) -> u32 {
    unsafe {
        p.read_volatile() as u32
            | ((p.add(1).read_volatile() as u32) << 8)
            | ((p.add(2).read_volatile() as u32) << 16)
            | ((p.add(3).read_volatile() as u32) << 24)
    }
}

fn u64le(p: *const u8) -> u64 {
    unsafe {
        u32le(p) as u64 | ((u32le(p.add(4)) as u64) << 32)
    }
}

/// 解析 SRAT:返回内存亲和(proximity, base, len)与处理器亲和(proximity, lapic_id)。
pub fn srat_parse() -> Option<SratInfo> {
    unsafe {
        let thdr = find_table(SRAT_SIG)?;
        let tbase = thdr as u64;
        let len = (*thdr).length as u64;
        let mut p = 48u64; // 头 36 + reserved 4 + 对齐后条目起点
        let mut info = SratInfo {
            mem_aff: [(0, 0, 0); MAX_SRAT],
            mem_cnt: 0,
            proc_aff: [(0, 0); MAX_SRAT],
            proc_cnt: 0,
        };
        while p + 2 <= len {
            let e = (tbase + p) as *const u8;
            let typ = e.read_volatile();
            let elen = e.add(1).read_volatile() as u64;
            if elen < 2 || p + elen > len {
                break;
            }
            match typ {
                0 => {
                    // ACPI 2.0: type, len, proximity[7:0], apic_id, flags u32,
                    // sapic_eid, proximity[31:8], reserved
                    if elen >= 8 {
                        let prox = e.add(2).read_volatile() as u32;
                        let apic = e.add(3).read_volatile() as u32;
                        let flags = u32le(e.add(4));
                        if flags & 1 != 0 && info.proc_cnt < MAX_SRAT {
                            info.proc_aff[info.proc_cnt] = (prox, apic);
                            info.proc_cnt += 1;
                        }
                    }
                }
                1 => {
                    // type, len, proximity u32@2, reserved u16, base u64@8,
                    // range u64@16, reserved u32, flags u32@28, reserved u64
                    if elen >= 32 {
                        let prox = u32le(e.add(2));
                        let base = u64le(e.add(8));
                        let range = u64le(e.add(16));
                        let flags = u32le(e.add(28));
                        if flags & 1 != 0 && info.mem_cnt < MAX_SRAT {
                            info.mem_aff[info.mem_cnt] = (prox, base, range);
                            info.mem_cnt += 1;
                        }
                    }
                }
                _ => {}
            }
            p += elen;
        }
        if info.mem_cnt == 0 && info.proc_cnt == 0 {
            return None;
        }
        Some(info)
    }
}

/// 解析 SLIT:返回 (节点数, 距离矩阵)。
pub fn slit_parse() -> Option<(usize, [u8; 64 * 64])> {
    unsafe {
        let thdr = find_table(SLIT_SIG)?;
        let tbase = thdr as u64;
        let len = (*thdr).length as u64;
        let num = u64le((tbase + 36) as *const u8) as usize;
        if num == 0 || num > 64 || 44 + (num as u64) * (num as u64) > len {
            return None;
        }
        let mut matrix = [0u8; 64 * 64];
        for i in 0..num {
            for j in 0..num {
                matrix[i * 64 + j] =
                    *((tbase + 44 + (i * num + j) as u64) as *const u8);
            }
        }
        Some((num, matrix))
    }
}

/// 解析 MADT:返回 (Local APIC 基址, 处理器 LAPIC ID 列表)。
pub fn madt_parse() -> Option<(u64, [u32; 64], usize)> {
    unsafe {
        let thdr = find_table(MADT_SIG)?;
        let tbase = thdr as u64;
        let len = (*thdr).length as u64;
        let lapic_addr = *((tbase + 36) as *const u32) as u64;
        let mut ids = [0u32; 64];
        let mut n = 0;
        let mut p = 44u64;
        while p + 2 <= len {
            let e = (tbase + p) as *const u8;
            let typ = e.read_volatile();
            let elen = e.add(1).read_volatile() as u64;
            if elen < 2 || p + elen > len {
                break;
            }
            if typ == 0 && elen >= 8 {
                let apic_id = e.add(3).read_volatile();
                let flags = e.add(4).read_volatile() as u32
                    | ((e.add(5).read_volatile() as u32) << 8)
                    | ((e.add(6).read_volatile() as u32) << 16)
                    | ((e.add(7).read_volatile() as u32) << 24);
                if flags & 1 != 0 && n < 64 {
                    ids[n] = apic_id as u32;
                    n += 1;
                }
            }
            p += elen;
        }
        if n == 0 {
            crate::log!("acpi: madt_parse: no processor entries (len={len})");
            return None;
        }
        Some((lapic_addr, ids, n))
    }
}

/// 解析 MADT 中的 IOAPIC 列表,最多 8 个(双路 X99 为 2 个)。
/// 返回 (MMIO 地址, GSIV 基址, 引脚数)。
pub fn madt_ioapics() -> [(u64, u32, u32); 8] {
    let mut out = [(0u64, 0u32, 0u32); 8];
    let mut n = 0;
    unsafe {
        let Some(thdr) = find_table(MADT_SIG) else {
            return out;
        };
        let tbase = thdr as u64;
        let len = (*thdr).length as u64;
        let mut p = 44u64;
        while p + 2 <= len {
            let e = (tbase + p) as *const u8;
            let typ = e.read_volatile();
            let elen = e.add(1).read_volatile() as u64;
            if elen < 2 || p + elen > len {
                break;
            }
            if typ == 1 && elen >= 12 && n < 8 {
                let mut b = [0u8; 4];
                for i in 0..4 {
                    b[i] = e.add(4 + i).read_volatile();
                }
                let addr = u32::from_le_bytes(b) as u64;
                for i in 0..4 {
                    b[i] = e.add(8 + i).read_volatile();
                }
                let gsiv = u32::from_le_bytes(b);
                out[n] = (addr, gsiv, ioapic_pins(addr));
                n += 1;
            }
            p += elen;
        }
    }
    out
}

/// 读 IOAPIC 版本寄存器得到引脚数:((ver >> 16) & 0xFF) + 1。
fn ioapic_pins(addr: u64) -> u32 {
    unsafe {
        core::ptr::write_volatile(addr as *mut u32, 1); // IOREGSEL: VER
        let ver = core::ptr::read_volatile((addr + 0x10) as *const u32);
        ((ver >> 16) & 0xFF) + 1
    }
}

/// 查中断源覆盖(IRQ -> GSIV):无覆盖时 GSIV == IRQ。
/// 返回 (GSIV, flags 低 16 位:polarity/trigger)。
pub fn madt_irq_override(src: u32) -> (u32, u16) {
    unsafe {
        let Some(thdr) = find_table(MADT_SIG) else {
            return (src, 0);
        };
        let tbase = thdr as u64;
        let len = (*thdr).length as u64;
        let mut p = 44u64;
        while p + 2 <= len {
            let e = (tbase + p) as *const u8;
            let typ = e.read_volatile();
            let elen = e.add(1).read_volatile() as u64;
            if elen < 2 || p + elen > len {
                break;
            }
            if typ == 2 && elen >= 10 {
                let irq = e.add(3).read_volatile() as u32;
                if irq == src {
                    let mut b = [0u8; 4];
                    for i in 0..4 {
                        b[i] = e.add(4 + i).read_volatile();
                    }
                    let gsiv = u32::from_le_bytes(b);
                    let flags = (e.add(8).read_volatile() as u16)
                        | ((e.add(9).read_volatile() as u16) << 8);
                    return (gsiv, flags);
                }
            }
            p += elen;
        }
    }
    (src, 0)
}

/// 当前 CPU 的 APIC ID(cpuid leaf 1 EBX[31:24])。
pub fn lapic_id() -> u32 {
    let ebx: u32;
    unsafe {
        core::arch::asm!(
            "push rbx",
            "mov eax, 1",
            "cpuid",
            "mov {0:e}, ebx",
            "pop rbx",
            out(reg) ebx,
            out("eax") _,
            out("ecx") _,
            out("edx") _,
            options(nostack)
        );
    }
    (ebx >> 24) & 0xFF
}

/// 调试:打印 ACPI 概况到 COM1。
pub fn acpi_log() {
    log!("acpi: rsdp = {:?}", rsdp_find().map(|(a, x)| (a, x)));
    if let Some((xdt, is_xsdt)) = rsdp_find() {
        unsafe {
            let entries = ((*(xdt as *const AcpiHeader)).length as usize - 36)
                / (if is_xsdt { 8 } else { 4 });
            for i in 0..entries {
                let addr = if is_xsdt {
                    *((xdt + 36 + (i * 8) as u64) as *const u64)
                } else {
                    *((xdt + 36 + (i * 4) as u64) as *const u32) as u64
                };
                let thdr = &*(addr as *const AcpiHeader);
                let sig = core::str::from_utf8(&thdr.sig).unwrap_or("????");
                log!("acpi:   xdt[{}] {} @ {:#x}", i, sig, addr);
                log!("acpi:   xdt ok");
            }
        }
    }
    if let Some(i) = srat_parse() {
        log!("acpi: SRAT {} mem affinity, {} proc affinity", i.mem_cnt, i.proc_cnt);
        for m in 0..i.mem_cnt {
            let (d, b, l) = i.mem_aff[m];
            log!("acpi:   mem node {}: {:#x} + {:#x}", d, b, l);
        }
        for p in 0..i.proc_cnt {
            let (d, id) = i.proc_aff[p];
            log!("acpi:   lapic {:#x} -> node {}", id, d);
        }
    } else {
        log!("acpi: no SRAT (single node)");
    }
    if let Some((n, m)) = slit_parse() {
        log!("acpi: SLIT {} nodes, dist[0][1] = {}", n, m[1]);
    }
    if let Some((base, ids, n)) = madt_parse() {
        log!("acpi: MADT lapic {:#x}, {} processors", base, n);
        for i in 0..n {
            log!("acpi:   cpu {} lapic {:#x}", i, ids[i]);
        }
    }
}