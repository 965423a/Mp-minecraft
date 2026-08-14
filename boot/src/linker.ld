ENTRY(_start)

SECTIONS {
    . = 1M;
    .multiboot2 : ALIGN(8) {
        KEEP(*(.multiboot2))
    }
    .text : ALIGN(4096) {
        *(.text .text.*)
    }
    .rodata : ALIGN(4096) {
        *(.rodata .rodata.*)
    }
    .data : ALIGN(4096) {
        *(.data .data.*)
    }
    .bss : ALIGN(4096) {
        *(COMMON)
        *(.bss .bss.*)
    }
    . = ALIGN(4096);
    _end = .;
    /DISCARD/ : {
        *(.comment)
        *(.note .note.*)
    }
}