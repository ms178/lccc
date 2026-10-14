# AVX2 vector masks are read/write operands. The illegal twin substitutes
# the copy source for the mask and thereby corrupts that still-live source.
.text
.macro GATHER_WITNESS name, opcode, rewrite
.globl \name
.type \name, @function
\name:
    leaq gather_data(%rip), %rdi
    vpcmpeqd %xmm0, %xmm0, %xmm0
    vmovaps %xmm0, %xmm2
    vpxor %xmm3, %xmm3, %xmm3
    vpxor %xmm4, %xmm4, %xmm4
.if \rewrite
    \opcode %xmm0, (%rdi,%xmm3,4), %xmm4
.else
    \opcode %xmm2, (%rdi,%xmm3,4), %xmm4
.endif
    vpmovmskb %xmm0, %eax
    cmpl $65535, %eax
    sete %al
    movzbl %al, %eax
    ret
.size \name, .-\name
.endm
GATHER_WITNESS original_vgatherdps, vgatherdps, 0
GATHER_WITNESS rewritten_vgatherdps, vgatherdps, 1
GATHER_WITNESS original_vgatherdpd, vgatherdpd, 0
GATHER_WITNESS rewritten_vgatherdpd, vgatherdpd, 1
GATHER_WITNESS original_vgatherqps, vgatherqps, 0
GATHER_WITNESS rewritten_vgatherqps, vgatherqps, 1
GATHER_WITNESS original_vgatherqpd, vgatherqpd, 0
GATHER_WITNESS rewritten_vgatherqpd, vgatherqpd, 1
GATHER_WITNESS original_vpgatherdd, vpgatherdd, 0
GATHER_WITNESS rewritten_vpgatherdd, vpgatherdd, 1
GATHER_WITNESS original_vpgatherdq, vpgatherdq, 0
GATHER_WITNESS rewritten_vpgatherdq, vpgatherdq, 1
GATHER_WITNESS original_vpgatherqd, vpgatherqd, 0
GATHER_WITNESS rewritten_vpgatherqd, vpgatherqd, 1
GATHER_WITNESS original_vpgatherqq, vpgatherqq, 0
GATHER_WITNESS rewritten_vpgatherqq, vpgatherqq, 1
.section .rodata
.p2align 4
gather_data:
    .long 1, 2, 3, 4
.section .note.GNU-stack,"",@progbits
