.LCPI0_0:
  .long 0
  .long 1
  .long 2
  .long 3
  .long 4
  .long 5
  .long 6
  .long 7
main:
  pushq %rbp
  pushq %r15
  pushq %r14
  pushq %r13
  pushq %r12
  pushq %rbx
  pushq %rax
  vstmxcsr 4(%rsp)
  orl $32832, 4(%rsp)
  vldmxcsr 4(%rsp)
  vmovups .LCPI0_0(%rip), %ymm0
  vmovups %ymm0, perm1(%rip)
  movabsq $38654705672, %rax
  movq %rax, perm1+32(%rip)
  movl $10, perm1+40(%rip)
  movl $11, %r14d
  xorl %ebx, %ebx
  movabsq $17179869180, %r12
  xorl %ebp, %ebp
  movl $0, (%rsp)
.LBB0_1:
  movl %r14d, %eax
  cmpl $2, %r14d
  jl .LBB0_5
  movl %r14d, %eax
  movl $1, %ecx
.LBB0_3:
  leaq 1(%rcx), %rdx
  movl %edx, count(,%rcx,4)
  movq %rdx, %rcx
  cmpq %rdx, %rax
  jne .LBB0_3
  movl $1, %eax
.LBB0_5:
  vmovups perm1(%rip), %ymm0
  vmovups %ymm0, perm(%rip)
  vmovups perm1+12(%rip), %ymm0
  vmovups %ymm0, perm+12(%rip)
  movl perm(%rip), %edx
  xorl %ecx, %ecx
  jmp .LBB0_6
.LBB0_11:
  incl %ecx
.LBB0_6:
  testl %edx, %edx
  je .LBB0_12
  testl %edx, %edx
  jle .LBB0_11
  leal 1(%rdx), %esi
  sarl %esi
  movslq %edx, %rdx
  shlq $2, %rsi
  leaq perm(,%rdx,4), %rdx
  xorl %edi, %edi
.LBB0_9:
  movl perm(%rdi), %r8d
  movl (%rdx), %r9d
  movl %r9d, perm(%rdi)
  movl %r8d, (%rdx)
  addq $4, %rdi
  addq $-4, %rdx
  cmpq %rdi, %rsi
  jne .LBB0_9
  movl perm(%rip), %edx
  jmp .LBB0_11
.LBB0_12:
  cmpl %ebp, %ecx
  cmovgl %ecx, %ebp
  movl %ecx, %edx
  negl %edx
  testb $1, (%rsp)
  cmovel %ecx, %edx
  addl %edx, %ebx
  cmpl $11, %eax
  je .LBB0_18
  incl (%rsp)
  testl %r14d, %r14d
  movl $1, %eax
  cmovgl %eax, %r14d
  movslq %r14d, %r14
  leaq (,%r14,4), %r15
.LBB0_14:
  movl perm1(%rip), %r13d
  testq %r14, %r14
  jle .LBB0_16
  movq %r15, %rdx
  andq %r12, %rdx
  movl $perm1, %edi
  movl $perm1+4, %esi
  vzeroupper
  callq memmove@PLT
.LBB0_16:
  movl %r13d, perm1(%r15)
  movl count(%r15), %eax
  leal -1(%rax), %ecx
  movl %ecx, count(%r15)
  cmpl $1, %eax
  jg .LBB0_1
  incq %r14
  addq $4, %r15
  cmpl $11, %r14d
  jne .LBB0_14
.LBB0_18:
  movl $.L.str, %edi
  movl %ebx, %esi
  movl $11, %edx
  movl %ebp, %ecx
  xorl %eax, %eax
  vzeroupper
  callq printf
  xorl %eax, %eax
  addq $8, %rsp
  popq %rbx
  popq %r12
  popq %r13
  popq %r14
  popq %r15
  popq %rbp
  retq

.L.str:
  .asciz "%d\nPfannkuchen(%d) = %d\n"

