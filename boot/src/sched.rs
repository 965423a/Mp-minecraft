//! 抢占调度:每核 1ms tick 触发,时间片轮转。
//! 任务独占 16KiB 内核栈;切换 = 交换中断帧(Frame),首跑用伪造帧。
//! idle 不入队:每核 CUR=0xFFFF 表示当前在 idle 上下文。

use core::arch::asm;
use core::sync::atomic::{AtomicU32, AtomicU64, Ordering};

use crate::idt::Frame;
use crate::spinlock::SpinLock;

pub const MAX_TASKS: usize = 64;
pub const STACK_PAGES: usize = 4;
pub const QUANTUM: u32 = 10; // 10 个 tick = 10ms

const IDLE: u32 = 0xFFFF;

#[repr(C)]
#[derive(Clone, Copy)]
struct Task {
    stack: u64, // 栈基址,0 = 空槽
    sp: u64,    // 帧地址:被切走 = 中断帧,首跑 = 伪造帧
    quantum: u32,
}

static mut TASKS: [Task; MAX_TASKS] = [Task { stack: 0, sp: 0, quantum: QUANTUM }; MAX_TASKS];
static mut QUEUE: Queue = Queue::new();
static QUEUE_LOCK: SpinLock = SpinLock::new();
static CUR: [AtomicU32; 64] = [const { AtomicU32::new(IDLE) }; 64];
static IDLE_SP: [AtomicU64; 64] = [const { AtomicU64::new(0) }; 64];
static PREEMPT: [AtomicU32; 64] = [const { AtomicU32::new(0) }; 64];

/// 持锁等临界区调用:期间 tick 不抢占当前任务。
#[unsafe(no_mangle)]
pub extern "C" fn sched_preempt_disable() {
    let cpu = crate::idt::lapic_id() as usize;
    PREEMPT[cpu].fetch_add(1, Ordering::Relaxed);
}

#[unsafe(no_mangle)]
pub extern "C" fn sched_preempt_enable() {
    let cpu = crate::idt::lapic_id() as usize;
    PREEMPT[cpu].fetch_sub(1, Ordering::Relaxed);
}

struct Queue {
    buf: [u32; MAX_TASKS],
    head: usize,
    tail: usize,
}

impl Queue {
    const fn new() -> Self {
        Queue {
            buf: [0; MAX_TASKS],
            head: 0,
            tail: 0,
        }
    }

    fn push(&mut self, t: u32) {
        let n = (self.tail + 1) % MAX_TASKS;
        if n != self.head {
            self.buf[self.tail] = t;
            self.tail = n;
        }
    }

    fn pop(&mut self) -> Option<u32> {
        if self.head == self.tail {
            return None;
        }
        let t = self.buf[self.head];
        self.head = (self.head + 1) % MAX_TASKS;
        Some(t)
    }
}

/// 伪造首跑帧:全零寄存器 + 入口地址,iretq 直接进任务。
fn init_frame(sp: u64, entry: u64) -> *mut Frame {
    let top = sp + (STACK_PAGES * 4096) as u64;
    let fr = (top - core::mem::size_of::<Frame>() as u64) as *mut Frame;
    unsafe {
        core::ptr::write_bytes(fr, 0, 1);
        (*fr).rip = entry;
        (*fr).cs = 0x18;
        (*fr).rflags = 0x202;
        (*fr).rsp = top;
        (*fr).ss = 0x10;
    }
    fr
}

unsafe fn tasks() -> *mut Task {
    core::ptr::addr_of_mut!(TASKS) as *mut Task
}

unsafe fn queue() -> &'static mut Queue {
    &mut *core::ptr::addr_of_mut!(QUEUE)
}

/// 创建内核任务(16KiB 栈,节点本地内存),入就绪队列。
pub fn spawn(entry: fn() -> !) -> Option<u32> {
    sched_preempt_disable(); // 持锁期间禁抢占,防 tick 里重入 QUEUE_LOCK
    QUEUE_LOCK.lock();
    let mut idx: Option<usize> = None;
    let tasks = unsafe { tasks() };
    for i in 0..MAX_TASKS {
        if unsafe { (*tasks.add(i)).stack == 0 } {
            idx = Some(i);
            break;
        }
    }
    let i = match idx {
        Some(i) => i,
        None => {
            QUEUE_LOCK.unlock();
            sched_preempt_enable();
            return None;
        }
    };
    let cpu = crate::idt::lapic_id() as usize;
    let node = crate::numa::node_for_lapic(cpu as u32).unwrap_or(0);
    let stack = match crate::numa::alloc_contig(node, STACK_PAGES) {
        Some(s) => s,
        None => {
            QUEUE_LOCK.unlock();
            sched_preempt_enable();
            return None;
        }
    };
    let fr = init_frame(stack, entry as u64);
    unsafe {
        *tasks.add(i) = Task {
            stack,
            sp: fr as u64,
            quantum: QUANTUM,
        };
        queue().push(i as u32);
    }
    QUEUE_LOCK.unlock();
    sched_preempt_enable();
    Some(i as u32)
}

/// 每核进入主循环前调用:idle 上下文就绪。
pub fn register_idle() {
    let cpu = crate::idt::lapic_id() as usize;
    CUR[cpu].store(IDLE, Ordering::Relaxed);
    IDLE_SP[cpu].store(0, Ordering::Relaxed);
}

/// tick 中断里调用:时间片用完则切栈换任务。
/// 旧任务现场留在旧栈(on_tick 自身调用链),新任务从 common_resume 恢复。
pub fn on_tick(fr: *mut Frame) {
    let cpu = crate::idt::lapic_id() as usize;
    if PREEMPT[cpu].load(Ordering::Relaxed) != 0 {
        return; // 持锁临界区,时间片冻结
    }
    let cur = CUR[cpu].load(Ordering::Relaxed);

    if cur != IDLE {
        QUEUE_LOCK.lock();
        let t = unsafe { &mut *tasks().add(cur as usize) };
        if t.quantum > 1 {
            t.quantum -= 1;
            QUEUE_LOCK.unlock();
            return;
        }
        t.quantum = QUANTUM;
        unsafe { queue().push(cur) };
        let next = unsafe { queue().pop() };
        QUEUE_LOCK.unlock();
        match next {
            Some(n) if n == cur => return,
            Some(n) => switch_to(cur, n, fr, cpu),
            None => {
                // 队列空,回 idle
                CUR[cpu].store(IDLE, Ordering::Relaxed);
                let isp = IDLE_SP[cpu].load(Ordering::Relaxed);
                if isp == 0 {
                    return;
                }
                unsafe { (*tasks().add(cur as usize)).sp = fr as u64 };
                sched_jump(isp);
            }
        }
    } else {
        // idle 被 tick 抢占:有任务就抢一个
        QUEUE_LOCK.lock();
        let next = unsafe { queue().pop() };
        QUEUE_LOCK.unlock();
        if let Some(n) = next {
            IDLE_SP[cpu].store(fr as u64, Ordering::Relaxed);
            CUR[cpu].store(n, Ordering::Relaxed);
            let sp = unsafe { (*tasks().add(n as usize)).sp };
            sched_jump(sp);
        }
    }
}

unsafe extern "C" {
    static common_resume: [u8; 0];
}

/// 切栈:旧任务帧记在旧栈,新任务栈顶就是它的帧,直接跳到恢复序列。
fn switch_to(cur: u32, next: u32, fr: *mut Frame, cpu: usize) -> ! {
    unsafe { (*tasks().add(cur as usize)).sp = fr as u64 };
    CUR[cpu].store(next, Ordering::Relaxed);
    let sp = unsafe { (*tasks().add(next as usize)).sp };
    sched_jump(sp);
}

unsafe extern "C" {
    static sched_switch_asm: [u8; 0];
}

/// 切栈跳转:目标栈顶即新任务的帧,换栈后从 common_resume 恢复。
/// 经 asm sym 取址,避免 extern fn 的 GOT 间接寻址。
fn sched_jump(sp: u64) -> ! {
    let jump: extern "C" fn(u64) -> !;
    unsafe {
        asm!(
            "lea {0}, [rip + {1}]",
            out(reg) jump,
            sym sched_switch_asm,
            options(nostack, readonly)
        );
        jump(sp);
    }
}