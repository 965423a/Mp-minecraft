//! 内核主体:VGA 文本模式 + HZK16 汉字字形 + 命令行 + 拼音输入法 + 服务器控制台。

#![no_std]
#![no_main]

extern crate alloc;

mod acpi;
mod emb;
mod fb;
mod mcver;
mod sqldb;
mod kalloc;
mod fs;
mod idt;
mod kb;
mod numa;
mod sched;
mod smp;
mod spinlock;

use core::alloc::{GlobalAlloc, Layout};
use core::fmt::{self, Write};
use core::panic::PanicInfo;

// ---------------- 数据包 ----------------

/// GB2312 16x16 点阵字库(区 16-87,每区 94 字,每字 32 字节)。
static HZK16: &[u8] = include_bytes!("../data/hzk16.bin");
static PINYIN_PACK: &[u8] = include_bytes!("../data/pinyin_pack.bin");

#[global_allocator]
static KALLOC: kalloc::KernelAlloc = kalloc::KernelAlloc;

fn parse_dec_u64(b: &[u8]) -> Option<u64> {
    if b.is_empty() {
        return None;
    }
    let mut v: u64 = 0;
    for c in b {
        if !c.is_ascii_digit() {
            return None;
        }
        v = v.saturating_mul(10).saturating_add((c - b'0') as u64);
    }
    Some(v)
}

// ---------------- 端口 I/O ----------------

fn outb(port: u16, val: u8) {
    unsafe {
        core::arch::asm!("out dx, al", in("dx") port, in("al") val, options(nomem, nostack));
    }
}

fn inb(port: u16) -> u8 {
    let v: u8;
    unsafe {
        core::arch::asm!("in al, dx", in("dx") port, out("al") v, options(nomem, nostack));
    }
    v
}

pub(crate) fn u32le_pub(p: *const u8) -> u32 {
    unsafe {
        p.read_volatile() as u32
            | ((p.add(1).read_volatile() as u32) << 8)
            | ((p.add(2).read_volatile() as u32) << 16)
            | ((p.add(3).read_volatile() as u32) << 24)
    }
}

pub(crate) fn u64le_pub(p: *const u8) -> u64 {
    u32le_pub(p) as u64 | ((u32le_pub(unsafe { p.add(4) }) as u64) << 32)
}

// ---------------- VGA 文本模式 + 汉字字形 ----------------

pub(crate) const VGA_MEM: *mut u16 = 0xB8000 as *mut u16; // 文本显存(plane 0/1)
const FONT_MEM: *mut u8 = 0xA0000 as *mut u8; // 字库 RAM(plane 2)
pub(crate) const COLS: usize = 80;
pub(crate) const ROWS: usize = 25;
pub(crate) const ATTR: u8 = 0x0F; // 白字黑底
const BLACK: u16 = 0x0000; // 黑底黑字(窗框黑条)

/// 逻辑屏幕缓冲:ASCII(< 0x80)或 GB2312 码(0xA1A1 起)或 0(空)。
pub(crate) static mut CELL: [u16; COLS * ROWS] = [0; COLS * ROWS];
/// 已分配字形槽数(调试用)。
static mut SLOT_NEXT: usize = 0;

/// 写平面掩码:0x03 = plane 0+1(文本),0x04 = plane 2(字库)。
fn set_map_mask(mask: u8) {
    outb(0x3C4, 0x02);
    outb(0x3C5, mask);
}

/// 设置 Sequencer Memory Mode(0x3C4 idx 4)的位。
/// bit1 = odd/even 使能,bit2 = chain-4 使能。
fn mm_set(bit: u8, on: bool) {
    outb(0x3C4, 0x04);
    let v = inb(0x3C5);
    outb(0x3C5, if on { v | bit } else { v & !bit });
}

fn font_init() {
    set_map_mask(0x03);
}

/// 上传一个 8x16 字形(16 字节,每行 8 bit)到槽位(字符码 c)。
/// 线性平面模式(odd/even off)下,CPU 地址 0xA0000+off 对应 plane[off],
/// 字符码 c 的字形位于 plane2 偏移 c*32。
fn font_upload(c: usize, glyph: &[u8; 16]) {
    mm_set(0x02, false);
    set_map_mask(0x04);
    let base = unsafe { FONT_MEM.add(c * 32) };
    for i in 0..16 {
        unsafe {
            base.add(i * 2).write_volatile(glyph[i]);
            base.add(i * 2 + 1).write_volatile(0);
        }
    }
    set_map_mask(0x03);
    mm_set(0x02, true);
}

/// 从 HZK16 取 16x16 字形(32 字节:第 i 行 = 左 byte[2i] + 右 byte[2i+1])。
fn hzk_glyph(gb: u16) -> Option<[u8; 32]> {
    let q = (gb >> 8) as u8 - 0xA0;
    let w = (gb & 0xFF) as u8 - 0xA0;
    if q < 16 || q > 87 || w < 1 || w > 94 {
        return None;
    }
    let off = ((q as usize - 16) * 94 + (w as usize - 1)) * 32;
    if off + 32 > HZK16.len() {
        return None;
    }
    let mut g = [0u8; 32];
    g.copy_from_slice(&HZK16[off..off + 32]);
    Some(g)
}

fn is_hanzi(c: u16) -> bool {
    c >= 0xA1A1
}

fn vga_write(offset: usize, ch: u16) {
    set_map_mask(0x03);
    unsafe {
        VGA_MEM.add(offset).write_volatile(ch);
    }
}

/// 渲染逻辑格到显存(ASCII 1 格,汉字 2 格并分配字形槽)。
/// framebuffer 激活时走像素渲染。
fn render_cell(pos: usize, disp_off: &mut usize) {
    let cell = unsafe { CELL[pos] };
    if fb::active() {
        fb::render_cell(pos, cell);
        *disp_off += if is_hanzi(cell) { 2 } else { 1 };
        return;
    }
    let a = (ATTR as u16) << 8;
    if !is_hanzi(cell) {
        vga_write(*disp_off, a | (cell & 0x7F) as u16);
        *disp_off += 1;
        return;
    }
    let slot = unsafe { SLOT_NEXT };
    if slot >= 64 {
        // 槽用尽:显示 '?' 占位
        vga_write(*disp_off, a | b'?' as u16);
        *disp_off += 2;
        return;
    }
    unsafe {
        SLOT_NEXT = slot + 1;
    }
    let c0 = 0x80 + slot * 2;
    let c1 = c0 + 1;
    if let Some(g) = hzk_glyph(cell) {
        let mut left = [0u8; 16];
        let mut right = [0u8; 16];
        for i in 0..16 {
            left[i] = g[i * 2];
            right[i] = g[i * 2 + 1];
        }
        font_upload(c0, &left);
        font_upload(c1, &right);
        vga_write(*disp_off, a | c0 as u16);
        vga_write(*disp_off + 1, a | c1 as u16);
    } else {
        vga_write(*disp_off, a | b'?' as u16);
        vga_write(*disp_off + 1, a | b' ' as u16);
    }
    *disp_off += 2;
}

fn render_all() {
    unsafe {
        SLOT_NEXT = 0;
    }
    if fb::active() {
        fb::render_all();
        return;
    }
    let mut off = 0usize;
    for pos in 0..(COLS * ROWS) {
        render_cell(pos, &mut off);
    }
}

fn screen_clear() {
    unsafe {
        let p = core::ptr::addr_of_mut!(CELL) as *mut u16;
        for i in 0..COLS * ROWS {
            p.add(i).write(0);
        }
    }
    if fb::active() {
        fb::clear_all();
        return;
    }
    for off in 0..(COLS * ROWS) {
        vga_write(off, BLACK | 0x20);
    }
}

/// 弹层(候选框/IME 状态条)写一个普通格:文本模式直接写显存,framebuffer 画像素。
fn text_put_cell(off: usize, ch: u8, attr: u8) {
    if fb::active() {
        fb::draw_off(off, ch as u16, attr);
    } else {
        vga_write(off, ((attr as u16) << 8) | ch as u16);
    }
}

/// 弹层写一个汉字(文本模式占字形槽 2 格;framebuffer 画 16x16)。
fn text_put_gb(off: usize, gb: u16, attr: u8) {
    if fb::active() {
        fb::draw_off(off, gb, attr);
    } else {
        let slot = unsafe { SLOT_NEXT };
        if slot >= 64 {
            vga_write(off, ((attr as u16) << 8) | b'?' as u16);
            vga_write(off + 1, ((attr as u16) << 8) | b' ' as u16);
            return;
        }
        unsafe {
            SLOT_NEXT = slot + 1;
        }
        let c0 = 0x80 + slot * 2;
        let c1 = c0 + 1;
        if let Some(g) = hzk_glyph(gb) {
            let mut left = [0u8; 16];
            let mut right = [0u8; 16];
            for i in 0..16 {
                left[i] = g[i * 2];
                right[i] = g[i * 2 + 1];
            }
            font_upload(c0, &left);
            font_upload(c1, &right);
            vga_write(off, ((attr as u16) << 8) | c0 as u16);
            vga_write(off + 1, ((attr as u16) << 8) | c1 as u16);
        } else {
            vga_write(off, ((attr as u16) << 8) | b'?' as u16);
            vga_write(off + 1, ((attr as u16) << 8) | b' ' as u16);
        }
    }
}

/// 弹层清格(黑条/黑底)。
fn text_clear_cell(off: usize) {
    if fb::active() {
        fb::fill_off(off);
    } else {
        vga_write(off, BLACK | 0x20);
    }
}

// ---------------- 光标定位输出 ----------------

pub(crate) struct Vga {
    row: usize,
    col: usize,
}

impl Vga {
    fn new() -> Self {
        Vga { row: 0, col: 0 }
    }

    fn set_cursor(&mut self, row: usize, col: usize) {
        self.row = row.min(ROWS - 1);
        self.col = col.min(COLS - 1);
    }

    fn clear(&mut self) {
        screen_clear();
        self.row = 0;
        self.col = 0;
    }

    fn scroll(&mut self) {
        if self.row >= ROWS {
            unsafe {
                let p = core::ptr::addr_of_mut!(CELL) as *mut u16;
                for i in 0..(ROWS - 1) * COLS {
                    p.add(i).write(p.add(i + COLS).read());
                }
                for i in (ROWS - 1) * COLS..COLS * ROWS {
                    p.add(i).write(0);
                }
            }
            self.row = ROWS - 1;
            render_all();
        }
    }

    fn put(&mut self, ch: u16) {
        if self.row > ROWS + 5 {
            let mut com = Com1;
            let _ = writeln!(
                com,
                "[dbg] put: self={:p} row={} col={} ch={:#x}",
                self as *const Vga,
                self.row,
                self.col,
                ch
            );
            loop {
                core::hint::spin_loop();
            }
        }
        if ch == '\n' as u16 {
            self.row += 1;
            self.col = 0;
            self.scroll();
            return;
        }
        let pos = self.row * COLS + self.col;
        unsafe {
            CELL[pos] = ch;
        }
        let mut off = pos;
        render_cell(pos, &mut off);
        self.col += if is_hanzi(ch) { 2 } else { 1 };
        if self.col >= COLS {
            self.col = 0;
            self.row += 1;
            self.scroll();
        }
    }

    fn backspace(&mut self) {
        if self.col > 0 {
            self.col -= 1;
            let pos = self.row * COLS + self.col;
            unsafe {
                CELL[pos] = 0;
            }
            let mut off = pos;
            render_cell(pos, &mut off);
        }
    }

    fn backspace2(&mut self) {
        if self.col >= 2 {
            self.col -= 2;
            let pos = self.row * COLS + self.col;
            unsafe {
                CELL[pos] = 0;
            }
            let mut off = pos;
            render_cell(pos, &mut off);
        }
    }
}

impl Write for Vga {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        for b in s.bytes() {
            if b < 0x80 {
                self.put(b as u16);
            } else {
                // 不应出现:内部字符串均为 ASCII;汉字走 print_gb 路径
                self.put(b'?' as u16);
            }
        }
        Ok(())
    }
}

/// 按 GB2312 字节串输出(ASCII + 汉字)。
fn print_bytes(vga: &mut Vga, bytes: &[u8]) {
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if b < 0x80 {
            vga.put(b as u16);
            i += 1;
        } else if i + 1 < bytes.len() && b >= 0xA1 && bytes[i + 1] >= 0xA1 {
            let gb = ((b as u16) << 8) | bytes[i + 1] as u16;
            vga.put(gb);
            i += 2;
        } else {
            vga.put(b'?' as u16);
            i += 1;
        }
    }
}

// ---------------- COM1 串口 ----------------

fn com1_init() {
    unsafe {
        core::arch::asm!(
            "out dx, al",
            in("dx") 0x3FBu16,
            in("al") 0x80u8,
            options(nomem, nostack)
        );
        core::arch::asm!(
            "out dx, al",
            in("dx") 0x3F8u16,
            in("al") 0x01u8,
            options(nomem, nostack)
        );
        core::arch::asm!(
            "out dx, al",
            in("dx") 0x3F9u16,
            in("al") 0x00u8,
            options(nomem, nostack)
        );
        core::arch::asm!(
            "out dx, al",
            in("dx") 0x3FBu16,
            in("al") 0x03u8,
            options(nomem, nostack)
        );
        core::arch::asm!(
            "out dx, al",
            in("dx") 0x3FAu16,
            in("al") 0xC7u8,
            options(nomem, nostack)
        );
        core::arch::asm!(
            "out dx, al",
            in("dx") 0x3FCu16,
            in("al") 0x0Bu8,
            options(nomem, nostack)
        );
    }
}

pub struct Com1;
impl Write for Com1 {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        for b in s.bytes() {
            unsafe {
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

/// 轮询读 COM1 RX(16550)。无数据返回 None。不可达端口读回 0xFF 时视为无 COM。
fn com1_rx() -> Option<u8> {
    unsafe {
        let lsr: u8;
        core::arch::asm!(
            "in al, dx",
            in("dx") 0x3FDu16,
            out("al") lsr,
            options(nomem, nostack)
        );
        if lsr == 0xFF || lsr & 0x01 == 0 {
            return None;
        }
        let c: u8;
        core::arch::asm!(
            "in al, dx",
            in("dx") 0x3F8u16,
            out("al") c,
            options(nomem, nostack)
        );
        Some(c)
    }
}

/// 串口字符是否有效输入(可打印、回车、退格)。
fn com1_valid(ch: u8) -> bool {
    ch == b'\r' || ch == b'\n' || ch == 0x08 || ch == 0x7F || ch >= 0x20
}

/// 栈上格式化缓冲(日志行 ≤256B),经 C klogf 输出 COM1 + 内存环形缓冲。
pub struct LogBuf {
    buf: [u8; 256],
    len: usize,
}
impl LogBuf {
    pub fn new() -> Self {
        LogBuf { buf: [0; 256], len: 0 }
    }
    pub fn as_cstr(&mut self) -> *const u8 {
        self.buf[self.len] = 0;
        self.buf.as_ptr()
    }
}
impl core::fmt::Write for LogBuf {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        for b in s.bytes() {
            if self.len < self.buf.len() - 1 {
                self.buf[self.len] = b;
                self.len += 1;
            }
        }
        Ok(())
    }
}

#[cfg(feature = "klog")]
#[macro_export]
macro_rules! log {
    ($($arg:tt)*) => {{
        use core::fmt::Write;
        let mut b = crate::LogBuf::new();
        let _ = write!(b, "[kernel] ");
        let _ = write!(b, $($arg)*);
        unsafe {
            crate::klogf(2, b"%s\0".as_ptr(), b.as_cstr());
        }
    }};
}

#[cfg(not(feature = "klog"))]
#[macro_export]
macro_rules! log {
    ($($arg:tt)*) => {{}};
}

#[cfg(feature = "klog")]
unsafe extern "C" {
    pub fn klog_init();
    pub fn klogf(level: i32, fmt: *const u8, ...);
    pub fn kerr(code: i32, what: *const u8, a: u64, b: u64, c: u64);
}

/// 无 klog 构建(nolog 版本):未处理中断 dump 退化为串口标记。
#[cfg(not(feature = "klog"))]
pub unsafe fn kerr(_code: i32, _what: *const u8, _a: u64, _b: u64, _c: u64) {
    unsafe {
        core::arch::asm!("mov dx, 0x3f8; mov al, 0x4b; out dx, al", options(nomem, nostack));
    }
}

// ---------------- PS/2 键盘 ----------------

/// 轮询读键盘扫描码(0x60)。无 i8042 时恒 None。
fn poll_scancode() -> Option<u8> {
    if !kb::PS2_OK.load(core::sync::atomic::Ordering::Relaxed) {
        return None;
    }
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

/// set1 扫描码 → ASCII(0 表示无字符)。
const KEY_NORMAL: [u8; 128] = {
    let mut k = [0u8; 128];
    k[0x02] = b'1'; k[0x03] = b'2'; k[0x04] = b'3'; k[0x05] = b'4';
    k[0x06] = b'5'; k[0x07] = b'6'; k[0x08] = b'7'; k[0x09] = b'8';
    k[0x0A] = b'9'; k[0x0B] = b'0'; k[0x0C] = b'-'; k[0x0D] = b'=';
    k[0x0F] = b'\t';
    k[0x10] = b'q'; k[0x11] = b'w'; k[0x12] = b'e'; k[0x13] = b'r';
    k[0x14] = b't'; k[0x15] = b'y'; k[0x16] = b'u'; k[0x17] = b'i';
    k[0x18] = b'o'; k[0x19] = b'p'; k[0x1A] = b'['; k[0x1B] = b']';
    k[0x1E] = b'a'; k[0x1F] = b's'; k[0x20] = b'd'; k[0x21] = b'f';
    k[0x22] = b'g'; k[0x23] = b'h'; k[0x24] = b'j'; k[0x25] = b'k';
    k[0x26] = b'l'; k[0x27] = b';'; k[0x28] = b'\''; k[0x29] = b'`';
    k[0x2B] = b'\\'; k[0x2C] = b'z'; k[0x2D] = b'x'; k[0x2E] = b'c';
    k[0x2F] = b'v'; k[0x30] = b'b'; k[0x31] = b'n'; k[0x32] = b'm';
    k[0x33] = b','; k[0x34] = b'.'; k[0x35] = b'/';
    k[0x37] = b'*'; k[0x39] = b' ';
    k
};

/// shift 状态下的映射。
const KEY_SHIFT: [u8; 128] = {
    let mut k = [0u8; 128];
    k[0x02] = b'!'; k[0x03] = b'@'; k[0x04] = b'#'; k[0x05] = b'$';
    k[0x06] = b'%'; k[0x07] = b'^'; k[0x08] = b'&'; k[0x09] = b'*';
    k[0x0A] = b'('; k[0x0B] = b')'; k[0x0C] = b'_'; k[0x0D] = b'+';
    k[0x10] = b'Q'; k[0x11] = b'W'; k[0x12] = b'E'; k[0x13] = b'R';
    k[0x14] = b'T'; k[0x15] = b'Y'; k[0x16] = b'U'; k[0x17] = b'I';
    k[0x18] = b'O'; k[0x19] = b'P'; k[0x1A] = b'{'; k[0x1B] = b'}';
    k[0x1E] = b'A'; k[0x1F] = b'S'; k[0x20] = b'D'; k[0x21] = b'F';
    k[0x22] = b'G'; k[0x23] = b'H'; k[0x24] = b'J'; k[0x25] = b'K';
    k[0x26] = b'L'; k[0x27] = b':'; k[0x28] = b'"'; k[0x29] = b'~';
    k[0x2B] = b'|'; k[0x2C] = b'Z'; k[0x2D] = b'X'; k[0x2E] = b'C';
    k[0x2F] = b'V'; k[0x30] = b'B'; k[0x31] = b'N'; k[0x32] = b'M';
    k[0x33] = b'<'; k[0x34] = b'>'; k[0x35] = b'?';
    k[0x37] = b'*'; k[0x39] = b' ';
    k
};

// ---------------- 拼音输入法 ----------------

static mut IME_CN: bool = false;
static mut IME_PY: [u8; 8] = [0; 8];
static mut IME_PY_LEN: usize = 0;
static mut IME_CAND: [u16; 9] = [0; 9];
static mut IME_CAND_N: usize = 0;
static mut IME_CAND_ROW: usize = 0;
static mut CTRL: bool = false;

/// 按拼音前缀查表,填候选(最多 9),返回候选数。
fn ime_query() -> usize {
    let pack = PINYIN_PACK;
    if pack.len() < 2 {
        return 0;
    }
    let n = u16::from_le_bytes([pack[0], pack[1]]) as usize;
    let py_len = unsafe { IME_PY_LEN };
    let mut p = 2usize;
    let mut got = 0usize;
    for _ in 0..n {
        let plen = pack[p] as usize;
        p += 1;
        let pin = &pack[p..p + plen];
        p += plen;
        let clen = pack[p] as usize;
        p += 1;
        if clen == 0 {
            continue;
        }
        let mut m = py_len <= plen;
        if m {
            for i in 0..py_len {
                if unsafe { IME_PY[i] } != pin[i] {
                    m = false;
                    break;
                }
            }
        }
        if m {
            for k in 0..clen {
                if got >= 9 {
                    break;
                }
                let gb = u16::from_le_bytes([pack[p + k * 2], pack[p + k * 2 + 1]]);
                unsafe {
                    IME_CAND[got] = gb;
                }
                got += 1;
            }
        }
        p += clen * 2;
    }
    got
}

/// 清除候选框区域(候选行 + 上下黑条),恢复空行。
fn candidates_clear() {
    let row = unsafe { IME_CAND_ROW };
    if row == 0 {
        return;
    }
    let base = row * COLS;
    for off in base..base + COLS * 3 {
        text_clear_cell(off);
    }
    unsafe {
        IME_CAND_ROW = 0;
    }
}

/// 在光标下方画黑底候选窗框(上黑条 + "1:字 2:字 ..." + 下黑条)。
fn candidates_render(vga: &Vga) {
    let py_len = unsafe { IME_PY_LEN };
    let cn = unsafe { IME_CAND_N };
    if py_len == 0 || cn == 0 {
        candidates_clear();
        return;
    }
    let y = if vga.row + 1 <= ROWS - 3 { vga.row + 1 } else { ROWS - 3 };
    let base = y * COLS;
    // 先清 3 行(候选行 + 上下黑条),避免旧状态条残留
    for off in base..base + COLS * 3 {
        text_clear_cell(off);
    }
    for off in base..base + COLS {
        text_clear_cell(off);
    }
    // 候选行(黑底白字,左右各留 1 列黑)
    let mut off = base + COLS + 1;
    text_clear_cell(base + COLS);
    for i in 0..cn {
        if i > 0 {
            text_put_cell(off, b' ', ATTR);
            off += 1;
        }
        let d = (b'1' + i as u8) as u16;
        text_put_cell(off, d as u8, ATTR);
        off += 1;
        text_put_cell(off, b':', ATTR);
        off += 1;
        let gb = unsafe { IME_CAND[i] };
        text_put_gb(off, gb, ATTR);
        off += 2;
    }
    text_clear_cell(off);
    for off in (y + 2) * COLS..(y + 3) * COLS {
        text_clear_cell(off);
    }
    unsafe {
        IME_CAND_ROW = y;
    }
}

/// 输入法状态条(切换中/英时显示在候选区位置)。
fn ime_status(vga: &Vga) {
    let y = if vga.row + 1 <= ROWS - 3 { vga.row + 1 } else { ROWS - 3 };
    let base = y * COLS;
    for off in base..base + COLS * 2 {
        text_clear_cell(off);
    }
    let msg: &[u8] = if unsafe { IME_CN } {
        b"  [Chinese IME]  (a-z pinyin, 1-9 pick, Esc clear, Ctrl+Space toggle)"
    } else {
        b"  [English]  (Ctrl+Space: Chinese IME)"
    };
    let mut off = base + COLS;
    for &b in msg {
        text_put_cell(off, b, ATTR);
        off += 1;
    }
    unsafe {
        IME_CAND_ROW = y;
    }
}

fn ime_refresh(vga: &Vga) {
    unsafe {
        IME_CAND_N = 0;
    }
    let py_len = unsafe { IME_PY_LEN };
    if py_len > 0 {
        unsafe {
            IME_CAND_N = ime_query();
        }
    }
    candidates_render(vga);
}

fn ime_clear() {
    unsafe {
        IME_PY_LEN = 0;
        IME_CAND_N = 0;
    }
    candidates_clear();
}

/// 候选字上屏:写 GB 字节到 buf,并在屏幕上显示。
fn ime_commit(vga: &mut Vga, idx: usize, buf: &mut [u8], len: &mut usize) {
    if idx >= unsafe { IME_CAND_N } {
        return;
    }
    let gb = unsafe { IME_CAND[idx] };
    if *len + 2 <= buf.len() {
        buf[*len] = (gb >> 8) as u8;
        buf[*len + 1] = (gb & 0xFF) as u8;
        *len += 2;
    }
    vga.put(gb);
    ime_clear();
}

/// 读一行输入(最长 buf.len()-1 字节,汉字占 2 字节)。回车返回长度。
/// 输入源:PS/2 键盘(优先)+ COM1 串口。无键盘平台完全走串口。
fn read_line(vga: &mut Vga, prompt: &str, buf: &mut [u8]) -> usize {
    let _ = vga.write_str(prompt);
    let mut len = 0usize;
    let mut shift = false;
    loop {
        if let Some(c) = com1_rx() {
            if !com1_valid(c) {
                continue;
            }
            let ch = if c == b'\r' { b'\n' } else { c };
            if ch == b'\n' {
                vga.put('\n' as u16);
                ime_clear();
                return len;
            }
            if ch == 0x08 || ch == 0x7F {
                if unsafe { IME_PY_LEN } > 0 {
                    unsafe {
                        IME_PY_LEN -= 1;
                    }
                    ime_refresh(vga);
                    vga.backspace();
                } else if len > 0 {
                    if len >= 2 && buf[len - 2] >= 0xA1 && buf[len - 1] >= 0xA1 {
                        len -= 2;
                        vga.backspace2();
                    } else {
                        len -= 1;
                        vga.backspace();
                    }
                }
                continue;
            }
            if ch >= 0x20 && len < buf.len() - 1 {
                buf[len] = ch;
                len += 1;
                vga.put(ch as u16);
            }
            continue;
        }
        if let Some(sc) = poll_scancode() {
            if sc & 0x80 != 0 {
                // 松开事件
                if sc == 0xAA || sc == 0xB6 {
                    shift = false;
                }
                if sc == 0x9D {
                    unsafe {
                        CTRL = false;
                    }
                }
                continue;
            }
            match sc {
                0x1D => {
                    unsafe {
                        CTRL = true;
                    }
                }
                0x2A | 0x36 => shift = true,
                0x39 if unsafe { CTRL } => {
                    // Ctrl+Space:切换中/英
                    unsafe {
                        CTRL = false;
                        IME_CN = !IME_CN;
                        IME_PY_LEN = 0;
                        IME_CAND_N = 0;
                    }
                    ime_status(vga);
                }
                0x1C => {
                    vga.put('\n' as u16);
                    ime_clear();
                    return len;
                }
                0x01 => {
                    // Esc:清拼音与候选
                    unsafe {
                        IME_PY_LEN = 0;
                        IME_CAND_N = 0;
                    }
                    ime_refresh(vga);
                }
                0x0E => {
                    // 退格:有拼音删拼音,否则删行缓冲
                    if unsafe { IME_PY_LEN } > 0 {
                        unsafe {
                            IME_PY_LEN -= 1;
                        }
                        ime_refresh(vga);
                        vga.backspace(); // 拼音字母已上屏,不重绘
                    } else if len > 0 {
                        if len >= 2
                            && buf[len - 2] >= 0xA1
                            && buf[len - 1] >= 0xA1
                        {
                            len -= 2;
                            vga.backspace2();
                        } else {
                            len -= 1;
                            vga.backspace();
                        }
                    }
                }
                0x39 => {
                    if unsafe { IME_CN && IME_PY_LEN > 0 && IME_CAND_N > 0 } {
                        ime_commit(vga, 0, buf, &mut len);
                    } else if !unsafe { IME_CN } {
                        vga.put(b' ' as u16);
                        if len < buf.len() - 1 {
                            buf[len] = b' ';
                            len += 1;
                        }
                    }
                }
                _ => {
                    if unsafe { IME_CN } {
                        let ch = KEY_NORMAL[sc as usize];
                        if ch >= b'a' && ch <= b'z' && unsafe { IME_PY_LEN } < 8 {
                            unsafe {
                                IME_PY[IME_PY_LEN] = ch;
                                IME_PY_LEN += 1;
                            }
                            vga.put(ch as u16);
                            ime_refresh(vga);
                        } else if ch >= b'1' && ch <= b'9' {
                            let idx = (ch - b'1') as usize;
                            if idx < unsafe { IME_CAND_N } {
                                ime_commit(vga, idx, buf, &mut len);
                            }
                        } else if ch == b' ' {
                            if unsafe { IME_CAND_N } > 0 {
                                ime_commit(vga, 0, buf, &mut len);
                            }
                        }
                    } else {
                        let ch = if shift {
                            KEY_SHIFT[sc as usize]
                        } else {
                            KEY_NORMAL[sc as usize]
                        };
                        if ch != 0 && len < buf.len() - 1 {
                            buf[len] = ch;
                            len += 1;
                            vga.put(ch as u16);
                        }
                    }
                }
            }
        }
        sleep(5);
    }
}

// ---------------- 定时与重启 ----------------

fn sleep(ms: u64) {
    let tpu = smp::tsc_per_us_global();
    unsafe {
        let (hi0, lo0): (u32, u32);
        core::arch::asm!("rdtsc", out("edx") hi0, out("eax") lo0, options(nomem));
        let start = ((hi0 as u64) << 32) | lo0 as u64;
        loop {
            let (hi, lo): (u32, u32);
            core::arch::asm!("rdtsc", out("edx") hi, out("eax") lo, options(nomem));
            let now = ((hi as u64) << 32) | lo as u64;
            if now.wrapping_sub(start) > ms * tpu {
                break;
            }
            core::hint::spin_loop();
        }
    }
}

pub(crate) fn sleep_short() {
    sleep(80);
}

/// 重启链:① ACPI FADT reset 寄存器 ② 8042 复位 ③ 空 IDT triple fault。
/// Hyper-V Gen2 / 无 8042 平台靠 ①/③ 兜底。
fn reboot() -> ! {
    if let Some((port, val)) = acpi::fadt_reset() {
        unsafe {
            core::arch::asm!(
                "out dx, al",
                in("dx") port,
                in("al") val,
                options(nostack, nomem)
            );
        }
        sleep(300);
    }
    unsafe {
        core::arch::asm!(
            "mov al, 0xFE
             out 0x64, al",
            options(nostack, nomem)
        );
    }
    sleep(100);
    // triple fault:加载空 IDT 后触发 #UD → #GP(双错)→ 复位
    static mut EMPTY_IDT: [u8; 10] = [0; 10];
    unsafe {
        let p = core::ptr::addr_of_mut!(EMPTY_IDT);
        core::arch::asm!(
            "cli
             lidt [{0}]",
            in(reg) p,
            options(nostack, nomem)
        );
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

fn total_memory(info: *const u8) -> u64 {
    if info.is_null() {
        return 0;
    }
    let total = unsafe { *(info as *const u32) };
    let mut pos = 8usize;
    let mut mem = 0u64;
    while pos + 8 <= total as usize {
        let tag = unsafe { &*(info.add(pos) as *const Mb2Tag) };
        let size = tag.size as usize;
        if tag.typ == 0 {
            break;
        }
        if tag.typ == 6 {
            let mut p = pos + 16;
            let entry_size = unsafe { *(info.add(pos + 8) as *const u32) } as usize;
            while p + entry_size <= pos + size {
                let e = unsafe { &*(info.add(p) as *const Mb2MmapEntry) };
                if e.mtype == 1 {
                    mem += e.length;
                }
                p += entry_size;
            }
        }
        pos += (size + 7) & !7;
    }
    mem
}

// ---------------- 命令行 ----------------


/// 等待一个可打印键(映射 shift/忽略释放/回车返回 None=Enter)。
/// PS/2 + 串口双输入源。
fn wait_ascii(vga: &mut Vga) -> Option<u8> {
    let mut shift = false;
    loop {
        if let Some(c) = com1_rx() {
            if c == b'\r' || c == b'\n' {
                return None;
            }
            if c >= 0x20 {
                let _ = vga.put(c as u16);
                return Some(c);
            }
        }
        if let Some(sc) = poll_scancode() {
            if sc & 0x80 != 0 {
                if sc == 0xAA || sc == 0xB6 {
                    shift = false;
                }
                continue;
            }
            let c = if shift { KEY_SHIFT[sc as usize] } else { KEY_NORMAL[sc as usize] };
            if c == b'\n' {
                return None;
            }
            if c != 0 {
                let _ = vga.put(c as u16);
                return Some(c);
            }
        }
        sleep(5);
    }
}

/// switch 交互界面:先选目标(Minecraft / MySQL / MariaDB)。
fn switch_ui(vga: &mut Vga) {
    loop {
        vga.clear();
        vga.set_cursor(2, 0);
        let _ = writeln!(vga, "=========== switch: pick target ===========");
        let _ = writeln!(vga, "");
        let _ = writeln!(vga, "   1) Minecraft server version");
        let _ = writeln!(vga, "   2) MySQL   (database backend)");
        let _ = writeln!(vga, "   3) MariaDB (database backend)");
        let _ = writeln!(vga, "");
        let _ = writeln!(vga, "   q) cancel");
        let _ = writeln!(vga, "");
        let sel = wait_ascii(vga);
        match sel {
            Some(b'1') => mc_versions_ui(vga),
            Some(b'2') => {
                sqldb::switch_backend("mysql");
                let _ = writeln!(vga, "");
                let _ = writeln!(vga, "  switch: backend -> mysql (shared tablespace, data kept)");
                crate::log!("switch: backend -> mysql (shared tablespace)");
                sleep_short();
                sleep_short();
                return;
            }
            Some(b'3') => {
                sqldb::switch_backend("mariadb");
                let _ = writeln!(vga, "");
                let _ = writeln!(vga, "  switch: backend -> mariadb (shared tablespace, data kept)");
                crate::log!("switch: backend -> mariadb (shared tablespace)");
                sleep_short();
                sleep_short();
                return;
            }
            _ => return,
        }
    }
}

/// MC 版本选择界面(分页,数字选择,n/p 翻页)。
fn mc_versions_ui(vga: &mut Vga) {
    const PAGE: usize = 10;
    let total = mcver::VERSIONS.len();
    let pages = (total + PAGE - 1) / PAGE;
    let mut page = 0usize;
    loop {
        vga.clear();
        vga.set_cursor(2, 0);
        let _ = writeln!(
            vga,
            "=== switch: Minecraft versions (page {}/{} of {}) ===",
            page + 1,
            pages,
            total
        );
        let _ = writeln!(vga, "");
        let start = page * PAGE;
        let end = (start + PAGE).min(total);
        for (i, v) in mcver::VERSIONS[start..end].iter().enumerate() {
            let n = start + i + 1;
            let label = if n == 10 { 0 } else { n };
            let _ = writeln!(
                vga,
                "   {})  {:<10}  proto={:<4} maxY={}",
                label,
                v.0,
                v.1,
                v.3.world_max_y
            );
        }
        let _ = writeln!(vga, "");
        let _ = writeln!(vga, "   n) next page    p) prev page    b) back    q) quit");
        let _ = writeln!(vga, "");
        let k = wait_ascii(vga);
        match k {
            Some(b'n') => {
                if page + 1 < pages {
                    page += 1;
                }
            }
            Some(b'p') => {
                if page > 0 {
                    page -= 1;
                }
            }
            Some(b'b') => return,
            Some(b'q') => {
                crate::log!("switch: cancelled");
                return;
            }
            Some(c) if c.is_ascii_digit() => {
                let mut idx = (c - b'0') as usize;
                if idx == 0 {
                    idx = 10;
                }
                let gi = start + idx - 1;
                if gi < total {
                    let name = mcver::VERSIONS[gi].0;
                    if mcver::switch(name) {
                        let _ = writeln!(vga, "");
                        let _ = writeln!(
                            vga,
                            "  switch: now {} proto={} maxY={} bits={}",
                            mcver::cur_name(),
                            mcver::cur_protocol(),
                            mcver::cur_features().world_max_y,
                            mcver::cur_features().pack_bits
                        );
                        crate::log!(
                            "switch: now {} proto={} maxY={}",
                            mcver::cur_name(),
                            mcver::cur_protocol(),
                            mcver::cur_features().world_max_y
                        );
                        sleep_short();
                        sleep_short();
                        return;
                    }
                }
            }
            _ => {}
        }
    }
}

/// 取 ASCII 首词(命令),返回 (词, 剩余部分)。
fn split_ascii_word(line: &[u8]) -> (&[u8], &[u8]) {
    let mut i = 0;
    while i < line.len() && (line[i] == b' ' || line[i] == b'\t') {
        i += 1;
    }
    let start = i;
    while i < line.len() && line[i] != b' ' && line[i] != b'\t' {
        i += 1;
    }
    (&line[start..i], &line[i..])
}

/// 跳过剩余部分开头的空格,返回参数区。
fn trim_space(mut rest: &[u8]) -> &[u8] {
    while rest.first() == Some(&b' ') {
        rest = &rest[1..];
    }
    rest
}

/// 当前目录节点 id 与服务器运行状态(shell 与 systemctl/控制台共享)。
static mut CWD: usize = 0;
static mut SERVER_RUNNING: bool = false;

fn system_shell(vga: &mut Vga, eula: bool) -> ! {
    let _ = writeln!(vga, "");
    let _ = writeln!(vga, "  Mp-minecraft system shell. Type 'help'. Ctrl+Space: CN/EN IME");
    // 无人值守(Hyper-V Gen2 等无 i8042):5s 无串口输入则自动启动服务器
    let mut no_kbd = !kb::PS2_OK.load(core::sync::atomic::Ordering::Relaxed);
    if no_kbd {
        let _ = writeln!(
            vga,
            "  Headless: no PS/2 keyboard. Auto-starting server in 5s (serial input cancels)"
        );
        let mut idle: u64 = 0;
        while idle < 5000 {
            if com1_rx().is_some() {
                no_kbd = false;
                break;
            }
            sleep(5);
            idle += 5;
        }
        if no_kbd {
            let _ = writeln!(vga, "  Auto-starting server ...");
            unsafe {
                SERVER_RUNNING = true;
            }
            let _ = writeln!(vga, "  [server] loading world generator (normal terrain)");
            let _ = writeln!(vga, "  [server] listening on 0.0.0.0:25565");
            let _ = writeln!(vga, "  [server] Done. Welcome to Mp-minecraft!");
            server_console(vga);
        }
    }
    loop {
        let mut buf = [0u8; 128];
        let n = read_line(vga, "mcs> ", &mut buf);
        let (cmd, rest) = split_ascii_word(&buf[..n]);
        if !cmd.is_empty() {
            log!("cmd: {}", core::str::from_utf8(cmd).unwrap_or("?"));
        }
        match cmd {
            b"help" => {
                let _ = writeln!(vga, "  standard commands:");
                let _ = writeln!(vga, "    pwd                     print working directory");
                let _ = writeln!(vga, "    cd [path]               change directory");
                let _ = writeln!(vga, "    ls [path]               list directory (ls -l style)");
                let _ = writeln!(vga, "    cat <file>              show file contents");
                let _ = writeln!(vga, "    head [-n N] <file>      first N lines (default 10)");
                let _ = writeln!(vga, "    wc <file>               line/word/byte count");
                let _ = writeln!(vga, "    grep <pat> <file>       print matching lines");
                let _ = writeln!(vga, "    echo <text>             print text");
                let _ = writeln!(vga, "    clear                   clear screen");
                let _ = writeln!(vga, "    ps                      process table (kernel tasks)");
                let _ = writeln!(vga, "    free                    memory per NUMA node");
                let _ = writeln!(vga, "    df                      mcsrootfs usage");
                let _ = writeln!(vga, "    uname [-a]              system info");
                let _ = writeln!(vga, "    whoami                  current user");
                let _ = writeln!(vga, "    hostname                host name");
                let _ = writeln!(vga, "    uptime                  system uptime");
                let _ = writeln!(vga, "    stats                   scheduler statistics");
                let _ = writeln!(vga, "    ver                     version info");
                let _ = writeln!(vga, "  Mp-minecraft commands:");
                let _ = writeln!(vga, "    systemctl               service control (start/stop/status)");
                let _ = writeln!(vga, "    tasks                   kernel task table");
                let _ = writeln!(vga, "    mem                     usable memory per NUMA node");
                let _ = writeln!(vga, "    numa                    NUMA topology + local alloc test");
                let _ = writeln!(vga, "    genworld                parallel world generation test");
                let _ = writeln!(vga, "    pkt                     protocol pipeline check (varint/status/chunk)");
                let _ = writeln!(vga, "    switch                  MC version or mysql|mariadb backend");
                let _ = writeln!(vga, "    sql                     SQL (default: current backend, or sql mysql|mariadb <stmt>)");
                let _ = writeln!(vga, "    dbtest                  database self-test (CRUD, both backends)");
                let _ = writeln!(vga, "    dbiso                   shared tablespace interop check (seamless switch)");
                let _ = writeln!(vga, "    eula                    EULA status");
                let _ = writeln!(vga, "    install                 install system (demo)");
                let _ = writeln!(vga, "    ctrls                   Minecraft server console");
                let _ = writeln!(vga, "    reboot                  restart");
            }
            b"tasks" => {
                let _ = writeln!(vga, "  task table:");
                let mut any = false;
                for i in 0..crate::sched::MAX_TASKS {
                    if let Some(t) = crate::sched::task_info(i) {
                        any = true;
                        let _ = writeln!(
                            vga,
                            "    [{i}] stack={:#x} sp={:#x} q={}",
                            t.0,
                            t.1,
                            t.2
                        );
                    }
                }
                if !any {
                    let _ = writeln!(vga, "    (no tasks)");
                }
                let _ = writeln!(
                    vga,
                    "    ready queue: {} entries, {} cpus online",
                    crate::sched::queue_len(),
                    crate::smp::cpu_count()
                );
                crate::sched::dump_tasks();
                crate::log!(
                    "tasks: queue_len={} cpus={}",
                    crate::sched::queue_len(),
                    crate::smp::cpu_count()
                );
            }
            b"uptime" => {
                let ms = idt::tick_total();
                let _ = writeln!(
                    vga,
                    "  uptime: {}s ({} ms, {} ticks)",
                    ms / 1000,
                    ms,
                    ms
                );
                crate::log!("uptime: {}s ({} ticks)", ms / 1000, ms);
            }
            b"stats" => {
                let sw = crate::sched::SWITCHES.load(core::sync::atomic::Ordering::Relaxed);
                let _ = writeln!(vga, "  stats: switches={sw}");
                for c in 0..crate::smp::cpu_count() {
                    let t = idt::tick_cpu(c);
                    let _ = writeln!(vga, "  stats: cpu{c} ticks={t}");
                }
                crate::log!(
                    "stats: switches={} ticks={}",
                    sw,
                    idt::tick_total()
                );
            }
            b"dbiso" => {
                let _ = writeln!(vga, "  == shared tablespace interop check ==");
                let out = sqldb::interop_probe();
                for line in out.lines() {
                    let _ = writeln!(vga, "{line}");
                    crate::log!("sql: {line}");
                }
                let ok = sqldb::interop_ok();
                let _ = writeln!(
                    vga,
                    "  interop: {} (mysql wrote isoprobe, mariadb reads it)",
                    if ok { "ok" } else { "FAIL" }
                );
                crate::log!("dbiso: interop {}", if ok { "ok" } else { "FAIL" });
                let out = sqldb::execute(sqldb::mysql(), "drop table isoprobe");
                for line in out.lines() {
                    let _ = writeln!(vga, "{line}");
                    crate::log!("sql: {line}");
                }
            }
            b"dbtest" => {
                let _ = writeln!(vga, "  == mysqld selftest ==");
                let out = sqldb::selftest(sqldb::mysql());
                for line in out.lines() {
                    let _ = writeln!(vga, "{line}");
                    crate::log!("sql: {line}");
                }
                let _ = writeln!(vga, "  == mariadb selftest ==");
                let out = sqldb::selftest(sqldb::mariadb());
                for line in out.lines() {
                    let _ = writeln!(vga, "{line}");
                    crate::log!("sql: {line}");
                }
            }
            b"sql" => {
                let rest = trim_space(rest);
                let (db, stmt) = if rest.starts_with(b"mysql ") {
                    (sqldb::mysql(), core::str::from_utf8(trim_space(&rest[6..])).unwrap_or(""))
                } else if rest.starts_with(b"mariadb ") {
                    (
                        sqldb::mariadb(),
                        core::str::from_utf8(trim_space(&rest[8..])).unwrap_or(""),
                    )
                } else {
                    let cur = sqldb::current_backend();
                    let db = if cur == "mariadb" { sqldb::mariadb() } else { sqldb::mysql() };
                    (db, core::str::from_utf8(rest).unwrap_or(""))
                };
                let which = if core::ptr::eq(db, sqldb::mariadb()) { "mariadb" } else { "mysql" };
                if !sqldb::server_running(db) {
                    let _ = writeln!(
                        vga,
                        "  sql: {which} server is not running (systemctl start {which}ld)"
                    );
                    crate::log!("sql: refused, {which} not running");
                } else {
                    let out = sqldb::execute(db, stmt);
                    for line in out.lines() {
                        let _ = writeln!(vga, "{line}");
                        crate::log!("sql: {line}");
                    }
                    crate::log!("sql[{which}]: executed '{}'", stmt);
                }
            }
            b"switch" => {
                if rest.is_empty() {
                    switch_ui(vga);
                    continue;
                }
                let name = core::str::from_utf8(trim_space(rest)).unwrap_or("");
                if name.eq_ignore_ascii_case("mysql") || name.eq_ignore_ascii_case("mysqld") || name.eq_ignore_ascii_case("mariadb") {
                    let cur = sqldb::current_backend();
                    if sqldb::switch_backend(name) {
                        let now = sqldb::current_backend();
                        let _ = writeln!(
                            vga,
                            "  switch: database backend {} -> {} (shared tablespace, data kept)",
                            cur,
                            now
                        );
                        crate::log!("switch: backend {} -> {} (shared tablespace)", cur, now);
                    }
                } else if mcver::switch(name) {
                    let _ = writeln!(
                        vga,
                        "  switch: now {} proto={} maxY={} bits={}",
                        mcver::cur_name(),
                        mcver::cur_protocol(),
                        mcver::cur_features().world_max_y,
                        mcver::cur_features().pack_bits
                    );
                    crate::log!(
                        "switch: now {} proto={} maxY={}",
                        mcver::cur_name(),
                        mcver::cur_protocol(),
                        mcver::cur_features().world_max_y
                    );
                } else {
                    let _ = writeln!(vga, "  switch: unknown '{}' (MC version or mysql|mariadb)", name);
                    crate::log!("switch: unknown '{}'", name);
                }
            }
            b"pkt" => {
                let (w1, _) = split_ascii_word(rest);
                let seed = parse_dec_u64(w1).unwrap_or(0xC0FFEE);
                emb::cmd_pkt(vga, seed);
            }
            b"genworld" => {
                let (w1, r2) = split_ascii_word(rest);
                let (w2, _) = split_ascii_word(r2);
                let seed = parse_dec_u64(w1).unwrap_or(0xC0FFEE);
                let jobs = parse_dec_u64(w2).unwrap_or(32) as usize;
                emb::cmd_genworld(vga, seed, jobs);
            }
            b"mem" => {
                for n in 0..numa::node_count() {
                    let (mb, free) = numa::node_mem(n);
                    let _ = writeln!(
                        vga,
                        "  node {}: {} MiB usable ({} frames free)",
                        n,
                        mb,
                        free
                    );
                }
            }
            b"numa" => {
                let ncnt = numa::node_count();
                let _ = writeln!(vga, "  NUMA topology: {ncnt} nodes");
                for n in 0..ncnt {
                    let (mb, free) = numa::node_mem(n);
                    let allocs = numa::node_allocs(n);
                    let mut laps = [0u32; 64];
                    let nl = numa::node_lapics(n, &mut laps);
                    let _ = write!(
                        vga,
                        "    node {} (domain {}): {} MiB, {} free, {} allocs, cpus:",
                        n,
                        numa::node_id(n),
                        mb,
                        free,
                        allocs
                    );
                    for i in 0..nl {
                        let _ = write!(vga, " {:#x}", laps[i]);
                    }
                    let _ = writeln!(vga);
                }
                if ncnt > 1 {
                    let _ = writeln!(vga, "  distance matrix:");
                    let _ = write!(vga, "    d |");
                    for j in 0..ncnt {
                        let _ = write!(vga, " {:>3}", j);
                    }
                    let _ = writeln!(vga);
                    for i in 0..ncnt {
                        let _ = write!(vga, "    {} |", i);
                        for j in 0..ncnt {
                            let _ = write!(vga, " {:>3}", numa::node_distance(i, j));
                        }
                        let _ = writeln!(vga);
                    }
                }
                let _ = writeln!(vga, "  allocation policies:");
                let mut frames = [0u64; 16];
                let mut got = 0;
                for n in 0..ncnt {
                    if let Some(p) = numa::alloc_local(n) {
                        frames[got] = p;
                        got += 1;
                        let owner = numa::node_of(p);
                        let _ = writeln!(
                            vga,
                            "    alloc_local({n}) -> 0x{p:x} (owner node {owner})"
                        );
                    }
                }
                for _ in 0..(ncnt * 2) {
                    if let Some(p) = numa::alloc_interleave() {
                        frames[got] = p;
                        got += 1;
                        let owner = numa::node_of(p);
                        let _ = writeln!(vga, "    alloc_interleave -> 0x{p:x} (node {owner})");
                    }
                }
                let _ = writeln!(vga, "  freeing {} frames...", got);
                for i in 0..got {
                    numa::free(frames[i]);
                }
                let _ = writeln!(vga, "  done");
            }
            b"ver" => {
                let _ = writeln!(
                    vga,
                    "  Mp-minecraft System v0.1  (protocol {}, MC {})",
                    mcver::cur_protocol(),
                    mcver::cur_name()
                );
            }
            b"eula" => {
                let _ = writeln!(
                    vga,
                    "  EULA: {}",
                    if eula { "accepted" } else { "rejected" }
                );
            }
            b"pwd" => {
                let mut p = [0u8; 96];
                let len = fs::full_path(unsafe { CWD }, &mut p);
                let _ = vga.write_str("  ");
                print_bytes(vga, &p[..len]);
                let _ = writeln!(vga, "");
                crate::log!(
                    "pwd: {}",
                    core::str::from_utf8(&p[..len]).unwrap_or("?")
                );
            }
            b"cd" => {
                let arg = trim_space(rest);
                let target = if arg.is_empty() {
                    0
                } else {
                    match fs::resolve(unsafe { CWD }, arg) {
                        Some(id) if fs::is_dir(id) => id,
                        _ => {
                            let _ = writeln!(vga, "  cd: no such directory");
                            continue;
                        }
                    }
                };
                unsafe {
                    CWD = target;
                }
            }
            b"ls" => {
                let arg = trim_space(rest);
                let dir = if arg.is_empty() {
                    unsafe { CWD }
                } else {
                    match fs::resolve(unsafe { CWD }, arg) {
                        Some(id) => id,
                        None => {
                            let _ = writeln!(vga, "  ls: no such file or directory");
                            continue;
                        }
                    }
                };
                if !fs::is_dir(dir) {
                    let _ = writeln!(vga, "  ls: not a directory");
                    continue;
                }
                let mut c = fs::first_child(dir);
                while let Some(id) = c {
                    if fs::is_dir(id) {
                        let _ = writeln!(
                            vga,
                            "  drwxr-xr-x  {:5}  {}/",
                            fs::size(id),
                            core::str::from_utf8(fs::name(id)).unwrap_or("?")
                        );
                    } else {
                        let _ = writeln!(
                            vga,
                            "  -rw-r--r--  {:5}  {}",
                            fs::size(id),
                            core::str::from_utf8(fs::name(id)).unwrap_or("?")
                        );
                    }
                    crate::log!(
                        "ls: {}",
                        core::str::from_utf8(fs::name(id)).unwrap_or("?")
                    );
                    c = fs::next(id);
                }
            }
            b"cat" => {
                let arg = trim_space(rest);
                if arg.is_empty() {
                    let _ = writeln!(vga, "  cat: missing operand");
                    continue;
                }
                match fs::resolve(unsafe { CWD }, arg) {
                    Some(id) if !fs::is_dir(id) => {
                        print_bytes(vga, fs::content(id));
                        let _ = writeln!(vga, "");
                        crate::log!(
                            "cat: {}",
                            core::str::from_utf8(fs::content(id)).unwrap_or("?")
                        );
                    }
                    _ => {
                        let _ = writeln!(vga, "  cat: no such file");
                        crate::log!("cat: no such file");
                    }
                }
            }
            b"echo" => {
                let arg = trim_space(rest);
                if !arg.is_empty() {
                    print_bytes(vga, arg);
                    let _ = writeln!(vga, "");
                }
            }
            b"clear" => {
                vga.clear();
            }
            b"ps" => {
                let _ = writeln!(vga, "  PID  ST  STACK       SP         NAME");
                let _ = writeln!(vga, "  -----------------------------------");
                for i in 0..crate::sched::MAX_TASKS {
                    if let Some(t) = crate::sched::task_info(i) {
                        let st = if t.2 == 0 { "R" } else { "S" };
                        let _ = writeln!(
                            vga,
                            "  {:>3}  {}   {:#010x}  {:#010x}",
                            i, st, t.0, t.1
                        );
                    }
                }
                let _ = writeln!(
                    vga,
                    "  ready queue: {} entries, {} cpus online",
                    crate::sched::queue_len(),
                    crate::smp::cpu_count()
                );
                crate::log!(
                    "ps: queue_len={} cpus={}",
                    crate::sched::queue_len(),
                    crate::smp::cpu_count()
                );
            }
            b"free" => {
                let (mut total_mb, mut total_free) = (0u64, 0u64);
                for n in 0..numa::node_count() {
                    let (mb, free) = numa::node_mem(n);
                    total_mb += mb;
                    total_free += free;
                    let _ = writeln!(
                        vga,
                        "  node {}: {} MiB usable, {} KiB free ({} frames)",
                        n,
                        mb,
                        free * 4,
                        free
                    );
                }
                let _ = writeln!(
                    vga,
                    "  total: {} MiB usable, {} KiB free",
                    total_mb,
                    total_free * 4
                );
                crate::log!(
                    "free: {} MiB usable, {} KiB free across {} nodes",
                    total_mb,
                    total_free * 4,
                    numa::node_count()
                );
            }
            b"df" => {
                let mut bytes = 0u32;
                let mut files = 0usize;
                let mut dirs = 0usize;
                for id in 0..fs::node_count() {
                    if fs::is_dir(id) {
                        dirs += 1;
                    } else {
                        files += 1;
                        bytes += fs::size(id);
                    }
                }
                let _ = writeln!(
                    vga,
                    "  Filesystem  Inodes  Dirs  Files  Size(B)",
                    );
                let _ = writeln!(
                    vga,
                    "  mcsrootfs   {:>5}  {:>4}  {:>5}  {:>7}",
                    fs::node_count(),
                    dirs,
                    files,
                    bytes
                );
                crate::log!("df: inodes={} dirs={} files={} bytes={}", fs::node_count(), dirs, files, bytes);
            }
            b"uname" => {
                let arg = trim_space(rest);
                if arg == b"-a" || arg == b"--all" {
                    let _ = writeln!(
                        vga,
                        "  Mp-minecraft mcs 1.0.0 {} x86_64 Mp-minecraft/SMP",
                        crate::mcver::cur_name()
                    );
                } else {
                    let _ = writeln!(vga, "  Mp-minecraft");
                    crate::log!("uname: Mp-minecraft");
                }
                crate::log!(
                    "uname: Mp-minecraft mcs 1.0.0 {} x86_64",
                    crate::mcver::cur_name()
                );
            }
            b"whoami" => {
                let _ = writeln!(vga, "  root");
                crate::log!("whoami: root");
            }
            b"hostname" => {
                let _ = writeln!(vga, "  mcs");
                crate::log!("hostname: mcs");
            }
            b"head" => {
                let arg = trim_space(rest);
                let mut lines = 10usize;
                let mut path: &[u8] = arg;
                if let Some(rest2) = arg.strip_prefix(b"-n ") {
                    let mut it = rest2.splitn(2, |c| *c == b' ');
                    if let Some(ps) = it.next() {
                        if let Ok(n) = core::str::from_utf8(ps).unwrap_or("0").parse::<usize>() {
                            lines = n;
                            path = it.next().unwrap_or(b"");
                        }
                    }
                }
                if path.is_empty() {
                    let _ = writeln!(vga, "  head: missing operand");
                    continue;
                }
                match fs::resolve(unsafe { CWD }, path) {
                    Some(id) if !fs::is_dir(id) => {
                        let c = fs::content(id);
                        let mut n = 0;
                        let mut start = 0usize;
                        for (i, b) in c.iter().enumerate() {
                            if *b == b'\n' {
                                print_bytes(vga, &c[start..=i]);
                                n += 1;
                                start = i + 1;
                                if n >= lines {
                                    break;
                                }
                            }
                        }
                        if n < lines && start < c.len() {
                            print_bytes(vga, &c[start..]);
                            let _ = writeln!(vga, "");
                        }
                    }
                    _ => {
                        let _ = writeln!(vga, "  head: no such file");
                    }
                }
                crate::log!("head: {} lines", lines);
            }
            b"wc" => {
                let arg = trim_space(rest);
                if arg.is_empty() {
                    let _ = writeln!(vga, "  wc: missing operand");
                    continue;
                }
                match fs::resolve(unsafe { CWD }, arg) {
                    Some(id) if !fs::is_dir(id) => {
                        let c = fs::content(id);
                        let mut lines = 0usize;
                        let mut words = 0usize;
                        let mut in_word = false;
                        for b in c {
                            if *b == b'\n' {
                                lines += 1;
                            }
                            if *b == b' ' || *b == b'\n' || *b == b'\t' {
                                in_word = false;
                            } else if !in_word {
                                words += 1;
                                in_word = true;
                            }
                        }
                        let name = core::str::from_utf8(fs::name(id)).unwrap_or("?");
                        let _ = writeln!(vga, "  {:>6} {:>6} {:>6} {}", lines, words, c.len(), name);
                        crate::log!("wc: {} {} {} {}", lines, words, c.len(), name);
                    }
                    _ => {
                        let _ = writeln!(vga, "  wc: no such file");
                    }
                }
            }
            b"grep" => {
                let arg = trim_space(rest);
                let mut it = arg.splitn(2, |c| *c == b' ');
                let pat = it.next().unwrap_or(b"");
                let path = it.next().unwrap_or(b"");
                if pat.is_empty() || path.is_empty() {
                    let _ = writeln!(vga, "  grep: usage: grep <pattern> <file>");
                    continue;
                }
                match fs::resolve(unsafe { CWD }, path) {
                    Some(id) if !fs::is_dir(id) => {
                        let c = fs::content(id);
                        let mut start = 0usize;
                        for (i, b) in c.iter().enumerate() {
                            if *b == b'\n' || i + 1 == c.len() {
                                let end = if *b == b'\n' { i + 1 } else { c.len() };
                                if c[start..end].windows(pat.len()).any(|w| w == pat) {
                                    print_bytes(vga, &c[start..end]);
                                    crate::log!(
                                        "grep: {}",
                                        core::str::from_utf8(&c[start..end]).unwrap_or("?")
                                    );
                                }
                                start = i + 1;
                            }
                        }
                    }
                    _ => {
                        let _ = writeln!(vga, "  grep: no such file");
                    }
                }
            }
            b"systemctl" => {
                systemctl(vga, trim_space(rest));
            }
            b"install" => {
                install_progress(vga);
            }
            b"ctrls" => {
                server_console(vga);
            }
            b"reboot" => {
                let _ = writeln!(vga, "  rebooting...");
                reboot();
            }
            b"" => {}
            _ => {
                let _ = writeln!(
                    vga,
                    "  unknown command '{}'. Type 'help'.",
                    core::str::from_utf8(cmd).unwrap_or("?")
                );
            }
        }
    }
}

/// 服务单元表。
const UNITS: &[(&[u8], &[u8])] = &[
    (b"mc-server.service", b"Mp-minecraft server (0.0.0.0:25565)"),
    (b"mysqld.service", b"MySQL in-kernel SQL server (127.0.0.1:3306)"),
    (b"mariadb.service", b"MariaDB in-kernel SQL server (127.0.0.1:3307)"),
    (b"console.service", b"server console interface (ctrls)"),
];

/// systemd 风格:省略 ".service" 后缀时也匹配。
fn unit_matches(unit: &[u8], name: &[u8]) -> bool {
    if unit == name {
        return true;
    }
    name.ends_with(b".service") && unit == &name[..name.len() - 8]
}

fn unit_status(vga: &mut Vga, unit: &[u8], running: bool) {
    for (name, desc) in UNITS {
        if unit_matches(unit, name) {
            let _ = writeln!(
                vga,
                "  {} - {}",
                core::str::from_utf8(unit).unwrap_or("?"),
                if running { "active (running)" } else { "inactive (dead)" }
            );
            let _ = write!(vga, "       Loaded: yes  Desc: ");
            let _ = vga.write_str(core::str::from_utf8(desc).unwrap_or("?"));
            let _ = writeln!(vga, "");
            return;
        }
    }
    let _ = writeln!(
        vga,
        "  unit '{}' not found",
        core::str::from_utf8(unit).unwrap_or("?")
    );
}

/// systemctl 子命令:status / list-units / start / stop / restart <unit>。
fn systemctl(vga: &mut Vga, args: &[u8]) {
    let (sub, unit) = split_ascii_word(args);
    match sub {
        b"status" => {
            let (running, made) = emb::server_stats();
            let db = sqldb::mysql();
            let dbrun = sqldb::server_running(db);
            let mrun = sqldb::server_running(sqldb::mariadb());
            let unit = trim_space(unit);
            if unit.is_empty() {
                let _ = writeln!(vga, "  systemctl status: showing all units");
                for (name, _) in UNITS {
                    let r = if *name == b"mc-server.service" {
                        running
                    } else if *name == b"mysqld.service" {
                        dbrun
                    } else if *name == b"mariadb.service" {
                        mrun
                    } else {
                        false
                    };
                    let _ = writeln!(
                        vga,
                        "    {}  {}",
                        core::str::from_utf8(name).unwrap_or("?"),
                        if r { "active (running)" } else { "inactive (dead)" }
                    );
                }
                if running {
                    let _ = writeln!(vga, "    mc-server.service: {made} chunks generated so far");
                }
                if dbrun {
                    let _ = writeln!(
                        vga,
                        "    mysqld.service: {} queries executed",
                        sqldb::query_count(sqldb::mysql())
                    );
                }
                if mrun {
                    let _ = writeln!(
                        vga,
                        "    mariadb.service: {} queries executed",
                        sqldb::query_count(sqldb::mariadb())
                    );
                }
                crate::log!(
                    "systemctl: status mc={running} mysql={dbrun} mariadb={mrun} chunks={made}"
                );
            } else if unit_matches(unit, b"mysqld.service") {
                unit_status(vga, unit, dbrun);
                crate::log!(
                    "systemctl: status {unit:?} running={dbrun} queries={}",
                    sqldb::query_count(sqldb::mysql())
                );
            } else if unit_matches(unit, b"mariadb.service") {
                unit_status(vga, unit, mrun);
                crate::log!(
                    "systemctl: status {unit:?} running={mrun} queries={}",
                    sqldb::query_count(sqldb::mariadb())
                );
            } else {
                unit_status(vga, unit, running);
                crate::log!(
                    "systemctl: status {unit:?} running={running} chunks={made}",
                    made = made
                );
            }
        }
        b"list-units" => {
            let _ = writeln!(vga, "  UNIT                LOAD  ACTIVE  SUB");
            for (name, _) in UNITS {
                let _ = writeln!(
                    vga,
                    "  {}  loaded active",
                    core::str::from_utf8(name).unwrap_or("?")
                );
            }
        }
        b"start" => {
            let unit = trim_space(unit);
            if unit.is_empty() {
                let _ = writeln!(vga, "  systemctl: start requires a unit");
                return;
            }
            if unit_matches(unit, b"mc-server.service") {
                let (running, _) = emb::server_stats();
                if running {
                    let _ = writeln!(vga, "  mc-server.service already running");
                } else if emb::server_start(0xC0FFEE) {
                    unsafe {
                        SERVER_RUNNING = true;
                    }
                    crate::log!("systemctl: mc-server.service task spawned");
                    let _ = writeln!(vga, "  starting mc-server.service ...");
                    let _ = writeln!(vga, "  [server] world generator task spawned (seed 0xC0FFEE)");
                    let _ = writeln!(vga, "  [server] generating chunks, console via 'ctrls'");
                } else {
                    let _ = writeln!(vga, "  mc-server.service failed to start (task table full?)");
                }
            } else if unit_matches(unit, b"mysqld.service") {
                let db = sqldb::mysql();
                if sqldb::server_running(db) {
                    let _ = writeln!(vga, "  mysqld.service already running");
                } else {
                    sqldb::server_start(db);
                    crate::log!("systemctl: mysqld.service started");
                    let _ = writeln!(vga, "  starting mysqld.service ...");
                    let _ = writeln!(vga, "  [mysql] server listening on 127.0.0.1:3306");
                    let _ = writeln!(vga, "  [mysql] use 'sql mysql <statement>' to execute queries");
                }
            } else if unit_matches(unit, b"mariadb.service") {
                let db = sqldb::mariadb();
                if sqldb::server_running(db) {
                    let _ = writeln!(vga, "  mariadb.service already running");
                } else {
                    sqldb::server_start(db);
                    crate::log!("systemctl: mariadb.service started");
                    let _ = writeln!(vga, "  starting mariadb.service ...");
                    let _ = writeln!(vga, "  [mariadb] server listening on 127.0.0.1:3307");
                    let _ = writeln!(vga, "  [mariadb] use 'sql mariadb <statement>' to execute queries");
                }
            } else if unit_matches(unit, b"console.service") {
                let _ = writeln!(vga, "  console.service: use 'ctrls' to attach");
            } else {
                unit_status(vga, unit, false);
            }
        }
        b"stop" => {
            let unit = trim_space(unit);
            if unit_matches(unit, b"mc-server.service") {
                if unsafe { SERVER_RUNNING } {
                    unsafe {
                        SERVER_RUNNING = false;
                    }
                    let _ = writeln!(vga, "  stopping mc-server.service ...");
                    let _ = writeln!(vga, "  [server] stopped");
                } else {
                    let _ = writeln!(vga, "  mc-server.service is not running");
                }
            } else if unit_matches(unit, b"mysqld.service") {
                let db = sqldb::mysql();
                if sqldb::server_running(db) {
                    sqldb::server_stop(db);
                    let _ = writeln!(vga, "  stopping mysqld.service ...");
                    let _ = writeln!(vga, "  [mysql] server stopped");
                } else {
                    let _ = writeln!(vga, "  mysqld.service is not running");
                }
            } else if unit_matches(unit, b"mariadb.service") {
                let db = sqldb::mariadb();
                if sqldb::server_running(db) {
                    sqldb::server_stop(db);
                    let _ = writeln!(vga, "  stopping mariadb.service ...");
                    let _ = writeln!(vga, "  [mariadb] server stopped");
                } else {
                    let _ = writeln!(vga, "  mariadb.service is not running");
                }
            } else if unit.is_empty() {
                let _ = writeln!(vga, "  systemctl: stop requires a unit");
            } else {
                unit_status(vga, unit, false);
            }
        }
        b"restart" => {
            let unit = trim_space(unit);
            if unit_matches(unit, b"mc-server.service") {
                unsafe {
                    SERVER_RUNNING = false;
                }
                let _ = writeln!(vga, "  restarting mc-server.service ...");
                unsafe {
                    SERVER_RUNNING = true;
                }
                let _ = writeln!(vga, "  [server] Done. Welcome to Mp-minecraft!");
            } else if unit.is_empty() {
                let _ = writeln!(vga, "  systemctl: restart requires a unit");
            } else {
                unit_status(vga, unit, false);
            }
        }
        b"" => {
            let _ = writeln!(vga, "  systemctl [status|list-units|start|stop|restart] [unit]");
        }
        _ => {
            let _ = writeln!(vga, "  systemctl: unknown subcommand");
        }
    }
}

fn server_console(vga: &mut Vga) {
    let _ = writeln!(vga, "");
    let _ = writeln!(vga, "  === Mp-minecraft server console ===");
    let _ = writeln!(vga, "    help | start | stop | restart | list | say <text> | version | fg");
    loop {
        let mut buf = [0u8; 128];
        let n = read_line(vga, "mcs-server> ", &mut buf);
        let (cmd, rest) = split_ascii_word(&buf[..n]);
        match cmd {
            b"help" => {
                let _ = writeln!(vga, "    help               this list");
                let _ = writeln!(vga, "    start              start the server");
                let _ = writeln!(vga, "    stop               stop the server");
                let _ = writeln!(vga, "    restart            restart the server");
                let _ = writeln!(vga, "    list               players online");
                let _ = writeln!(vga, "    say <text>         broadcast message");
                let _ = writeln!(vga, "    version            server version");
                let _ = writeln!(vga, "    fg                 back to system shell");
                let _ = writeln!(vga, "    (systemctl start/stop/restart mc-server works too)");
            }
            b"start" => {
                if unsafe { SERVER_RUNNING } {
                    let _ = writeln!(vga, "  server already running");
                } else {
                    unsafe {
                        SERVER_RUNNING = true;
                    }
                    let _ = writeln!(vga, "  [server] loading world generator (normal terrain)");
                    let _ = writeln!(vga, "  [server] listening on 0.0.0.0:25565");
                    let _ = writeln!(vga, "  [server] Done. Welcome to Mp-minecraft!");
                }
            }
            b"stop" => {
                if unsafe { SERVER_RUNNING } {
                    unsafe {
                        SERVER_RUNNING = false;
                    }
                    let _ = writeln!(vga, "  [server] stopped");
                } else {
                    let _ = writeln!(vga, "  server is not running");
                }
            }
            b"restart" => {
                unsafe {
                    SERVER_RUNNING = false;
                }
                let _ = writeln!(vga, "  [server] stopping ...");
                unsafe {
                    SERVER_RUNNING = true;
                }
                let _ = writeln!(vga, "  [server] reloading world ...");
                let _ = writeln!(vga, "  [server] Done. Welcome to Mp-minecraft!");
            }
            b"list" => {
                let _ = writeln!(vga, "  [server] 0 players online");
            }
            b"say" => {
                let _ = vga.write_str("  [server] ");
                print_bytes(vga, trim_space(rest));
                let _ = writeln!(vga, "");
            }
            b"version" => {
                let _ = writeln!(
                    vga,
                    "  Mp-minecraft Server 0.1 (protocol {}, MC {})",
                    mcver::cur_protocol(),
                    mcver::cur_name()
                );
            }
            b"fg" => {
                let _ = writeln!(vga, "  back to system shell");
                return;
            }
            b"" => {}
            _ => {
                let _ = writeln!(
                    vga,
                    "  unknown command '{}'. Type 'help'.",
                    core::str::from_utf8(cmd).unwrap_or("?")
                );
            }
        }
    }
}

fn install_progress(vga: &mut Vga) {
    let _ = writeln!(vga, "  Installing: partition table ... done");
    sleep_short();
    let _ = writeln!(vga, "  Installing: rootfs ... done");
    let _ = writeln!(vga, "  Installing: server binary ... done");
    let _ = writeln!(vga, "  Installing: world seed ... done");
    let _ = writeln!(vga, "  Install complete.");
}

/// EULA 第一步:按 Y 接受进入命令行,N 拒绝重启。
fn eula_prompt(vga: &mut Vga) -> bool {
    vga.clear();
    vga.set_cursor(3, 0);
    let _ = writeln!(vga, "================================================================");
    let _ = writeln!(vga, "                    MINECRAFT END USER LICENSE");
    let _ = writeln!(vga, "                    AGREEMENT (EULA)");
    let _ = writeln!(vga, "================================================================");
    let _ = writeln!(vga, "");
    let _ = writeln!(vga, "   By accepting you agree to the Minecraft EULA");
    let _ = writeln!(vga, "   (https://aka.ms/MinecraftEULA).");
    let _ = writeln!(vga, "");
    let _ = writeln!(vga, "   This system installs and runs a Minecraft server.");
    let _ = writeln!(vga, "   Your use of Minecraft is subject to the EULA.");
    let _ = writeln!(vga, "");
    let _ = writeln!(vga, "   [Y] I agree to the EULA");
    let _ = writeln!(vga, "   [N] I do not agree (reboot)");
    let _ = writeln!(vga, "");
    // 无人值守:无 PS/2 键盘时 5s 无串口输入则自动接受
    // (Hyper-V Gen2 等平台无 i8042,可能也无配置串口)
    let mut idle_ms: u64 = 0;
    let no_kbd = !kb::PS2_OK.load(core::sync::atomic::Ordering::Relaxed);
    loop {
        if let Some(c) = com1_rx() {
            match c {
                b'y' | b'Y' => {
                    log!("EULA accepted (serial)");
                    let _ = writeln!(vga, "   EULA accepted.");
                    sleep_short();
                    return true;
                }
                b'n' | b'N' => {
                    log!("EULA rejected, rebooting");
                    let _ = writeln!(vga, "   EULA rejected, rebooting...");
                    sleep_short();
                    reboot();
                }
                _ => {
                    idle_ms = 0;
                }
            }
        }
        if let Some(sc) = kb::pop().or_else(poll_scancode) {
            match sc {
                0x15 => {
                    log!("EULA accepted");
                    let _ = writeln!(vga, "   EULA accepted.");
                    sleep_short();
                    return true;
                }
                0x31 => {
                    log!("EULA rejected, rebooting");
                    let _ = writeln!(vga, "   EULA rejected, rebooting...");
                    sleep_short();
                    reboot();
                }
                _ => {}
            }
        }
        if no_kbd {
            idle_ms += 5;
            if idle_ms >= 5000 {
                log!("EULA: no input devices, auto-accepting");
                let _ = writeln!(vga, "   No input device detected, auto-accepted.");
                sleep_short();
                return true;
            }
        }
        sleep(5);
    }
}

// ---------------- 分页(64 位全量恒等映射) ----------------

/// 完整页表:pml4(4096) + 16 x pdpt(每 1GiB) + 16 x pd,共 16GiB。
/// 全部地址运算走 u64,不存在 32 位截断。
const PAGING_GIB: usize = 16;
const PD_ENTRIES: usize = 512;

#[repr(align(4096))]
struct Aligned4096([u8; 4096 + 16 * 4096 + 16 * 4096]);

static mut PAGE_TABLES: Aligned4096 = Aligned4096([0u8; 4096 + 16 * 4096 + 16 * 4096]);

/// 建立 16GiB 恒等 2MiB 大页映射并切换 CR3。
/// 虚拟 [0,16G) 全部走 PML4[0] -> 单张 PDPT;PDPT[i] -> PD[i],
/// PD[i][j] -> 物理 i*1G + j*2M。
/// 必须在 kernel_main 最先调用(过渡表仍在,代码/栈/数据全在低 1GiB)。
fn setup_paging() {
    unsafe {
        let base = core::ptr::addr_of!(PAGE_TABLES.0) as *const u8 as u64;
        let pml4 = base;
        let pdpts = base + 4096;
        let pds = base + 4096 + (PAGING_GIB as u64) * 4096;
        let pml4_p = pml4 as *mut u64;
        let pdpt_p = pdpts as *mut u64;
        pml4_p.write_volatile(pdpts | 0x3);
        for i in 0..PAGING_GIB as u64 {
            let pd_p = (pds + i * 4096) as *mut u64;
            pdpt_p
                .add(i as usize)
                .write_volatile(pds + i * 4096 | 0x3);
            for j in 0..PD_ENTRIES as u64 {
                pd_p.add(j as usize).write_volatile(
                    (i << 30) | (j << 21) | 0x83,
                );
            }
        }
        core::arch::asm!("mov cr3, rax", in("rax") pml4, options(nostack, nomem));
        let probes: [u64; 4] = [
            0x200000u64,    // 2M
            0x40000000u64,  // 1G
            0xBFE00000u64,  // ~3G
            0x100000000u64, // 4G
        ];
        for (i, a) in probes.iter().enumerate() {
            let v = (*a as *const u8).read_volatile();
            log!("paging: probe[{}] {:#x} = {:#x}", i, a, v);
        }
        log!("paging: cr3 = {:#x}, 16GiB mapped", pml4);
    }
}

// ---------------- 入口 ----------------

#[unsafe(no_mangle)]
pub extern "C" fn kernel_main(mb2_info: *const u8) -> ! {
    setup_paging();
    fb::init(mb2_info);
    com1_init();
    #[cfg(feature = "klog")]
    unsafe {
        klog_init();
    }
    if !fb::active() {
        font_init();
    }
    fs::init();
    numa::init(mb2_info);
    acpi::set_mb2(mb2_info);
    idt::init();
    idt::apic_detect();
    kb::init();
    smp::init();
    numa::selftest();
    sched::register_idle();
    let _ = sched::spawn(demo_task_a);
    let _ = sched::spawn(demo_task_b);
    unsafe {
        CWD = 0;
        SERVER_RUNNING = false;
    }
    let mut vga = Vga::new();
    vga.clear();
    let mut com = Com1;

    let _ = writeln!(com, "=== Mp-minecraft kernel ===");
    log!("entry: multiboot2 info = {:p}", mb2_info);
    let mem = total_memory(mb2_info);
    log!("memory map: {mem} bytes total usable");
    log!("mb2 info raw: ptr={:p} total={}", mb2_info, unsafe {
        if mb2_info.is_null() {
            0
        } else {
            *(mb2_info as *const u32)
        }
    });
    acpi::acpi_log();
    log!(
        "{} ready, COM1 ready",
        if fb::active() { "framebuffer console + HZK16" } else { "VGA text mode + HZK16 font" }
    );
    log!("rootfs: {} nodes mounted", fs::node_count());
    log!("numa: {} nodes", numa::node_count());

    // EULA 第一步
    let accepted = eula_prompt(&mut vga);

    vga.clear();
    vga.set_cursor(2, 0);
    let _ = writeln!(vga, "================================================================");
    let _ = writeln!(vga, "         Mp-minecraft System  v0.1");
    let _ = writeln!(vga, "================================================================");
    let _ = writeln!(vga, "");
    let _ = writeln!(vga, "   Memory:   {:.1} MiB usable", mem as f64 / 1048576.0);
    let _ = writeln!(
        vga,
        "   Protocol: {}  (Minecraft Java {})",
        mcver::cur_protocol(),
        mcver::cur_name()
    );
    let _ = writeln!(vga, "   EULA:     {}", if accepted { "accepted" } else { "rejected" });
    let _ = writeln!(vga, "");

    system_shell(&mut vga, accepted);
}

fn demo_task_a() -> ! {
    let mut n = 0u64;
    loop {
        log!("sched: task A round {n}");
        n += 1;
        for _ in 0..100_000 {
            core::hint::spin_loop();
        }
    }
}

fn demo_task_b() -> ! {
    let mut n = 0u64;
    loop {
        log!("sched: task B round {n}");
        n += 1;
        for _ in 0..100_000 {
            core::hint::spin_loop();
        }
    }
}

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    let mut com = Com1;
    let _ = writeln!(com, "[panic] {info}");
    loop {
        core::hint::spin_loop();
    }
}
