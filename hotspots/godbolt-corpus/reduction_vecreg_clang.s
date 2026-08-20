.LCPI0_0:
  .long 0
  .long 1
  .long 2
  .long 3
  .long 4
  .long 5
  .long 6
  .long 7
.LCPI0_1:
  .long 15
.LCPI0_2:
  .long 8
.LCPI0_3:
  .long 0x3e800000
.LCPI0_4:
  .long 7
.LCPI0_5:
  .long 0x3e000000
.LCPI0_6:
  .long 32
.LCPI0_7:
  .long 0x48370000
main:
  pushq %rbx
  subq $16, %rsp
  movl $5000, %ebx
  cmpl $2, %edi
  jl .LBB0_2
  movq 8(%rsi), %rdi
  xorl %esi, %esi
  movl $10, %edx
  callq strtol@PLT
  movq %rax, %rbx
.LBB0_2:
  vmovaps .LCPI0_0(%rip), %ymm0
  xorl %eax, %eax
  vbroadcastss .LCPI0_1(%rip), %ymm1
  vbroadcastss .LCPI0_2(%rip), %ymm2
  vbroadcastss .LCPI0_3(%rip), %ymm3
  leaq input_a(%rip), %rcx
  vbroadcastss .LCPI0_4(%rip), %ymm4
  vbroadcastss .LCPI0_5(%rip), %ymm5
  leaq input_b(%rip), %rdx
  vpbroadcastd .LCPI0_6(%rip), %ymm6
.LBB0_3:
  vandps %ymm1, %ymm0, %ymm7
  vxorps %ymm2, %ymm7, %ymm8
  vcvtdq2ps %ymm7, %ymm7
  vcvtdq2ps %ymm8, %ymm8
  vmulps %ymm3, %ymm7, %ymm7
  vmulps %ymm3, %ymm8, %ymm8
  vmovups %ymm7, (%rcx,%rax,4)
  vmovups %ymm8, 32(%rcx,%rax,4)
  vmovups %ymm7, 64(%rcx,%rax,4)
  vmovups %ymm8, 96(%rcx,%rax,4)
  vandps %ymm4, %ymm0, %ymm7
  vcvtdq2ps %ymm7, %ymm7
  vmulps %ymm5, %ymm7, %ymm7
  vmovups %ymm7, (%rdx,%rax,4)
  vmovups %ymm7, 32(%rdx,%rax,4)
  vmovups %ymm7, 64(%rdx,%rax,4)
  vmovups %ymm7, 96(%rdx,%rax,4)
  addq $32, %rax
  vpaddd %ymm6, %ymm0, %ymm0
  cmpq $65536, %rax
  jne .LBB0_3
  testl %ebx, %ebx
  jle .LBB0_7
  vzeroupper
  callq sum_f32
  vmovss %xmm0, 12(%rsp)
.LBB0_6:
  vmovss 12(%rsp), %xmm0
  vmovss %xmm0, sink(%rip)
  callq dot_f32
  vaddss sink(%rip), %xmm0, %xmm0
  vmovss %xmm0, sink(%rip)
  decl %ebx
  jne .LBB0_6
.LBB0_7:
  vmovss sink(%rip), %xmm0
  vcvtss2sd %xmm0, %xmm0, %xmm0
  leaq .L.str(%rip), %rdi
  movb $1, %al
  vzeroupper
  callq printf@PLT
  vmovss .LCPI0_7(%rip), %xmm0
  vcmpneqss sink(%rip), %xmm0, %xmm0
  vmovd %xmm0, %eax
  andl $1, %eax
  addq $16, %rsp
  popq %rbx
  retq

sum_f32:
  vxorps %xmm0, %xmm0, %xmm0
  xorl %eax, %eax
  leaq input_a(%rip), %rcx
.LBB1_1:
  vaddss (%rcx,%rax,4), %xmm0, %xmm0
  vaddss 4(%rcx,%rax,4), %xmm0, %xmm0
  vaddss 8(%rcx,%rax,4), %xmm0, %xmm0
  vaddss 12(%rcx,%rax,4), %xmm0, %xmm0
  vaddss 16(%rcx,%rax,4), %xmm0, %xmm0
  vaddss 20(%rcx,%rax,4), %xmm0, %xmm0
  vaddss 24(%rcx,%rax,4), %xmm0, %xmm0
  vaddss 28(%rcx,%rax,4), %xmm0, %xmm0
  addq $8, %rax
  cmpq $65536, %rax
  jne .LBB1_1
  retq

dot_f32:
  vxorps %xmm0, %xmm0, %xmm0
  xorl %eax, %eax
  leaq input_a(%rip), %rcx
  leaq input_b(%rip), %rdx
.LBB2_1:
  vmovss (%rcx,%rax,4), %xmm1
  vmovss 4(%rcx,%rax,4), %xmm2
  vfmadd132ss (%rdx,%rax,4), %xmm0, %xmm1
  vfmadd231ss 4(%rdx,%rax,4), %xmm2, %xmm1
  vmovss 8(%rcx,%rax,4), %xmm0
  vfmadd132ss 8(%rdx,%rax,4), %xmm1, %xmm0
  vmovss 12(%rcx,%rax,4), %xmm1
  vfmadd132ss 12(%rdx,%rax,4), %xmm0, %xmm1
  vmovss 16(%rcx,%rax,4), %xmm0
  vfmadd132ss 16(%rdx,%rax,4), %xmm1, %xmm0
  vmovss 20(%rcx,%rax,4), %xmm1
  vfmadd132ss 20(%rdx,%rax,4), %xmm0, %xmm1
  vmovss 24(%rcx,%rax,4), %xmm2
  vfmadd132ss 24(%rdx,%rax,4), %xmm1, %xmm2
  vmovss 28(%rcx,%rax,4), %xmm0
  vfmadd132ss 28(%rdx,%rax,4), %xmm2, %xmm0
  addq $8, %rax
  cmpq $65536, %rax
  jne .LBB2_1
  retq

.L.str:
  .asciz "%.0f\n"

