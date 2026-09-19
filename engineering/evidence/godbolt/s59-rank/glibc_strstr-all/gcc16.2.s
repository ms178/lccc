two_way_short_needle.constprop.0:
  pushq %rbp
  vmovd %esi, %xmm0
  leal -2(%rsi), %eax
  movq %rdi, %r10
  vpbroadcastb %xmm0, %ymm0
  leaq 1(%rdi,%rax), %r8
  leal -1(%rsi), %r9d
  movq %rdi, %rax
  leal -1(%rsi,%rdi), %edi
  movq %rsp, %rbp
  subq $136, %rsp
  vmovdqu %ymm0, -120(%rsp)
  vmovdqu %ymm0, -88(%rsp)
  vmovdqu %ymm0, -56(%rsp)
  vmovdqu %ymm0, -24(%rsp)
  vmovdqu %ymm0, 8(%rsp)
  vmovdqu %ymm0, 40(%rsp)
  vmovdqu %ymm0, 72(%rsp)
  vmovdqu %ymm0, 104(%rsp)
.L2:
  movzbl (%rax), %edx
  movl %edi, %ecx
  subl %eax, %ecx
  addq $1, %rax
  movb %cl, -120(%rsp,%rdx)
  cmpq %r8, %rax
  jne .L2
  movl $524288, %r8d
  movl %r9d, %edx
  xorl %eax, %eax
  subl %esi, %r8d
  movl %esi, %esi
  leaq (%r10,%rsi), %r11
  leaq -1(%r10,%rsi), %rsi
  movl %r9d, %r10d
  subq %rdx, %rsi
  subl %r11d, %r10d
.L8:
  movq %r11, %rdx
  leal (%r10,%rax), %edi
.L3:
  leal (%rdi,%rdx), %ecx
  movzbl haystack(%rcx), %ecx
  cmpb %cl, -1(%rdx)
  jne .L14
  subq $1, %rdx
  cmpq %rdx, %rsi
  jne .L3
  vzeroupper
  leave
  ret
.L14:
  leal (%r9,%rax), %edx
  movzbl haystack(%rdx), %edx
  movzbl -120(%rsp,%rdx), %edx
  addl %edx, %eax
  cmpl %eax, %r8d
  jnb .L8
  vzeroupper
  movl $-1, %eax
  leave
  ret
.LC0:
main:
  pushq %rbp
  movl $2084213038, %edx
  movq %rsp, %rbp
  pushq %r15
  pushq %r14
  pushq %r13
  pushq %r12
  movl $haystack, %r12d
  pushq %rbx
  movq %r12, %rcx
  subq $40, %rsp
.L16:
  imull $1664525, %edx, %edx
  addq $1, %rcx
  leal 1013904223(%rdx), %eax
  movq %rax, %rdx
  imulq $1321528399, %rax, %rax
  movl %edx, %esi
  shrq $35, %rax
  imull $26, %eax, %eax
  subl %eax, %esi
  leal 97(%rsi), %eax
  movb %al, -1(%rcx)
  cmpq $haystack+524288, %rcx
  jne .L16
  movb $0, haystack+524288(%rip)
  movl $523124044, %r15d
  xorl %ebx, %ebx
  movb $97, 15(%rsp)
.L17:
  xorl %r13d, %r13d
  xorl %r14d, %r14d
  jmp .L33
.L50:
  imull $31337, %r15d, %edx
  movl %edx, %eax
  shrl $5, %eax
  imulq $134225921, %rax, %rax
  shrq $41, %rax
  imull $524256, %eax, %eax
  subl %eax, %edx
  testb $8, %r10b
  jne .L46
  testb $4, %r10b
  jne .L47
  testl %r10d, %r10d
  je .L29
  movzbl haystack(%rdx), %eax
  movb %al, 16(%rsp)
  testb $2, %r10b
  jne .L48
.L29:
  movl %r10d, %esi
  leaq 16(%rsp), %rdi
  call two_way_short_needle.constprop.0
  cmpl $-1, %eax
  je .L31
  movq %r14, %rdx
  movl %eax, %eax
  salq $24, %rdx
  xorq %rax, %rdx
  addq %rdx, %rbx
.L32:
  addq $1, %r14
  addq $104729, %r13
  cmpq $2048, %r14
  je .L49
.L33:
  imull $1664525, %r15d, %r15d
  movl %r14d, %eax
  movl %r14d, %ecx
  andl $7, %eax
  leal 4(%rax), %r10d
  addl $1013904223, %r15d
  andl $1, %ecx
  jne .L50
  leaq 16(%rsp), %rsi
  leal 12(%rax,%rax,2), %edi
.L30:
  shrx %ecx, %r15d, %eax
  movq %rax, %rdx
  imulq $1321528399, %rax, %rax
  addl $3, %ecx
  addq $1, %rsi
  shrq $35, %rax
  imull $26, %eax, %eax
  subl %eax, %edx
  addl $97, %edx
  movb %dl, -1(%rsi)
  cmpl %ecx, %edi
  jne .L30
  jmp .L29
.L31:
  addq %r13, %rbx
  jmp .L32
.L46:
  movq haystack(%rdx), %rax
  movq %rax, 16(%rsp)
  movl %r10d, %eax
  movq haystack-8(%rdx,%rax), %rdx
  movq %rdx, 8(%rsp,%rax)
  jmp .L29
.L47:
  movl haystack(%rdx), %eax
  movl %eax, 16(%rsp)
  movl %r10d, %eax
  movl haystack-4(%rdx,%rax), %edx
  movl %edx, 12(%rsp,%rax)
  jmp .L29
.L49:
  movzbl 15(%rsp), %eax
  addq $16381, %r12
  movb %al, -16381(%r12)
  addl $1, %eax
  movb %al, 15(%rsp)
  movl $haystack+131048, %eax
  cmpq %r12, %rax
  jne .L17
  movq %rbx, %rsi
  movl $.LC0, %edi
  xorl %eax, %eax
  call printf
  addq $40, %rsp
  xorl %eax, %eax
  popq %rbx
  popq %r12
  popq %r13
  popq %r14
  popq %r15
  popq %rbp
  ret
.L48:
  movl %r10d, %eax
  movzwl haystack-2(%rdx,%rax), %edx
  movw %dx, 14(%rsp,%rax)
  jmp .L29
