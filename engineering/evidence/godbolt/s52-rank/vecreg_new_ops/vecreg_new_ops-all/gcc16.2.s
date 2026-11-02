sat_kernel:
  xorl %eax, %eax
.L2:
  vmovdqu (%rsi), %xmm1
  vpaddusb (%rdi), %xmm1, %xmm1
  addl $1, %eax
  vpaddusb (%rdx), %xmm1, %xmm2
  vpavgb %xmm2, %xmm1, %xmm0
  vpsubq %xmm1, %xmm0, %xmm0
  vpxor %xmm2, %xmm0, %xmm0
  vmovdqu %xmm0, (%rcx)
  cmpl %eax, %r8d
  jne .L2
  vpsrldq $8, %xmm0, %xmm1
  vpxor %xmm1, %xmm0, %xmm0
  vmovq %xmm0, %rax
  ret
.LC3:
main:
  subq $72, %rsp
  movl $1000, %r8d
  cmpl $1, %edi
  jg .L12
.L6:
  vmovdqa .LC0(%rip), %xmm0
  leaq 48(%rsp), %rcx
  leaq 32(%rsp), %rdx
  movq %rsp, %rdi
  leaq 16(%rsp), %rsi
  vmovdqa %xmm0, (%rsp)
  vmovdqa .LC1(%rip), %xmm0
  vmovdqa %xmm0, 16(%rsp)
  vmovdqa .LC2(%rip), %xmm0
  vmovdqa %xmm0, 32(%rsp)
  call sat_kernel
  movl $.LC3, %edi
  movq %rax, %rsi
  xorl %eax, %eax
  call printf
  xorl %eax, %eax
.L5:
  addq $72, %rsp
  ret
.L12:
  movq 8(%rsi), %rdi
  xorl %edx, %edx
  xorl %esi, %esi
  call __isoc23_strtoul
  movl $4294967294, %ecx
  leaq -1(%rax), %rdx
  cmpq %rdx, %rcx
  jb .L9
  movl %eax, %r8d
  jmp .L6
.L9:
  movl $2, %eax
  jmp .L5
.LC0:
.LC1:
.LC2:
