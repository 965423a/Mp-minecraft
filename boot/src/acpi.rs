//! ACPI 解析:RSDP 扫描 + RSDT/XSDT 遍历,提取 SRAT/SLIT/MADT。

#![allow(dead_code)]

use crate::log;

pub const SRAT_SIG: &[u8; 4] = b"SRAT";
pub const SLIT_SIG: &[u8; 4] = b"SLIT";
pub const MADT_SIG: &[u8; 4] = b"APIC";

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
fn rsdp_find() -> Option<(u64, bool)> {
    unsafe {
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