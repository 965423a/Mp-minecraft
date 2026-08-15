// AP trampoline:复制到 0x7000,SIPI vector 7 启动。
// 布局(相对 tramp_start):
//   0x00  real_entry(实模式,代码在前)
//   pmode_entry / long_entry 由编译后 xxd 校准
//   0xC0  params(32 字节):GDT_LO GDT_HI CR3 STACK READY APID PMODE_IP/CS LONG_IP

.set PARAM, 0xC0

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
    lgdtl PARAM(%ebx)            // GDT desc @ params
    movl %cr0, %eax
    orl $1, %eax
    movl %eax, %cr0
    ljmp *(PARAM+0x18)(%ebx)     // -> pmode_entry(经 params+0x18)

    .code32
pmode_entry:
    movw $2, %ax
    movw %ax, 0x61E0             // stage 2
    movl %cr4, %eax
    orl $0x20, %eax
    movl %eax, %cr4              // PAE
    movl PARAM+0x08(%ebx), %eax
    movl %eax, %cr3              // 页表(物理 < 4G,高 32 位清零)
    movl $0xC0000080, %ecx
    rdmsr
    orl $0x100, %eax
    wrmsr                        // EFER.LME
    movl %cr0, %eax
    orl $0x80000000, %eax
    movl %eax, %cr0              // PG
    pushl $0x18
    movl PARAM+0x1C(%ebx), %eax
    pushl %eax
    lret                         // -> long_entry

    .code64
long_entry:
    movw $3, %ax
    movw %ax, 0x61E0             // stage 3
    movw $0x10, %ax
    movw %ax, %ds
    movw %ax, %es
    movw %ax, %ss
    movw %ax, %fs
    movw %ax, %gs
    movq PARAM+0x0C(%ebx), %rsp  // stack
    movl PARAM+0x10(%ebx), %eax  // READY
    movl PARAM+0x14(%ebx), %ecx  // APID
    addl $1, %ecx
    movl %ecx, (%eax)
    movw $4, %ax
    movw %ax, 0x61E0             // stage 4
1:  hlt
    jmp 1b

    .org PARAM
params:
    .fill 0x20, 1, 0
tramp_end:
