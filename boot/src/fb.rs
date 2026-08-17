//! Framebuffer 控制台(GOP/VBE):解析 multiboot2 framebuffer tag,
//! 用 8x16 ASCII + HZK16 汉字字形把 80x25 逻辑屏渲染到像素显存。
//! 与文本模式(0xB8000)并存,按运行平台自动选择。

use crate::{COLS, ROWS};

/// 8x16 ASCII 字形(0x20-0x7E,每字 16 字节,每行 1 bit/px)。
static ASCII8X16: &[u8] = include_bytes!("../data/ascii8x16.bin");

#[derive(Clone, Copy)]
pub struct FbInfo {
    pub addr: u64,
    pub pitch: u32,
    pub width: u32,
    pub height: u32,
    pub bpp: u8,
}

static mut FB: Option<FbInfo> = None;

pub fn active() -> bool {
    unsafe { core::ptr::addr_of!(FB).read().is_some() }
}

pub fn info() -> Option<FbInfo> {
    unsafe { core::ptr::addr_of!(FB).read() }
}

/// 解析 multiboot2 framebuffer tag(type 8),找到则启用。
pub fn init(mb2_info: *const u8) {
    if mb2_info.is_null() {
        crate::log!("fb: no multiboot2 info, VGA text fallback");
        return;
    }
    let total = unsafe { *(mb2_info as *const u32) };
    let mut pos = 8usize;
    while pos + 8 <= total as usize {
        let tag = unsafe { &*(mb2_info.add(pos) as *const crate::Mb2Tag) };
        let size = tag.size as usize;
        if tag.typ == 0 {
            break;
        }
        if tag.typ == 8 && size >= 32 {
            let p = unsafe { mb2_info.add(pos) } as *const u8;
            let addr = crate::u64le_pub(unsafe { p.add(8) });
            let pitch = crate::u32le_pub(unsafe { p.add(16) });
            let width = crate::u32le_pub(unsafe { p.add(20) });
            let height = crate::u32le_pub(unsafe { p.add(24) });
            let bpp = unsafe { p.add(28).read_volatile() };
            let ftype = unsafe { p.add(29).read_volatile() };
            // type==2 = direct RGB(标准 VBE/GOP framebuffer,GRUB 2.x 唯一传的);
            // type==1 = indexed。GRUB 无图形模式时会把 VGA 文本(0xB8000)也
            // 打包成 tag,必须用地址下限(>1MiB)排除它,否则会误当像素显存写坏。
            if addr != 0 && addr >= 0x100000 && (ftype == 1 || ftype == 2) {
                unsafe {
                    FB = Some(FbInfo { addr, pitch, width, height, bpp });
                }
                crate::log!(
                    "fb: {}x{} @ {:#x} pitch={} bpp={} type={}",
                    width,
                    height,
                    addr,
                    pitch,
                    bpp,
                    ftype
                );
                return;
            }
            crate::log!(
                "fb: framebuffer tag found but unusable (addr={:#x} type={}, bpp={})",
                addr,
                ftype,
                bpp
            );
        }
        pos += (size + 7) & !7;
    }
    crate::log!("fb: no framebuffer tag in mb2 (VGA text fallback; invisible on UEFI-only platforms)");
}

const FG: u32 = 0xD0D0D0;
const BG: u32 = 0x000000;

/// 写一个像素(支持 32/24/16/8 bpp)。
fn px(info: &FbInfo, x: u32, y: u32, color: u32) {
    if x >= info.width || y >= info.height {
        return;
    }
    let off = (y as u64) * info.pitch as u64 + (x as u64) * (info.bpp as u64 / 8);
    unsafe {
        match info.bpp {
            32 => {
                ((info.addr + off) as *mut u32).write_volatile(color);
            }
            24 => {
                ((info.addr + off) as *mut u8).write_volatile(color as u8);
                ((info.addr + off + 1) as *mut u8).write_volatile((color >> 8) as u8);
                ((info.addr + off + 2) as *mut u8).write_volatile((color >> 16) as u8);
            }
            16 => {
                let r5 = ((color >> 19) & 0x1F) as u16;
                let g6 = ((color >> 10) & 0x3F) as u16;
                let b5 = ((color >> 3) & 0x1F) as u16;
                ((info.addr + off) as *mut u16).write_volatile(r5 << 11 | g6 << 5 | b5);
            }
            8 => {
                ((info.addr + off) as *mut u8).write_volatile((color >> 16) as u8);
            }
            _ => {}
        }
    }
}

/// 画一个逻辑格(x 为格列,y 为格行;ASCII 8x16,汉字 16x16)。
fn draw_cell(info: &FbInfo, x: u32, y: u32, ch: u16, attr: u8) {
    let fg = if attr & 0x08 != 0 { 0xFFFFFF } else { FG };
    if ch == 0 {
        fill_cell(info, x, y, BG);
        return;
    }
    if ch < 0x80 {
        let c = ch as usize;
        if c < 0x20 || c > 0x7E {
            fill_cell(info, x, y, BG);
            return;
        }
        let glyph = &ASCII8X16[(c - 0x20) * 16..(c - 0x20 + 1) * 16];
        for r in 0..16u32 {
            let row = glyph[r as usize];
            for b in 0..8u32 {
                if row & (0x80 >> b) != 0 {
                    px(info, x * 8 + b, y * 16 + r, fg);
                } else {
                    px(info, x * 8 + b, y * 16 + r, BG);
                }
            }
        }
        return;
    }
    // 汉字:16x16,两格宽(x, x+1)
    let Some(g) = crate::hzk_glyph(ch) else {
        fill_cell(info, x, y, BG);
        fill_cell(info, x + 1, y, BG);
        return;
    };
    for r in 0..16u32 {
        let left = g[(r * 2) as usize];
        let right = g[(r * 2 + 1) as usize];
        for b in 0..8u32 {
            let lc = if left & (0x80 >> b) != 0 { fg } else { BG };
            px(info, x * 8 + b, y * 16 + r, lc);
            let rc = if right & (0x80 >> b) != 0 { fg } else { BG };
            px(info, (x + 1) * 8 + b, y * 16 + r, rc);
        }
    }
}

/// 填满一格(背景色)。
pub fn fill_cell(info: &FbInfo, x: u32, y: u32, color: u32) {
    for r in 0..16u32 {
        for b in 0..8u32 {
            px(info, x * 8 + b, y * 16 + r, color);
        }
    }
}

/// 全屏清黑。
pub fn clear_all() {
    let Some(info) = info() else { return };
    for y in 0..ROWS as u32 {
        for x in 0..COLS as u32 {
            fill_cell(&info, x, y, BG);
        }
    }
}

/// 渲染整个逻辑屏(80x25)。
pub fn render_all() {
    let Some(info) = info() else { return };
    for pos in 0..COLS * ROWS {
        let cell = unsafe { crate::CELL[pos] };
        let x = (pos % COLS) as u32;
        let y = (pos / COLS) as u32;
        draw_cell(&info, x, y, cell, crate::ATTR);
    }
}

/// 渲染一个逻辑格(按 pos 计算坐标)。
pub fn render_cell(pos: usize, cell: u16) {
    let Some(info) = info() else { return };
    let x = (pos % COLS) as u32;
    let y = (pos / COLS) as u32;
    draw_cell(&info, x, y, cell, crate::ATTR);
}

/// 弹层:按逻辑格偏移(off)画一格。
pub fn draw_off(off: usize, ch: u16, attr: u8) {
    let Some(info) = info() else { return };
    let x = (off % COLS) as u32;
    let y = (off / COLS) as u32;
    draw_cell(&info, x, y, ch, attr);
}

/// 弹层:按逻辑格偏移填黑。
pub fn fill_off(off: usize) {
    let Some(info) = info() else { return };
    let x = (off % COLS) as u32;
    let y = (off / COLS) as u32;
    fill_cell(&info, x, y, BG);
}