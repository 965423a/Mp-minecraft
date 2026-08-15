//! 最小自旋锁:单核无抢占、无中断嵌套的轮询内核用。
//! 未来引入 IRQ 抢占时,锁内须配关中断/中断嵌套保护。

use core::sync::atomic::{AtomicBool, Ordering};

/// 自旋锁。`lock()` 忙等 CAS,`unlock()` release 写。
pub struct SpinLock {
    locked: AtomicBool,
}

impl SpinLock {
    pub const fn new() -> Self {
        Self {
            locked: AtomicBool::new(false),
        }
    }

    #[inline]
    pub fn lock(&self) {
        while self
            .locked
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_err()
        {
            core::hint::spin_loop();
        }
    }

    #[inline]
    pub fn unlock(&self) {
        self.locked.store(false, Ordering::Release);
    }
}
