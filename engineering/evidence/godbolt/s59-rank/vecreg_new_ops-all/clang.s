.LCPI0_0:
.LCPI0_1:
.LCPI0_2:
main:
  pushq %rbx
  subq $64, %rsp
  movl $1000, %r8d
  cmpl $2, %edi
  jl .LBB0_2
  movq 8(%rsi), %rdi
  xorl %esi, %esi
  xorl %edx, %edx
  callq strtoul@PLT
  movq %rax, %r8
  decq %rax
  movl $4294967294, %ecx
  movl $2, %ebx
  cmpq %rcx, %rax
  ja .LBB0_3
.LBB0_2:
  vmovaps .LCPI0_0(%rip), %xmm0
  vmovaps %xmm0, 32(%rsp)
  vmovaps .LCPI0_1(%rip), %xmm0
  vmovaps %xmm0, 16(%rsp)
  vmovaps .LCPI0_2(%rip), %xmm0
  vmovaps %xmm0, (%rsp)
  leaq 32(%rsp), %rdi
  leaq 16(%rsp), %rsi
  movq %rsp, %rdx
  leaq 48(%rsp), %rcx
  callq sat_kernel
  leaq .L.str(%rip), %rdi
  xorl %ebx, %ebx
  movq %rax, %rsi
  xorl %eax, %eax
  callq printf@PLT
.LBB0_3:
  movl %ebx, %eax
  addq $64, %rsp
  popq %rbx
  retq

sat_kernel:
  movl %r8d, %eax
  andl $3, %eax
  cmpl $4, %r8d
  jb .LBB1_4
  andl $-4, %r8d
.LBB1_2:
  vmovdqu (%rdi), %xmm0
  vpaddusb (%rsi), %xmm0, %xmm0
  vpaddusb (%rdx), %xmm0, %xmm1
  vpavgb %xmm1, %xmm0, %xmm2
  vpsubq %xmm0, %xmm2, %xmm0
  vpxor %xmm1, %xmm0, %xmm0
  vmovdqu %xmm0, (%rcx)
  vmovdqu (%rdi), %xmm0
  vpaddusb (%rsi), %xmm0, %xmm0
  vpaddusb (%rdx), %xmm0, %xmm1
  vpavgb %xmm1, %xmm0, %xmm2
  vpsubq %xmm0, %xmm2, %xmm0
  vpxor %xmm1, %xmm0, %xmm0
  vmovdqu %xmm0, (%rcx)
  vmovdqu (%rdi), %xmm0
  vpaddusb (%rsi), %xmm0, %xmm0
  vpaddusb (%rdx), %xmm0, %xmm1
  vpavgb %xmm1, %xmm0, %xmm2
  vpsubq %xmm0, %xmm2, %xmm0
  vpxor %xmm1, %xmm0, %xmm0
  vmovdqu %xmm0, (%rcx)
  vmovdqu (%rdi), %xmm0
  vpaddusb (%rsi), %xmm0, %xmm0
  vpaddusb (%rdx), %xmm0, %xmm1
  vpavgb %xmm1, %xmm0, %xmm2
  vpsubq %xmm0, %xmm2, %xmm0
  vpxor %xmm1, %xmm0, %xmm0
  vmovdqu %xmm0, (%rcx)
  addl $-4, %r8d
  jne .LBB1_2
  testl %eax, %eax
  je .LBB1_5
.LBB1_4:
  vmovdqu (%rdi), %xmm0
  vpaddusb (%rsi), %xmm0, %xmm0
  vpaddusb (%rdx), %xmm0, %xmm1
  vpavgb %xmm1, %xmm0, %xmm2
  vpsubq %xmm0, %xmm2, %xmm0
  vpxor %xmm1, %xmm0, %xmm0
  vmovdqu %xmm0, (%rcx)
  decl %eax
  jne .LBB1_4
.LBB1_5:
  vpshufd $238, %xmm0, %xmm1
  vpxor %xmm0, %xmm1, %xmm0
  vmovq %xmm0, %rax
  retq

.L.str:

