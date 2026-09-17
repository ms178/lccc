two_way_short_needle.constprop.0:
  pushq %rbp
  vmovd %esi, %xmm0
  leal -1(%rsi), %ecx
  leal -1(%rsi), %edx
  vpbroadcastb %xmm0, %ymm0
  movq %rsp, %rbp
  subq $136, %rsp
  vmovdqu %ymm0, -120(%rsp)
  vmovdqu %ymm0, -88(%rsp)
  movzbl (%rdi), %eax
  vmovdqu %ymm0, -56(%rsp)
  vmovdqu %ymm0, -24(%rsp)
  vmovdqu %ymm0, 8(%rsp)
  movq %rax, %r10
  vmovdqu %ymm0, 40(%rsp)
  vmovdqu %ymm0, 72(%rsp)
  vmovdqu %ymm0, 104(%rsp)
  movb %cl, -120(%rsp,%rax)
  movzbl 1(%rdi), %eax
  leal -2(%rsi), %ecx
  movb %cl, -120(%rsp,%rax)
  movzbl 2(%rdi), %eax
  leal -3(%rsi), %ecx
  movb %cl, -120(%rsp,%rax)
  cmpl $4, %esi
  je .L2
  movzbl 3(%rdi), %eax
  leal -4(%rsi), %ecx
  movb %cl, -120(%rsp,%rax)
  cmpl $4, %edx
  je .L2
  movzbl 4(%rdi), %eax
  leal -5(%rsi), %ecx
  movb %cl, -120(%rsp,%rax)
  cmpl $5, %edx
  je .L2
  movzbl 5(%rdi), %eax
  leal -6(%rsi), %ecx
  movb %cl, -120(%rsp,%rax)
  cmpl $6, %edx
  je .L2
  movzbl 6(%rdi), %eax
  leal -7(%rsi), %ecx
  movb %cl, -120(%rsp,%rax)
  cmpl $7, %edx
  je .L2
  movzbl 7(%rdi), %eax
  leal -8(%rsi), %ecx
  movb %cl, -120(%rsp,%rax)
  cmpl $8, %edx
  je .L2
  movzbl 8(%rdi), %eax
  leal -9(%rsi), %ecx
  movb %cl, -120(%rsp,%rax)
  cmpl $10, %edx
  jne .L2
  movzbl 9(%rdi), %eax
  movb $1, -120(%rsp,%rax)
.L2:
  movl %edx, %ecx
  xorl %eax, %eax
  movzbl (%rdi,%rcx), %r9d
  cmpb haystack(%rcx), %r9b
  jne .L4
.L3:
  leal -2(%rsi), %r8d
  leal -2(%rax,%rsi), %ecx
  movzbl (%rdi,%r8), %r11d
  cmpb %r11b, haystack(%rcx)
  jne .L4
  leal -3(%rsi), %r8d
  leal -3(%rax,%rsi), %ecx
  movzbl (%rdi,%r8), %r11d
  cmpb %r11b, haystack(%rcx)
  jne .L4
  leal -4(%rsi), %r8d
  leal -4(%rax,%rsi), %ecx
  movzbl (%rdi,%r8), %r11d
  cmpb %r11b, haystack(%rcx)
  jne .L4
  cmpl $4, %esi
  je .L1
  leal -5(%rax,%rsi), %ecx
  leal -5(%rsi), %r8d
  movzbl haystack(%rcx), %ecx
  cmpb %cl, (%rdi,%r8)
  jne .L4
  cmpl $5, %esi
  je .L1
  leal -6(%rax,%rsi), %ecx
  leal -6(%rsi), %r8d
  movzbl haystack(%rcx), %ecx
  cmpb %cl, (%rdi,%r8)
  jne .L4
  cmpl $6, %esi
  je .L1
  leal -7(%rax,%rsi), %ecx
  leal -7(%rsi), %r8d
  movzbl haystack(%rcx), %ecx
  cmpb %cl, (%rdi,%r8)
  jne .L4
  cmpl $7, %esi
  je .L1
  leal -8(%rax,%rsi), %ecx
  leal -8(%rsi), %r8d
  movzbl haystack(%rcx), %ecx
  cmpb %cl, (%rdi,%r8)
  jne .L4
  cmpl $8, %esi
  je .L1
  leal -9(%rax,%rsi), %ecx
  leal -9(%rsi), %r8d
  movzbl haystack(%rcx), %ecx
  cmpb %cl, (%rdi,%r8)
  jne .L4
  cmpl $9, %esi
  je .L1
  leal -10(%rax,%rsi), %ecx
  leal -10(%rsi), %r8d
  movzbl haystack(%rcx), %ecx
  cmpb %cl, (%rdi,%r8)
  jne .L4
  cmpl $10, %esi
  je .L1
  movl %eax, %ecx
  cmpb %r10b, haystack(%rcx)
  jne .L4
.L1:
  vzeroupper
  leave
  ret
.L4:
  movl $524288, %ecx
  subl %esi, %ecx
.L5:
  leal (%rdx,%rax), %r8d
  movzbl haystack(%r8), %r8d
  movzbl -120(%rsp,%r8), %r8d
  addl %r8d, %eax
  cmpl %eax, %ecx
  jb .L53
  leal (%rax,%rdx), %r8d
  cmpb haystack(%r8), %r9b
  je .L3
  jmp .L5
.L53:
  vzeroupper
  movl $-1, %eax
  leave
  ret
.LC0:
main:
  pushq %rbp
  movl $haystack+524288, %edi
  movl $2084213038, %edx
  movq %rsp, %rbp
  pushq %r15
  pushq %r14
  pushq %r13
  movl $haystack, %r13d
  pushq %r12
  movq %r13, %rcx
  pushq %rbx
  subq $40, %rsp
.L55:
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
  cmpq %rcx, %rdi
  jne .L55
  movb $0, haystack+524288(%rip)
  movl $523124044, %ebx
  xorl %r12d, %r12d
  movb $97, 15(%rsp)
.L56:
  xorl %r14d, %r14d
  xorl %r15d, %r15d
  jmp .L71
.L96:
  imull $31337, %ebx, %edi
  movl %edi, %eax
  shrl $5, %eax
  imulq $134225921, %rax, %rax
  shrq $41, %rax
  imull $524256, %eax, %eax
  subl %eax, %edi
  testb $8, %sil
  jne .L92
  testb $4, %sil
  jne .L93
  testl %esi, %esi
  je .L68
  movzbl haystack(%rdi), %eax
  movb %al, 16(%rsp)
  testb $2, %sil
  jne .L94
.L68:
  leaq 16(%rsp), %rdi
  call two_way_short_needle.constprop.0
  cmpl $-1, %eax
  je .L69
.L97:
  movq %r15, %rsi
  movl %eax, %eax
  salq $24, %rsi
  xorq %rax, %rsi
  addq %rsi, %r12
.L70:
  addq $1, %r15
  addq $104729, %r14
  cmpq $2048, %r15
  je .L95
.L71:
  imull $1664525, %ebx, %ebx
  movl %r15d, %esi
  andl $7, %esi
  addl $4, %esi
  addl $1013904223, %ebx
  testb $1, %r15b
  jne .L96
  movl %ebx, %eax
  movl %ebx, %edi
  imulq $1321528399, %rax, %rax
  shrq $35, %rax
  imull $26, %eax, %eax
  subl %eax, %edi
  leal 97(%rdi), %eax
  movl %ebx, %edi
  shrl $3, %edi
  movb %al, 16(%rsp)
  movl %edi, %eax
  imulq $1321528399, %rax, %rax
  shrq $35, %rax
  imull $26, %eax, %eax
  subl %eax, %edi
  addl $97, %edi
  movb %dil, 17(%rsp)
  movl %ebx, %edi
  shrl $6, %edi
  movl %edi, %eax
  imulq $1321528399, %rax, %rax
  shrq $35, %rax
  imull $26, %eax, %eax
  subl %eax, %edi
  addl $97, %edi
  movb %dil, 18(%rsp)
  movl %ebx, %edi
  shrl $9, %edi
  movl %edi, %eax
  imulq $1321528399, %rax, %rax
  shrq $35, %rax
  imull $26, %eax, %eax
  subl %eax, %edi
  addl $97, %edi
  movb %dil, 19(%rsp)
  cmpl $4, %esi
  je .L68
  movl %ebx, %edi
  shrl $12, %edi
  movl %edi, %eax
  imulq $1321528399, %rax, %rax
  shrq $35, %rax
  imull $26, %eax, %eax
  subl %eax, %edi
  addl $97, %edi
  movb %dil, 20(%rsp)
  movl %ebx, %edi
  shrl $15, %edi
  movl %edi, %eax
  imulq $1321528399, %rax, %rax
  shrq $35, %rax
  imull $26, %eax, %eax
  subl %eax, %edi
  addl $97, %edi
  movb %dil, 21(%rsp)
  cmpl $6, %esi
  je .L68
  movl %ebx, %edi
  shrl $18, %edi
  movl %edi, %eax
  imulq $1321528399, %rax, %rax
  shrq $35, %rax
  imull $26, %eax, %eax
  subl %eax, %edi
  addl $97, %edi
  movb %dil, 22(%rsp)
  movl %ebx, %edi
  shrl $21, %edi
  movl %edi, %eax
  imulq $1321528399, %rax, %rax
  shrq $35, %rax
  imull $26, %eax, %eax
  subl %eax, %edi
  addl $97, %edi
  movb %dil, 23(%rsp)
  cmpl $8, %esi
  je .L68
  movl %ebx, %edi
  shrl $24, %edi
  movl %edi, %eax
  imulq $1321528399, %rax, %rax
  shrq $35, %rax
  imull $26, %eax, %eax
  subl %eax, %edi
  addl $97, %edi
  movb %dil, 24(%rsp)
  movl %ebx, %edi
  shrl $27, %edi
  movl %edi, %eax
  imulq $1321528399, %rax, %rax
  shrq $35, %rax
  imull $26, %eax, %eax
  subl %eax, %edi
  addl $97, %edi
  movb %dil, 25(%rsp)
  leaq 16(%rsp), %rdi
  call two_way_short_needle.constprop.0
  cmpl $-1, %eax
  jne .L97
.L69:
  addq %r14, %r12
  jmp .L70
.L92:
  movq haystack(%rdi), %rax
  movq %rax, 16(%rsp)
  movl %esi, %eax
  movq haystack-8(%rdi,%rax), %rdi
  movq %rdi, 8(%rsp,%rax)
  jmp .L68
.L93:
  movl haystack(%rdi), %eax
  movl %eax, 16(%rsp)
  movl %esi, %eax
  movl haystack-4(%rdi,%rax), %edi
  movl %edi, 12(%rsp,%rax)
  jmp .L68
.L95:
  movzbl 15(%rsp), %eax
  addq $16381, %r13
  movb %al, -16381(%r13)
  addl $1, %eax
  movb %al, 15(%rsp)
  movl $haystack+131048, %eax
  cmpq %r13, %rax
  jne .L56
  movq %r12, %rsi
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
.L94:
  movl %esi, %eax
  movzwl haystack-2(%rdi,%rax), %edi
  movw %di, 14(%rsp,%rax)
  jmp .L68
