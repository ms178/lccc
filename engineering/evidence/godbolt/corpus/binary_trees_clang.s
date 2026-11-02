main:
  pushq %rbp
  pushq %r15
  pushq %r14
  pushq %r13
  pushq %r12
  pushq %rbx
  subq $24, %rsp
  movl $19, %edi
  callq make
  movq %rax, %rbx
  movq %rax, %rdi
  callq check
  leaq .L.str(%rip), %rdi
  movl $19, %esi
  movl %eax, %edx
  xorl %eax, %eax
  callq printf@PLT
  movq %rbx, %rdi
  callq destroy
  movl $18, %ebx
  movl $18, %edi
  callq make
  movq %rax, 8(%rsp)
  movl $4, %r15d
  xorl %ecx, %ecx
.LBB0_1:
  leal -11(,%rcx,2), %eax
  movl $1, %r14d
  cmpl $7, %eax
  jb .LBB0_5
  movl %ebx, %eax
  andl $-8, %eax
.LBB0_3:
  shll $8, %r14d
  addl $-8, %eax
  jne .LBB0_3
  testb $6, %bl
  je .LBB0_7
.LBB0_5:
  movl %ebx, %eax
  andl $6, %eax
.LBB0_6:
  addl %r14d, %r14d
  decl %eax
  jne .LBB0_6
.LBB0_7:
  movq %rcx, 16(%rsp)
  xorl %r12d, %r12d
  movl %r14d, %ebp
.LBB0_8:
  movl %r15d, %edi
  callq make
  movq %rax, %r13
  movq %rax, %rdi
  callq check
  addl %eax, %r12d
  movq %r13, %rdi
  callq destroy
  decl %ebp
  jne .LBB0_8
  leaq .L.str.1(%rip), %rdi
  movl %r14d, %esi
  movl %r15d, %edx
  movl %r12d, %ecx
  xorl %eax, %eax
  callq printf@PLT
  addl $-2, %ebx
  movq 16(%rsp), %rcx
  incl %ecx
  cmpl $17, %r15d
  leal 2(%r15), %eax
  movl %eax, %r15d
  jb .LBB0_1
  movq 8(%rsp), %rbx
  movq %rbx, %rdi
  callq check
  leaq .L.str.2(%rip), %rdi
  movl $18, %esi
  movl %eax, %edx
  xorl %eax, %eax
  callq printf@PLT
  movq %rbx, %rdi
  callq destroy
  xorl %eax, %eax
  addq $24, %rsp
  popq %rbx
  popq %r12
  popq %r13
  popq %r14
  popq %r15
  popq %rbp
  retq

make:
  pushq %r15
  pushq %r14
  pushq %rbx
  movl %edi, %ebx
  movl $16, %edi
  callq malloc@PLT
  testl %ebx, %ebx
  jle .LBB1_1
  decl %ebx
  movl %ebx, %edi
  movq %rax, %r15
  callq make
  movq %rax, %r14
  movl %ebx, %edi
  callq make
  movq %rax, %rcx
  movq %r15, %rax
  jmp .LBB1_2
.LBB1_1:
  xorl %r14d, %r14d
  xorl %ecx, %ecx
.LBB1_2:
  movq %r14, (%rax)
  movq %rcx, 8(%rax)
  popq %rbx
  popq %r14
  popq %r15
  retq

check:
  pushq %r14
  pushq %rbx
  pushq %rax
  movq %rdi, %r14
  movq (%rdi), %rdi
  testq %rdi, %rdi
  je .LBB2_1
  xorl %ebx, %ebx
.LBB2_3:
  callq check
  movq 8(%r14), %r14
  addl %eax, %ebx
  incl %ebx
  movq (%r14), %rdi
  testq %rdi, %rdi
  jne .LBB2_3
  incl %ebx
  jmp .LBB2_5
.LBB2_1:
  movl $1, %ebx
.LBB2_5:
  movl %ebx, %eax
  addq $8, %rsp
  popq %rbx
  popq %r14
  retq

destroy:
  pushq %rbx
  movq %rdi, %rbx
  movq (%rdi), %rdi
  testq %rdi, %rdi
  je .LBB3_2
  callq destroy
  movq 8(%rbx), %rdi
  callq destroy
.LBB3_2:
  movq %rbx, %rdi
  popq %rbx
  jmp free@PLT

.L.str:
  .asciz "stretch tree of depth %d\t check: %d\n"

.L.str.1:
  .asciz "%d\t trees of depth %d\t check: %d\n"

.L.str.2:
  .asciz "long lived tree of depth %d\t check: %d\n"

