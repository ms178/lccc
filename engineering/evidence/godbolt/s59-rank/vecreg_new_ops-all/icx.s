.LCPI0_0:
.LCPI0_1:
.LCPI0_2:
main:
  pushq %rbx
  subq $64, %rsp
  vstmxcsr (%rsp)
  orl $32832, (%rsp)
  vldmxcsr (%rsp)
  movl $1000, %r8d
  cmpl $2, %edi
  jl .LBB0_2
  movq 8(%rsi), %rdi
  xorl %esi, %esi
  xorl %edx, %edx
  callq strtoul
  movq %rax, %r8
  decq %rax
  movl $2, %ebx
  movl $4294967294, %ecx
  cmpq %rcx, %rax
  ja .LBB0_3
.LBB0_2:
  vmovups .LCPI0_0(%rip), %xmm0
  vmovups %xmm0, (%rsp)
  vmovups .LCPI0_1(%rip), %xmm0
  vmovups %xmm0, 32(%rsp)
  vmovups .LCPI0_2(%rip), %xmm0
  vmovups %xmm0, 16(%rsp)
  movq %rsp, %rdi
  leaq 32(%rsp), %rsi
  leaq 16(%rsp), %rdx
  leaq 48(%rsp), %rcx
  callq sat_kernel
  xorl %ebx, %ebx
  movl $.L.str, %edi
  movq %rax, %rsi
  xorl %eax, %eax
  callq printf
.LBB0_3:
  movl %ebx, %eax
  addq $64, %rsp
  popq %rbx
  retq

sat_kernel:
  testl %r8d, %r8d
  je .LBB1_1
  vmovdqu (%rsi), %xmm1
  vmovdqu (%rdx), %xmm0
  vpaddusb (%rdi), %xmm1, %xmm1
  cmpl $1, %r8d
  je .LBB1_6
  movl %r8d, %eax
  andl $-2, %eax
  vpaddusb %xmm0, %xmm1, %xmm2
  vpavgb %xmm2, %xmm1, %xmm3
  vpsubq %xmm1, %xmm3, %xmm3
  vpxor %xmm2, %xmm3, %xmm2
.LBB1_4:
  addl $-2, %eax
  jne .LBB1_4
  vmovdqu %xmm2, (%rcx)
.LBB1_6:
  testb $1, %r8b
  je .LBB1_8
  vpaddusb %xmm0, %xmm1, %xmm0
  vpavgb %xmm0, %xmm1, %xmm2
  vpsubq %xmm1, %xmm2, %xmm1
  vpxor %xmm0, %xmm1, %xmm2
  vmovdqu %xmm2, (%rcx)
  jmp .LBB1_8
.LBB1_1:
  vmovdqu (%rcx), %xmm2
.LBB1_8:
  vpshufd $238, %xmm2, %xmm0
  vpxor %xmm2, %xmm0, %xmm0
  vmovq %xmm0, %rax
  retq

.L.str:

