.LCPI0_0:
.LCPI0_1:
.LCPI0_2:
main:
  pushq %rbx
  subq $16, %rsp
  cmpl $2, %edi
  jl .LBB0_2
  movq 8(%rsi), %rdi
  xorl %esi, %esi
  xorl %edx, %edx
  callq strtoul@PLT
  decq %rax
  movl $2, %ebx
  movl $4294967294, %ecx
  cmpq %rcx, %rax
  ja .LBB0_3
.LBB0_2:
  vmovaps .LCPI0_0(%rip), %xmm0
  vmovaps .LCPI0_1(%rip), %xmm1
  vmovaps .LCPI0_2(%rip), %xmm2
  movq %rsp, %rdi
  callq sat_kernel
  leaq .L.str(%rip), %rdi
  xorl %ebx, %ebx
  movq %rax, %rsi
  xorl %eax, %eax
  callq printf@PLT
.LBB0_3:
  movl %ebx, %eax
  addq $16, %rsp
  popq %rbx
  retq

sat_kernel:
  vpaddusb %xmm1, %xmm0, %xmm0
  vpaddusb %xmm2, %xmm0, %xmm1
  vpavgb %xmm1, %xmm0, %xmm2
  vpsubq %xmm0, %xmm2, %xmm0
  vpxor %xmm1, %xmm0, %xmm0
  vmovdqu %xmm0, (%rdi)
  vpshufd $238, %xmm0, %xmm1
  vpxor %xmm0, %xmm1, %xmm0
  vmovq %xmm0, %rax
  retq

.L.str:

