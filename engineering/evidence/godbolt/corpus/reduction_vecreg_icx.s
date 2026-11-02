.LCPI0_0:
  .long 0
  .long 1
  .long 2
  .long 3
.LCPI0_3:
  .long 0x00000000
  .long 0x3e000000
  .long 0x3e800000
  .long 0x3ec00000
.LCPI0_4:
  .long 0x3f000000
  .long 0x3f200000
  .long 0x3f400000
  .long 0x3f600000
.LCPI0_1:
  .long 15
.LCPI0_2:
  .long 0x3e800000
.LCPI0_5:
  .long 0x48370000
main:
  pushq %rbx
  subq $16, %rsp
  vstmxcsr 12(%rsp)
  orl $32832, 12(%rsp)
  vldmxcsr 12(%rsp)
  movl $5000, %ebx
  cmpl $2, %edi
  jl .LBB0_2
  movq 8(%rsi), %rdi
  xorl %esi, %esi
  movl $10, %edx
  callq strtol
  movq %rax, %rbx
.LBB0_2:
  movl $4, %eax
  vmovdqu .LCPI0_0(%rip), %xmm0
  vpbroadcastd .LCPI0_1(%rip), %xmm1
  vbroadcastss .LCPI0_2(%rip), %xmm2
  vmovups .LCPI0_3(%rip), %xmm3
  vmovups .LCPI0_4(%rip), %xmm4
.LBB0_3:
  leal 12(%rax), %ecx
  vmovd %ecx, %xmm5
  vpbroadcastb %xmm5, %xmm5
  vpor %xmm0, %xmm5, %xmm5
  vpand %xmm1, %xmm5, %xmm5
  vcvtdq2ps %xmm5, %xmm5
  vmulps %xmm2, %xmm5, %xmm5
  vmovups %xmm5, input_a-16(,%rax,4)
  vmovups %xmm3, input_b-16(,%rax,4)
  vmovd %eax, %xmm5
  vpbroadcastd %xmm5, %xmm5
  vpor %xmm0, %xmm5, %xmm5
  vpand %xmm1, %xmm5, %xmm5
  vcvtdq2ps %xmm5, %xmm5
  vmulps %xmm2, %xmm5, %xmm5
  vmovups %xmm5, input_a(,%rax,4)
  vmovups %xmm4, input_b(,%rax,4)
  leaq 8(%rax), %rcx
  cmpq $65532, %rax
  movq %rcx, %rax
  jb .LBB0_3
  testl %ebx, %ebx
  jle .LBB0_6
.LBB0_5:
  callq sum_f32
  vmovd %xmm0, sink(%rip)
  callq dot_f32
  vaddss sink(%rip), %xmm0, %xmm0
  vmovss %xmm0, sink(%rip)
  decl %ebx
  jne .LBB0_5
.LBB0_6:
  vmovss sink(%rip), %xmm0
  vcvtss2sd %xmm0, %xmm0, %xmm0
  movl $.L.str, %edi
  movb $1, %al
  callq printf
  vmovss .LCPI0_5(%rip), %xmm0
  xorl %eax, %eax
  vucomiss sink(%rip), %xmm0
  setne %al
  addq $16, %rsp
  popq %rbx
  retq

sum_f32:
  vxorps %xmm0, %xmm0, %xmm0
  movq $-8, %rax
.LBB1_1:
  vmovups input_a+64(,%rax,4), %ymm1
  vmovups input_a+128(,%rax,4), %ymm2
  vaddps input_a+32(,%rax,4), %ymm0, %ymm0
  vaddps input_a+96(,%rax,4), %ymm1, %ymm1
  vmovups input_a+192(,%rax,4), %ymm3
  vaddps %ymm0, %ymm1, %ymm0
  vaddps input_a+160(,%rax,4), %ymm2, %ymm1
  vaddps input_a+224(,%rax,4), %ymm3, %ymm2
  vaddps %ymm1, %ymm2, %ymm1
  vaddps %ymm0, %ymm1, %ymm0
  vmovups input_a+288(,%rax,4), %ymm1
  vmovups input_a+352(,%rax,4), %ymm2
  vaddps input_a+256(,%rax,4), %ymm0, %ymm0
  vmovups input_a+416(,%rax,4), %ymm3
  vaddps input_a+320(,%rax,4), %ymm1, %ymm1
  vaddps %ymm0, %ymm1, %ymm0
  vaddps input_a+384(,%rax,4), %ymm2, %ymm1
  vaddps input_a+448(,%rax,4), %ymm3, %ymm2
  vaddps %ymm1, %ymm2, %ymm1
  vaddps %ymm0, %ymm1, %ymm0
  vaddps input_a+480(,%rax,4), %ymm0, %ymm0
  vaddps input_a+512(,%rax,4), %ymm0, %ymm0
  subq $-128, %rax
  cmpq $65528, %rax
  jb .LBB1_1
  vextractf128 $1, %ymm0, %xmm1
  vaddps %xmm1, %xmm0, %xmm0
  vshufpd $1, %xmm0, %xmm0, %xmm1
  vaddps %xmm1, %xmm0, %xmm0
  vmovshdup %xmm0, %xmm1
  vaddss %xmm1, %xmm0, %xmm0
  vzeroupper
  retq

dot_f32:
  vxorps %xmm0, %xmm0, %xmm0
  movq $-8, %rax
.LBB2_1:
  vmovups input_b+32(,%rax,4), %ymm1
  vfmadd132ps input_a+32(,%rax,4), %ymm0, %ymm1
  vmovups input_b+64(,%rax,4), %ymm0
  vfmadd132ps input_a+64(,%rax,4), %ymm1, %ymm0
  vmovups input_b+96(,%rax,4), %ymm1
  vfmadd132ps input_a+96(,%rax,4), %ymm0, %ymm1
  vmovups input_b+128(,%rax,4), %ymm0
  vfmadd132ps input_a+128(,%rax,4), %ymm1, %ymm0
  vmovups input_b+160(,%rax,4), %ymm1
  vfmadd132ps input_a+160(,%rax,4), %ymm0, %ymm1
  vmovups input_b+192(,%rax,4), %ymm0
  vfmadd132ps input_a+192(,%rax,4), %ymm1, %ymm0
  vmovups input_b+224(,%rax,4), %ymm1
  vfmadd132ps input_a+224(,%rax,4), %ymm0, %ymm1
  vmovups input_b+256(,%rax,4), %ymm0
  vfmadd132ps input_a+256(,%rax,4), %ymm1, %ymm0
  addq $64, %rax
  cmpq $65528, %rax
  jb .LBB2_1
  vextractf128 $1, %ymm0, %xmm1
  vaddps %xmm1, %xmm0, %xmm0
  vshufpd $1, %xmm0, %xmm0, %xmm1
  vaddps %xmm1, %xmm0, %xmm0
  vmovshdup %xmm0, %xmm1
  vaddss %xmm1, %xmm0, %xmm0
  vzeroupper
  retq

.L.str:
  .asciz "%.0f\n"

