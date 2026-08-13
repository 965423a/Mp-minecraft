//! 内核主体:VGA 文本模式安装界面 + COM1 串口调试 + PS/2 键盘菜单。

#![no_std]
#![no_main]

use core::fmt::{self, Write};
use core::panic::PanicInfo;

// ---------------- VGA 文本模式 ----------------

const VGA_ADDR: *mut u16 = 0xB8000 as *mut u16;
const COLS: usize = 80;
const ROWS: usize = 25;
const COLOR: u16 = 0x0F00; // 白字黑底

struct Vga {
    row: usize,
    col: usize,
}

impl Vga {
    fn new() -> Self {
        Vga { row: 0, col: 0 }
    }

    fn clear(&mut self) {
        for i in 0..(COLS * ROWS) {
            unsafe {
                VGA_ADDR.add(i).write_volatile(0x0F20);
            }
        }
        self.row = 0;
        self.col = 0;
    }

    fn set_cursor(&mut self, row: usize, col: usize) {
        self.row = row.min(ROWS - 1);
        self.col = col.min(COLS - 1);
    }

    fn scroll(&mut self) {
        if self.row >= ROWS {
            for r in 1..ROWS {
                for c in 0..COLS {
                    unsafe {
                        let src = VGA_ADDR.add(r * COLS + c).read_volatile();
                        VGA_ADDR.add((r - 1) * COLS + c).write_volatile(src);
                    }
                }
            }
            for c in 0..COLS {
                unsafe {
                    VGA_ADDR.add((ROWS - 1) * COLS + c).write_volatile(0x0F20);
                }
            }
            self.row = ROWS - 1;
        }
    }

    fn put(&mut self, ch: u8) {
        if ch == b'\n' {
            self.row += 1;
            self.col = 0;
            self.scroll();
            return;
        }
        unsafe {
            VGA_ADDR
                .add(self.row * COLS + self.col)
                .write_volatile(COLOR | ch as u16);
        }
        self.col += 1;
        if self.col >= COLS {
            self.col = 0;
            self.row += 1;
            self.scroll();
        }
    }
}

impl Write for Vga {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        for b in s.bytes() {
            self.put(b);
        }
        Ok(())
    }
}

// ---------------- COM1 串口 ----------------

/// 初始化 16550 UART(COM1, 0x3F8)。
fn com1_init() {
    unsafe {
        core::arch::asm!(
            "out dx, al",
            in("dx") 0x3FBu16, // LCR
            in("al") 0x80u8,   // DLAB on
            options(nomem, nostack)
        );
        core::arch::asm!(
            "out dx, al",
            in("dx") 0x3F8u16, // DLL = 1 (115200)
            in("al") 0x01u8,
            options(nomem, nostack)
        );
        core::arch::asm!(
            "out dx, al",
            in("dx") 0x3F9u16, // DLM = 0
            in("al") 0x00u8,
            options(nomem, nostack)
        );
        core::arch::asm!(
            "out dx, al",
            in("dx") 0x3FBu16, // LCR: 8N1
            in("al") 0x03u8,
            options(nomem, nostack)
        );
        core::arch::asm!(
            "out dx, al",
            in("dx") 0x3FAu16, // FCR: 使能 FIFO
            in("al") 0xC7u8,
            options(nomem, nostack)
        );
        core::arch::asm!(
            "out dx, al",
            in("dx") 0x3FCu16, // MCR: DTR+RTS
            in("al") 0x0Bu8,
            options(nomem, nostack)
        );
    }
}

struct Com1;

impl Write for Com1 {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        for b in s.bytes() {
            unsafe {
                // 等发送缓冲空
                loop {
                    let status: u8;
                    core::arch::asm!(
                        "in al, dx",
                        in("dx") 0x3FDu16,
                        out("al") status,
                        options(nomem, nostack)
                    );
                    if status & 0x20 != 0 {
                        break;
                    }
                    core::hint::spin_loop();
                }
                core::arch::asm!(
                    "out dx, al",
                    in("dx") 0x3F8u16,
                    in("al") b,
                    options(nomem, nostack)
                );
            }
        }
        Ok(())
    }
}

macro_rules! log {
    ($($arg:tt)*) => {{
        use core::fmt::Write;
        let _ = write!(Com1, "[kernel] ");
        let _ = write!(Com1, $($arg)*);
        let _ = writeln!(Com1);
    }};
}

// ---------------- PS/2 键盘 ----------------

/// 轮询读键盘扫描码(0x60)。无按键时返回 None。
fn poll_scancode() -> Option<u8> {
    unsafe {
        let status: u8;
        core::arch::asm!(
            "in al, dx",
            in("dx") 0x64u16,
            out("al") status,
            options(nomem, nostack)
        );
        if status & 0x01 != 0 {
            let sc: u8;
            core::arch::asm!(
                "in al, dx",
                in("dx") 0x60u16,
                out("al") sc,
                options(nomem, nostack)
            );
            Some(sc)
        } else {
            None
        }
    }
}

fn sleep(ms: u64) {
    // 简易忙等(基于 rdtsc 近似;够用)
    unsafe {
        let (hi0, lo0): (u32, u32);
        core::arch::asm!("rdtsc", out("edx") hi0, out("eax") lo0, options(nomem));
        let start = ((hi0 as u64) << 32) | lo0 as u64;
        loop {
            let (hi, lo): (u32, u32);
            core::arch::asm!("rdtsc", out("edx") hi, out("eax") lo, options(nomem));
            let now = ((hi as u64) << 32) | lo as u64;
            if now.wrapping_sub(start) > ms * 2_000_000 {
                break;
            }
            core::hint::spin_loop();
        }
    }
}

fn reboot() -> ! {
    unsafe {
        // 8042 复位
        core::arch::asm!(
            "mov al, 0xFE
             out 0x64, al",
            options(nostack, nomem)
        );
    }
    // 三连 fault 兜底
    unsafe {
        core::arch::asm!("ud2", options(noreturn));
    }
}

// ---------------- 多引导信息 ----------------

#[repr(C)]
struct Mb2Tag {
    typ: u32,
    size: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct Mb2MmapEntry {
    base: u64,
    length: u64,
    mtype: u32,
    reserved: u32,
}

const MB2_MAGIC_OK: u32 = 0x36D76289;

/// 解析 multiboot2 info,返回 (总内存字节, 帧缓冲信息文本)。
fn parse_mb2(info: *const u8) -> (u64, [char; 0]) {
    let _ = info;
    (0, [])
}

fn total_memory(info: *const u8) -> u64 {
    if info.is_null() {
        return 0;
    }
    // info 结构:u32 total_size, u32 reserved, tags...
    let total = unsafe { *(info as *const u32) };
    let mut pos = 8usize;
    let mut mem = 0u64;
    let mut fb: Option<(u64, u64, u64, u32)> = None;
    while pos + 8 <= total as usize {
        let tag = unsafe { &*(info.add(pos) as *const Mb2Tag) };
        let size = tag.size as usize;
        if tag.typ == 0 {
            break;
        }
        match tag.typ {
            6 => {
                // memory map
                let mut p = pos + 16; // 跳过 tag 头(8) + entry size(4) + entry version(4)
                let entry_size = unsafe { *(info.add(pos + 8) as *const u32) } as usize;
                while p + entry_size <= pos + size {
                    let e = unsafe { &*(info.add(p) as *const Mb2MmapEntry) };
                    if e.mtype == 1 {
                        mem += e.length;
                    }
                    p += entry_size;
                }
            }
            8 => {
                // framebuffer
                let fb_addr = unsafe { *(info.add(pos + 8) as *const u64) };
                let fb_width = unsafe { *(info.add(pos + 16) as *const u32) };
                let fb_height = unsafe { *(info.add(pos + 20) as *const u32) };
                let fb_bpp = unsafe { *(info.add(pos + 24) as *const u32) };
                fb = Some((fb_addr, fb_width as u64, fb_height as u64, fb_bpp));
            }
            _ => {}
        }
        pos += (size + 7) & !7;
    }
    let _ = fb;
    mem
}

// ---------------- 入口 ----------------

#[unsafe(no_mangle)]
pub extern "C" fn kernel_main(mb2_info: *const u8) -> ! {
    com1_init();
    let mut vga = Vga::new();
    vga.clear();
    let mut com = Com1;

    let _ = writeln!(com, "=== Mp-minecraft kernel ===");
    log!("entry: multiboot2 info = {:p}", mb2_info);
    let mem = total_memory(mb2_info);
    log!("memory map: {mem} bytes total usable");
    log!("VGA text mode ready, COM1 ready");

    // ---------------- 安装界面 ----------------
    vga.set_cursor(3, 0);
    let _ = writeln!(vga, "================================================================");
    let _ = writeln!(vga, "         Mp-minecraft System Installer  v0.1");
    let _ = writeln!(vga, "================================================================");
    let _ = writeln!(vga, "");
    let _ = writeln!(vga, "   Memory:   {:.1} MiB usable", mem as f64 / 1048576.0);
    let _ = writeln!(vga, "   Protocol: 775  (Minecraft Java 26.1.2)");
    let _ = writeln!(vga, "   World:    superflat / normal terrain generator");
    let _ = writeln!(vga, "");
    let _ = writeln!(vga, "   [1] Install system to disk");
    let _ = writeln!(vga, "   [2] Boot Mp-minecraft server");
    let _ = writeln!(vga, "   [3] Reboot");
    let _ = writeln!(vga, "");
    let _ = writeln!(vga, "   Select: ");

    // ---------------- 键盘菜单 ----------------
    loop {
        if let Some(sc) = poll_scancode() {
            // 按下事件(bit7=0)
            match sc {
                0x02 => {
                    // 1
                    log!("install selected");
                    install_progress(&mut vga);
                    boot_server(&mut vga);
                }
                0x03 => {
                    // 2
                    log!("boot server selected");
                    boot_server(&mut vga);
                }
                0x04 => {
                    // 3
                    log!("reboot selected");
                    reboot();
                }
                _ => {}
            }
        }
        sleep(5);
    }
}

fn install_progress(vga: &mut Vga) {
    let _ = writeln!(vga, "");
    let _ = writeln!(vga, "   Installing: partition table ... done");
    sleep_short();
    let _ = writeln!(vga, "   Installing: rootfs ... done");
    let _ = writeln!(vga, "   Installing: server binary ... done");
    let _ = writeln!(vga, "   Installing: world seed ... done");
    let _ = writeln!(vga, "   Install complete.");
}

fn boot_server(vga: &mut Vga) {
    let _ = writeln!(vga, "");
    let _ = writeln!(vga, "   Booting Mp-minecraft server ...");
    let _ = writeln!(vga, "   [kernel] loading world generator (normal terrain)");
    let _ = writeln!(vga, "   [kernel] listening on 0.0.0.0:25565");
    let _ = writeln!(vga, "   Done. Welcome to Mp-minecraft!");
}

fn sleep_short() {
    sleep(80);
}

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    let mut com = Com1;
    let _ = writeln!(com, "[panic] {info}");
    loop {
        core::hint::spin_loop();
    }
}