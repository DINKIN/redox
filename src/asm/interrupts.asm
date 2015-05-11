struc IDTEntry
    .offsetl resw 1
    .selector resw 1
    .zero resb 1
    .attribute resb 1
        .present equ 1 << 7
        .ring.1	equ 1 << 5
        .ring.2 equ 1 << 6
        .ring.3 equ 1 << 5 | 1 << 6
        .task32 equ 0x5
        .interrupt16 equ 0x6
        .trap16 equ 0x7
        .interrupt32 equ 0xE
        .trap32 equ 0xF
    .offseth resw 1
endstruc

[section .text]
[BITS 32]
interrupts:
.first:
    mov [0x200000], byte 0
    jmp dword .handle
.second:
%assign i 1
%rep 255
    mov [0x200000], byte i
    jmp dword .handle
%assign i i+1
%endrep
.handle:
    pushad
    mov ebp, esp
    push dword [0x200000]
    call [.handler]
    add esp, 4
    popad
    ; The kernel runs hlt and handles each IRQ individually, TODO: Syscalls
    iretd

.handler: dw 0

idtr:
    dw (idt_end - idt) + 1
    dd idt

idt:
%assign i 0
%rep 256	;fill in overrideable functions
	istruc IDTEntry
		at IDTEntry.offsetl, dw interrupts+(interrupts.second-interrupts.first)*i
		at IDTEntry.selector, dw 0x08
		at IDTEntry.attribute, db IDTEntry.present | IDTEntry.interrupt32
	iend
%assign i i+1
%endrep
idt_end:
