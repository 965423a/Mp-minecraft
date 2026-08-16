/* IDT 中断 stub 表:256 入口,统一经 common_intr 进 int_handler。
   带错误码向量(10-14/17)CPU 已压 err,其余补 0 保持栈对称。 */

.section .text
.p2align 4

/* 调度器切栈:rdi = 目标栈顶(帧首),换栈后从 common_resume 恢复。 */
.global sched_switch_asm
sched_switch_asm:
    movq %rdi, %rsp
    jmp common_resume

.macro STUB n
stub_\n:
    .if \n == 10 || \n == 11 || \n == 12 || \n == 13 || \n == 14 || \n == 17
        pushq $\n
        popq %rax
        popq %rcx
        pushq %rax
        pushq %rcx
    .else
        pushq $\n
        pushq $0
    .endif
    jmp common_intr
.endm

.altmacro
.set i, 0
.rept 256
    STUB %i
    .set i, i + 1
.endr

common_intr:
    pushq %rax
    pushq %rcx
    pushq %rdx
    pushq %rbx
    pushq %rbp
    pushq %rsi
    pushq %rdi
    pushq %r8
    pushq %r9
    pushq %r10
    pushq %r11
    pushq %r12
    pushq %r13
    pushq %r14
    pushq %r15
    movq %rsp, %rdi
    call int_handler
common_resume:
    .global common_resume
    popq %r15
    popq %r14
    popq %r13
    popq %r12
    popq %r11
    popq %r10
    popq %r9
    popq %r8
    popq %rdi
    popq %rsi
    popq %rbp
    popq %rbx
    popq %rdx
    popq %rcx
    popq %rax
    addq $16, %rsp
    iretq

.section .rodata
.p2align 8
.global stub_table

.macro TAB n
    .quad stub_\n
.endm

stub_table:
.set i, 0
.rept 256
    TAB %i
    .set i, i + 1
.endr