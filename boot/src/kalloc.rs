use core::alloc::{GlobalAlloc, Layout};
use core::sync::atomic::{AtomicU64, Ordering};

/// 基于 NUMA 帧的简单内核堆分配器:分配取本节点连续帧(页对齐),
/// 释放逐页归还节点。适合"分配多、释放少"的嵌入负载(世界生成)。
pub struct KernelAlloc;

pub static ALLOC_CNT: AtomicU64 = AtomicU64::new(0);
pub static FREE_CNT: AtomicU64 = AtomicU64::new(0);

unsafe impl GlobalAlloc for KernelAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        crate::sched::sched_preempt_disable();
        let align = layout.align();
        let mut size = layout.size();
        if align > 0x1000 {
            size += align; // 手动对齐余量
        }
        let pages = (size + 0xfff) >> 12;
        let cpu = crate::idt::lapic_id();
        let node = crate::numa::node_for_lapic(cpu).unwrap_or(0);
        let base = match crate::numa::alloc_contig(node, pages) {
            Some(b) => b,
            None => {
                crate::sched::sched_preempt_enable();
                return core::ptr::null_mut();
            }
        };
        ALLOC_CNT.fetch_add(1, Ordering::Relaxed);
        if align <= 0x1000 {
            crate::sched::sched_preempt_enable();
            return base as *mut u8;
        }
        let b = base as usize;
        let aligned = (b + align - 1) & !(align - 1);
        let waste = aligned - b;
        let waste_pages = waste >> 12;
        if waste_pages > 0 {
            crate::numa::free_contig(base, waste_pages);
        }
        let used = (layout.size() + 0xfff) >> 12;
        let tail = pages - waste_pages - used;
        if tail > 0 {
            crate::numa::free_contig(base + (waste_pages as u64 + used as u64) * 0x1000, tail);
        }
        crate::sched::sched_preempt_enable();
        aligned as *mut u8
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        crate::sched::sched_preempt_disable();
        let pages = (layout.size() + 0xfff) >> 12;
        if pages > 0 {
            crate::numa::free_contig(ptr as u64, pages);
        }
        FREE_CNT.fetch_add(1, Ordering::Relaxed);
        crate::sched::sched_preempt_enable();
    }
}

pub fn stats() -> (u64, u64) {
    (
        ALLOC_CNT.load(Ordering::Relaxed),
        FREE_CNT.load(Ordering::Relaxed),
    )
}