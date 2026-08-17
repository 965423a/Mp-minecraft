// AP trampoline:复制到 0x7000,SIPI vector 7 启动。
// 布局(相对 tramp_start):
//   0x00  real_entry(实模式,代码在前)
//   pmode_entry / long_entry 由编译后 xxd 校准
//   0x100  params(64 字节,64 位地址安全;smp.rs 的 patch 与其对齐):
//   +0x00  GDTR(10B): limit u16 + base u64(实模式 lgdtl 读 6B = limit+base 低 32;
//                     长模式 lgdt 读 10B = 完整 limit+base64,同址)
//   +0x0C  CR3 u64
//   +0x14  STACK u64
//   +0x1C  READY u64
//   +0x24  APID u32
//   +0x28  PMODE_IP u16 + PMODE_CS u16
//   +0x2C  LONG_IP u64(经 32 位 lret 进入,低 32 位生效,tramp 在 0x7000 必 <4GiB)
//   +0x34  AP_ENTRY u64
//   +0x3C  pad

.set PARAM, 0x100

.global tramp_start
.global real_entry
.global pmode_entry
.global long_entry
.global tramp_end

tramp_start:
real_entry:
    .code16
    cli
    cld
    xorl %ebx, %ebx
    movw %cs, %bx
    shll $4, %ebx                // ebx = 0x7000 = tramp 物理基址
    movw $1, %ax
    movw %ax, 0x61E0             // stage 1
    lgdtl PARAM(%ebx)
    movl %cr0, %eax
    orl $1, %eax
    movl %eax, %cr0
    ljmp *(PARAM+0x28)(%ebx)     // -> pmode_entry(经 params+0x28 的 PMODE_IP/CS)

    .code32
pmode_entry:
    movw $2, %ax
    movw %ax, 0x61E0             // stage 2
    movl %cr4, %eax
    orl $0x20, %eax
    movl %eax, %cr4              // PAE
    movl PARAM+0x0C(%ebx), %eax
    movl %eax, %cr3              // 32 位模式下 CR3 只能写低 32 位,页表暂须 <4GiB;
                                 // 进长模式后 long_entry 会用 u64 重载完整 CR3
    movl $0xC0000080, %ecx
    rdmsr
    orl $0x100, %eax
    wrmsr                        // EFER.LME
    movl %cr0, %eax
    orl $0x80000000, %eax
    movl %eax, %cr0              // PG
    pushl $0x18
    movl PARAM+0x2C(%ebx), %eax  // LONG_IP 低 32 位
    pushl %eax
    lret                         // -> long_entry

    .code64
long_entry:
    movw $3, %ax
    movw %ax, 0x61E0             // stage 3
    movq PARAM+0x0C(%ebx), %rax
    movq %rax, %cr3              // 完整 64 位 CR3(支持 >4GiB 页表)
    lgdt PARAM(%ebx)             // 64 位 GDTR:limit u16 + base u64
    movw $0x10, %ax
    movw %ax, %ds
    movw %ax, %es
    movw %ax, %ss
    movw %ax, %fs
    movw %ax, %gs
    movq PARAM+0x14(%ebx), %rsp  // 64 位栈指针(独立字段,无跨字段风险)
    movq PARAM+0x1C(%ebx), %rax  // READY 64 位地址
    movl PARAM+0x24(%ebx), %ecx  // APID
    addl $1, %ecx
    movl %ecx, (%eax)
    movw $4, %ax
    movw %ax, 0x61E0             // stage 4
    movl PARAM+0x24(%ebx), %edi  // APID -> ap_entry(ap_id)
    movq PARAM+0x34(%ebx), %rax  // AP_ENTRY u64
    jmp *%rax                    // 交给内核 AP 入口,不再 hlt

    .global enable_sse_ap
    .type enable_sse_ap, @function
enable_sse_ap:
    mov %cr4, %rax
    or $0x600, %rax              // OSFXSR | OSXMMEXCPT
    mov %rax, %cr4
    ret

    .org PARAM
params:
    .fill 0x40, 1, 0
tramp_end:
