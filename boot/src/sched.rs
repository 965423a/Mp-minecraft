//! 抢占调度:每核 1ms tick 触发,时间片轮转。
//! 任务独占 16KiB 内核栈;切换 = 交换中断帧(Frame),首跑用伪造帧。
//! idle 不入队:每核 CUR=0xFFFF 表示当前在 idle 上下文。

use core::arch::asm;
use core::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};

use crate::idt::Frame;
use crate::spinlock::SpinLock;

pub const MAX_TASKS: usize = 64;
pub const STACK_PAGES: usize = 4;
pub const QUANTUM: u32 = 10; // 10 个 tick = 10ms


/// 读任务表槽(shell 诊断用)。返回 (stack, sp, quantum),空槽返回 None。
pub fn task_info(i: usize) -> Option<(u64, u64, u32)> {
    if i >= MAX_TASKS {
        return None;
    }
    unsafe {
        let t = &*tasks().add(i);
        if t.stack == 0 {
            None
        } else {
            Some((t.stack, t.sp, t.quantum))
        }
    }
}

/// 就绪队列长度(shell 诊断用)。
pub fn queue_len() -> usize {
    unsafe {
        if QUEUE.tail >= QUEUE.head {
            QUEUE.tail - QUEUE.head
        } else {
            MAX_TASKS - QUEUE.head + QUEUE.tail
        }
    }
}

const IDLE: u32 = 0xFFFF;
const IDLE_TAG: u32 = 0x8000; // 队列中空闲核标记:IDLE_TAG | cpu

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
/// 累计上下文切换次数(stats 命令)。
pub static SWITCHES: AtomicU64 = AtomicU64::new(0);
/// 退出标记:exit() 后等 tick 切走并释放槽位。
static DEAD: [AtomicBool; 64] = [const { AtomicBool::new(false) }; 64];

/// 当前任务主动退出(genworld 等一次性任务用)。
/// 标记 dead 后 spin,下一个 tick 切回 idle 并释放槽位。
pub fn exit() -> ! {
    let cpu = crate::idt::lapic_id() as usize;
    let cur = CUR[cpu].load(Ordering::Relaxed);
    if cur != IDLE {
        DEAD[cur as usize].store(true, Ordering::Relaxed);
    }
    loop {
        core::hint::spin_loop();
    }
}
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
/// 空闲核被切走时把自己的标记(IDLE_TAG|cpu)入队尾,
/// 队列轮转回自己的标记时恢复现场(主流程继续)。
pub fn on_tick(fr: *mut Frame) {
    let cpu = crate::idt::lapic_id() as usize;
    if PREEMPT[cpu].load(Ordering::Relaxed) != 0 {
        return; // 持锁临界区,时间片冻结
    }
    let cur = CUR[cpu].load(Ordering::Relaxed);

    if cur != IDLE {
        QUEUE_LOCK.lock();
        if DEAD[cur as usize].load(Ordering::Relaxed) {
            // 已退出:释放槽位,回 idle(下一轮 tick 恢复主流程)
            DEAD[cur as usize].store(false, Ordering::Relaxed);
            unsafe { (*tasks().add(cur as usize)).stack = 0 };
            CUR[cpu].store(IDLE, Ordering::Relaxed);
            SWITCHES.fetch_add(1, Ordering::Relaxed);
            QUEUE_LOCK.unlock();
            let isp = IDLE_SP[cpu].load(Ordering::Relaxed);
            if isp == 0 {
                return;
            }
            check_frame(isp, "dead");
            sched_jump(isp);
        }
        let t = unsafe { &mut *tasks().add(cur as usize) };
        if t.quantum > 1 {
            t.quantum -= 1;
            QUEUE_LOCK.unlock();
            return;
        }
        t.quantum = QUANTUM;
        t.sp = fr as u64; // 现场先写进任务表(锁内),再入队,防他核 pop 到旧帧
        unsafe { queue().push(cur) };
        let mut next: Option<u32> = None;
        let mut nsp = 0u64;
        for _ in 0..MAX_TASKS {
            match unsafe { queue().pop() } {
                Some(n) if n & IDLE_TAG != 0 => {
                    if n as usize == IDLE_TAG as usize | cpu {
                        // 轮转回本核空闲标记:恢复主流程现场
                        QUEUE_LOCK.unlock();
                        let isp = IDLE_SP[cpu].load(Ordering::Relaxed);
                        CUR[cpu].store(IDLE, Ordering::Relaxed);
                        if isp == 0 {
                            return;
                        }
                        check_frame(isp, "task-restore");
                        sched_jump(isp);
                    }
                    unsafe { queue().push(n) }; // 别人的标记,放回队尾
                }
                Some(n) => {
                    next = Some(n);
                    nsp = unsafe { (*tasks().add(n as usize)).sp };
                    check_nsp(n, nsp, "task");
                    break;
                }
                None => break,
            }
        }
        QUEUE_LOCK.unlock();
        match next {
            Some(n) if n == cur => return,
            Some(n) => {
                CUR[cpu].store(n, Ordering::Relaxed);
                check_frame(nsp, "switch");
                sched_jump(nsp);
            }
            None => {
                // 队列只剩自己的标记或空,回 idle
                CUR[cpu].store(IDLE, Ordering::Relaxed);
                let isp = IDLE_SP[cpu].load(Ordering::Relaxed);
                if isp == 0 {
                    return;
                }
                check_frame(isp, "task-none");
                sched_jump(isp);
            }
        }
    } else {
        // idle 被 tick 抢占:有任务就抢一个
        let mut next: Option<u32> = None;
        let mut nsp = 0u64;
        QUEUE_LOCK.lock();
        for _ in 0..MAX_TASKS {
            match unsafe { queue().pop() } {
                Some(n) if n & IDLE_TAG != 0 => {
                    if n as usize == IDLE_TAG as usize | cpu {
                        // 恢复本核主流程(之前被切走)
                        QUEUE_LOCK.unlock();
                        let isp = IDLE_SP[cpu].load(Ordering::Relaxed);
                        if isp == 0 {
                            return;
                        }
                        SWITCHES.fetch_add(1, Ordering::Relaxed);
                        check_frame(isp, "idle-restore");
                        sched_jump(isp);
                    }
                    unsafe { queue().push(n) };
                }
                Some(n) => {
                    next = Some(n);
                    nsp = unsafe { (*tasks().add(n as usize)).sp };
                    check_nsp(n, nsp, "idle");
                    break;
                }
                None => break,
            }
        }
        if let Some(n) = next {
            IDLE_SP[cpu].store(fr as u64, Ordering::Relaxed);
            CUR[cpu].store(n, Ordering::Relaxed);
            unsafe { queue().push(IDLE_TAG | cpu as u32) }; // 本核标记入队(锁内)
        }
        QUEUE_LOCK.unlock();
        if let Some(_n) = next {
            SWITCHES.fetch_add(1, Ordering::Relaxed);
            check_frame(nsp, "idle->task");
            sched_jump(nsp);
        }
    }
}

unsafe extern "C" {
    static common_resume: [u8; 0];
}

unsafe extern "C" {
    static sched_switch_asm: [u8; 0];
}

/// 检查目标帧有效性:rip 为零说明帧被破坏,打印现场后停机。
/// 打印任务表(崩溃诊断用)。
pub fn dump_tasks() {
    unsafe {
        let tbase = tasks();
        for i in 0..MAX_TASKS {
            let x = &*tbase.add(i);
            if x.stack != 0 {
                crate::log!(
                    "sched:   t{i}: st={:#x} sp={:#x} q={}",
                    x.stack,
                    x.sp,
                    x.quantum
                );
            }
        }
    }
}
/// 校验切换目标帧地址是否在任务栈内,异常才打印(平时零日志)。
fn check_nsp(n: u32, nsp: u64, tag: &str) {
    unsafe {
        let st = (*tasks().add(n as usize)).stack;
        if st != 0 && (nsp < st || nsp >= st + (STACK_PAGES * 4096) as u64) {
            crate::log!(
                "sched: BAD NSP {tag} task={n} stack={st:#x} sp={nsp:#x}"
            );
        }
    }
}

fn check_frame(sp: u64, tag: &str) {
    unsafe {
        let fr = sp as *const Frame;
        let f = &*fr;
        if f.rip == 0 || f.rip < 0x100000 || f.cs != 0x18 || f.ss != 0x10 {
            let cpu = crate::idt::lapic_id() as usize;
            crate::log!(
                "sched: BAD FRAME {tag} sp={sp:#x} cpu{cpu} CUR={:#x} IDLE_SP={:#x} rip=0 cs={:#x} rflags={:#x} rsp={:#x} ss={:#x} err={:#x} vec={:#x}",
                CUR[cpu].load(Ordering::Relaxed),
                IDLE_SP[cpu].load(Ordering::Relaxed),
                f.cs,
                f.rflags,
                f.rsp,
                f.ss,
                f.err,
                f.vec
            );
            let p = sp as *const u64;
            let p = sp as *const u64;
            crate::log!(
                "sched:   frm: {:x} {:x} {:x} {:x} | {:x} {:x} {:x} {:x} | {:x} {:x} {:x} {:x} | {:x} {:x} {:x} {:x}",
                p.read_volatile(),
                p.add(1).read_volatile(),
                p.add(2).read_volatile(),
                p.add(3).read_volatile(),
                p.add(4).read_volatile(),
                p.add(5).read_volatile(),
                p.add(6).read_volatile(),
                p.add(7).read_volatile(),
                p.add(8).read_volatile(),
                p.add(9).read_volatile(),
                p.add(10).read_volatile(),
                p.add(11).read_volatile(),
                p.add(12).read_volatile(),
                p.add(13).read_volatile(),
                p.add(14).read_volatile(),
                p.add(15).read_volatile()
            );
            let qp = unsafe { core::ptr::addr_of_mut!(QUEUE).cast::<u8>() as *const u32 };
            crate::log!(
                "sched:   q: {:x} {:x} {:x} {:x} {:x} {:x} {:x} {:x}",
                qp.read_volatile(),
                qp.add(1).read_volatile(),
                qp.add(2).read_volatile(),
                qp.add(3).read_volatile(),
                qp.add(4).read_volatile(),
                qp.add(5).read_volatile(),
                qp.add(6).read_volatile(),
                qp.add(7).read_volatile()
            );
            let tbase = tasks();
            for i in 0..MAX_TASKS {
                let t = &*tbase.add(i);
                if t.stack != 0 {
                    crate::log!(
                        "sched:   task{i}: stack={:#x} sp={:#x} q={}",
                        t.stack,
                        t.sp,
                        t.quantum
                    );
                }
            }
            loop {
                core::arch::asm!("cli");
                core::arch::asm!("hlt");
            }
        }
    }
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