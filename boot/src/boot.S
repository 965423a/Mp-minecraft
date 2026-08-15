/* Multiboot2 header + 长模式入口。 */
.set MB2_MAGIC, 0xE85250D6
.set MB2_ARCH_I386, 0

.section .multiboot2, "a"
.align 8
header_start:
    .long MB2_MAGIC
    .long MB2_ARCH_I386
    .long header_end - header_start
    .long 0x100000000 - (MB2_MAGIC + MB2_ARCH_I386 + (header_end - header_start))
    /* 结束 tag:type=0, flags=0, size=8 */
    .short 0, 0
    .long 8
header_end:
    .align 8

.section .bss
.align 4096
pml4:
    .skip 4096
pdpt:
    .skip 4096
pd:
    .skip 4096

.section .text
.global _start
.code32
_start:
    /* 关中断:内核尚无 IDT,PIT 定时器中断会跳垃圾 IDT 破坏内存 */
    cli

    /* 保存 multiboot2 info 指针 (ebx) 与 magic (eax) */
    movl %ebx, %ebp
    movl %eax, %edi

    /* 1. 检查 CPUID */
    pushfl
    popl %eax
    movl %eax, %ecx
    xorl $0x200000, %eax
    pushl %eax
    popfl
    pushfl
    popl %eax
    xorl %ecx, %eax
    je halt_error
    /* CPUID 支持,检查长模式 */
    movl $0x80000000, %eax
    cpuid
    cmpl $0x80000001, %eax
    jb halt_error
    movl $0x80000001, %eax
    cpuid
    testl $0x20000000, %edx   /* LM bit 29 */
    jz halt_error

    /* 2. 建立过渡恒等页表:2MiB 大页映射前 1GiB(仅用于进入 64 位;
          完整 16GiB 映射由 Rust 侧 64 位代码 setup_paging() 建立) */
    movl $pml4, %eax
    movl %eax, %cr3
    /* PML4[0] -> PDPT */
    movl $pdpt, %eax
    orl $0x3, %eax
    movl %eax, pml4
    /* PDPT[0] -> PD */
    movl $pd, %eax
    orl $0x3, %eax
    movl %eax, pdpt
    /* PD[0..511] -> 2MiB 页 */
    movl $pd, %edi
    xorl %ecx, %ecx
1:
    movl %ecx, %eax
    shll $21, %eax
    orl $0x83, %eax          /* present + writable + huge */
    movl %eax, (%edi, %ecx, 8)
    addl $1, %ecx
    cmpl $512, %ecx
    jb 1b

    /* 3. 启用 PAE + SSE (OSFXSR | OSXMMEXCPT) */
    movl %cr4, %eax
    orl $0x620, %eax
    movl %eax, %cr4

    /* 4. EFER.LME */
    movl $0xC0000080, %ecx
    rdmsr
    orl $0x100, %eax
    wrmsr

    /* 5. 加载 GDT */
    lgdt gdt_desc

    /* 6. 开 PE(PG 一起) */
    movl %cr0, %eax
    orl $0x80000001, %eax
    movl %eax, %cr0

    /* 7. 远跳进入 64 位 */
    pushl $0x18
    pushl $long_start
    lret

.code64
long_start:
    movq %rbp, %rdi      /* multiboot2 info */
    movq %rdi, %r15      /* 保存 */
    /* 数据段选择器 */
    movw $0x10, %ax
    movw %ax, %ds
    movw %ax, %es
    movw %ax, %ss
    /* 栈 */
    movq $stack_top, %rsp
    movq %r15, %rdi
    call kernel_main
1:
    hlt
    jmp 1b

halt_error:
    /* 长模式不可用:死循环(串口可观察) */
1:
    hlt
    jmp 1b

.section .bss
.align 16
stack_bottom:
    .skip 65536
stack_top:

.section .data
.align 8
.global gdt_desc
gdt:
    .quad 0x0000000000000000  /* null */
    .quad 0x00CF9A000000FFFF  /* 0x08 32-bit code */
    .quad 0x00CF92000000FFFF  /* 0x10 data */
    .quad 0x00AF9A0000000000  /* 0x18 64-bit code */
gdt_desc:
    .short gdt_desc - gdt - 1
    .quad gdt