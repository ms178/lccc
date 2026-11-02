kernel:
  testl %edi, %edi
  jle .LBB0_1
  pushq %rbp
  pushq %r15
  pushq %r14
  pushq %r13
  pushq %r12
  pushq %rbx
  pushq %rax
  movl %edi, %ebx
  xorl %r13d, %r13d
  leaq perm(%rip), %r14
  leaq perm1(%rip), %r15
  leaq count(%rip), %r12
.LBB0_4:
  movq %r14, %rdi
  movq %r15, %rsi
  movq %r12, %rdx
  callq mix
  addl %eax, %r13d
  movq %r15, %rdi
  movq %r12, %rsi
  movq %r14, %rdx
  callq mix
  movl %eax, %ebp
  movq %r12, %rdi
  movq %r14, %rsi
  movq %r15, %rdx
  callq mix
  addl %ebp, %eax
  addl %r13d, %eax
  movl %eax, %r13d
  decl %ebx
  jne .LBB0_4
  addq $8, %rsp
  popq %rbx
  popq %r12
  popq %r13
  popq %r14
  popq %r15
  popq %rbp
  retq
.LBB0_1:
  xorl %eax, %eax
  retq

mix:
  xorl %eax, %eax
  xorl %ecx, %ecx
.LBB1_1:
  addl (%rdi,%rcx,4), %eax
  addl (%rsi,%rcx,4), %eax
  addl (%rdx,%rcx,4), %eax
  movl %eax, (%rdi,%rcx,4)
  addl 4(%rdi,%rcx,4), %eax
  addl 4(%rsi,%rcx,4), %eax
  addl 4(%rdx,%rcx,4), %eax
  movl %eax, 4(%rdi,%rcx,4)
  addl 8(%rdi,%rcx,4), %eax
  addl 8(%rsi,%rcx,4), %eax
  addl 8(%rdx,%rcx,4), %eax
  movl %eax, 8(%rdi,%rcx,4)
  addl 12(%rdi,%rcx,4), %eax
  addl 12(%rsi,%rcx,4), %eax
  addl 12(%rdx,%rcx,4), %eax
  movl %eax, 12(%rdi,%rcx,4)
  addq $4, %rcx
  cmpq $128, %rcx
  jne .LBB1_1
  retq

.LCPI2_0:
.LCPI2_1:
.LCPI2_2:
.LCPI2_3:
.LCPI2_5:
.LCPI2_4:
main:
  pushq %rax
  movl %edi, %eax
  movl $2000, %edi
  cmpl $2, %eax
  jl .LBB2_2
  movq 8(%rsi), %rdi
  xorl %esi, %esi
  movl $10, %edx
  callq strtol@PLT
  movq %rax, %rdi
.LBB2_2:
  vmovaps .LCPI2_0(%rip), %ymm0
  vmovaps .LCPI2_1(%rip), %ymm1
  vmovdqa .LCPI2_2(%rip), %ymm2
  xorl %eax, %eax
  leaq perm(%rip), %rcx
  leaq perm1(%rip), %rdx
  vpbroadcastd .LCPI2_3(%rip), %ymm3
  leaq count(%rip), %rsi
  vpbroadcastq .LCPI2_4(%rip), %ymm4
  vpbroadcastd .LCPI2_5(%rip), %ymm5
  vmovdqa %ymm2, %ymm6
.LBB2_3:
  vmovdqu %ymm2, (%rax,%rcx)
  vpsubd %ymm6, %ymm3, %ymm7
  vmovdqu %ymm7, (%rax,%rdx)
  vshufps $136, %ymm0, %ymm1, %ymm7
  vpermpd $216, %ymm7, %ymm7
  vpmulld %ymm7, %ymm7, %ymm7
  vmovdqu %ymm7, (%rax,%rsi)
  vpaddq %ymm4, %ymm1, %ymm1
  vpaddq %ymm4, %ymm0, %ymm0
  vpaddd %ymm5, %ymm2, %ymm2
  vpaddd %ymm5, %ymm6, %ymm6
  addq $32, %rax
  cmpq $512, %rax
  jne .LBB2_3
  vzeroupper
  callq kernel
  leaq .L.str(%rip), %rdi
  movl %eax, %esi
  xorl %eax, %eax
  callq printf@PLT
  xorl %eax, %eax
  popq %rcx
  retq

.L.str:

