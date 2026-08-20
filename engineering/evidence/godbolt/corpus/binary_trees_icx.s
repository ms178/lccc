main:
  pushq %rbp
  pushq %r15
  pushq %r14
  pushq %r13
  pushq %r12
  pushq %rbx
  subq $24, %rsp
  vstmxcsr 12(%rsp)
  orl $32832, 12(%rsp)
  vldmxcsr 12(%rsp)
  movl $19, %edi
  callq make
  movq %rax, %rbx
  movq %rax, %rdi
  callq check
  movl $.L.str, %edi
  movl $19, %esi
  movl %eax, %edx
  xorl %eax, %eax
  callq printf
  movq %rbx, %rdi
  callq destroy
  movl $18, %r13d
  movl $18, %edi
  callq make
  movq %rax, 16(%rsp)
  movl $4, %r14d
.LBB0_1:
  movl $1, %ebp
  movl %r13d, %eax
.LBB0_2:
  movl %ebp, %ebx
  leal (%rbx,%rbx), %ebp
  decl %eax
  jne .LBB0_2
  addl %ebx, %ebx
  xorl %r15d, %r15d
.LBB0_4:
  movl %r14d, %edi
  callq make
  movq %rax, %r12
  movq %rax, %rdi
  callq check
  addl %eax, %r15d
  movq %r12, %rdi
  callq destroy
  decl %ebx
  jne .LBB0_4
  movl $.L.str.1, %edi
  movl %ebp, %esi
  movl %r14d, %edx
  movl %r15d, %ecx
  xorl %eax, %eax
  callq printf
  leal 2(%r14), %eax
  addl $-2, %r13d
  cmpl $17, %r14d
  movl %eax, %r14d
  jb .LBB0_1
  movq 16(%rsp), %rbx
  movq %rbx, %rdi
  callq check
  movl $.L.str.2, %edi
  movl $18, %esi
  movl %eax, %edx
  xorl %eax, %eax
  callq printf
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
  pushq %rbp
  pushq %r14
  pushq %rbx
  movl %edi, %ebp
  movl $16, %edi
  callq malloc
  movq %rax, %rbx
  testl %ebp, %ebp
  jle .LBB1_1
  decl %ebp
  movl %ebp, %edi
  callq make
  movq %rax, %r14
  movl %ebp, %edi
  callq make
  jmp .LBB1_2
.LBB1_1:
  xorl %r14d, %r14d
  xorl %eax, %eax
.LBB1_2:
  movq %r14, (%rbx)
  movq %rax, 8(%rbx)
  movq %rbx, %rax
  popq %rbx
  popq %r14
  popq %rbp
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
  jmp free

.L.str:
  .asciz "stretch tree of depth %d\t check: %d\n"

.L.str.1:
  .asciz "%d\t trees of depth %d\t check: %d\n"

.L.str.2:
  .asciz "long lived tree of depth %d\t check: %d\n"

