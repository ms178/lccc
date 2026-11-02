.LCPI0_0:
.LCPI0_10:
.LCPI0_11:
.LCPI0_12:
.LCPI0_13:
.LCPI0_14:
.LCPI0_15:
.LCPI0_16:
.LCPI0_17:
.LCPI0_18:
main:
  vmovdqa .LCPI0_0(%rip), %ymm0
  xorl %eax, %eax
  vpbroadcastd .LCPI0_10(%rip), %ymm1
  vpbroadcastd .LCPI0_11(%rip), %ymm2
  vpbroadcastd .LCPI0_12(%rip), %ymm3
  vpbroadcastd .LCPI0_13(%rip), %ymm4
  leaq main.a(%rip), %rcx
  vpbroadcastd .LCPI0_14(%rip), %ymm5
  vpbroadcastd .LCPI0_15(%rip), %ymm6
  vpbroadcastd .LCPI0_16(%rip), %ymm7
  vpbroadcastd .LCPI0_17(%rip), %ymm8
  vpbroadcastd .LCPI0_18(%rip), %ymm9
.LBB0_1:
  vpsllw $5, %ymm0, %ymm10
  vpsubw %ymm0, %ymm10, %ymm10
  vpaddw %ymm1, %ymm10, %ymm11
  vpaddw %ymm2, %ymm10, %ymm12
  vpaddw %ymm3, %ymm10, %ymm13
  vpaddw %ymm4, %ymm10, %ymm14
  vmovdqu %ymm11, (%rcx,%rax,2)
  vmovdqu %ymm12, 32(%rcx,%rax,2)
  vmovdqu %ymm13, 64(%rcx,%rax,2)
  vmovdqu %ymm14, 96(%rcx,%rax,2)
  vpaddw %ymm5, %ymm10, %ymm11
  vpaddw %ymm6, %ymm10, %ymm12
  vpaddw %ymm7, %ymm10, %ymm13
  vpaddw %ymm8, %ymm10, %ymm10
  vmovdqu %ymm11, 128(%rcx,%rax,2)
  vmovdqu %ymm12, 160(%rcx,%rax,2)
  vmovdqu %ymm13, 192(%rcx,%rax,2)
  vmovdqu %ymm10, 224(%rcx,%rax,2)
  subq $-128, %rax
  vpaddw %ymm0, %ymm9, %ymm0
  cmpq $1024, %rax
  jne .LBB0_1
  xorl %esi, %esi
  leaq main.d(%rip), %rax
  xorl %edx, %edx
.LBB0_3:
  movzwl (%rcx,%rdx,2), %edi
  addl %edi, %esi
  movl %esi, (%rax,%rdx,4)
  movzwl 2(%rcx,%rdx,2), %edi
  addl %esi, %edi
  movl %edi, 4(%rax,%rdx,4)
  movzwl 4(%rcx,%rdx,2), %r8d
  addl %edi, %r8d
  movl %r8d, 8(%rax,%rdx,4)
  movzwl 6(%rcx,%rdx,2), %esi
  addl %r8d, %esi
  movl %esi, 12(%rax,%rdx,4)
  addq $4, %rdx
  cmpq $1024, %rdx
  jne .LBB0_3
  movl main.d+2044(%rip), %esi
  addl main.d(%rip), %esi
  addl main.d+4092(%rip), %esi
  movq $-49, %rcx
.LBB0_5:
  movq %rsi, %rdx
  shlq $5, %rdx
  addq %rsi, %rdx
  movl 196(%rax,%rcx,4), %esi
  movl 224(%rax,%rcx,4), %edi
  addq %rdx, %rsi
  addq %rsi, %rdi
  shlq $5, %rsi
  addq %rsi, %rdi
  movl 252(%rax,%rcx,4), %edx
  addq %rdi, %rdx
  shlq $5, %rdi
  addq %rdi, %rdx
  movl 280(%rax,%rcx,4), %esi
  addq %rdx, %rsi
  shlq $5, %rdx
  addq %rdx, %rsi
  movl 308(%rax,%rcx,4), %edx
  addq %rsi, %rdx
  shlq $5, %rsi
  addq %rsi, %rdx
  movl 336(%rax,%rcx,4), %edi
  addq %rdx, %rdi
  shlq $5, %rdx
  addq %rdx, %rdi
  movl 364(%rax,%rcx,4), %esi
  addq %rdi, %rsi
  shlq $5, %rdi
  addq %rdi, %rsi
  addq $49, %rcx
  cmpq $975, %rcx
  jb .LBB0_5
  pushq %rax
  leaq .L.str(%rip), %rdi
  xorl %eax, %eax
  vzeroupper
  callq printf@PLT
  xorl %eax, %eax
  popq %rcx
  retq

.L.str:

