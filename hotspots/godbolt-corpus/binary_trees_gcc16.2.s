check:
  subq $136, %rsp
  movl $1, %eax
  movq %rbp, 96(%rsp)
  movq (%rdi), %rbp
  testq %rbp, %rbp
  je .L1
  movq %r12, 104(%rsp)
  movq %r13, 112(%rsp)
  movq %rbp, %r13
  movq %r14, 120(%rsp)
  xorl %r14d, %r14d
  movq %r15, 128(%rsp)
  movq %rdi, %r15
.L7:
  movq 0(%r13), %r12
  movl $2, %eax
  testq %r12, %r12
  je .L3
  movq %rbx, 88(%rsp)
  movq %r15, %rax
  movq %r12, %rbx
  movl %r14d, %r15d
  xorl %ebp, %ebp
  movq %r13, %r14
.L4:
  movq (%rbx), %r12
  movl $2, %edx
  testq %r12, %r12
  je .L5
  movq %r14, 32(%rsp)
  movq %rbx, %r13
  movl %r15d, %r14d
  movq %rax, %rsi
  movl %ebp, 44(%rsp)
  xorl %ebp, %ebp
.L6:
  movq (%r12), %r15
  movl $2, %ecx
  testq %r15, %r15
  je .L8
  movq %r13, 48(%rsp)
  movq %r12, %r13
  movl %ebp, 64(%rsp)
  movq %r15, %rbp
  xorl %r15d, %r15d
.L9:
  movq 0(%rbp), %rbx
  movl $2, %r12d
  testq %rbx, %rbx
  je .L10
  movq %r13, 56(%rsp)
  movq %rbp, %rax
  xorl %ecx, %ecx
  movl %r15d, 68(%rsp)
  movq %rsi, 72(%rsp)
.L11:
  movq (%rbx), %rbp
  movl $2, %r12d
  testq %rbp, %rbp
  je .L12
  movl %ecx, 40(%rsp)
  movq %rax, 24(%rsp)
  xorl %eax, %eax
.L13:
  movq 0(%rbp), %r12
  movl $2, %ecx
  testq %r12, %r12
  je .L14
  movl %eax, 20(%rsp)
  xorl %r15d, %r15d
  movq %rbx, 8(%rsp)
  movq %rbp, %rbx
.L15:
  movq (%r12), %rbp
  movl $2, %ecx
  testq %rbp, %rbp
  je .L16
  movq %rbx, (%rsp)
  movq %r12, %r13
  xorl %ebx, %ebx
.L17:
  movq 0(%rbp), %rdi
  movl $2, %eax
  xorl %r12d, %r12d
  testq %rdi, %rdi
  je .L18
.L19:
  call check
  movq 8(%rbp), %rbp
  leal 1(%r12,%rax), %r12d
  movq 0(%rbp), %rdi
  testq %rdi, %rdi
  jne .L19
  leal 2(%r12), %eax
.L18:
  movq 8(%r13), %r13
  addl %eax, %ebx
  movq 0(%r13), %rbp
  testq %rbp, %rbp
  jne .L17
  leal 2(%rbx), %ecx
  movq (%rsp), %rbx
.L16:
  movq 8(%rbx), %rbx
  addl %ecx, %r15d
  movq (%rbx), %r12
  testq %r12, %r12
  jne .L15
  movq 8(%rsp), %rbx
  movl 20(%rsp), %eax
  leal 2(%r15), %ecx
.L14:
  movq 8(%rbx), %rbx
  addl %ecx, %eax
  movq (%rbx), %rbp
  testq %rbp, %rbp
  jne .L13
  leal 2(%rax), %r12d
  movl 40(%rsp), %ecx
  movq 24(%rsp), %rax
.L12:
  movq 8(%rax), %rax
  addl %r12d, %ecx
  movq (%rax), %rbx
  testq %rbx, %rbx
  jne .L11
  movq 56(%rsp), %r13
  movl 68(%rsp), %r15d
  leal 2(%rcx), %r12d
  movq 72(%rsp), %rsi
.L10:
  movq 8(%r13), %r13
  addl %r12d, %r15d
  movq 0(%r13), %rbp
  testq %rbp, %rbp
  jne .L9
  movq 48(%rsp), %r13
  movl 64(%rsp), %ebp
  leal 2(%r15), %ecx
.L8:
  movq 8(%r13), %r13
  addl %ecx, %ebp
  movq 0(%r13), %r12
  testq %r12, %r12
  jne .L6
  movl %r14d, %r15d
  leal 2(%rbp), %edx
  movq 32(%rsp), %r14
  movl 44(%rsp), %ebp
  movq %rsi, %rax
.L5:
  movq 8(%r14), %r14
  addl %edx, %ebp
  movq (%r14), %rbx
  testq %rbx, %rbx
  jne .L4
  movq 88(%rsp), %rbx
  movl %r15d, %r14d
  movq %rax, %r15
  leal 2(%rbp), %eax
.L3:
  movq 8(%r15), %r15
  addl %eax, %r14d
  movq (%r15), %r13
  testq %r13, %r13
  jne .L7
  leal 1(%r14), %eax
  movq 104(%rsp), %r12
  movq 112(%rsp), %r13
  movq 120(%rsp), %r14
  movq 128(%rsp), %r15
.L1:
  movq 96(%rsp), %rbp
  addq $136, %rsp
  ret
destroy:
  subq $40, %rsp
  movq %rbp, 24(%rsp)
  movq (%rdi), %rbp
  movq %rbx, 16(%rsp)
  movq %rdi, %rbx
  testq %rbp, %rbp
  je .L51
  movq 0(%rbp), %rax
  movq %r14, 32(%rsp)
  testq %rax, %rax
  je .L52
  movq (%rax), %rdx
  testq %rdx, %rdx
  je .L53
  movq (%rdx), %rdi
  testq %rdi, %rdi
  je .L54
  movq %rax, 8(%rsp)
  movq %rdx, (%rsp)
  call destroy
  movq (%rsp), %rdx
  movq 8(%rdx), %rdi
  call destroy
  movq 8(%rsp), %rax
  movq (%rsp), %rdx
.L54:
  movq %rdx, %rdi
  movq %rax, (%rsp)
  call free
  movq (%rsp), %rax
  movq 8(%rax), %rdx
  movq (%rdx), %rdi
  testq %rdi, %rdi
  je .L55
  movq %rax, 8(%rsp)
  movq %rdx, (%rsp)
  call destroy
  movq (%rsp), %rdx
  movq 8(%rdx), %rdi
  call destroy
  movq 8(%rsp), %rax
  movq (%rsp), %rdx
.L55:
  movq %rdx, %rdi
  movq %rax, (%rsp)
  call free
  movq (%rsp), %rax
.L53:
  movq %rax, %rdi
  call free
  movq 8(%rbp), %rax
  movq %rax, %r14
  movq (%rax), %rax
  testq %rax, %rax
  je .L56
  movq (%rax), %rdi
  testq %rdi, %rdi
  je .L57
  movq %rax, (%rsp)
  call destroy
  movq (%rsp), %rax
  movq 8(%rax), %rdi
  call destroy
  movq (%rsp), %rax
.L57:
  movq %rax, %rdi
  call free
  movq 8(%r14), %rax
  movq (%rax), %rdi
  testq %rdi, %rdi
  je .L58
  movq %rax, (%rsp)
  call destroy
  movq (%rsp), %rax
  movq 8(%rax), %rdi
  call destroy
  movq (%rsp), %rax
.L58:
  movq %rax, %rdi
  call free
.L56:
  movq %r14, %rdi
  call free
.L52:
  movq %rbp, %rdi
  call free
  movq 8(%rbx), %rbp
  movq 0(%rbp), %rax
  testq %rax, %rax
  je .L59
  movq (%rax), %rdx
  testq %rdx, %rdx
  je .L60
  movq (%rdx), %rdi
  testq %rdi, %rdi
  je .L61
  movq %rax, 8(%rsp)
  movq %rdx, (%rsp)
  call destroy
  movq (%rsp), %rdx
  movq 8(%rdx), %rdi
  call destroy
  movq 8(%rsp), %rax
  movq (%rsp), %rdx
.L61:
  movq %rdx, %rdi
  movq %rax, (%rsp)
  call free
  movq (%rsp), %rax
  movq 8(%rax), %rdx
  movq (%rdx), %rdi
  testq %rdi, %rdi
  je .L62
  movq %rax, 8(%rsp)
  movq %rdx, (%rsp)
  call destroy
  movq (%rsp), %rdx
  movq 8(%rdx), %rdi
  call destroy
  movq 8(%rsp), %rax
  movq (%rsp), %rdx
.L62:
  movq %rdx, %rdi
  movq %rax, (%rsp)
  call free
  movq (%rsp), %rax
.L60:
  movq %rax, %rdi
  call free
  movq 8(%rbp), %rax
  movq %rax, %r14
  movq (%rax), %rax
  testq %rax, %rax
  je .L63
  movq (%rax), %rdi
  testq %rdi, %rdi
  je .L64
  movq %rax, (%rsp)
  call destroy
  movq (%rsp), %rax
  movq 8(%rax), %rdi
  call destroy
  movq (%rsp), %rax
.L64:
  movq %rax, %rdi
  call free
  movq 8(%r14), %rax
  movq (%rax), %rdi
  testq %rdi, %rdi
  je .L65
  movq %rax, (%rsp)
  call destroy
  movq (%rsp), %rax
  movq 8(%rax), %rdi
  call destroy
  movq (%rsp), %rax
.L65:
  movq %rax, %rdi
  call free
.L63:
  movq %r14, %rdi
  call free
.L59:
  movq %rbp, %rdi
  call free
  movq 32(%rsp), %r14
.L51:
  movq 24(%rsp), %rbp
  movq %rbx, %rdi
  movq 16(%rsp), %rbx
  addq $40, %rsp
  jmp free
make:
  subq $72, %rsp
  movq %rbp, 48(%rsp)
  movl %edi, %ebp
  movl $16, %edi
  movq %rbx, 40(%rsp)
  call malloc
  movq %rax, %rbx
  testl %ebp, %ebp
  jne .L120
  vpxor %xmm0, %xmm0, %xmm0
  vmovdqu %xmm0, (%rax)
.L112:
  movq %rbx, %rax
  movq 48(%rsp), %rbp
  movq 40(%rsp), %rbx
  addq $72, %rsp
  ret
.L120:
  movl $16, %edi
  call malloc
  cmpl $1, %ebp
  jne .L121
  movq %rax, (%rbx)
  vpxor %xmm0, %xmm0, %xmm0
  movl $16, %edi
  vmovdqu %xmm0, (%rax)
  vmovdqa %xmm0, (%rsp)
  call malloc
  vmovdqa (%rsp), %xmm0
  movq %rax, %rbp
  vmovdqu %xmm0, (%rax)
.L117:
  movq %rbp, 8(%rbx)
  jmp .L112
.L121:
  movl $16, %edi
  movq %rax, (%rsp)
  call malloc
  cmpl $2, %ebp
  movq (%rsp), %rdx
  jne .L122
  movq %rax, (%rdx)
  vpxor %xmm1, %xmm1, %xmm1
  movl $16, %edi
  vmovdqu %xmm1, (%rax)
  movq %rdx, 24(%rsp)
  vmovdqa %xmm1, (%rsp)
  call malloc
  movq 24(%rsp), %rdx
  vmovdqa (%rsp), %xmm1
  movl $16, %edi
  movq %rax, 8(%rdx)
  movq %rdx, (%rbx)
  vmovdqu %xmm1, (%rax)
  call malloc
  movl $16, %edi
  movq %rax, %rbp
  call malloc
  vmovdqa (%rsp), %xmm1
  movl $16, %edi
  movq %rax, 0(%rbp)
  vmovdqu %xmm1, (%rax)
  call malloc
  vmovdqa (%rsp), %xmm1
  movq %rax, %rcx
  vmovdqu %xmm1, (%rax)
.L116:
  movq %rcx, 8(%rbp)
  jmp .L117
.L122:
  movq %r14, 56(%rsp)
  leal -3(%rbp), %r14d
  movl %r14d, %edi
  movq %r15, 64(%rsp)
  movq %rdx, 24(%rsp)
  movq %rax, (%rsp)
  call make
  movq (%rsp), %rcx
  movl %r14d, %edi
  movq %rax, (%rcx)
  call make
  movq (%rsp), %rcx
  movq 24(%rsp), %rdx
  movl $16, %edi
  movq %rcx, (%rdx)
  movq %rax, 8(%rcx)
  movq %rdx, (%rsp)
  call malloc
  movl %r14d, %edi
  movq %rax, %rbp
  call make
  movl %r14d, %edi
  movq %rax, 0(%rbp)
  call make
  movq (%rsp), %rdx
  movl $16, %edi
  movq %rax, 8(%rbp)
  movq %rbp, 8(%rdx)
  movq %rdx, (%rbx)
  call malloc
  movl $16, %edi
  movq %rax, %rbp
  call malloc
  movl %r14d, %edi
  movq %rax, %r15
  call make
  movl %r14d, %edi
  movq %rax, (%r15)
  call make
  movq %r15, 0(%rbp)
  movl $16, %edi
  movq %rax, 8(%r15)
  call malloc
  movl %r14d, %edi
  movq %rax, (%rsp)
  call make
  movq (%rsp), %rcx
  movl %r14d, %edi
  movq %rax, (%rcx)
  call make
  movq (%rsp), %rcx
  movq 56(%rsp), %r14
  movq 64(%rsp), %r15
  movq %rax, 8(%rcx)
  jmp .L116
.LC0:
  .string "stretch tree of depth %d\t check: %d\n"
.LC1:
  .string "%d\t trees of depth %d\t check: %d\n"
.LC2:
  .string "long lived tree of depth %d\t check: %d\n"
main:
  pushq %r15
  movl $16, %edi
  pushq %r14
  pushq %r13
  movl $18, %r13d
  pushq %r12
  pushq %rbp
  pushq %rbx
  subq $88, %rsp
  call malloc
  movl $18, %edi
  movq %rax, %rbx
  call make
  movl $18, %edi
  movq %rax, (%rbx)
  call make
  movq %rbx, %rdi
  movq %rax, 8(%rbx)
  call check
  movl $19, %esi
  movl $.LC0, %edi
  movl %eax, %edx
  xorl %eax, %eax
  vzeroupper
  call printf
  movq %rbx, %rdi
  call destroy
  movl $16, %edi
  call malloc
  movl $17, %edi
  movq %rax, %rbp
  call make
  movl $17, %edi
  movq %rax, 0(%rbp)
  call make
  movl $4, 20(%rsp)
  movq %rax, 8(%rbp)
  movq %rbp, 72(%rsp)
.L124:
  xorl %eax, %eax
  movl $1, %edx
  testb $1, %r13b
  je .L125
  movl $1, %eax
  movl $2, %edx
  cmpl %r13d, %eax
  je .L381
.L125:
  addl $2, %eax
  sall $2, %edx
  cmpl %r13d, %eax
  jne .L125
.L381:
  movl $0, 16(%rsp)
  movl $0, 4(%rsp)
  movl %edx, 56(%rsp)
  movl %r13d, 60(%rsp)
.L192:
  movl $16, %edi
  call malloc
  movl $16, %edi
  movq %rax, %rbx
  call malloc
  movl $16, %edi
  movq %rax, 8(%rsp)
  call malloc
  movl $16, %edi
  movq %rax, %rbp
  call malloc
  movl $16, %edi
  movq %rax, %r12
  call malloc
  movl 20(%rsp), %ecx
  movq %rax, %r13
  cmpl $4, %ecx
  jne .L385
  movq %rax, (%r12)
  vpxor %xmm0, %xmm0, %xmm0
  movl $16, %edi
  vmovdqu %xmm0, (%rax)
  call malloc
  movq %r12, 0(%rbp)
  vpxor %xmm0, %xmm0, %xmm0
  movl $16, %edi
  movq %rax, 8(%r12)
  vmovdqu %xmm0, (%rax)
  call malloc
  movl $16, %edi
  movq %rax, %r12
  call malloc
  vpxor %xmm0, %xmm0, %xmm0
  movl $16, %edi
  movq %rax, (%r12)
  vmovdqu %xmm0, (%rax)
  call malloc
  movq 8(%rsp), %r14
  movq %r12, 8(%rbp)
  vpxor %xmm0, %xmm0, %xmm0
  movq %rax, 8(%r12)
  movl $16, %edi
  movq %rbp, (%r14)
  vmovdqu %xmm0, (%rax)
  call malloc
  movl $16, %edi
  movq %rax, %r15
  movq %rax, 32(%rsp)
  call malloc
  movl $16, %edi
  movq %rax, %r12
  call malloc
  vpxor %xmm0, %xmm0, %xmm0
  movl $16, %edi
  movq %rax, (%r12)
  vmovdqu %xmm0, (%rax)
  call malloc
  movq %r12, (%r15)
  vpxor %xmm0, %xmm0, %xmm0
  movl $16, %edi
  movq %rax, 8(%r12)
  vmovdqu %xmm0, (%rax)
  call malloc
  movl $16, %edi
  movq %rax, %r12
  call malloc
  vpxor %xmm0, %xmm0, %xmm0
  movl $16, %edi
  movq %rax, (%r12)
  vmovdqu %xmm0, (%rax)
  call malloc
  movq %r12, 8(%r15)
  vpxor %xmm0, %xmm0, %xmm0
  movl $16, %edi
  movq %r15, 8(%r14)
  movq %rax, 8(%r12)
  movq %r14, (%rbx)
  vmovdqu %xmm0, (%rax)
  call malloc
  movl $16, %edi
  movq %rax, %r15
  call malloc
  movl $16, %edi
  movq %rax, %r14
  movq %rax, 24(%rsp)
  call malloc
  movl $16, %edi
  movq %rax, %r12
  call malloc
  vpxor %xmm0, %xmm0, %xmm0
  movl $16, %edi
  movq %rax, (%r12)
  vmovdqu %xmm0, (%rax)
  call malloc
  movq %r12, (%r14)
  vpxor %xmm0, %xmm0, %xmm0
  movl $16, %edi
  movq %rax, 8(%r12)
  vmovdqu %xmm0, (%rax)
  call malloc
  movl $16, %edi
  movq %rax, %r12
  call malloc
  vpxor %xmm0, %xmm0, %xmm0
  movl $16, %edi
  movq %rax, (%r12)
  vmovdqu %xmm0, (%rax)
  call malloc
  movq %r12, 8(%r14)
  vpxor %xmm0, %xmm0, %xmm0
  movl $16, %edi
  movq %rax, 8(%r12)
  movq %r14, (%r15)
  vmovdqu %xmm0, (%rax)
  call malloc
  movl $16, %edi
  movq %rax, %r13
  call malloc
  movl $16, %edi
  movq %rax, %r12
  call malloc
  vpxor %xmm0, %xmm0, %xmm0
  movl $16, %edi
  movq %rax, (%r12)
  vmovdqu %xmm0, (%rax)
  call malloc
  movq %r12, 0(%r13)
  vpxor %xmm0, %xmm0, %xmm0
  movl $16, %edi
  movq %rax, 8(%r12)
  vmovdqu %xmm0, (%rax)
  call malloc
  movl $16, %edi
  movq %rax, %r14
  movq %rax, 48(%rsp)
  call malloc
  vpxor %xmm0, %xmm0, %xmm0
  movl $16, %edi
  movq %rax, (%r14)
  vmovdqu %xmm0, (%rax)
  call malloc
  vpxor %xmm0, %xmm0, %xmm0
  movq %r14, 8(%r13)
  movq %rax, 8(%r14)
  vmovdqu %xmm0, (%rax)
  movq %r13, 8(%r15)
.L127:
  movq %r15, 8(%rbx)
  movq 8(%rsp), %rdi
  movq %rbx, %r8
  xorl %r9d, %r9d
.L131:
  call check
  movq 8(%r8), %r8
  leal 1(%r9,%rax), %r9d
  movq (%r8), %rdi
  testq %rdi, %rdi
  jne .L131
  movl 16(%rsp), %eax
  leal 1(%rax,%r9), %eax
  movl %eax, 16(%rsp)
  testq %rbp, %rbp
  je .L129
  movq 0(%rbp), %r12
  testq %r12, %r12
  je .L132
  movq (%r12), %r14
  testq %r14, %r14
  je .L133
  movq (%r14), %rax
  testq %rax, %rax
  je .L134
  movq (%rax), %rdi
  testq %rdi, %rdi
  je .L135
  movq %rax, 40(%rsp)
  call destroy
  movq 40(%rsp), %rax
  movq 8(%rax), %rdi
  call destroy
  movq 40(%rsp), %rax
.L135:
  movq %rax, %rdi
  call free
  movq 8(%r14), %rax
  movq (%rax), %rdi
  testq %rdi, %rdi
  je .L136
  movq %rax, 40(%rsp)
  call destroy
  movq 40(%rsp), %rax
  movq 8(%rax), %rdi
  call destroy
  movq 40(%rsp), %rax
.L136:
  movq %rax, %rdi
  call free
.L134:
  movq %r14, %rdi
  call free
  movq 8(%r12), %r14
  movq (%r14), %rax
  testq %rax, %rax
  je .L137
  movq (%rax), %rdi
  testq %rdi, %rdi
  je .L138
  movq %rax, 40(%rsp)
  call destroy
  movq 40(%rsp), %rax
  movq 8(%rax), %rdi
  call destroy
  movq 40(%rsp), %rax
.L138:
  movq %rax, %rdi
  call free
  movq 8(%r14), %rax
  movq (%rax), %rdi
  testq %rdi, %rdi
  je .L139
  movq %rax, 40(%rsp)
  call destroy
  movq 40(%rsp), %rax
  movq 8(%rax), %rdi
  call destroy
  movq 40(%rsp), %rax
.L139:
  movq %rax, %rdi
  call free
.L137:
  movq %r14, %rdi
  call free
.L133:
  movq %r12, %rdi
  call free
  movq 8(%rbp), %r12
  movq (%r12), %r14
  testq %r14, %r14
  je .L140
  movq (%r14), %rax
  testq %rax, %rax
  je .L141
  movq (%rax), %rdi
  testq %rdi, %rdi
  je .L142
  movq %rax, 40(%rsp)
  call destroy
  movq 40(%rsp), %rax
  movq 8(%rax), %rdi
  call destroy
  movq 40(%rsp), %rax
.L142:
  movq %rax, %rdi
  call free
  movq 8(%r14), %rax
  movq (%rax), %rdi
  testq %rdi, %rdi
  je .L143
  movq %rax, 40(%rsp)
  call destroy
  movq 40(%rsp), %rax
  movq 8(%rax), %rdi
  call destroy
  movq 40(%rsp), %rax
.L143:
  movq %rax, %rdi
  call free
.L141:
  movq %r14, %rdi
  call free
  movq 8(%r12), %r14
  movq (%r14), %rax
  testq %rax, %rax
  je .L144
  movq (%rax), %rdi
  testq %rdi, %rdi
  je .L145
  movq %rax, 40(%rsp)
  call destroy
  movq 40(%rsp), %rax
  movq 8(%rax), %rdi
  call destroy
  movq 40(%rsp), %rax
.L145:
  movq %rax, %rdi
  call free
  movq 8(%r14), %rax
  movq (%rax), %rdi
  testq %rdi, %rdi
  je .L146
  movq %rax, 40(%rsp)
  call destroy
  movq 40(%rsp), %rax
  movq 8(%rax), %rdi
  call destroy
  movq 40(%rsp), %rax
.L146:
  movq %rax, %rdi
  call free
.L144:
  movq %r14, %rdi
  call free
.L140:
  movq %r12, %rdi
  call free
.L132:
  movq %rbp, %rdi
  call free
  movq 32(%rsp), %rax
  movq (%rax), %rbp
  testq %rbp, %rbp
  je .L147
  movq 0(%rbp), %r12
  testq %r12, %r12
  je .L148
  movq (%r12), %r14
  testq %r14, %r14
  je .L149
  movq (%r14), %rdi
  testq %rdi, %rdi
  je .L150
  call destroy
  movq 8(%r14), %rdi
  call destroy
.L150:
  movq %r14, %rdi
  call free
  movq 8(%r12), %r14
  movq (%r14), %rdi
  testq %rdi, %rdi
  je .L151
  call destroy
  movq 8(%r14), %rdi
  call destroy
.L151:
  movq %r14, %rdi
  call free
.L149:
  movq %r12, %rdi
  call free
  movq 8(%rbp), %r12
  movq (%r12), %r14
  testq %r14, %r14
  je .L152
  movq (%r14), %rdi
  testq %rdi, %rdi
  je .L153
  call destroy
  movq 8(%r14), %rdi
  call destroy
.L153:
  movq %r14, %rdi
  call free
  movq 8(%r12), %r14
  movq (%r14), %rdi
  testq %rdi, %rdi
  je .L154
  call destroy
  movq 8(%r14), %rdi
  call destroy
.L154:
  movq %r14, %rdi
  call free
.L152:
  movq %r12, %rdi
  call free
.L148:
  movq %rbp, %rdi
  call free
  movq 32(%rsp), %rax
  movq 8(%rax), %rbp
  movq 0(%rbp), %r12
  testq %r12, %r12
  je .L155
  movq (%r12), %r14
  testq %r14, %r14
  je .L156
  movq (%r14), %rdi
  testq %rdi, %rdi
  je .L157
  call destroy
  movq 8(%r14), %rdi
  call destroy
.L157:
  movq %r14, %rdi
  call free
  movq 8(%r12), %r14
  movq (%r14), %rdi
  testq %rdi, %rdi
  je .L158
  call destroy
  movq 8(%r14), %rdi
  call destroy
.L158:
  movq %r14, %rdi
  call free
.L156:
  movq %r12, %rdi
  call free
  movq 8(%rbp), %r12
  movq (%r12), %r14
  testq %r14, %r14
  je .L159
  movq (%r14), %rdi
  testq %rdi, %rdi
  je .L160
  call destroy
  movq 8(%r14), %rdi
  call destroy
.L160:
  movq %r14, %rdi
  call free
  movq 8(%r12), %r14
  movq (%r14), %rdi
  testq %rdi, %rdi
  je .L161
  call destroy
  movq 8(%r14), %rdi
  call destroy
.L161:
  movq %r14, %rdi
  call free
.L159:
  movq %r12, %rdi
  call free
.L155:
  movq %rbp, %rdi
  call free
.L147:
  movq 32(%rsp), %rdi
  call free
.L129:
  movq 8(%rsp), %rdi
  call free
  movq 24(%rsp), %rax
  movq (%rax), %rbp
  testq %rbp, %rbp
  je .L162
  movq 0(%rbp), %r12
  testq %r12, %r12
  je .L163
  movq (%r12), %r14
  testq %r14, %r14
  je .L164
  movq (%r14), %rdi
  testq %rdi, %rdi
  je .L165
  call destroy
  movq 8(%r14), %rdi
  call destroy
.L165:
  movq %r14, %rdi
  call free
  movq 8(%r12), %r14
  movq (%r14), %rdi
  testq %rdi, %rdi
  je .L166
  call destroy
  movq 8(%r14), %rdi
  call destroy
.L166:
  movq %r14, %rdi
  call free
.L164:
  movq %r12, %rdi
  call free
  movq 8(%rbp), %r12
  movq (%r12), %r14
  testq %r14, %r14
  je .L167
  movq (%r14), %rdi
  testq %rdi, %rdi
  je .L168
  call destroy
  movq 8(%r14), %rdi
  call destroy
.L168:
  movq %r14, %rdi
  call free
  movq 8(%r12), %r14
  movq (%r14), %rdi
  testq %rdi, %rdi
  je .L169
  call destroy
  movq 8(%r14), %rdi
  call destroy
.L169:
  movq %r14, %rdi
  call free
.L167:
  movq %r12, %rdi
  call free
.L163:
  movq %rbp, %rdi
  call free
  movq 24(%rsp), %rax
  movq 8(%rax), %rbp
  movq 0(%rbp), %r12
  testq %r12, %r12
  je .L170
  movq (%r12), %r14
  testq %r14, %r14
  je .L171
  movq (%r14), %rdi
  testq %rdi, %rdi
  je .L172
  call destroy
  movq 8(%r14), %rdi
  call destroy
.L172:
  movq %r14, %rdi
  call free
  movq 8(%r12), %r14
  movq (%r14), %rdi
  testq %rdi, %rdi
  je .L173
  call destroy
  movq 8(%r14), %rdi
  call destroy
.L173:
  movq %r14, %rdi
  call free
.L171:
  movq %r12, %rdi
  call free
  movq 8(%rbp), %r12
  movq (%r12), %r14
  testq %r14, %r14
  je .L174
  movq (%r14), %rdi
  testq %rdi, %rdi
  je .L175
  call destroy
  movq 8(%r14), %rdi
  call destroy
.L175:
  movq %r14, %rdi
  call free
  movq 8(%r12), %r14
  movq (%r14), %rdi
  testq %rdi, %rdi
  je .L176
  call destroy
  movq 8(%r14), %rdi
  call destroy
.L176:
  movq %r14, %rdi
  call free
.L174:
  movq %r12, %rdi
  call free
.L170:
  movq %rbp, %rdi
  call free
.L162:
  movq 24(%rsp), %rdi
  call free
  movq 0(%r13), %rbp
  testq %rbp, %rbp
  je .L177
  movq 0(%rbp), %r12
  testq %r12, %r12
  je .L178
  movq (%r12), %r14
  testq %r14, %r14
  je .L179
  movq (%r14), %rdi
  testq %rdi, %rdi
  je .L180
  call destroy
  movq 8(%r14), %rdi
  call destroy
.L180:
  movq %r14, %rdi
  call free
  movq 8(%r12), %r14
  movq (%r14), %rdi
  testq %rdi, %rdi
  je .L181
  call destroy
  movq 8(%r14), %rdi
  call destroy
.L181:
  movq %r14, %rdi
  call free
.L179:
  movq %r12, %rdi
  call free
  movq 8(%rbp), %r12
  movq (%r12), %r14
  testq %r14, %r14
  je .L182
  movq (%r14), %rdi
  testq %rdi, %rdi
  je .L183
  call destroy
  movq 8(%r14), %rdi
  call destroy
.L183:
  movq %r14, %rdi
  call free
  movq 8(%r12), %r14
  movq (%r14), %rdi
  testq %rdi, %rdi
  je .L184
  call destroy
  movq 8(%r14), %rdi
  call destroy
.L184:
  movq %r14, %rdi
  call free
.L182:
  movq %r12, %rdi
  call free
.L178:
  movq %rbp, %rdi
  call free
  movq 48(%rsp), %rax
  movq (%rax), %rbp
  testq %rbp, %rbp
  je .L185
  movq 0(%rbp), %r12
  testq %r12, %r12
  je .L186
  movq (%r12), %rdi
  testq %rdi, %rdi
  je .L187
  call destroy
  movq 8(%r12), %rdi
  call destroy
.L187:
  movq %r12, %rdi
  call free
  movq 8(%rbp), %r12
  movq (%r12), %rdi
  testq %rdi, %rdi
  je .L188
  call destroy
  movq 8(%r12), %rdi
  call destroy
.L188:
  movq %r12, %rdi
  call free
.L186:
  movq %rbp, %rdi
  call free
  movq 48(%rsp), %rax
  movq 8(%rax), %rbp
  movq 0(%rbp), %r12
  testq %r12, %r12
  je .L189
  movq (%r12), %rdi
  testq %rdi, %rdi
  je .L190
  call destroy
  movq 8(%r12), %rdi
  call destroy
.L190:
  movq %r12, %rdi
  call free
  movq 8(%rbp), %r12
  movq (%r12), %rdi
  testq %rdi, %rdi
  je .L191
  call destroy
  movq 8(%r12), %rdi
  call destroy
.L191:
  movq %r12, %rdi
  call free
.L189:
  movq %rbp, %rdi
  call free
.L185:
  movq 48(%rsp), %rdi
  call free
.L177:
  movq %r13, %rdi
  call free
  movq %r15, %rdi
  call free
  movq %rbx, %rdi
  call free
  addl $1, 4(%rsp)
  movl 4(%rsp), %eax
  cmpl 56(%rsp), %eax
  jne .L192
  movl 20(%rsp), %ebx
  movl 16(%rsp), %ecx
  movl %eax, %esi
  movl $.LC1, %edi
  movl 60(%rsp), %r13d
  xorl %eax, %eax
  movl %ebx, %edx
  call printf
  leal 2(%rbx), %eax
  subl $2, %r13d
  movl %eax, 20(%rsp)
  cmpl $20, %eax
  jne .L124
  movq 72(%rsp), %rbp
  movq %rbp, %rdi
  call check
  movl $18, %esi
  movl $.LC2, %edi
  movl %eax, %edx
  xorl %eax, %eax
  call printf
  movq %rbp, %rdi
  call destroy
  addq $88, %rsp
  xorl %eax, %eax
  popq %rbx
  popq %rbp
  popq %r12
  popq %r13
  popq %r14
  popq %r15
  ret
.L385:
  leal -5(%rcx), %r15d
  movl %r15d, %edi
  call make
  movl %r15d, %edi
  movq %rax, 0(%r13)
  call make
  movq %r13, (%r12)
  movl $16, %edi
  movq %rax, 8(%r13)
  call malloc
  movl %r15d, %edi
  movq %rax, %r13
  call make
  movl %r15d, %edi
  movl %r15d, 40(%rsp)
  movq %rax, 0(%r13)
  call make
  movq %r13, 8(%r12)
  movl $16, %edi
  movq %rax, 8(%r13)
  movq %r12, 0(%rbp)
  call malloc
  movl $16, %edi
  movq %rax, %r13
  call malloc
  movl $16, %edi
  movq %rax, %r14
  call malloc
  movl 20(%rsp), %ecx
  movq %rax, %r15
  leal -6(%rcx), %r12d
  movl %r12d, %edi
  call make
  movl %r12d, %edi
  movq %rax, (%r15)
  call make
  movq %r15, (%r14)
  movl $16, %edi
  movq %rax, 8(%r15)
  call malloc
  movl %r12d, %edi
  movq %rax, %r15
  call make
  movl %r12d, %edi
  movq %rax, (%r15)
  call make
  movq %r15, 8(%r14)
  movl $16, %edi
  movq %r14, 0(%r13)
  movq %rax, 8(%r15)
  call malloc
  movl $16, %edi
  movq %rax, %r14
  call malloc
  movl %r12d, %edi
  movq %rax, %r15
  call make
  movl %r12d, %edi
  movq %rax, (%r15)
  call make
  movq %r15, (%r14)
  movl $16, %edi
  movq %rax, 8(%r15)
  call malloc
  movl %r12d, %edi
  movq %rax, %r15
  call make
  movl %r12d, %edi
  movq %rax, (%r15)
  call make
  movq %r15, 8(%r14)
  movl $16, %edi
  movq %rax, 8(%r15)
  movq 8(%rsp), %rax
  movq %r14, 8(%r13)
  movq %r13, 8(%rbp)
  movq %rbp, (%rax)
  call malloc
  movl $16, %edi
  movq %rax, 32(%rsp)
  call malloc
  movl $16, %edi
  movq %rax, %r13
  call malloc
  movl $16, %edi
  movq %rax, %r14
  call malloc
  movl %r12d, %edi
  movq %rax, %r15
  call make
  movl %r12d, %edi
  movq %rax, (%r15)
  call make
  movq %r15, (%r14)
  movl $16, %edi
  movq %rax, 8(%r15)
  call malloc
  movl %r12d, %edi
  movq %rax, %r15
  call make
  movl %r12d, %edi
  movq %rax, (%r15)
  call make
  movq %r15, 8(%r14)
  movl $16, %edi
  movq %r14, 0(%r13)
  movq %rax, 8(%r15)
  call malloc
  movl $16, %edi
  movq %rax, %r14
  call malloc
  movl %r12d, %edi
  movq %rax, %r15
  call make
  movl %r12d, %edi
  movq %rax, (%r15)
  call make
  movq %r15, (%r14)
  movl $16, %edi
  movq %rax, 8(%r15)
  call malloc
  movl %r12d, %edi
  movq %rax, %r15
  call make
  movl %r12d, %edi
  movq %rax, (%r15)
  call make
  movq 32(%rsp), %rdx
  movq %r15, 8(%r14)
  movl $16, %edi
  movq %r14, 8(%r13)
  movq %r13, (%rdx)
  movq %rax, 8(%r15)
  call malloc
  movl $16, %edi
  movq %rax, %r13
  call malloc
  movl $16, %edi
  movq %rax, %r14
  call malloc
  movl %r12d, %edi
  movq %rax, %r15
  call make
  movl %r12d, %edi
  movq %rax, (%r15)
  call make
  movq %r15, (%r14)
  movl $16, %edi
  movq %rax, 8(%r15)
  call malloc
  movl %r12d, %edi
  movq %rax, %r15
  call make
  movl %r12d, %edi
  movq %rax, (%r15)
  call make
  movq %r15, 8(%r14)
  movl $16, %edi
  movq %r14, 0(%r13)
  movq %rax, 8(%r15)
  call malloc
  movl $16, %edi
  movq %rax, %r14
  call malloc
  movl %r12d, %edi
  movq %rax, %r15
  call make
  movl %r12d, %edi
  movq %rax, (%r15)
  call make
  movq %r15, (%r14)
  movl $16, %edi
  movq %rax, 8(%r15)
  call malloc
  movl %r12d, %edi
  movq %rax, %r15
  call make
  movl %r12d, %edi
  movq %rax, (%r15)
  call make
  movq 32(%rsp), %rdx
  movq %r15, 8(%r14)
  movl $16, %edi
  movq %rax, 8(%r15)
  movq 8(%rsp), %rax
  movq %r13, 8(%rdx)
  movq %rdx, 8(%rax)
  movq %r14, 8(%r13)
  movq %rax, (%rbx)
  call malloc
  movl $16, %edi
  movq %rax, %r15
  call malloc
  movl $16, %edi
  movq %rax, 24(%rsp)
  call malloc
  movl $16, %edi
  movq %rax, %r13
  call malloc
  movl 40(%rsp), %edi
  movq %rax, %r14
  call make
  movl 40(%rsp), %edi
  movq %rax, (%r14)
  call make
  movq %r14, 0(%r13)
  movl $16, %edi
  movq %rax, 8(%r14)
  call malloc
  movl 40(%rsp), %edi
  movq %rax, %r14
  call make
  movl 40(%rsp), %edi
  movq %rax, (%r14)
  call make
  movq %r14, 8(%r13)
  movl $16, %edi
  movq %rax, 8(%r14)
  movq 24(%rsp), %rax
  movq %r13, (%rax)
  call malloc
  movl $16, %edi
  movq %rax, %r13
  call malloc
  movl 40(%rsp), %edi
  movq %rax, %r14
  call make
  movl 40(%rsp), %edi
  movq %rax, (%r14)
  call make
  movq %r14, 0(%r13)
  movl $16, %edi
  movq %rax, 8(%r14)
  call malloc
  movl 40(%rsp), %edi
  movq %rax, %r14
  call make
  movl 40(%rsp), %edi
  movq %rax, (%r14)
  call make
  movq %r14, 8(%r13)
  movl $16, %edi
  movq %rax, 8(%r14)
  movq 24(%rsp), %rax
  movq %r13, 8(%rax)
  movq %rax, (%r15)
  call malloc
  movl $16, %edi
  movq %rax, %r13
  call malloc
  movl $16, %edi
  movq %rax, 48(%rsp)
  call malloc
  movl $16, %edi
  movq %rax, 64(%rsp)
  call malloc
  movl %r12d, %edi
  movq %rax, %r14
  call make
  movl %r12d, %edi
  movq %rax, (%r14)
  call make
  movq 64(%rsp), %rdx
  movl $16, %edi
  movq %rax, 8(%r14)
  movq %r14, (%rdx)
  call malloc
  movl %r12d, %edi
  movq %rax, %r14
  call make
  movl %r12d, %edi
  movq %rax, (%r14)
  call make
  movq 64(%rsp), %rdx
  movl $16, %edi
  movq %rax, 8(%r14)
  movq 48(%rsp), %rax
  movq %r14, 8(%rdx)
  movq %rdx, (%rax)
  call malloc
  movl $16, %edi
  movq %rax, 64(%rsp)
  call malloc
  movl %r12d, %edi
  movq %rax, %r14
  call make
  movl %r12d, %edi
  movq %rax, (%r14)
  call make
  movq 64(%rsp), %rdx
  movl $16, %edi
  movq %rax, 8(%r14)
  movq %r14, (%rdx)
  call malloc
  movl %r12d, %edi
  movq %rax, %r14
  call make
  movl %r12d, %edi
  movq %rax, (%r14)
  call make
  movq 64(%rsp), %rdx
  movl $16, %edi
  movq %rax, 8(%r14)
  movq 48(%rsp), %rax
  movq %r14, 8(%rdx)
  movq %rdx, 8(%rax)
  movq %rax, 0(%r13)
  call malloc
  movl $16, %edi
  movq %rax, 48(%rsp)
  call malloc
  movl 40(%rsp), %r14d
  movq %rax, %r12
  movl %r14d, %edi
  call make
  movl %r14d, %edi
  movq %rax, (%r12)
  call make
  movl $16, %edi
  movq %rax, 8(%r12)
  movq 48(%rsp), %rax
  movq %r12, (%rax)
  call malloc
  movl %r14d, %edi
  movq %rax, %r12
  call make
  movl %r14d, %edi
  movq %rax, (%r12)
  call make
  movq %rax, 8(%r12)
  movq 48(%rsp), %rax
  movq %r12, 8(%rax)
  movq %rax, 8(%r13)
  movq %r13, 8(%r15)
  jmp .L127
