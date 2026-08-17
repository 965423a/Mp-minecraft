//! NUMA:节点表 + 分桶连续帧分配器(SRAT > 命令行 > 单节点)。
//!
//! 优化(相对旧版单链):1) 连续块按 2^k 页分桶,alloc_contig 从最小
//! 可容纳桶取块,O(#桶) 而非 O(帧数);块首 16B = [len_pages, next]。
//! 2) 每节点独立锁,跨节点分配并行;3) NumaNode 64B 缓存行对齐,
//! 避免伪共享。

const MAX_NODES: usize = 8;
const MAX_SPANS: usize = 8;
/// 连续块桶数:2^0..2^10 页(1..1024),与 alloc_contig 上限一致。
const NUM_BUCKETS: usize = 11;

/// 页表静态映射上限(与 boot.S 一致),超出部分不参与帧链。
const MAX_PHYS: u64 = 16 * 1024 * 1024 * 1024;

/// 无 SLIT 时的默认距离:同节点 10,跨节点 20(仅用于排序,非真实延迟)。
const DIST_SAME: u8 = 10;
const DIST_REMOTE: u8 = 20;

/// 单帧链与块桶共享 free_cnt(空闲帧总数)。
#[derive(Clone, Copy)]
#[repr(C, align(64))]
struct NumaNode {
    id: u8,
    pages: u64,
    free_cnt: u64,
    alloc_cnt: u64,
    /// 单帧链(8B/帧,pop/单帧 free 用)。
    free_head: u64,
    /// 连续块桶:buckets[k] = 块链头,块首 16B = [len_pages, next]。
    buckets: [u64; NUM_BUCKETS],
    /// 本节点 usable 区间集合(node_of 精确归属判定,支持不连续内存)。
    spans: [(u64, u64); MAX_SPANS],
    span_cnt: usize,
}

static mut NODES: [NumaNode; MAX_NODES] = [NumaNode {
    id: 0,
    pages: 0,
    free_cnt: 0,
    alloc_cnt: 0,
    free_head: 0,
    buckets: [0; NUM_BUCKETS],
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

/// 帧链临界区锁(BSP 与 AP 并发分配/释放时保护空闲链)。
/// 每节点独立锁(跨节点分配并行)。
static NODES_LOCK: [crate::spinlock::SpinLock; MAX_NODES] = [const {
    crate::spinlock::SpinLock::new()
}; MAX_NODES];

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

fn floor_log2(x: u64) -> u32 {
    (63 - x.leading_zeros()) as u32
}

fn ceil_log2(x: u64) -> u32 {
    let f = floor_log2(x);
    if x & (x - 1) == 0 {
        f
    } else {
        f + 1
    }
}

/// 块首 16B = [len_pages, next]。len 为 1 时走单帧链(8B/帧)。
fn block_read(addr: u64) -> (u64, u64) {
    unsafe {
        (
            (addr as *const u64).read_volatile(),
            (addr as *const u64).add(1).read_volatile(),
        )
    }
}

fn block_write(addr: u64, len: u64, next: u64) {
    unsafe {
        (addr as *mut u64).write_volatile(len);
        (addr as *mut u64).add(1).write_volatile(next);
    }
}

/// 块/单帧入空闲(调用方须持有节点锁)。
fn push_free_locked(n: usize, addr: u64, len: u64) {
    unsafe {
        if len == 1 {
            (addr as *mut u64).write_volatile(NODES[n].free_head);
            NODES[n].free_head = addr;
        } else {
            let k = floor_log2(len) as usize;
            let k = k.min(NUM_BUCKETS - 1);
            block_write(addr, len, NODES[n].buckets[k]);
            NODES[n].buckets[k] = addr;
        }
        NODES[n].free_cnt += len;
    }
}

/// 把 [a, b) 切成 2^k 页连续块入桶(高端向低端,优先大块)。
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
        let mut p = b;
        while p - 0x1000 >= a {
            let len = (p - a) / 0x1000;
            let k = floor_log2(len).min(NUM_BUCKETS as u32 - 1);
            let blk = 1u64 << k;
            let start = p - blk * 0x1000;
            push_free_locked(n, start, blk);
            NODES[n].pages += blk;
            p = start;
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
                buckets: [0; NUM_BUCKETS],
                free_cnt: 0,
                alloc_cnt: 0,
                spans: [(0, 0); MAX_SPANS],
                span_cnt: 0,
            };
        }
        for_each_usable(info, |base, len| {
            let e1 = base + len;
            for n in 0..cnt {
                let a = base.max(ranges[n].start);
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
    if unsafe { NODE_CNT == 0 } {
        return None;
    }
    let n = if node < unsafe { NODE_CNT } { node } else { 0 };
    NODES_LOCK[n].lock();
    let r = unsafe {
        if NODES[n].free_cnt > 0 {
            take_one_locked(n)
        } else {
            None
        }
    };
    NODES_LOCK[n].unlock();
    r
}

/// 本节点无帧时,找 SLIT 距离最近的邻节点(锁外决策,避免持多锁)。
pub fn alloc_local_fallback(node: usize) -> Option<u64> {
    if unsafe { NODE_CNT == 0 } {
        return None;
    }
    let n = if node < unsafe { NODE_CNT } { node } else { 0 };
    let mut best = usize::MAX;
    let mut best_d = 0u8;
    unsafe {
        for i in 0..NODE_CNT {
            if i != n && NODES[i].free_cnt > 0 {
                let d = node_distance(n, i);
                if best == usize::MAX || d < best_d {
                    best = i;
                    best_d = d;
                }
            }
        }
    }
    if best == usize::MAX {
        return None;
    }
    NODES_LOCK[best].lock();
    let r = unsafe { take_one_locked(best) };
    NODES_LOCK[best].unlock();
    r
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
            NODES_LOCK[n].lock();
            let r = unsafe {
                if NODES[n].free_cnt > 0 {
                    take_one_locked(n)
                } else {
                    None
                }
            };
            NODES_LOCK[n].unlock();
            if r.is_some() {
                return r;
            }
        }
    }
    None
}

/// 从节点 n 的帧链摘出 `n_frames` 个物理连续帧,返回起始地址。
/// 帧链的 next 指针跨区间拼接,物理地址不单调,故扫描全链找连续段。
/// 释放:对每帧分别 `free(start + k*0x1000)` 即可。
pub fn alloc_contig(node: usize, n_frames: usize) -> Option<u64> {
    if n_frames == 0 || n_frames > 1024 || node >= unsafe { NODE_CNT } {
        return None;
    }
    NODES_LOCK[node].lock();
    let r = unsafe { alloc_contig_locked(node, n_frames) };
    NODES_LOCK[node].unlock();
    r
}

fn alloc_contig_locked(node: usize, n: usize) -> Option<u64> {
    unsafe {
        // 最小可容纳桶:ceil_log2(n)
        let k0 = ceil_log2(n as u64) as usize;
        for k in k0.min(NUM_BUCKETS - 1)..NUM_BUCKETS {
            let head = NODES[node].buckets[k];
            if head == 0 {
                continue;
            }
            let (len, next) = block_read(head);
            NODES[node].buckets[k] = next;
            if len > n as u64 {
                // 剩余部分(连续)重新入桶
                let rem = len - n as u64;
                let addr = head + (n as u64) * 0x1000;
                push_free_locked(node, addr, rem);
            }
            NODES[node].free_cnt -= n as u64;
            NODES[node].alloc_cnt += n as u64;
            return Some(head);
        }
        // 无大块:从单帧链逐帧拼(退化路径)
        let mut prev: u64 = 0;
        let mut cur = NODES[node].free_head;
        while cur != 0 {
            let start = cur;
            let mut ok = true;
            for _ in 0..n - 1 {
                let nxt = (cur as *const u64).read_volatile();
                if nxt != cur + 0x1000 {
                    ok = false;
                    break;
                }
                cur = nxt;
            }
            if ok {
                let after = ((start + (n as u64 - 1) * 0x1000) as *const u64).read_volatile();
                if prev == 0 {
                    NODES[node].free_head = after;
                } else {
                    (prev as *mut u64).write_volatile(after);
                }
                NODES[node].free_cnt -= n as u64;
                NODES[node].alloc_cnt += n as u64;
                return Some(start);
            }
            let nxt = (start as *const u64).read_volatile();
            if nxt == 0 {
                break;
            }
            cur = nxt;
            prev = start;
        }
        None
    }
}

/// 取单帧:单帧链有则取,无则从连续块桶取 1 帧(调用方须已持有节点锁)。
fn take_one_locked(n: usize) -> Option<u64> {
    unsafe {
        if NODES[n].free_head != 0 {
            pop(n)
        } else {
            alloc_contig_locked(n, 1)
        }
    }
}

/// 内部取帧(调用方须已持有节点锁)。
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

/// 按节点 usable 区间精确查所属节点(支持不连续内存)。不在任何节点区间时返回 0。
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

/// 释放一帧(归还到所属节点)。地址不在任何节点区间(低 1MiB/内核镜像/越界)时拒绝。
pub fn free(phys: u64) {
    if phys == 0 {
        return;
    }
    let n = node_of(phys);
    NODES_LOCK[n].lock();
    unsafe {
        if !node_covered(n, phys) {
            crate::log!("numa: free({phys:#x}) outside node spans, rejected");
            NODES_LOCK[n].unlock();
            return;
        }
        push_free_locked(n, phys, 1);
    }
    NODES_LOCK[n].unlock();
}

/// 批量释放连续块(整块入桶,保持连续性,优于逐帧)。
pub fn free_contig(phys: u64, pages: usize) {
    if phys == 0 || pages == 0 {
        return;
    }
    let n = node_of(phys);
    NODES_LOCK[n].lock();
    unsafe {
        if !node_covered(n, phys) || !node_covered(n, phys + (pages as u64 - 1) * 0x1000) {
            crate::log!("numa: free_contig({phys:#x}, {pages}) outside node spans, rejected");
            NODES_LOCK[n].unlock();
            return;
        }
        push_free_locked(n, phys, pages as u64);
    }
    NODES_LOCK[n].unlock();
}

fn node_covered(n: usize, phys: u64) -> bool {
    unsafe { (0..NODES[n].span_cnt).any(|s| { let (a, b) = NODES[n].spans[s]; phys >= a && phys < b }) }
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

/// 启动自检:验证本地分配归属、跨节点 fallback 计数、交错分布与非法释放拒绝。
/// 在 SMP 唤醒后调用;帧链操作已有自旋锁保护,AP 亦可安全分配。
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
        // 4. 非法释放必须被拒绝:低 1MiB / 内核镜像 / 越界地址
        let before = node_free_total();
        for bad in [0x7000u64, 0x100000, 0x295000, 0x100000_000000] {
            free(bad);
        }
        let after = node_free_total();
        if after == before {
            crate::log!("numa:   invalid free rejected OK");
        } else {
            crate::log!("numa:   invalid free NOT rejected (BUG)");
        }
        // 5. 分配 + 写入模式 + 校验:验证页表映射与内存真实可用(含高端帧)
        for n in 0..nn {
            if let Some(p) = alloc_local(n) {
                let w = p as *mut u64;
                for i in 0..512 {
                    w.add(i).write_volatile(i as u64 ^ 0x5A5A_5A5A_5A5A_5A5A);
                }
                let mut ok = true;
                for i in 0..512 {
                    if w.add(i).read_volatile() != i as u64 ^ 0x5A5A_5A5A_5A5A_5A5A {
                        ok = false;
                    }
                }
                free(p);
                if ok {
                    crate::log!("numa:   rw-check node {n} @ 0x{p:x} OK");
                } else {
                    crate::log!("numa:   rw-check node {n} @ 0x{p:x} FAILED");
                }
            } else {
                crate::log!("numa:   rw-check node {n} alloc failed");
            }
        }
        // 6. 连续多帧分配(alloc_contig):4 帧(16KiB)必须物理连续
        if let Some(p) = alloc_contig(0, 4) {
            let mut contig = true;
            for k in 0..4 {
                if node_of(p + k * 0x1000) != 0 {
                    contig = false;
                }
                free(p + k * 0x1000);
            }
            if contig {
                crate::log!("numa:   contig(0,4) @ 0x{p:x} OK");
            } else {
                crate::log!("numa:   contig(0,4) @ 0x{p:x} BAD");
            }
        } else {
            crate::log!("numa:   contig(0,4) failed");
        }
        crate::log!("numa: selftest done");
    }
}

fn node_free_total() -> u64 {
    unsafe {
        let mut t = 0;
        for n in 0..NODE_CNT {
            t += NODES[n].free_cnt;
        }
        t
    }
}