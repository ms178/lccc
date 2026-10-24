# Native freestanding witness: AVX512F/VL and Linux IA32 execution required.
.text
.globl _start
_start:
    call 1f
1:  popl %ebx
    addl $_GLOBAL_OFFSET_TABLE_+(.-1b), %ebx
    vpbroadcastd input@GOTOFF(%ebx), %xmm0
    vmovd %xmm0, %eax
    cmpl $0x12345678, %eax
    jne .Lfail
    vpslld $3, input@GOTOFF(%ebx), %xmm1
    vmovd %xmm1, %eax
    cmpl $0x91a2b3c0, %eax
    jne .Lfail
    vpbroadcastd input@GOT(%ebx), %xmm2
    vmovd %xmm2, %eax
    leal input@GOTOFF(%ebx), %ecx
    cmpl %ecx, %eax
    jne .Lfail
    xorl %ebx, %ebx
    jmp .Lexit
.Lfail:
    movl $1, %ebx
.Lexit:
    movl $1, %eax
    int $0x80
.data
.p2align 4
input:
    .long 0x12345678, 1, 2, 3
.section .note.GNU-stack,"",@progbits
