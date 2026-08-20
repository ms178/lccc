fib:
  movslq %edi, %rax
  cmpl $1, %eax
  jle .L44
  subq $168, %rsp
  leal -1(%rax), %edx
  movq %r14, 152(%rsp)
  andl $-2, %edx
  movl %eax, %r14d
  subl %edx, %r14d
  movq %rbx, 120(%rsp)
  xorl %ebx, %ebx
  cmpl %eax, %r14d
  je .L48
.L3:
  movq %r12, 136(%rsp)
  leal -2(%rax), %r12d
  movq %r13, 144(%rsp)
  leal -1(%rax), %r13d
  movl %r12d, %eax
  andl $-2, %eax
  movl %r13d, %ecx
  movq %rbx, 48(%rsp)
  subl %eax, %ecx
  movl %r12d, 64(%rsp)
  movl %r13d, %eax
  movq %r15, 160(%rsp)
  movl %ecx, 100(%rsp)
  movq %rbp, 128(%rsp)
  xorl %ebp, %ebp
.L9:
  cmpl 100(%rsp), %eax
  je .L37
  leal -1(%rax), %ecx
  subl $2, %eax
  movq %rbp, 56(%rsp)
  xorl %r15d, %r15d
  movl %eax, %edx
  movl %ecx, %esi
  movl %r14d, 68(%rsp)
  andl $-2, %edx
  subl %edx, %esi
  movl %esi, 104(%rsp)
.L13:
  cmpl 104(%rsp), %ecx
  je .L38
  leal -1(%rcx), %r12d
  subl $2, %ecx
  movq %r15, 72(%rsp)
  movl %ecx, %edx
  movl %r12d, %esi
  movl %ecx, 88(%rsp)
  andl $-2, %edx
  subl %edx, %esi
  xorl %edx, %edx
  movl %esi, 108(%rsp)
.L17:
  cmpl 108(%rsp), %r12d
  je .L39
  leal -1(%r12), %r9d
  subl $2, %r12d
  movq %rdx, 80(%rsp)
  xorl %r14d, %r14d
  movl %r12d, %r10d
  movl %eax, 92(%rsp)
  movl %r9d, %r15d
  andl $-2, %r10d
  movl %r12d, 96(%rsp)
  movl %r9d, %r12d
  subl %r10d, %r15d
.L21:
  cmpl %r15d, %r12d
  je .L40
  leal -2(%r12), %edx
  leal -1(%r12), %r13d
  xorl %ebp, %ebp
  subl $1, %r12d
  movl %edx, %eax
  movq %r14, 32(%rsp)
  andl $-2, %eax
  movq %rbp, 16(%rsp)
  subl %eax, %r13d
  movl %r15d, 40(%rsp)
  movl %r13d, 28(%rsp)
  movl %edx, 44(%rsp)
.L25:
  cmpl 28(%rsp), %r12d
  je .L41
  leal -2(%r12), %ecx
  leal -3(%r12), %ebx
  subl $5, %r12d
  movl %ecx, %eax
  movl %ebx, %edx
  movl %ecx, 24(%rsp)
  andl $-2, %eax
  subl %eax, %edx
  movl %ebx, %eax
  andl $-2, %eax
  movl %edx, 8(%rsp)
  subl %eax, %r12d
  xorl %eax, %eax
  movl %r12d, 12(%rsp)
  movl %ebx, %r12d
  cmpl %r12d, 8(%rsp)
  je .L49
.L26:
  movq %rax, (%rsp)
  leal 1(%r12), %ebp
  movl %r12d, %r13d
  movl %r12d, %r14d
  xorl %r12d, %r12d
.L33:
  movl %r13d, %ebx
  cmpl $1, %r13d
  je .L50
  xorl %r15d, %r15d
.L30:
  leal -1(%rbx), %edi
  subl $2, %ebx
  call fib
  addq %rax, %r15
  cmpl $1, %ebx
  jg .L30
  leal -3(%rbp), %esi
  subl $2, %r13d
  subl $2, %ebp
  andl $-2, %esi
  movl %r13d, %eax
  subl %esi, %eax
  addq %r15, %rax
  addq %rax, %r12
  cmpl $1, %ebp
  jne .L33
  leaq 1(%r12), %rsi
  movq (%rsp), %rax
  movl %r14d, %r12d
  jmp .L31
.L50:
  movq (%rsp), %rax
  leaq 1(%r12), %rsi
  movl %r14d, %r12d
.L31:
  addq %rsi, %rax
  leal -2(%r12), %edx
  cmpl %edx, 12(%rsp)
  je .L28
  movl %edx, %r12d
  cmpl %r12d, 8(%rsp)
  jne .L26
.L49:
  movl 24(%rsp), %ecx
  addq $1, %rax
.L27:
  addq %rax, 16(%rsp)
  movl %ecx, %r12d
  cmpl $1, %ecx
  jne .L25
.L41:
  movq 16(%rsp), %rbp
  movq 32(%rsp), %r14
  movl 44(%rsp), %edx
  movl 40(%rsp), %r15d
  addq $1, %rbp
  movl %edx, %r12d
  addq %rbp, %r14
  cmpl $1, %edx
  jne .L21
.L40:
  movq 80(%rsp), %rdx
  movl 96(%rsp), %r12d
  addq $1, %r14
  movl 92(%rsp), %eax
  addq %r14, %rdx
  cmpl $1, %r12d
  jne .L17
.L39:
  movq 72(%rsp), %r15
  movl 88(%rsp), %ecx
  addq $1, %rdx
  addq %rdx, %r15
  cmpl $1, %ecx
  jne .L13
.L38:
  movq 56(%rsp), %rbp
  leaq 1(%r15), %rdx
  movl 68(%rsp), %r14d
  addq %rdx, %rbp
  cmpl $1, %eax
  jne .L9
.L37:
  movq 48(%rsp), %rbx
  movl 64(%rsp), %r12d
  leaq 1(%rbp), %rdx
  movl %r12d, %eax
  addq %rdx, %rbx
  cmpl $1, %r12d
  je .L51
  movq 128(%rsp), %rbp
  movq 136(%rsp), %r12
  movq 144(%rsp), %r13
  movq 160(%rsp), %r15
  cmpl %eax, %r14d
  jne .L3
.L48:
  leaq 1(%rbx), %rax
.L1:
  movq 120(%rsp), %rbx
  movq 152(%rsp), %r14
  addq $168, %rsp
  ret
.L44:
  ret
.L51:
  movq 128(%rsp), %rbp
  movq 136(%rsp), %r12
  leaq 1(%rbx), %rax
  movq 144(%rsp), %r13
  movq 160(%rsp), %r15
  jmp .L1
.L28:
  movl %r12d, %ebx
  movl 24(%rsp), %ecx
  addq %rbx, %rax
  jmp .L27
.LC0:
  .string "fib(40) = %ld\n"
main:
  subq $24, %rsp
  movl $39, %r8d
  xorl %r11d, %r11d
.L53:
  movl %r8d, %edi
  subl $2, %r8d
  call fib
  addq %rax, %r11
  cmpl $-1, %r8d
  jne .L53
  movq %r11, 8(%rsp)
  movq 8(%rsp), %rsi
  movl $.LC0, %edi
  xorl %eax, %eax
  call printf
  xorl %eax, %eax
  addq $24, %rsp
  ret
