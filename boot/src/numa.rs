//! NUMA:节点表 + 每节点空闲帧链分配器。
//!
//! 拓扑来源:multiboot2 命令行 `numa=<n>;<id>:<startMB>-<endMB>;...`,
//! 例如 `numa=2;0:0-1024;1:1024-2048`(单位 MiB)。无参数时单节点,
//! 覆盖全部 usable 内存(单 U 实体机 / 未开 NUMA 的 QEMU 即此情况)。
//!
//! 分配器:每节点一条空闲帧链表(空闲帧内容存 next 物理地址,零额外
//! 内存开销),本地节点优先,耗尽可跨节点 fallback。

const MAX_NODES: usize = 8;
const MAX_SPANS: usize = 8;

/// 页表静态映射上限(与 boot.S 一致),超出部分不参与帧链。
const MAX_PHYS: u64 = 16 * 1024 * 1024 * 1024;

/// 无 SLIT 时的默认距离:同节点 10,跨节点 20(仅用于排序,非真实延迟)。
const DIST_SAME: u8 = 10;
const DIST_REMOTE: u8 = 20;

#[derive(Clone, Copy)]
struct NumaNode {
    id: u8,
    pages: u64,
    free_head: u64,
    free_cnt: u64,
    alloc_cnt: u64,
    /// 本节点 usable 区间集合(node_of 精确归属判定,支持不连续内存)。
    spans: [(u64, u64); MAX_SPANS],
    span_cnt: usize,
}

static mut NODES: [NumaNode; MAX_NODES] = [NumaNode {
    id: 0,
    pages: 0,
    free_head: 0,
    free_cnt: 0,
    alloc_cnt: 0,
    spans: [(0, 0); MAX_SPANS],
    span_cnt: 0,
}; MAX_NODES];
static mut NODE_CNT: usize = 0;

/// LAPIC ID → 节点索引 映射(来自 SRAT 处理器亲和)。
static mut CPU_NODES: [(u32, usize); 64] = [(0, 0); 64];
static mut CPU_NODE_CNT: usize = 0;

/// SLIT 节点距离矩阵(无 SLIT 时用默认距离)。
static mut SLIT_N: usize = 0;
static mut SLIT: [[u8; 64]; 64] = [[0; 64]; 64];

static mut INTERLEAVE_LAST: usize = 0;

unsafe extern "C" {
    static _start: u8;
    static _end: u8;
}

#[repr(C)]
struct Mb2Tag {
    typ: u32,
    size: u32,
}

#[repr(C)]
struct Mb2MmapEntry {
    base: u64,
    length: u64,
    mtype: u32,
    reserved: u32,
}

#[derive(Clone, Copy)]
struct Range {
    id: u8,
    start: u64,
    end: u64,
}

fn parse_uint(s: &[u8]) -> (u64, &[u8]) {
    let mut v = 0u64;
    let mut i = 0;
    while i < s.len() && s[i] >= b'0' && s[i] <= b'9' {
        v = v * 10 + (s[i] - b'0') as u64;
        i += 1;
    }
    (v, &s[i..])
}

/// 从命令行解析 `numa=...`。返回节点数(缺省/非法返回 0)。
fn parse_cmdline(cmdline: &[u8], out: &mut [Range]) -> usize {
    let mut i = 0;
    while i + 5 <= cmdline.len() {
        if cmdline[i..].starts_with(b"numa=") {
            let mut rest = &cmdline[i + 5..];
            let (cnt, r2) = parse_uint(rest);
            rest = r2;
            if cnt == 0 || cnt as usize > MAX_NODES || rest.first() != Some(&b';') {
                return 0;
            }
            rest = &rest[1..];
            let mut parsed = 0;
            while !rest.is_empty() && parsed < cnt as usize {
                let (id, r2) = parse_uint(rest);
                rest = r2;
                if rest.first() != Some(&b':') {
                    return 0;
                }
                rest = &rest[1..];
                let (start, r2) = parse_uint(rest);
                rest = r2;
                if rest.first() != Some(&b'-') {
                    return 0;
                }
                rest = &rest[1..];
                let (end, r2) = parse_uint(rest);
                rest = r2;
                if !rest.is_empty() {
                    if rest.first() != Some(&b';') {
                        return 0;
                    }
                    rest = &rest[1..];
                }
                if parsed > 0 && id <= out[parsed - 1].id as u64 {
                    return 0;
                }
                out[parsed] = Range {
                    id: id as u8,
                    start: start << 20,
                    end: end << 20,
                };
                parsed += 1;
            }
            return parsed;
        }
        i += 1;
    }
    0
}

/// 找 multiboot2 command line tag(type=1)。
fn mb2_cmdline(info: *const u8) -> Option<&'static [u8]> {
    if info.is_null() {
        return None;
    }
    unsafe {
        let total = *(info as *const u32);
        let mut pos = 8usize;
        while pos + 8 <= total as usize {
            let tag = &*(info.add(pos) as *const Mb2Tag);
            let size = tag.size as usize;
            if tag.typ == 0 {
                break;
            }
            if tag.typ == 1 {
                let s = info.add(pos + 8) as *const u8;
                let mut len = 0;
                while *s.add(len) != 0 && len < 1024 {
                    len += 1;
                }
                return Some(core::slice::from_raw_parts(s, len));
            }
            pos += (size + 7) & !7;
        }
    }
    None
}

fn for_each_usable(info: *const u8, mut f: impl FnMut(u64, u64)) {
    unsafe {
        let total = *(info as *const u32);
        let mut pos = 8usize;
        while pos + 8 <= total as usize {
            let tag = &*(info.add(pos) as *const Mb2Tag);
            let size = tag.size as usize;
            if tag.typ == 0 {
                break;
            }
            if tag.typ == 6 {
                let mut p = pos + 16;
                let entry_size = *(info.add(pos + 8) as *const u32) as usize;
                while p + entry_size <= pos + size {
                    let e = &*(info.add(p) as *const Mb2MmapEntry);
                    if e.mtype == 1 {
                        f(e.base, e.length);
                    }
                    p += entry_size;
                }
            }
            pos += (size + 7) & !7;
        }
    }
}

/// 内核镜像区间,分配时排除。
/// 注意:镜像从 1M 加载(multiboot2),`_start` 符号只是代码入口,不是镜像起点。
fn kernel_range() -> (u64, u64) {
    unsafe {
        (
            1u64 << 20,
            &_end as *const u8 as u64,
        )
    }
}

/// 把 [a, b) 的物理内存并入节点 n 的空闲链(帧对齐,排除内核镜像与低 1MiB)。
/// 低 1MiB 含实模式 IVT/BDA/EBDA/multiboot info/trampoline 区,一律保留。
fn add_span(n: usize, a0: u64, b0: u64) {
    let (ks, ke) = kernel_range();
    let a0 = a0.max(1u64 << 20);
    // 与内核镜像求差,最多两段
    let mut a = (a0 + 0xFFF) & !0xFFF;
    let mut b = b0 & !0xFFF;
    if b <= a {
        return;
    }
    let mut segs = [0u64; 4];
    let mut segn = 0;
    if b <= ks || a >= ke {
        segs[0] = a;
        segs[1] = b;
        segn = 1;
    } else if a < ks && b > ke {
        segs[0] = a;
        segs[1] = ks;
        segs[2] = ke;
        segs[3] = b;
        segn = 2;
    } else if a < ks {
        // 与镜像重叠在尾部
        segs[0] = a;
        segs[1] = ks.min(b);
        segn = 1;
    } else if b > ke {
        segs[0] = ke.max(a);
        segs[1] = b;
        segn = 1;
    }
    for s in 0..segn {
        let sa = segs[s * 2];
        let sb = segs[s * 2 + 1];
        link_frames(n, sa, sb);
        // 记录区间,供 node_of 精确归属
        unsafe {
            if NODES[n].span_cnt < MAX_SPANS {
                NODES[n].spans[NODES[n].span_cnt] = (sa, sb);
                NODES[n].span_cnt += 1;
            }
        }
    }
}

/// 把 [a, b) 串成空闲帧链(高端向低端)。
fn link_frames(n: usize, a: u64, b: u64) {
    unsafe {
        if b <= a {
            return;
        }
        let b = b.min(MAX_PHYS);
        let a = a.min(MAX_PHYS);
        if b <= a {
            return;
        }
        let mut prev = NODES[n].free_head;
        let mut cnt = 0u64;
        let mut p = b;
        loop {
            p -= 0x1000;
            if p < a {
                break;
            }
            (p as *mut u64).write_volatile(prev);
            prev = p;
            cnt += 1;
            if p <= a {
                break;
            }
        }
        if cnt > 0 {
            NODES[n].free_head = prev;
            NODES[n].free_cnt += cnt;
            NODES[n].pages += cnt;
        }
    }
}

/// 初始化 NUMA:拓扑来源优先级 ACPI SRAT > 命令行 > 单节点。
/// 扫描 usable 内存建节点帧链。返回节点数。
pub fn init(info: *const u8) -> usize {
    unsafe {
        NODE_CNT = 0;
        CPU_NODE_CNT = 0;
        let mut ranges = [Range {
            id: 0,
            start: 0,
            end: 0,
        }; MAX_NODES];
        let mut domains = [0u32; MAX_NODES];
        let mut cnt = 0usize;
        // 1. ACPI SRAT
        if let Some(srat) = crate::acpi::srat_parse() {
            for m in 0..srat.mem_cnt {
                let (d, b, l) = srat.mem_aff[m];
                let mut idx = None;
                for i in 0..cnt {
                    if domains[i] == d {
                        idx = Some(i);
                    }
                }
                let i = match idx {
                    Some(i) => i,
                    None => {
                        if cnt >= MAX_NODES {
                            break;
                        }
                        domains[cnt] = d;
                        ranges[cnt].id = d as u8;
                        cnt += 1;
                        cnt - 1
                    }
                };
                // 同 domain 多段取并集
                if ranges[i].start == 0 && ranges[i].end == 0 {
                    ranges[i].start = b;
                    ranges[i].end = b + l;
                } else {
                    ranges[i].start = ranges[i].start.min(b);
                    ranges[i].end = ranges[i].end.max(b + l);
                }
            }
            for p in 0..srat.proc_cnt {
                let (d, lapic) = srat.proc_aff[p];
                for i in 0..cnt {
                    if domains[i] == d && CPU_NODE_CNT < 64 {
                        CPU_NODES[CPU_NODE_CNT] = (lapic, i);
                        CPU_NODE_CNT += 1;
                    }
                }
            }
        }
        // 2. 命令行 fallback
        if cnt == 0 {
            let cl_cnt = match mb2_cmdline(info) {
                Some(cl) => parse_cmdline(cl, &mut ranges),
                None => 0,
            };
            if cl_cnt > 0 {
                cnt = cl_cnt;
            }
        }
        // 3. 单节点兜底
        if cnt == 0 {
            cnt = 1;
            ranges[0] = Range {
                id: 0,
                start: 0,
                end: u64::MAX,
            };
        }
        for i in 0..cnt {
            NODES[i] = NumaNode {
                id: ranges[i].id,
                pages: 0,
                free_head: 0,
                free_cnt: 0,
                alloc_cnt: 0,
                spans: [(0, 0); MAX_SPANS],
                span_cnt: 0,
            };
        }
        for_each_usable(info, |base, len| {
            let e0 = base;
            let e1 = base + len;
            for n in 0..cnt {
                let a = e0.max(ranges[n].start);
                let b = e1.min(ranges[n].end);
                if b > a {
                    add_span(n, a, b);
                }
            }
        });
        NODE_CNT = cnt;
        // SLIT 距离矩阵(无则走默认距离)
        SLIT_N = 0;
        if let Some((sn, m)) = crate::acpi::slit_parse() {
            SLIT_N = sn;
            for i in 0..sn {
                for j in 0..sn {
                    SLIT[i][j] = m[i * 64 + j];
                }
            }
        }
        for n in 0..cnt {
            let (mi, free) = node_mem(n);
            let base = if NODES[n].span_cnt > 0 {
                NODES[n].spans[0].0
            } else {
                0
            };
            crate::log!(
                "numa: node {} id {} base {:#x} {mi} MiB, {free} frames free",
                n,
                ranges[n].id,
                base
            );
        }
        let cpu_entries = CPU_NODE_CNT;
        crate::log!("numa: {cnt} nodes, {cpu_entries} cpu->node entries");
        if SLIT_N > 0 {
            let d = node_distance(0, 1);
            crate::log!("numa: slit {cnt} nodes, dist[0][1] = {d}");
        }
        cnt
    }
}

/// LAPIC ID → 节点索引。
pub fn node_for_lapic(lapic: u32) -> Option<usize> {
    unsafe {
        for i in 0..CPU_NODE_CNT {
            if CPU_NODES[i].0 == lapic {
                return Some(CPU_NODES[i].1);
            }
        }
    }
    None
}

/// 节点间距离(SLIT 缺失时默认同节点 10 / 跨节点 20)。
pub fn node_distance(i: usize, j: usize) -> u8 {
    unsafe {
        if SLIT_N == 0 || i >= SLIT_N || j >= SLIT_N {
            if i == j {
                DIST_SAME
            } else {
                DIST_REMOTE
            }
        } else {
            SLIT[i][j]
        }
    }
}

/// 本地节点优先分配一帧;本地耗尽按 SLIT 距离就近 fallback。返回物理地址。
pub fn alloc_local(node: usize) -> Option<u64> {
    unsafe {
        if NODE_CNT == 0 {
            return None;
        }
        let n = if node < NODE_CNT { node } else { 0 };
        if NODES[n].free_cnt > 0 {
            return pop(n);
        }
        let mut best = usize::MAX;
        let mut best_d = 0u8;
        for i in 0..NODE_CNT {
            if i != n && NODES[i].free_cnt > 0 {
                let d = node_distance(n, i);
                if best == usize::MAX || d < best_d {
                    best = i;
                    best_d = d;
                }
            }
        }
        if best != usize::MAX {
            return pop(best);
        }
        None
    }
}

/// 节点 0 优先分配。
pub fn alloc() -> Option<u64> {
    alloc_local(0)
}

/// 交错分配:跨节点轮流取帧(round-robin)。
pub fn alloc_interleave() -> Option<u64> {
    unsafe {
        for _ in 0..NODE_CNT {
            INTERLEAVE_LAST = (INTERLEAVE_LAST + 1) % NODE_CNT;
            let n = INTERLEAVE_LAST;
            if NODES[n].free_cnt > 0 {
                return pop(n);
            }
        }
        None
    }
}

fn pop(n: usize) -> Option<u64> {
    unsafe {
        let head = NODES[n].free_head;
        if head == 0 || NODES[n].free_cnt == 0 {
            return None;
        }
        let next = (head as *const u64).read_volatile();
        NODES[n].free_head = next;
        NODES[n].free_cnt -= 1;
        NODES[n].alloc_cnt += 1;
        Some(head)
    }
}

/// 按节点 usable 区间精确查所属节点(支持不连续内存)。
pub fn node_of(phys: u64) -> usize {
    unsafe {
        for i in 0..NODE_CNT {
            for s in 0..NODES[i].span_cnt {
                let (a, b) = NODES[i].spans[s];
                if phys >= a && phys < b {
                    return i;
                }
            }
        }
        0
    }
}

/// 释放一帧(归还到所属节点)。
pub fn free(phys: u64) {
    unsafe {
        if phys == 0 {
            return;
        }
        let n = node_of(phys);
        let head = NODES[n].free_head;
        (phys as *mut u64).write_volatile(head);
        NODES[n].free_head = phys;
        NODES[n].free_cnt += 1;
    }
}

pub fn node_count() -> usize {
    unsafe { NODE_CNT }
}

/// 返回 (总 MiB, 空闲帧数)。
pub fn node_mem(node: usize) -> (u64, u64) {
    unsafe {
        if node >= NODE_CNT {
            return (0, 0);
        }
        (NODES[node].pages * 4 / 1024, NODES[node].free_cnt)
    }
}

/// 节点 ACPI proximity domain id。
pub fn node_id(node: usize) -> u8 {
    unsafe {
        if node >= NODE_CNT {
            0
        } else {
            NODES[node].id
        }
    }
}

/// 节点累计分配帧数。
pub fn node_allocs(node: usize) -> u64 {
    unsafe {
        if node >= NODE_CNT {
            0
        } else {
            NODES[node].alloc_cnt
        }
    }
}

/// 填入节点下全部 LAPIC ID,返回数量。
pub fn node_lapics(node: usize, out: &mut [u32; 64]) -> usize {
    unsafe {
        let mut k = 0;
        for i in 0..CPU_NODE_CNT {
            if CPU_NODES[i].1 == node && k < 64 {
                out[k] = CPU_NODES[i].0;
                k += 1;
            }
        }
        k
    }
}

/// 启动自检:验证本地分配归属、跨节点 fallback 计数与交错分布。
/// 在 SMP 唤醒后调用(BSP 单线程,AP 不触碰分配器)。
pub fn selftest() {
    unsafe {
        let nn = NODE_CNT;
        crate::log!("numa: selftest start ({nn} nodes)");
        // 1. 每节点本地分配,校验 node_of 归属
        for n in 0..nn {
            match alloc_local(n) {
                Some(p) => {
                    let owner = node_of(p);
                    let ok = if owner == n { "OK" } else { "WRONG" };
                    crate::log!("numa:   alloc_local({n}) -> 0x{p:x} owner {owner} {ok}");
                    free(p);
                }
                None => crate::log!("numa:   alloc_local({n}) -> failed"),
            }
        }
        // 2. 交错分配 8 帧,统计分布
        let mut dist = [0u64; MAX_NODES];
        for _ in 0..8 {
            if let Some(p) = alloc_interleave() {
                dist[node_of(p)] += 1;
                free(p);
            }
        }
        for n in 0..nn {
            crate::log!("numa:   interleave[{}] = {}", n, dist[n]);
        }
        // 3. 距离矩阵
        if nn > 1 {
            for i in 0..nn {
                for j in 0..nn {
                    let d = node_distance(i, j);
                    crate::log!("numa:   dist[{i}->{j}] = {d}");
                }
            }
        }
        crate::log!("numa: selftest done");
    }
}