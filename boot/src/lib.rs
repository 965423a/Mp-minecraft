//! 内核主体:VGA 文本模式(内建汉字字形)+ 系统命令行 + 拼音输入法 + 服务器控制台。
//!
//! 汉字显示原理:VGA 文本模式有 8KB 字库 RAM(plane 2),字符码 0x80-0xFF
//! 的 128 个字形可上传;每个汉字 16x16 拆成左/右两个 8x16 槽位显示,
//! 因此一屏最多同时显示 64 个汉字。字形数据来自 GB2312 点阵 HZK16。

#![no_std]
#![no_main]

use core::fmt::{self, Write};
use core::panic::PanicInfo;

// ---------------- 数据包 ----------------

/// GB2312 16x16 点阵字库(区 16-87,每区 94 字,每字 32 字节)。
static HZK16: &[u8] = include_bytes!("../data/hzk16.bin");
/// 拼音码表:pinyin_pack.bin。
static PINYIN_PACK: &[u8] = include_bytes!("../data/pinyin_pack.bin");

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

// ---------------- VGA 文本模式 + 汉字字形 ----------------

const VGA_MEM: *mut u16 = 0xB8000 as *mut u16; // 文本显存(plane 0/1)
const FONT_MEM: *mut u8 = 0xA0000 as *mut u8; // 字库 RAM(plane 2)
const COLS: usize = 80;
const ROWS: usize = 25;
const ATTR: u8 = 0x0F; // 白字黑底
const BLACK: u16 = 0x0000; // 黑底黑字(窗框黑条)

/// 逻辑屏幕缓冲:ASCII(< 0x80)或 GB2312 码(0xA1A1 起)或 0(空)。
static mut CELL: [u16; COLS * ROWS] = [0; COLS * ROWS];
/// 字形槽当前内容(GB 码,调试用)。
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

/// 初始化字库访问:写平面掩码回文本模式。
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

/// 写一格到文本显存。
fn vga_write(offset: usize, ch: u16) {
    set_map_mask(0x03);
    unsafe {
        VGA_MEM.add(offset).write_volatile(ch);
    }
}

/// 渲染逻辑格到显存(ASCII 1 格,汉字 2 格并分配字形槽)。
fn render_cell(pos: usize, disp_off: &mut usize) {
    let cell = unsafe { CELL[pos] };
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

/// 全屏重绘(重置字形槽分配)。
fn render_all() {
    unsafe {
        SLOT_NEXT = 0;
    }
    let mut off = 0usize;
    for pos in 0..(COLS * ROWS) {
        render_cell(pos, &mut off);
    }
}

/// 清屏:逻辑缓冲 + 显存。
fn screen_clear() {
    unsafe {
        let p = core::ptr::addr_of_mut!(CELL) as *mut u16;
        for i in 0..COLS * ROWS {
            p.add(i).write(0);
        }
    }
    for off in 0..(COLS * ROWS) {
        vga_write(off, BLACK | 0x20);
    }
}

// ---------------- 光标定位输出 ----------------

struct Vga {
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
        let mut off = self.row * COLS + self.col;
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
            let mut off = self.row * COLS + self.col;
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
            let mut off = self.row * COLS + self.col;
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

struct Com1;

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
        vga_write(off, BLACK | 0x20);
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
        vga_write(off, BLACK | 0x20);
    }
    // 上黑条
    for off in base..base + COLS {
        vga_write(off, BLACK | 0x20);
    }
    // 候选行(黑底白字,左右各留 1 列黑)
    let mut off = base + COLS + 1;
    vga_write(base + COLS, BLACK | 0x20);
    for i in 0..cn {
        if i > 0 {
            vga_write(off, (ATTR as u16) << 8 | b' ' as u16);
            off += 1;
        }
        let d = (b'1' + i as u8) as u16;
        vga_write(off, (ATTR as u16) << 8 | d);
        off += 1;
        vga_write(off, (ATTR as u16) << 8 | b':' as u16);
        off += 1;
        let gb = unsafe { IME_CAND[i] };
        let slot = unsafe { SLOT_NEXT };
        if slot < 64 {
            unsafe {
                SLOT_NEXT = slot + 1;
            }
            let c0 = 0x80 + slot * 2;
            let c1 = c0 + 1;
            if let Some(g) = hzk_glyph(gb) {
                let mut left = [0u8; 16];
                let mut right = [0u8; 16];
                for k in 0..16 {
                    left[k] = g[k * 2];
                    right[k] = g[k * 2 + 1];
                }
                font_upload(c0, &left);
                font_upload(c1, &right);
                vga_write(off, (ATTR as u16) << 8 | c0 as u16);
                vga_write(off + 1, (ATTR as u16) << 8 | c1 as u16);
            }
            off += 2;
        }
    }
    // 右缘 + 下黑条
    vga_write(off, BLACK | 0x20);
    for off in (y + 2) * COLS..(y + 3) * COLS {
        vga_write(off, BLACK | 0x20);
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
        vga_write(off, BLACK | 0x20);
    }
    let msg: &[u8] = if unsafe { IME_CN } {
        b"  [Chinese IME]  (a-z pinyin, 1-9 pick, Esc clear, Ctrl+Space toggle)"
    } else {
        b"  [English]  (Ctrl+Space: Chinese IME)"
    };
    let mut off = base + COLS;
    for &b in msg {
        vga_write(off, (ATTR as u16) << 8 | b as u16);
        off += 1;
    }
    unsafe {
        IME_CAND_ROW = y;
    }
}

/// 拼音变化后刷新候选框。
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

/// 清拼音与候选。
fn ime_clear(vga: &Vga) {
    unsafe {
        IME_PY_LEN = 0;
        IME_CAND_N = 0;
    }
    candidates_clear();
    let _ = vga;
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
    ime_clear(vga);
}

/// 读一行输入(最长 buf.len()-1 字节,汉字占 2 字节)。回车返回长度。
fn read_line(vga: &mut Vga, prompt: &str, buf: &mut [u8]) -> usize {
    let _ = vga.write_str(prompt);
    let mut len = 0usize;
    let mut shift = false;
    loop {
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
                    ime_clear(vga);
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
                        let _ = vga; // 拼音字母已上屏,不重绘
                        // 从屏幕删一个字母
                        vga.backspace();
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
                    // 空格
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

fn sleep_short() {
    sleep(80);
}

fn reboot() -> ! {
    unsafe {
        core::arch::asm!(
            "mov al, 0xFE
             out 0x64, al",
            options(nostack, nomem)
        );
    }
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

fn system_shell(vga: &mut Vga, mem: u64, eula: bool) -> ! {
    let _ = writeln!(vga, "");
    let _ = writeln!(vga, "  Mp-minecraft system shell. Type 'help'. Ctrl+Space: CN/EN IME");
    loop {
        let mut buf = [0u8; 128];
        let n = read_line(vga, "mcs> ", &mut buf);
        let (cmd, rest) = split_ascii_word(&buf[..n]);
        if !cmd.is_empty() {
            log!("cmd: {}", core::str::from_utf8(cmd).unwrap_or("?"));
        }
        match cmd {
            b"help" => {
                let _ = writeln!(vga, "  commands:");
                let _ = writeln!(vga, "    help      this list");
                let _ = writeln!(vga, "    mem       usable memory");
                let _ = writeln!(vga, "    ver       version info");
                let _ = writeln!(vga, "    eula      EULA status");
                let _ = writeln!(vga, "    install   install system (demo)");
                let _ = writeln!(vga, "    ctrls     Minecraft server console");
                let _ = writeln!(vga, "    reboot    restart");
            }
            b"mem" => {
                let _ = writeln!(vga, "  usable memory: {:.1} MiB", mem as f64 / 1048576.0);
            }
            b"ver" => {
                let _ = writeln!(vga, "  Mp-minecraft System v0.1  (protocol 775, MC 26.1.2)");
            }
            b"eula" => {
                let _ = writeln!(
                    vga,
                    "  EULA: {}",
                    if eula { "accepted" } else { "rejected" }
                );
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
        let _ = rest;
    }
}

fn server_console(vga: &mut Vga) {
    let _ = writeln!(vga, "");
    let _ = writeln!(vga, "  === Mp-minecraft server console ===");
    let _ = writeln!(vga, "    help | start | stop | list | say <text> | version | exit");
    let mut running = false;
    loop {
        let mut buf = [0u8; 128];
        let n = read_line(vga, "mcs-server> ", &mut buf);
        let (cmd, rest) = split_ascii_word(&buf[..n]);
        match cmd {
            b"help" => {
                let _ = writeln!(vga, "    help               this list");
                let _ = writeln!(vga, "    start              start the server");
                let _ = writeln!(vga, "    stop               stop the server");
                let _ = writeln!(vga, "    list               players online");
                let _ = writeln!(vga, "    say <text>         broadcast message");
                let _ = writeln!(vga, "    version            server version");
                let _ = writeln!(vga, "    exit               back to system shell");
            }
            b"start" => {
                if running {
                    let _ = writeln!(vga, "  server already running");
                } else {
                    running = true;
                    let _ = writeln!(vga, "  [server] loading world generator (normal terrain)");
                    let _ = writeln!(vga, "  [server] listening on 0.0.0.0:25565");
                    let _ = writeln!(vga, "  [server] Done. Welcome to Mp-minecraft!");
                }
            }
            b"stop" => {
                running = false;
                let _ = writeln!(vga, "  [server] stopped");
            }
            b"list" => {
                let _ = writeln!(vga, "  [server] 0 players online");
            }
            b"say" => {
                let mut i = 0;
                while i < rest.len() && rest[i] == b' ' {
                    i += 1;
                }
                let _ = vga.write_str("  [server] ");
                print_bytes(vga, &rest[i..]);
                let _ = writeln!(vga, "");
            }
            b"version" => {
                let _ = writeln!(vga, "  Mp-minecraft Server 0.1 (protocol 775, MC 26.1.2)");
            }
            b"exit" => {
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
    loop {
        if let Some(sc) = poll_scancode() {
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
        sleep(5);
    }
}

// ---------------- 入口 ----------------

#[unsafe(no_mangle)]
pub extern "C" fn kernel_main(mb2_info: *const u8) -> ! {
    com1_init();
    font_init();
    let mut vga = Vga::new();
    vga.clear();
    let mut com = Com1;

    let _ = writeln!(com, "=== Mp-minecraft kernel ===");
    log!("entry: multiboot2 info = {:p}", mb2_info);
    let mem = total_memory(mb2_info);
    log!("memory map: {mem} bytes total usable");
    log!("VGA text mode + HZK16 font ready, COM1 ready");

    // EULA 第一步
    let accepted = eula_prompt(&mut vga);

    // 系统命令行
    vga.clear();
    vga.set_cursor(2, 0);
    let _ = writeln!(vga, "================================================================");
    let _ = writeln!(vga, "         Mp-minecraft System  v0.1");
    let _ = writeln!(vga, "================================================================");
    let _ = writeln!(vga, "");
    let _ = writeln!(vga, "   Memory:   {:.1} MiB usable", mem as f64 / 1048576.0);
    let _ = writeln!(vga, "   Protocol: 775  (Minecraft Java 26.1.2)");
    let _ = writeln!(vga, "   EULA:     {}", if accepted { "accepted" } else { "rejected" });
    let _ = writeln!(vga, "");

    system_shell(&mut vga, mem, accepted);
}

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    let mut com = Com1;
    let _ = writeln!(com, "[panic] {info}");
    loop {
        core::hint::spin_loop();
    }
}
