ackermann:
  pushq %r15
  movl %esi, %eax
  pushq %r14
  pushq %r13
  pushq %r12
  pushq %rbp
  pushq %rbx
  movl %edi, %ebx
  subq $24, %rsp
.L2:
  movl %ebx, %ebp
  subl $1, %ebx
  testl %eax, %eax
  jne .L42
  movl $1, %eax
.L3:
  testl %ebx, %ebx
  jne .L2
  addq $24, %rsp
  addl $1, %eax
  popq %rbx
  popq %rbp
  popq %r12
  popq %r13
  popq %r14
  popq %r15
  ret
.L42:
  subl $1, %eax
.L4:
  movl %ebp, %r12d
  subl $1, %ebp
  testl %eax, %eax
  jne .L43
  movl $1, %eax
.L5:
  testl %ebp, %ebp
  jne .L4
  addl $1, %eax
  jmp .L3
.L43:
  subl $1, %eax
.L6:
  movl %r12d, %r13d
  subl $1, %r12d
  testl %eax, %eax
  jne .L44
  movl $1, %eax
.L7:
  testl %r12d, %r12d
  jne .L6
  addl $1, %eax
  jmp .L5
.L44:
  subl $1, %eax
.L8:
  movl %r13d, %r14d
  subl $1, %r13d
  testl %eax, %eax
  jne .L45
  movl $1, %eax
.L9:
  testl %r13d, %r13d
  jne .L8
  addl $1, %eax
  jmp .L7
.L45:
  subl $1, %eax
.L10:
  movl %r14d, %r15d
  subl $1, %r14d
  testl %eax, %eax
  jne .L46
  movl $1, %eax
.L11:
  testl %r14d, %r14d
  jne .L10
  addl $1, %eax
  jmp .L9
.L46:
  subl $1, %eax
.L12:
  movl %r15d, %edx
  subl $1, %r15d
  testl %eax, %eax
  jne .L47
  movl $1, %eax
.L13:
  testl %r15d, %r15d
  jne .L12
  addl $1, %eax
  jmp .L11
.L47:
  subl $1, %eax
.L14:
  movl %edx, %ecx
  subl $1, %edx
  testl %eax, %eax
  jne .L48
  movl $1, %eax
.L15:
  testl %edx, %edx
  jne .L14
  addl $1, %eax
  jmp .L13
.L48:
  subl $1, %eax
.L16:
  movl %ecx, %r8d
  subl $1, %ecx
  testl %eax, %eax
  jne .L49
  movl $1, %eax
.L17:
  testl %ecx, %ecx
  jne .L16
  addl $1, %eax
  jmp .L15
.L49:
  subl $1, %eax
.L18:
  movl %r8d, %edi
  subl $1, %r8d
  testl %eax, %eax
  jne .L50
  movl $1, %eax
.L19:
  testl %r8d, %r8d
  jne .L18
  addl $1, %eax
  jmp .L17
.L50:
  leal -1(%rax), %esi
  movl %r8d, 12(%rsp)
  movl %ecx, 8(%rsp)
  movl %edx, 4(%rsp)
  call ackermann
  movl 4(%rsp), %edx
  movl 8(%rsp), %ecx
  movl 12(%rsp), %r8d
  jmp .L19
.LC0:
  .string "ackermann(3,11) = %d\n"
main:
  subq $8, %rsp
  movl $3, %r9d
  movl $11, %eax
.L52:
  testl %eax, %eax
  jne .L59
  movl $1, %eax
.L53:
  subl $1, %r9d
  jne .L52
  leal 1(%rax), %esi
  movl $.LC0, %edi
  xorl %eax, %eax
  call printf
  xorl %eax, %eax
  addq $8, %rsp
  ret
.L59:
  leal -1(%rax), %esi
  movl %r9d, %edi
  call ackermann
  jmp .L53
