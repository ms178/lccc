# Differential assembler corpus: LCCC's assembler must emit byte-identical
# output to GNU as for every instruction LCCC's codegen can produce.
.text
broadcast:
    vpbroadcastb (%rax), %ymm0
    vpbroadcastw (%rax), %ymm0
    vpbroadcastd (%rax), %ymm0
    vpbroadcastq (%rax), %ymm0
    vpbroadcastb %xmm2, %ymm1
    vpbroadcastw %xmm2, %ymm1
    vpbroadcastd %xmm2, %ymm1
    vpbroadcastq %xmm2, %ymm1
    {vex} vpdpbusd %xmm2, %xmm1, %xmm0
    {vex} vpdpbusds %ymm2, %ymm1, %ymm0
    vpdpwusd %xmm2, %xmm1, %xmm0
    vpdpwusds %ymm2, %ymm1, %ymm0
    vpdpbssd %xmm2, %xmm1, %xmm0
    vpdpbssds %xmm2, %xmm1, %xmm0
    vpdpbsud %xmm2, %xmm1, %xmm0
    vpdpbsuds %xmm2, %xmm1, %xmm0
    vpdpbuud %xmm2, %xmm1, %xmm0
    vpdpbuuds %xmm2, %xmm1, %xmm0
    vpdpwuud %xmm2, %xmm1, %xmm0
    vpdpwuuds %xmm2, %xmm1, %xmm0
    {vex} vpdpwssd %xmm2, %xmm1, %xmm0
    {vex} vpdpwssds %xmm2, %xmm1, %xmm0
    gf2p8mulb %xmm2, %xmm1
    gf2p8affineqb $0x3f, %xmm2, %xmm1
    gf2p8affineinvqb $0x3f, %xmm2, %xmm1
    vaesenc %ymm2, %ymm1, %ymm0
    vaesenclast %ymm2, %ymm1, %ymm0
    vaesdec %ymm2, %ymm1, %ymm0
    vaesdeclast %ymm2, %ymm1, %ymm0
    vaesenc %xmm2, %xmm1, %xmm0
    vaesimc %xmm1, %xmm0
    vaeskeygenassist $0x12, %xmm1, %xmm0
    vpclmulqdq $0x11, %ymm2, %ymm1, %ymm0
    vpclmulqdq $0x10, %xmm2, %xmm1, %xmm0
sse2:
    paddb %xmm2, %xmm1
    psubb %xmm2, %xmm1
    psubusw %xmm2, %xmm1
    psadbw %xmm2, %xmm1
    pmullw %xmm2, %xmm1
    pmaddubsw %xmm2, %xmm1
    pshufb %xmm2, %xmm1
    phaddw %xmm2, %xmm1
    phaddd %xmm2, %xmm1
    palignr $0x07, %xmm2, %xmm1
    pabsb %xmm1, %xmm0
    pabsw %xmm1, %xmm0
    pabsd %xmm1, %xmm0
    pmaxub %xmm2, %xmm1
    pminub %xmm2, %xmm1
    pblendvb %xmm2, %xmm1
    pmovzxbw %xmm1, %xmm0
    pmovzxwd %xmm1, %xmm0
avx2:
    vpaddb %ymm2, %ymm1, %ymm0
    vpaddw %ymm2, %ymm1, %ymm0
    vpaddd %ymm2, %ymm1, %ymm0
    vpsubb %ymm2, %ymm1, %ymm0
    vpsubw %ymm2, %ymm1, %ymm0
    vpsubusw %ymm2, %ymm1, %ymm0
    vpsadbw %ymm2, %ymm1, %ymm0
    vpmaddubsw %ymm2, %ymm1, %ymm0
    vpcmpeqb %ymm2, %ymm1, %ymm0
    vpshufb %ymm2, %ymm1, %ymm0
    vpabsb %ymm1, %ymm0
    vpmaxub %ymm2, %ymm1, %ymm0
    vpxor %ymm2, %ymm1, %ymm0
    vpor %ymm2, %ymm1, %ymm0
    vpand %ymm2, %ymm1, %ymm0
    vpslld $0x03, %ymm1, %ymm0
    vpsrld $0x03, %ymm1, %ymm0
    vpsllw $0x03, %ymm1, %ymm0
    vpsrlw $0x03, %ymm1, %ymm0
    vpmovmskb %ymm1, %eax
    vbroadcasti128 32(%rax), %ymm0
    vinserti128 $1, %xmm1, %ymm0, %ymm0
    vextracti128 $1, %ymm0, %xmm1
    vmovdqu 32(%rax), %ymm1
    vmovdqu %ymm1, 32(%rax)
addressing_v7:
    vpaddb 32(%r8,%r9,4), %ymm1, %ymm0
    vpxor -128(%rbp), %ymm8, %ymm15
    vmovdqu -128(%rbp), %ymm15
    vmovdqu %ymm15, 4660(%r12,%r13,8)
    vpbroadcastd 127(%r8,%r9,2), %ymm14
    paddb %xmm15, %xmm8
    paddb %mm2, %mm1
    paddb 32(%r8,%r9,4), %mm1
    psubb %mm2, %mm1
    psubb 32(%r8,%r9,4), %mm1
    movq %r15, 4660(%r12,%r13,8)
    movq 4660(%r12,%r13,8), %r15
    addq %r15, %r8
    subq 127(%r12,%r13,4), %r9
