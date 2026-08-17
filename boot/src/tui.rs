//! archinstall 风格全屏 TUI 安装向导(80x25,中英文)。
//!
//! 渲染直接走弹层绘制(text_put_cell/text_put_gb),不经过 CELL/滚动缓冲;
//! 输入合并 PS/2 键盘(方向键 E0 48/50)与串口(ESC [ A/B)两条通道。
//! 第一步选择语言(默认中文),选英文则全部页面切换为英文文案。

use core::sync::atomic::{AtomicBool, AtomicU8, Ordering};

use crate::{com1_rx, kb, log, reboot, sleep, text_put_cell, text_put_gb, Vga};

const W: usize = crate::COLS;
const H: usize = crate::ROWS;

const WHITE: u8 = 0x0F; // 白字黑底
const BRIGHT: u8 = 0x1F; // 亮白(标题/当前值)
const SEL: u8 = 0x70; // 反色(选中菜单项)
const DIM: u8 = 0x07; // 灰(次要信息)

// ---------------- 文案(双语) ----------------

pub struct Texts {
    pub lang_title: &'static [u8],
    pub lang_zh: &'static [u8],
    pub lang_en: &'static [u8],
    pub welcome_title: &'static [u8],
    pub subtitle: &'static [u8],
    pub start: &'static [u8],
    pub shell: &'static [u8],
    pub reboot: &'static [u8],
    pub eula_title: &'static [u8],
    pub eula_l1: &'static [u8],
    pub eula_l2: &'static [u8],
    pub eula_l3: &'static [u8],
    pub agree: &'static [u8],
    pub disagree: &'static [u8],
    pub disk_title: &'static [u8],
    pub disk_probe: &'static [u8],
    pub disk0: &'static [u8],
    pub back: &'static [u8],
    pub opt_title: &'static [u8],
    pub l_type: &'static [u8],
    pub l_mem: &'static [u8],
    pub l_world: &'static [u8],
    pub v_surv: &'static [u8],
    pub v_crea: &'static [u8],
    pub v_hard: &'static [u8],
    pub hint_opt: &'static [u8],
    pub confirm_title: &'static [u8],
    pub c_l1: &'static [u8],
    pub c_l2: &'static [u8],
    pub ok: &'static [u8],
    pub prog_title: &'static [u8],
    pub steps: [&'static [u8]; 4],
    pub done_title: &'static [u8],
    pub done_msg: &'static [u8],
    pub enter_sys: &'static [u8],
    pub hint_sel: &'static [u8],
    pub world_prompt: &'static [u8],
    pub done_tag: &'static [u8],
}

static ZH: Texts = Texts {
    lang_title: b"Language / \xd3\xef\xd1\xd4",
    lang_zh: b"\xbc\xf2\xcc\xe5\xd6\xd0\xce\xc4",
    lang_en: b"English",
    welcome_title: b"Mp-minecraft \xcf\xb5\xcd\xb3\xb0\xb2\xd7\xb0\xb3\xcc\xd0\xf2",
    subtitle: b"\xbb\xf9\xd3\xda GRUB \xd3\xeb\xd7\xd4\xd6\xc6\xc4\xda\xba\xcb\xb5\xc4 Minecraft \xb7\xfe\xce\xf1\xc6\xf7\xd2\xbb\xcc\xe5\xbb\xfa",
    start: b"\xbf\xaa\xca\xbc\xb0\xb2\xd7\xb0",
    shell: b"\xbd\xf8\xc8\xeb\xc3\xfc\xc1\xee\xd0\xd0",
    reboot: b"\xd6\xd8\xd0\xc2\xc6\xf4\xb6\xaf",
    eula_title: b"Minecraft \xd7\xee\xd6\xd5\xd3\xc3\xbb\xa7\xd0\xed\xbf\xc9\xd0\xad\xd2\xe9 (EULA)",
    eula_l1: b"\xb1\xbe\xcf\xb5\xcd\xb3\xbd\xab\xb0\xb2\xd7\xb0\xb2\xa2\xd4\xcb\xd0\xd0 Minecraft \xb7\xfe\xce\xf1\xc6\xf7\xa1\xa3",
    eula_l2: b"\xbd\xd3\xca\xdc\xd0\xad\xd2\xe9\xbc\xb4\xb1\xed\xca\xbe\xc4\xfa\xcd\xac\xd2\xe2 Minecraft EULA:",
    eula_l3: b"https://aka.ms/MinecraftEULA",
    agree: b"\xce\xd2\xcd\xac\xd2\xe2",
    disagree: b"\xce\xd2\xb2\xbb\xcd\xac\xd2\xe2(\xd6\xd8\xc6\xf4)",
    disk_title: b"\xd1\xa1\xd4\xf1\xb0\xb2\xd7\xb0\xb4\xc5\xc5\xcc",
    disk_probe: b"\xbc\xec\xb2\xe2\xb5\xbd\xd2\xd4\xcf\xc2\xb4\xc5\xc5\xcc:",
    disk0: b"\xb4\xc5\xc5\xcc 0: \xd0\xe9\xc4\xe2\xb4\xc5\xc5\xcc 8 GiB(\xd1\xdd\xca\xbe)",
    back: b"\xb7\xb5\xbb\xd8",
    opt_title: b"\xb0\xb2\xd7\xb0\xd1\xa1\xcf\xee",
    l_type: b"\xb7\xfe\xce\xf1\xc6\xf7\xc0\xe0\xd0\xcd: ",
    l_mem: b"\xc4\xda\xb4\xe6\xb7\xd6\xc5\xe4: ",
    l_world: b"\xca\xc0\xbd\xe7\xc3\xfb\xb3\xc6: ",
    v_surv: b"\xc9\xfa\xb4\xe6\xc4\xa3\xca\xbd",
    v_crea: b"\xb4\xb4\xd4\xec\xc4\xa3\xca\xbd",
    v_hard: b"\xbc\xab\xcf\xde\xc4\xa3\xca\xbd",
    hint_opt: b"\xbb\xd8\xb3\xb5\xd0\xde\xb8\xc4\xd1\xa1\xcf\xee; \xd1\xa1\xd4\xf1 \xbf\xaa\xca\xbc\xb0\xb2\xd7\xb0 \xbc\xcc\xd0\xf8",
    confirm_title: b"\xc8\xb7\xc8\xcf\xb0\xb2\xd7\xb0",
    c_l1: b"\xbd\xab\xc7\xe5\xbf\xd5\xb4\xc5\xc5\xcc 0 \xb5\xc4\xc8\xab\xb2\xbf\xca\xfd\xbe\xdd,\xb2\xa2\xb0\xb2\xd7\xb0\xcf\xb5\xcd\xb3\xa1\xa3",
    c_l2: b"\xb4\xcb\xb2\xd9\xd7\xf7\xb2\xbb\xbf\xc9\xb3\xb7\xcf\xfa!",
    ok: b"\xc8\xb7\xc8\xcf\xb0\xb2\xd7\xb0",
    prog_title: b"\xd5\xfd\xd4\xda\xb0\xb2\xd7\xb0",
    steps: [
        b"\xd0\xb4\xc8\xeb\xb7\xd6\xc7\xf8\xb1\xed",
        b"\xb4\xb4\xbd\xa8\xb8\xf9\xce\xc4\xbc\xfe\xcf\xb5\xcd\xb3",
        b"\xb0\xb2\xd7\xb0\xb7\xfe\xce\xf1\xc6\xf7\xb3\xcc\xd0\xf2",
        b"\xc9\xfa\xb3\xc9\xca\xc0\xbd\xe7\xca\xfd\xbe\xdd",
    ],
    done_title: b"\xb0\xb2\xd7\xb0\xcd\xea\xb3\xc9",
    done_msg: b"Mp-minecraft \xcf\xb5\xcd\xb3\xb0\xb2\xd7\xb0\xcd\xea\xb3\xc9\xa1\xa3",
    enter_sys: b"\xbd\xf8\xc8\xeb\xcf\xb5\xcd\xb3",
    hint_sel: b"\xa1\xfc/\xa1\xfb \xd1\xa1\xd4\xf1, \xbb\xd8\xb3\xb5 \xc8\xb7\xc8\xcf",
    world_prompt: b"\xca\xc0\xbd\xe7\xc3\xfb\xb3\xc6 (ASCII, Enter \xc8\xb7\xc8\xcf): ",
    done_tag: b"done ",
};

static EN: Texts = Texts {
    lang_title: b"Language / \xd3\xef\xd1\xd4",
    lang_zh: b"\xbc\xf2\xcc\xe5\xd6\xd0\xce\xc4",
    lang_en: b"English",
    welcome_title: b"Mp-minecraft Installer",
    subtitle: b"A Minecraft server appliance on GRUB + custom kernel",
    start: b"Install",
    shell: b"Shell",
    reboot: b"Reboot",
    eula_title: b"Minecraft End User License Agreement (EULA)",
    eula_l1: b"This system installs and runs a Minecraft server.",
    eula_l2: b"By accepting you agree to the Minecraft EULA:",
    eula_l3: b"https://aka.ms/MinecraftEULA",
    agree: b"I agree",
    disagree: b"I disagree (reboot)",
    disk_title: b"Select install disk",
    disk_probe: b"Detected disks:",
    disk0: b"Disk 0: Virtual disk 8 GiB (demo)",
    back: b"Back",
    opt_title: b"Install options",
    l_type: b"Server type: ",
    l_mem: b"Memory: ",
    l_world: b"World name: ",
    v_surv: b"Survival",
    v_crea: b"Creative",
    v_hard: b"Hardcore",
    hint_opt: b"Enter to change option; select Install to continue",
    confirm_title: b"Confirm install",
    c_l1: b"All data on disk 0 will be erased and the system installed.",
    c_l2: b"This operation cannot be undone!",
    ok: b"Confirm install",
    prog_title: b"Installing",
    steps: [
        b"Writing partition table",
        b"Creating root filesystem",
        b"Installing server program",
        b"Generating world data",
    ],
    done_title: b"Install complete",
    done_msg: b"Mp-minecraft installed successfully.",
    enter_sys: b"Enter system",
    hint_sel: b"Use arrows to select, Enter to confirm",
    world_prompt: b"World name (ASCII, Enter to confirm): ",
    done_tag: b"done ",
};

// ---------------- 输入 ----------------

#[derive(Clone, Copy, PartialEq)]
pub enum Key {
    Up,
    Down,
    Enter,
    Backspace,
    Char(u8),
    None,
}

static E0_PEND: AtomicBool = AtomicBool::new(false);
static ESC_ST: AtomicU8 = AtomicU8::new(0);

/// 合并 PS/2(IRQ 缓冲优先,其次 0x64 轮询)与串口输入。
pub fn poll_key() -> Key {
    if let Some(c) = com1_rx() {
        let st = ESC_ST.load(Ordering::Relaxed);
        match st {
            0 => {
                if c == 0x1B {
                    ESC_ST.store(1, Ordering::Relaxed);
                } else if c == b'\r' {
                    return Key::Enter;
                } else if c == 0x08 || c == 0x7F {
                    return Key::Backspace;
                } else if (0x20..0x7F).contains(&c) {
                    return Key::Char(c);
                }
            }
            1 => {
                if c == b'[' {
                    ESC_ST.store(2, Ordering::Relaxed);
                } else {
                    ESC_ST.store(0, Ordering::Relaxed);
                }
            }
            2 => {
                ESC_ST.store(0, Ordering::Relaxed);
                return match c {
                    b'A' => Key::Up,
                    b'B' => Key::Down,
                    _ => Key::None,
                };
            }
            _ => ESC_ST.store(0, Ordering::Relaxed),
        }
        return Key::None;
    }
    if let Some(sc) = kb::pop().or_else(crate::poll_scancode) {
        if E0_PEND.load(Ordering::Relaxed) {
            E0_PEND.store(false, Ordering::Relaxed);
            let k = match sc {
                0x48 => Key::Up,
                0x50 => Key::Down,
                _ => Key::None,
            };
            return k;
        }
        if sc & 0x80 != 0 {
            return Key::None;
        }
        if sc & 0x80 != 0 {
            return Key::None;
        }
        // 方向键:真实 PS/2 键盘带 E0 前缀(QEMU 交互注入裸 0x48/0x50),
        // 两种都要接受;寄存器同时覆盖普通与 Extended 扫描码。
        if sc == 0x48 || sc == 0x50 || sc == 0x4B || sc == 0x4D {
            // 菜单只需上下;左右键统一映射为上下
            return if sc == 0x48 || sc == 0x4B {
                Key::Up
            } else {
                Key::Down
            };
        }
        return match sc {
            0xE0 => {
                E0_PEND.store(true, Ordering::Relaxed);
                Key::None
            }
            0x1C => Key::Enter,
            0x0E => Key::Backspace,
            0x39 => Key::Char(b' '),
            _ => Key::None,
        };
    }
    Key::None
}

/// 无人值守判断:无 PS/2 键盘时,5s 无串口输入则返回 true(自动确认)。
/// 一旦检测到任何输入,本会话切换为交互模式。
fn idle_auto(had_input: &mut bool) -> bool {
    if *had_input {
        return false;
    }
    if !kb::PS2_OK.load(Ordering::Relaxed) {
        for _ in 0..1000 {
            if com1_rx().is_some() {
                *had_input = true;
                return false;
            }
            sleep(5);
        }
        return true;
    }
    false
}

// ---------------- 绘制 ----------------

/// 在 off 处绘制 GB2312/ASCII 字节串,返回推进后的格位置。
fn ttext(off: usize, s: &[u8], attr: u8) -> usize {
    let mut o = off;
    let mut i = 0;
    while i < s.len() {
        let b = s[i];
        if b < 0x80 {
            text_put_cell(o, b, attr);
            o += 1;
            i += 1;
        } else if i + 1 < s.len() {
            text_put_gb(o, ((b as u16) << 8) | s[i + 1] as u16, attr);
            o += 2;
            i += 2;
        } else {
            break;
        }
    }
    o
}

/// 清一行的可写区域(2..W-2)并用 attr 填充。
fn line_clear(row: usize, attr: u8) {
    for c in 2..W - 2 {
        text_put_cell(row * W + c, b' ', attr);
    }
}

/// 画页面骨架:边框 + 居中标题 + 底部快捷键栏。
fn frame(vga: &mut Vga, title: &[u8], hint: &[u8]) {
    vga.clear();
    for c in 0..W {
        text_put_cell(c, b'-', WHITE);
        text_put_cell((H - 1) * W + c, b'-', WHITE);
    }
    for r in 1..H - 1 {
        text_put_cell(r * W, b'|', WHITE);
        text_put_cell(r * W + W - 1, b'|', WHITE);
    }
    text_put_cell(0, b'+', WHITE);
    text_put_cell(W - 1, b'+', WHITE);
    text_put_cell((H - 1) * W, b'+', WHITE);
    text_put_cell((H - 1) * W + W - 1, b'+', WHITE);
    // 标题栏
    let mut off = 2;
    off = ttext(off, title, BRIGHT);
    while off < W - 2 {
        text_put_cell(off, b'-', WHITE);
        off += 1;
    }
    // 底部快捷键栏
    let boff = 2;
    let boff = ttext((H - 1) * W + boff, hint, DIM);
    let mut b2 = boff;
    while b2 < W - 2 {
        text_put_cell((H - 1) * W + b2, b'-', WHITE);
        b2 += 1;
    }
    let _ = vga;
}

/// 绘制菜单(items 从 row0 行开始),选中项整行反色。
fn menu_draw(sel: usize, items: &[&[u8]], row0: usize) {
    for (i, it) in items.iter().enumerate() {
        let row = row0 + i;
        if row >= H - 1 {
            break;
        }
        let attr = if i == sel { SEL } else { WHITE };
        line_clear(row, attr);
        ttext(row * W + 4, it, attr);
    }
}

/// 简单 ASCII 行编辑框(世界名称)。返回 true = Enter 确认,false = 取消。
fn input_line(vga: &mut Vga, t: &Texts, buf: &mut [u8]) -> bool {
    let _ = vga;
    let row = 14usize;
    let mut len = 0usize;
    loop {
        let attr = WHITE;
        line_clear(row, attr);
        let mut off = row * W + 4;
        off = ttext(off, t.world_prompt, attr);
        for i in 0..30 {
            let ch = if i < len { buf[i] } else { b' ' };
            let a = if i == len { SEL } else { attr };
            text_put_cell(off + i, ch, a);
        }
        let k = poll_key();
        match k {
            Key::Enter => {
                buf[len] = 0;
                return true;
            }
            Key::Backspace => {
                if len > 0 {
                    len -= 1;
                }
            }
            Key::Char(c) => {
                if c >= b'0' && c <= b'9' || c >= b'a' && c <= b'z' || c >= b'A' && c <= b'Z' || c == b'_' || c == b'-' || c == b'.' {
                    if len < 30 {
                        buf[len] = c;
                        len += 1;
                    }
                }
            }
            _ => {}
        }
        sleep(30);
    }
}

// ---------------- 安装向导 ----------------

pub fn install_wizard(vga: &mut Vga) -> bool {
    let mut had_input = false;

    // 0. 语言选择(默认中文)
    let mut zh = true;
    let mut sel = 0usize;
    loop {
        frame(vga, ZH.lang_title, ZH.hint_sel);
        menu_draw(sel, &[ZH.lang_zh, ZH.lang_en], 8);
        match poll_key() {
            Key::Up => sel = 0,
            Key::Down => sel = 1,
            Key::Enter => {
                zh = sel == 0;
                log!("wizard: language = {}", if zh { "zh" } else { "en" });
                break;
            }
            _ => {}
        }
        if idle_auto(&mut had_input) {
            log!("wizard: no input, language = zh");
            break;
        }
        sleep(30);
    }
    let t: &Texts = if zh { &ZH } else { &EN };

    // 1. 欢迎页
    log!("wizard: page=welcome");
    sel = 0;
    loop {
        frame(vga, t.welcome_title, t.hint_sel);
        let mut off = 3 * W + 2;
        off = ttext(off, t.subtitle, DIM);
        ttext(off + 2, b" (v0.1)", DIM);
        menu_draw(sel, &[t.start, t.shell, t.reboot], 10);
        match poll_key() {
            Key::Up => { sel = sel.saturating_sub(1); log!("wizard: sel={}", sel); }
            Key::Down => {
                if sel < 2 {
                    sel += 1;
                }
            }
            Key::Enter => match sel {
                0 => break,
                1 => return false,
                2 => reboot(),
                _ => {}
            },
            _ => {}
        }
        if idle_auto(&mut had_input) {
            log!("wizard: no input, auto start install");
            break;
        }
        sleep(30);
    }

    // 2. EULA
    log!("wizard: page=eula");
    sel = 0;
    loop {
        frame(vga, t.eula_title, t.hint_sel);
        ttext(3 * W + 2, t.eula_l1, WHITE);
        ttext(4 * W + 2, t.eula_l2, WHITE);
        ttext(5 * W + 2, t.eula_l3, WHITE);
        menu_draw(sel, &[t.agree, t.disagree], 10);
        match poll_key() {
            Key::Up => sel = 0,
            Key::Down => sel = 1,
            Key::Enter => match sel {
                0 => break,
                1 => {
                    log!("wizard: EULA rejected, rebooting");
                    reboot();
                }
                _ => {}
            },
            _ => {}
        }
        if idle_auto(&mut had_input) {
            log!("wizard: EULA auto-accepted");
            break;
        }
        sleep(30);
    }

    // 3. 磁盘选择
    log!("wizard: page=disk");
    sel = 0;
    loop {
        frame(vga, t.disk_title, t.hint_sel);
        ttext(3 * W + 2, t.disk_probe, WHITE);
        menu_draw(sel, &[t.disk0, t.back], 10);
        match poll_key() {
            Key::Up => sel = sel.saturating_sub(1),
            Key::Down => {
                if sel < 1 {
                    sel += 1;
                }
            }
            Key::Enter => match sel {
                0 => break,
                1 => return false,
                _ => {}
            },
            _ => {}
        }
        if idle_auto(&mut had_input) {
            log!("wizard: auto select disk 0");
            break;
        }
        sleep(30);
    }

    // 4. 安装选项
    log!("wizard: page=options");
    let mut mode = 0usize; // 0 生存 1 创造 2 极限
    let mut mem_idx = 1usize; // [1,2,4] GiB
    let mut world: [u8; 32] = [0; 32];
    world[..5].copy_from_slice(b"world");
    sel = 0;
    'optloop: loop {
        frame(vga, t.opt_title, t.hint_opt);
        let mut off = 4 * W + 2;
        off = ttext(off, t.l_type, WHITE);
        ttext(off, [t.v_surv, t.v_crea, t.v_hard][mode], BRIGHT);
        off = 5 * W + 2;
        off = ttext(off, t.l_mem, WHITE);
        ttext(off, [b"1 GiB", b"2 GiB", b"4 GiB"][mem_idx], BRIGHT);
        off = 6 * W + 2;
        off = ttext(off, t.l_world, WHITE);
        let mut wlen = 0;
        while wlen < 31 && world[wlen] != 0 {
            wlen += 1;
        }
        ttext(off, &world[..wlen], BRIGHT);
        menu_draw(sel, &[b"", b"", b"", t.start, t.back], 8);
        match poll_key() {
            Key::Up => sel = sel.saturating_sub(1),
            Key::Down => {
                if sel < 4 {
                    sel += 1;
                }
            }
            Key::Enter => match sel {
                0 => mode = (mode + 1) % 3,
                1 => mem_idx = (mem_idx + 1) % 3,
                2 => {
                    if !input_line(vga, t, &mut world) {
                        world[..5].copy_from_slice(b"world");
                        world[5] = 0;
                    }
                }
                3 => break 'optloop,
                4 => return false,
                _ => {}
            },
            _ => {}
        }
        if idle_auto(&mut had_input) {
            log!("wizard: auto use default options");
            break 'optloop;
        }
        sleep(30);
    }

    // 5. 确认
    log!("wizard: page=confirm");
    sel = 0;
    loop {
        frame(vga, t.confirm_title, t.hint_sel);
        ttext(3 * W + 2, t.c_l1, WHITE);
        ttext(4 * W + 2, t.c_l2, WHITE);
        menu_draw(sel, &[t.ok, t.back], 10);
        match poll_key() {
            Key::Up => sel = sel.saturating_sub(1),
            Key::Down => {
                if sel < 1 {
                    sel += 1;
                }
            }
            Key::Enter => match sel {
                0 => break,
                1 => return false,
                _ => {}
            },
            _ => {}
        }
        if idle_auto(&mut had_input) {
            log!("wizard: auto confirm install");
            break;
        }
        sleep(30);
    }

    // 6. 安装进度
    log!("wizard: page=progress");
    frame(vga, t.prog_title, b"");
    for (i, s) in t.steps.iter().enumerate() {
        let row = 4 + i;
        line_clear(row, WHITE);
        ttext(row * W + 4, s, WHITE);
        for _ in 0..60 {
            sleep(5);
        }
        let pct = ((i + 1) * 100) / 4;
        let barw = 48usize;
        let fill = (pct * barw) / 100;
        let bar_off = 16usize;
        for b in 0..barw {
            let ch = if b < fill { b'#' } else { b'-' };
            text_put_cell(
                (H - 2) * W + bar_off + b,
                ch,
                if b < fill { BRIGHT } else { DIM },
            );
        }
        let poff = (H - 2) * W + bar_off + barw + 2;
        ttext(poff, t.done_tag, WHITE);
        log!("wizard: step {} done ({}%)", i + 1, pct);
        sleep(10);
    }

    // 7. 完成
    log!("wizard: page=done");
    sel = 0;
    loop {
        frame(vga, t.done_title, t.hint_sel);
        ttext(3 * W + 2, t.done_msg, WHITE);
        menu_draw(sel, &[t.enter_sys, t.reboot], 10);
        match poll_key() {
            Key::Up => sel = sel.saturating_sub(1),
            Key::Down => {
                if sel < 1 {
                    sel += 1;
                }
            }
            Key::Enter => match sel {
                0 => return true,
                1 => reboot(),
                _ => {}
            },
            _ => {}
        }
        if idle_auto(&mut had_input) {
            log!("wizard: auto enter system");
            return true;
        }
        sleep(30);
    }
}