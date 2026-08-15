/* SMP AP trampoline:实模式入口,复制到 0x7000(SIPI vector 7)执行。
   布局:[tramp_start, tramp_end) 整体被复制,参数区位于开头:
     0x00 GDT_LO u32   (limit<<16 | gdt_base_lo)
     0x04 GDT_HI u32   (gdt_base >> 16)
     0x08 CR3    u32
     0x0C STACK  u32   (per-AP 栈顶)
     0x10 READY  u32   (就绪标记地址,写 apid+1)
     0x14 APID   u32
     0x18 PMODE_IP u16 (0x7000 + pmode_entry 偏移)
     0x1A PMODE_CS u16 (0x08)
     0x1C LONG_IP u32  (0x7000 + long_entry 偏移)
   代码从 0x20 开始。偏移常量由 smp.rs 硬编码(经 hexdump 校准)。 */

.section .text
.global tramp_start
tramp_start:
    .fill 0x20, 1, 0

.code16
real_entry:
    cli
    cld
    /* bx = 运行基址(0x7000) */
    call 1f
1:  popw %bx
    subl $1f - real_entry, %ebx
    /* 加载 GDT(参数区 {limit32, base64},高 32 位 base = 0) */
    lgdtl 0x00(%ebx)
    /* 开 PE */
    movl %cr0, %eax
    orl $1, %eax
    movl %eax, %cr0
    /* 远跳 32 位:目标 = 参数区 PMODE_IP/CS */
    ljmp *0x18(%bx)

.code32
pmode_entry:
    movw $0x10, %ax
    movw %ax, %ds
    movw %ax, %es
    /* PAE + OSFXSR | OSXMMEXCPT */
    movl %cr4, %eax
    orl $0x620, %eax
    movl %eax, %cr4
    /* EFER.LME */
    movl $0xC0000080, %ecx
    rdmsr
    orl $0x100, %eax
    wrmsr
    /* cr3 = BSP 页表 */
    movl 0x08(%ebx), %eax
    movl %eax, %cr3
    /* 开 PG */
    movl %cr0, %eax
    orl $0x80000001, %eax
    movl %eax, %cr0
    /* 远跳 64 位:lret 栈 {CS, LONG_IP} */
    pushl $0x18
    movl 0x1C(%ebx), %eax
    pushl %eax
    lret

.code64
long_entry:
    movw $0x10, %ax
    movw %ax, %ds
    movw %ax, %es
    movw %ax, %ss
    movq 0x0C(%rbx), %rsp
    movq 0x10(%rbx), %rax
    movl 0x14(%rbx), %ecx
    addl $1, %ecx
    movl %ecx, (%rax)
    /* AP 主循环:等待调度 */
1:  hlt
    jmp 1b

.global tramp_end
tramp_end:
