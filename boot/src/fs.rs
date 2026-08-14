//! 内核内存文件系统:只读目录树,支撑 cd/ls/cat 等命令。
//!
//! 节点表 + 数据池,启动时构建(仿 Linux FHS 目录树)。
//! 名字最长 15 字节,节点表 96 个。

const MAX_NODES: usize = 96;
const DATA_POOL: usize = 8192;

#[derive(Clone, Copy)]
struct Node {
    name: [u8; 16],
    is_dir: bool,
    size: u32,
    parent: i32,
    first_child: i32,
    next: i32,
    data_off: u32,
}

static mut NODES: [Node; MAX_NODES] = [Node {
    name: [0; 16],
    is_dir: true,
    size: 0,
    parent: -1,
    first_child: -1,
    next: -1,
    data_off: 0,
}; MAX_NODES];
static mut NODES_N: usize = 0;
static mut DATA: [u8; DATA_POOL] = [0; DATA_POOL];
static mut DATA_LEN: usize = 0;

fn put_name(dst: &mut [u8; 16], name: &[u8]) {
    for (i, b) in dst.iter_mut().enumerate() {
        *b = if i < name.len() { name[i] } else { 0 };
    }
}

fn add_dir(name: &[u8], parent: i32) -> usize {
    unsafe {
        let id = NODES_N;
        NODES_N += 1;
        NODES[id] = Node {
            name: [0; 16],
            is_dir: true,
            size: 0,
            parent,
            first_child: -1,
            next: -1,
            data_off: 0,
        };
        put_name(&mut NODES[id].name, name);
        link(parent, id as i32);
        id
    }
}

fn add_file(name: &[u8], parent: i32, content: &[u8]) -> usize {
    unsafe {
        let off = DATA_LEN;
        for (i, b) in content.iter().enumerate() {
            DATA[off + i] = *b;
        }
        DATA_LEN += content.len();
        let id = NODES_N;
        NODES_N += 1;
        NODES[id] = Node {
            name: [0; 16],
            is_dir: false,
            size: content.len() as u32,
            parent,
            first_child: -1,
            next: -1,
            data_off: off as u32,
        };
        put_name(&mut NODES[id].name, name);
        link(parent, id as i32);
        id
    }
}

fn link(parent: i32, child: i32) {
    unsafe {
        if parent < 0 {
            return;
        }
        let p = parent as usize;
        if NODES[p].first_child < 0 {
            NODES[p].first_child = child;
        } else {
            let mut c = NODES[p].first_child;
            while NODES[c as usize].next >= 0 {
                c = NODES[c as usize].next;
            }
            NODES[c as usize].next = child;
        }
    }
}

/// 构建 rootfs(Linux FHS 目录树)。返回根节点 id(0)。
pub fn init() -> usize {
    unsafe {
        NODES_N = 0;
        DATA_LEN = 0;
    }
    let root = add_dir(b"/", -1);
    // 一级目录(FHS)
    let bin = add_dir(b"bin", root as i32);
    let boot = add_dir(b"boot", root as i32);
    let dev = add_dir(b"dev", root as i32);
    let etc = add_dir(b"etc", root as i32);
    let home = add_dir(b"home", root as i32);
    let lib = add_dir(b"lib", root as i32);
    let media = add_dir(b"media", root as i32);
    let mnt = add_dir(b"mnt", root as i32);
    let opt = add_dir(b"opt", root as i32);
    let proc = add_dir(b"proc", root as i32);
    let root_home = add_dir(b"root", root as i32);
    let run = add_dir(b"run", root as i32);
    let sbin = add_dir(b"sbin", root as i32);
    let srv = add_dir(b"srv", root as i32);
    let sys = add_dir(b"sys", root as i32);
    let tmp = add_dir(b"tmp", root as i32);
    let usr = add_dir(b"usr", root as i32);
    let var = add_dir(b"var", root as i32);
    // 二级目录
    let grub = add_dir(b"grub", boot as i32);
    let usr_bin = add_dir(b"bin", usr as i32);
    let usr_lib = add_dir(b"lib", usr as i32);
    let usr_sbin = add_dir(b"sbin", usr as i32);
    let usr_share = add_dir(b"share", usr as i32);
    let var_log = add_dir(b"log", var as i32);
    let var_run = add_dir(b"run", var as i32);
    let var_lib = add_dir(b"lib", var as i32);
    let mc_lib = add_dir(b"minecraft", var_lib as i32);
    let mc_world = add_dir(b"world", mc_lib as i32);
    // 可执行占位
    add_file(b"ls", bin as i32, b"# builtin\n");
    add_file(b"cat", bin as i32, b"# builtin\n");
    add_file(b"mcssh", bin as i32, b"# shell\n");
    add_file(b"reboot", sbin as i32, b"# builtin\n");
    add_file(b"systemctl", sbin as i32, b"# builtin\n");
    add_file(b"init", sbin as i32, b"# kernel init\n");
    // boot
    add_file(
        b"grub.cfg",
        grub as i32,
        b"set timeout=3\nmenuentry \"Mp-minecraft\" {\n    multiboot2 /boot/mcs-kernel\n    boot\n}\n",
    );
    add_file(b"mcs-kernel", boot as i32, b"Mp-minecraft kernel v0.1 (x86_64)\n");
    add_file(b"mc-server", boot as i32, b"ELF placeholder: real server runs on host\n");
    // etc
    add_file(b"hostname", etc as i32, b"mcs\n");
    add_file(
        b"os-release",
        etc as i32,
        b"NAME=\"Mp-minecraft OS\"\nVERSION=\"0.1\"\nPRETTY_NAME=\"Mp-minecraft System 0.1\"\n",
    );
    add_file(
        b"fstab",
        etc as i32,
        b"# <device> <dir> <type> <opts>\nproc    /proc   proc    defaults 0 0\nsysfs   /sys    sysfs   defaults 0 0\n",
    );
    add_file(b"passwd", etc as i32, b"root:x:0:0:root:/root:/bin/mcssh\ndev:x:1000:1000:dev:/home/dev:/bin/mcssh\n");
    add_file(b"motd", etc as i32, b"Welcome to Mp-minecraft System!\n");
    add_file(b"eula.txt", etc as i32, b"eula=true\n");
    add_file(
        b"mcs.conf",
        etc as i32,
        b"# Mp-minecraft server config\nport=25565\nmotd=Mp-minecraft 26.1.2\nview-distance=2\n",
    );
    // 用户目录
    let dev_home = add_dir(b"dev", home as i32);
    let _ = root_home;
    add_file(
        b".mcsrc",
        dev_home as i32,
        b"# Mp-minecraft shell rc\nset motd=1\n",
    );
    // proc/sys(虚拟文件演示)
    add_file(
        b"meminfo",
        proc as i32,
        b"MemTotal:       512000 kB\nMemFree:        400000 kB\n",
    );
    add_file(b"cpuinfo", proc as i32, b"processor: 0\nmodel: QEMU x86_64\n");
    add_file(b"version", proc as i32, b"Mp-minecraft kernel v0.1\n");
    add_file(
        b"cmdline",
        proc as i32,
        b"BOOT_IMAGE=/boot/mcs-kernel quiet\n",
    );
    let _ = sys;
    let _ = dev;
    let _ = media;
    let _ = mnt;
    let _ = opt;
    let _ = run;
    let _ = srv;
    let _ = lib;
    let _ = tmp;
    // usr
    add_file(
        b"mcsctl",
        usr_bin as i32,
        b"#!/bin/sh\nexec mcs-server --config /etc/mcs.conf\n",
    );
    add_file(b"libc.so", usr_lib as i32, b"MCS libc 0.1\n");
    let _ = usr_sbin;
    add_file(b"licenses", usr_share as i32, b"MIT + EULA\n");
    // var
    add_file(
        b"server.log",
        var_log as i32,
        b"[00:00:00] [main/INFO]: Starting minecraft server version 26.1.2\n[00:00:01] [main/INFO]: Done\n",
    );
    add_file(
        b"messages",
        var_log as i32,
        b"[00:00:00] [kernel] boot complete\n",
    );
    let _ = var_run;
    let _ = mc_lib;
    add_file(
        b"level.dat",
        mc_world as i32,
        b"level-name=world\nseed=0\n",
    );
    root
}

/// 节点总数(供内核日志)。
pub fn node_count() -> usize {
    unsafe { NODES_N }
}

/// 取节点名字(去尾 0)。
pub fn name(id: usize) -> &'static [u8] {
    unsafe {
        let n = &NODES[id].name;
        let mut len = 0;
        while len < 16 && n[len] != 0 {
            len += 1;
        }
        &n[..len]
    }
}

pub fn is_dir(id: usize) -> bool {
    unsafe { NODES[id].is_dir }
}

pub fn size(id: usize) -> u32 {
    unsafe { NODES[id].size }
}

pub fn parent(id: usize) -> Option<usize> {
    let p = unsafe { NODES[id].parent };
    if p < 0 {
        None
    } else {
        Some(p as usize)
    }
}

pub fn first_child(id: usize) -> Option<usize> {
    let c = unsafe { NODES[id].first_child };
    if c < 0 {
        None
    } else {
        Some(c as usize)
    }
}

pub fn next(id: usize) -> Option<usize> {
    let c = unsafe { NODES[id].next };
    if c < 0 {
        None
    } else {
        Some(c as usize)
    }
}

pub fn content(id: usize) -> &'static [u8] {
    unsafe {
        let off = NODES[id].data_off as usize;
        let len = NODES[id].size as usize;
        &DATA[off..off + len]
    }
}

/// 从 start 节点解析路径(绝对路径以 '/' 开头则从根)。支持 / . ..
pub fn resolve(start: usize, path: &[u8]) -> Option<usize> {
    let mut cur = if path.first() == Some(&b'/') { 0 } else { start };
    let mut i = 0;
    let mut comp = [0u8; 32];
    let mut cl = 0;
    while i <= path.len() {
        if i == path.len() || path[i] == b'/' {
            if cl > 0 {
                if comp[..cl] == *b".." {
                    cur = parent(cur)?;
                } else if !(comp[..cl] == *b".") {
                    let mut found = None;
                    let mut c = first_child(cur);
                    while let Some(id) = c {
                        if name(id) == &comp[..cl] {
                            found = Some(id);
                            break;
                        }
                        c = next(id);
                    }
                    cur = found?;
                }
                cl = 0;
            }
        } else if cl < 32 {
            comp[cl] = path[i];
            cl += 1;
        }
        i += 1;
    }
    Some(cur)
}

/// 从节点向上构建完整路径(写到 buf,返回长度)。
pub fn full_path(id: usize, buf: &mut [u8]) -> usize {
    let mut stack = [0usize; 16];
    let mut n = 0;
    let mut cur = Some(id);
    while let Some(c) = cur {
        stack[n] = c;
        n += 1;
        cur = parent(c);
    }
    if n <= 1 {
        buf[0] = b'/';
        return 1;
    }
    let mut len = 0;
    for i in (0..n).rev() {
        if i == n - 1 {
            continue; // 根
        }
        if len == 0 {
            buf[len] = b'/';
            len += 1;
        }
        let nm = name(stack[i]);
        for j in 0..nm.len() {
            buf[len] = nm[j];
            len += 1;
        }
        buf[len] = b'/';
        len += 1;
    }
    len - 1
}
