.LCPI0_0:
  .long 0x00000000
  .long 0x3e800000
  .long 0x3f000000
  .long 0x3f400000
.LCPI0_1:
  .long 0x3f800000
  .long 0x3fa00000
  .long 0x3fc00000
  .long 0x3fe00000
.LCPI0_2:
  .long 0x40000000
  .long 0x40100000
  .long 0x40200000
  .long 0x40300000
.LCPI0_3:
  .long 0x40400000
  .long 0x40500000
  .long 0x40600000
  .long 0x40700000
.LCPI0_4:
  .quad 0x4122ebc000000000
.LCPI0_5:
  .quad 0x41224f8000000000
main:
  pushq %rbx
  subq $32, %rsp
  vstmxcsr 12(%rsp)
  orl $32832, 12(%rsp)
  vldmxcsr 12(%rsp)
  movl $1000, %ebx
  cmpl $2, %edi
  jl .LBB0_2
  movq 8(%rsi), %rdi
  xorl %esi, %esi
  movl $10, %edx
  callq strtol
  movq %rax, %rbx
.LBB0_2:
  movq $-4, %rax
  vmovupd .LCPI0_0(%rip), %xmm0
  vmovupd .LCPI0_1(%rip), %xmm1
  vmovupd .LCPI0_2(%rip), %xmm2
  vmovupd .LCPI0_3(%rip), %xmm3
.LBB0_3:
  vmovupd %xmm0, input+16(,%rax,4)
  vmovupd %xmm1, input+32(,%rax,4)
  vmovupd %xmm2, input+48(,%rax,4)
  vmovupd %xmm3, input+64(,%rax,4)
  addq $16, %rax
  cmpq $65532, %rax
  jb .LBB0_3
  testl %ebx, %ebx
  jle .LBB0_6
.LBB0_5:
  callq stencil5
  decl %ebx
  jne .LBB0_5
.LBB0_6:
  vxorpd %xmm0, %xmm0, %xmm0
  movq $-4, %rax
.LBB0_7:
  vcvtps2pd output+16(,%rax,4), %ymm1
  vcvtps2pd output+32(,%rax,4), %ymm2
  vcvtps2pd output+48(,%rax,4), %ymm3
  vcvtps2pd output+64(,%rax,4), %ymm4
  vcvtps2pd output+80(,%rax,4), %ymm5
  vcvtps2pd output+96(,%rax,4), %ymm6
  vcvtps2pd output+112(,%rax,4), %ymm7
  vcvtps2pd output+128(,%rax,4), %ymm8
  vaddpd %ymm7, %ymm8, %ymm7
  vaddpd %ymm5, %ymm4, %ymm4
  vaddpd %ymm6, %ymm3, %ymm3
  vaddpd %ymm2, %ymm1, %ymm1
  vaddpd %ymm0, %ymm7, %ymm0
  vaddpd %ymm4, %ymm3, %ymm2
  vaddpd %ymm1, %ymm0, %ymm0
  vaddpd %ymm2, %ymm0, %ymm0
  addq $32, %rax
  cmpq $65532, %rax
  jb .LBB0_7
  vextractf128 $1, %ymm0, %xmm1
  vaddpd %xmm1, %xmm0, %xmm0
  vshufpd $1, %xmm0, %xmm0, %xmm1
  vaddpd %xmm1, %xmm0, %xmm0
  vmovupd %xmm0, 16(%rsp)
  movl $.L.str, %edi
  movb $1, %al
  vzeroupper
  callq printf
  vmovsd .LCPI0_4(%rip), %xmm0
  vmovupd 16(%rsp), %xmm2
  vcmplepd %xmm2, %xmm0, %xmm0
  vmovsd .LCPI0_5(%rip), %xmm1
  vcmplepd %xmm1, %xmm2, %xmm1
  vorpd %xmm0, %xmm1, %xmm0
  vmovd %xmm0, %eax
  andl $1, %eax
  addq $32, %rsp
  popq %rbx
  retq

stencil5:
  vmovss input(%rip), %xmm2
  vmovss input+4(%rip), %xmm3
  vmovss input+8(%rip), %xmm0
  movq $-8, %rax
  vmovss input+12(%rip), %xmm1
.LBB1_1:
  vaddss %xmm2, %xmm3, %xmm4
  vmovaps %xmm3, %xmm2
  vmovaps %xmm0, %xmm3
  vmovaps %xmm1, %xmm0
  vaddss %xmm1, %xmm3, %xmm1
  vaddss %xmm1, %xmm4, %xmm4
  vmovss input+24(%rax), %xmm1
  vaddss %xmm1, %xmm4, %xmm4
  vmovss %xmm4, output+16(%rax)
  addq $4, %rax
  jne .LBB1_1
  movq $-6, %rax
.LBB1_3:
  vmovups input+32(,%rax,4), %ymm0
  vmovups input+40(,%rax,4), %ymm1
  vaddps input+36(,%rax,4), %ymm0, %ymm0
  vaddps input+44(,%rax,4), %ymm1, %ymm1
  vaddps input+48(,%rax,4), %ymm0, %ymm0
  vaddps %ymm1, %ymm0, %ymm0
  vmovups %ymm0, output+40(,%rax,4)
  addq $8, %rax
  cmpq $65522, %rax
  jb .LBB1_3
  vmovss input+262120(%rip), %xmm3
  vmovss input+262124(%rip), %xmm0
  vmovss input+262128(%rip), %xmm1
  movq $-8, %rax
  vmovss input+262132(%rip), %xmm2
.LBB1_5:
  vaddss %xmm3, %xmm0, %xmm3
  vaddss %xmm2, %xmm1, %xmm4
  vaddss %xmm4, %xmm3, %xmm4
  vmovaps %xmm0, %xmm3
  vmovaps %xmm1, %xmm0
  vmovaps %xmm2, %xmm1
  vmovss input+262144(%rax), %xmm2
  vaddss %xmm2, %xmm4, %xmm4
  vmovss %xmm4, output+262136(%rax)
  addq $4, %rax
  jne .LBB1_5
  vzeroupper
  retq

.L.str:
  .asciz "%.0f\n"

