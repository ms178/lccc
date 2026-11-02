kernel:
  pushq %rbp
  pushq %r15
  pushq %r14
  pushq %rbx
  pushq %rax
  testl %edi, %edi
  jle .LBB0_1
  movl %edi, %ebx
  xorl %ebp, %ebp
.LBB0_4:
  movl $perm, %edi
  movl $perm1, %esi
  movl $count, %edx
  callq mix
  movl %eax, %r14d
  addl %ebp, %r14d
  movl $perm1, %edi
  movl $count, %esi
  movl $perm, %edx
  callq mix
  movl %eax, %r15d
  movl $count, %edi
  movl $perm, %esi
  movl $perm1, %edx
  callq mix
  movl %eax, %ebp
  addl %r15d, %ebp
  addl %r14d, %ebp
  decl %ebx
  jne .LBB0_4
  jmp .LBB0_2
.LBB0_1:
  xorl %ebp, %ebp
.LBB0_2:
  movl %ebp, %eax
  addq $8, %rsp
  popq %rbx
  popq %r14
  popq %r15
  popq %rbp
  retq

mix:
  leaq 508(%rdi), %rax
  cmpq %rsi, %rax
  setae %r8b
  leaq 508(%rsi), %rcx
  cmpq %rdi, %rcx
  setae %r9b
  cmpq %rdx, %rax
  setb %al
  leaq 508(%rdx), %rcx
  cmpq %rdi, %rcx
  setb %cl
  testb %r9b, %r8b
  jne .LBB1_2
  orb %cl, %al
  je .LBB1_2
  xorl %ecx, %ecx
  xorl %eax, %eax
.LBB1_5:
  movl (%rdi,%rcx), %r8d
  movl 4(%rdi,%rcx), %r9d
  addl (%rsi,%rcx), %r8d
  addl (%rdx,%rcx), %r8d
  addl %eax, %r8d
  movl %r8d, (%rdi,%rcx)
  addl 4(%rsi,%rcx), %r9d
  addl 4(%rdx,%rcx), %r9d
  addl %r8d, %r9d
  movl %r9d, 4(%rdi,%rcx)
  movl 8(%rdi,%rcx), %r8d
  addl 8(%rsi,%rcx), %r8d
  addl 8(%rdx,%rcx), %r8d
  addl %r9d, %r8d
  movl %r8d, 8(%rdi,%rcx)
  movl 12(%rdi,%rcx), %eax
  addl 12(%rsi,%rcx), %eax
  addl 12(%rdx,%rcx), %eax
  addl %r8d, %eax
  movl %eax, 12(%rdi,%rcx)
  addq $16, %rcx
  cmpq $512, %rcx
  jne .LBB1_5
  jmp .LBB1_6
.LBB1_2:
  xorl %ecx, %ecx
  xorl %eax, %eax
.LBB1_3:
  movl (%rdi,%rcx,4), %r8d
  addl (%rsi,%rcx,4), %r8d
  addl (%rdx,%rcx,4), %r8d
  addl %r8d, %eax
  movl %eax, (%rdi,%rcx,4)
  incq %rcx
  cmpq $128, %rcx
  jne .LBB1_3
.LBB1_6:
  retq

.LCPI2_0:
.LCPI2_1:
main:
  pushq %rax
  vstmxcsr 4(%rsp)
  orl $32832, 4(%rsp)
  vldmxcsr 4(%rsp)
  movl %edi, %eax
  movl $2000, %edi
  cmpl $2, %eax
  jl .LBB2_2
  movq 8(%rsi), %rdi
  xorl %esi, %esi
  movl $10, %edx
  callq strtol
  movq %rax, %rdi
.LBB2_2:
  xorl %eax, %eax
  vmovdqu .LCPI2_0(%rip), %ymm0
  vmovdqu .LCPI2_1(%rip), %ymm1
  xorl %ecx, %ecx
.LBB2_3:
  vmovd %ecx, %xmm2
  vpbroadcastd %xmm2, %ymm2
  vpor %ymm0, %ymm2, %ymm2
  vmovdqu %ymm2, perm(,%rcx,4)
  vmovd %eax, %xmm3
  vpbroadcastd %xmm3, %ymm3
  vpaddd %ymm1, %ymm3, %ymm3
  vmovdqu %ymm3, perm1(,%rcx,4)
  vpmulld %ymm2, %ymm2, %ymm2
  vmovdqu %ymm2, count(,%rcx,4)
  leaq 8(%rcx), %rdx
  addl $-8, %eax
  cmpq $120, %rcx
  movq %rdx, %rcx
  jb .LBB2_3
  vzeroupper
  callq kernel
  movl $.L.str, %edi
  movl %eax, %esi
  xorl %eax, %eax
  callq printf
  xorl %eax, %eax
  popq %rcx
  retq

.L.str:

