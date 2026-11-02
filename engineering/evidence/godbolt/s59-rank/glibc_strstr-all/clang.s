main:
  movl $2084213038, %ecx
  xorl %edx, %edx
  leaq haystack(%rip), %rax
.LBB0_1:
  imull $1664525, %ecx, %esi
  addl $1013904223, %esi
  imulq $1321528399, %rsi, %rdi
  shrq $35, %rdi
  leal (%rdi,%rdi,4), %r8d
  leal (%r8,%r8,4), %r8d
  addl %edi, %r8d
  subl %r8d, %esi
  addb $97, %sil
  movb %sil, (%rdx,%rax)
  imull $389569705, %ecx, %esi
  addl $1196435762, %esi
  imulq $1321528399, %rsi, %rdi
  shrq $35, %rdi
  leal (%rdi,%rdi,4), %r8d
  leal (%r8,%r8,4), %r8d
  addl %edi, %r8d
  subl %r8d, %esi
  addb $97, %sil
  movb %sil, 1(%rdx,%rax)
  imull $-1354167659, %ecx, %esi
  addl $-775096599, %esi
  imulq $1321528399, %rsi, %rdi
  shrq $35, %rdi
  leal (%rdi,%rdi,4), %r8d
  leal (%r8,%r8,4), %r8d
  addl %edi, %r8d
  subl %r8d, %esi
  addb $97, %sil
  movb %sil, 2(%rdx,%rax)
  imull $158984081, %ecx, %ecx
  addl $-1426500812, %ecx
  imulq $1321528399, %rcx, %rsi
  shrq $35, %rsi
  leal (%rsi,%rsi,4), %edi
  leal (%rdi,%rdi,4), %edi
  addl %esi, %edi
  movl %ecx, %esi
  subl %edi, %esi
  addb $97, %sil
  movb %sil, 3(%rdx,%rax)
  addq $4, %rdx
  cmpq $524288, %rdx
  jne .LBB0_1
  pushq %rbp
  pushq %r15
  pushq %r14
  pushq %r13
  pushq %r12
  pushq %rbx
  subq $328, %rsp
  movb $0, haystack+524288(%rip)
  movl $523124044, %ecx
  xorl %r12d, %r12d
  xorl %edi, %edi
.LBB0_3:
  movq %rdi, 32(%rsp)
  movb $4, %r10b
  movb $3, %r11b
  xorl %r15d, %r15d
  xorl %esi, %esi
.LBB0_4:
  movzbl %r15b, %r13d
  movl %r13d, %edx
  andl $7, %edx
  movq %rdx, 56(%rsp)
  leaq 4(%rdx), %rbp
  movzbl %r10b, %r10d
  andl $7, %r10d
  movl %esi, %ebx
  andl $7, %ebx
  imull $1664525, %ecx, %ecx
  addl $1013904223, %ecx
  movq %rsi, 24(%rsp)
  testb $1, %sil
  jne .LBB0_9
  movq %rbx, 40(%rsp)
  movq %r12, 48(%rsp)
  movl %r13d, %edx
  andl $3, %edx
  subq %rdx, %rbp
  xorl %r9d, %r9d
  movl $12, %esi
  leaq 4(%rsp), %r8
  xorl %r12d, %r12d
.LBB0_6:
  movq %r8, %r14
  movl %esi, %edi
  shrxl %r12d, %ecx, %esi
  imulq $1321528399, %rsi, %r8
  shrq $35, %r8
  leal (%r8,%r8,4), %ebx
  leal (%rbx,%rbx,4), %ebx
  addl %r8d, %ebx
  subl %ebx, %esi
  addb $97, %sil
  movb %sil, (%rsp,%r9)
  leal 3(%r12), %esi
  shrxl %esi, %ecx, %esi
  imulq $1321528399, %rsi, %r8
  shrq $35, %r8
  leal (%r8,%r8,4), %ebx
  leal (%rbx,%rbx,4), %ebx
  addl %r8d, %ebx
  subl %ebx, %esi
  addb $97, %sil
  movb %sil, 1(%rsp,%r9)
  leal 6(%r12), %esi
  shrxl %esi, %ecx, %esi
  imulq $1321528399, %rsi, %r8
  shrq $35, %r8
  leal (%r8,%r8,4), %ebx
  leal (%rbx,%rbx,4), %ebx
  addl %r8d, %ebx
  subl %ebx, %esi
  addb $97, %sil
  movb %sil, 2(%rsp,%r9)
  leal 9(%r12), %esi
  shrxl %esi, %ecx, %esi
  imulq $1321528399, %rsi, %r8
  shrq $35, %r8
  leal (%r8,%r8,4), %ebx
  leal (%rbx,%rbx,4), %ebx
  addl %r8d, %ebx
  subl %ebx, %esi
  addb $97, %sil
  movb %sil, 3(%rsp,%r9)
  addq $4, %r9
  addl $12, %r12d
  leaq 4(%r14), %r8
  leal 12(%rdi), %esi
  cmpq %r9, %rbp
  jne .LBB0_6
  testb $3, 24(%rsp)
  movq 48(%rsp), %r12
  movq 40(%rsp), %rbx
  je .LBB0_16
.LBB0_8:
  shrxl %edi, %ecx, %esi
  imulq $1321528399, %rsi, %r8
  shrq $35, %r8
  leal (%r8,%r8,4), %r9d
  leal (%r9,%r9,4), %r9d
  addl %r8d, %r9d
  subl %r9d, %esi
  addb $97, %sil
  movb %sil, (%r14)
  incq %r14
  addl $3, %edi
  decq %rdx
  jne .LBB0_8
  jmp .LBB0_16
.LBB0_9:
  imull $31337, %ecx, %edx
  movabsq $35186519711744, %rsi
  mulxq %rsi, %rsi, %rsi
  imull $524256, %esi, %esi
  subl %esi, %edx
  cmpl $4, %ebx
  jae .LBB0_11
  xorl %edi, %edi
  jmp .LBB0_14
.LBB0_11:
  andl $-8, %ebp
  leaq 4(%rbx), %r8
  leaq (%rdx,%rax), %r9
  addq $7, %r9
  xorl %edi, %edi
.LBB0_12:
  movzbl -7(%r9,%rdi), %esi
  movb %sil, (%rsp,%rdi)
  movzbl -6(%r9,%rdi), %esi
  movb %sil, 1(%rsp,%rdi)
  movzbl -5(%r9,%rdi), %esi
  movb %sil, 2(%rsp,%rdi)
  movzbl -4(%r9,%rdi), %esi
  movb %sil, 3(%rsp,%rdi)
  movzbl -3(%r9,%rdi), %esi
  movb %sil, 4(%rsp,%rdi)
  movzbl -2(%r9,%rdi), %esi
  movb %sil, 5(%rsp,%rdi)
  movzbl -1(%r9,%rdi), %esi
  movb %sil, 6(%rsp,%rdi)
  movzbl (%r9,%rdi), %esi
  movb %sil, 7(%rsp,%rdi)
  addq $8, %rdi
  cmpq %rdi, %rbp
  jne .LBB0_12
  testb $7, %r8b
  je .LBB0_16
.LBB0_14:
  leaq (%rsp,%rdi), %rsi
  addq %rdi, %rdx
  addq %rax, %rdx
  xorl %edi, %edi
.LBB0_15:
  movzbl (%rdx,%rdi), %r8d
  movb %r8b, (%rsi,%rdi)
  incq %rdi
  cmpq %rdi, %r10
  jne .LBB0_15
.LBB0_16:
  andb $7, %r15b
  movzbl %r11b, %r11d
  leal 4(%rbx), %edx
  vmovd %edx, %xmm0
  vpbroadcastb %xmm0, %ymm0
  vmovdqu %ymm0, 288(%rsp)
  vmovdqu %ymm0, 256(%rsp)
  vmovdqu %ymm0, 224(%rsp)
  vmovdqu %ymm0, 192(%rsp)
  vmovdqu %ymm0, 160(%rsp)
  vmovdqu %ymm0, 128(%rsp)
  vmovdqu %ymm0, 96(%rsp)
  andl $7, %r11d
  vmovdqu %ymm0, 64(%rsp)
  cmpl $5, %ebx
  jae .LBB0_23
  xorl %edi, %edi
  jmp .LBB0_19
.LBB0_23:
  leal -4(%r15), %r8d
  andl $7, %r13d
  addl $3, %r13d
  andl $-8, %r13d
  movq %rbx, %r14
  leaq 3(%rbx), %r9
  xorl %edi, %edi
.LBB0_24:
  leal 7(%r8), %esi
  movzbl (%rsp,%rdi), %ebx
  movb %sil, 64(%rsp,%rbx)
  leal 6(%r8), %esi
  movzbl 1(%rsp,%rdi), %ebx
  movb %sil, 64(%rsp,%rbx)
  leal 5(%r8), %esi
  movzbl 2(%rsp,%rdi), %ebx
  movb %sil, 64(%rsp,%rbx)
  leal 4(%r8), %esi
  movzbl 3(%rsp,%rdi), %ebx
  movb %sil, 64(%rsp,%rbx)
  leal 3(%r8), %esi
  movzbl 4(%rsp,%rdi), %ebx
  movb %sil, 64(%rsp,%rbx)
  leal 2(%r8), %esi
  movzbl 5(%rsp,%rdi), %ebx
  movb %sil, 64(%rsp,%rbx)
  leal 1(%r8), %esi
  movzbl 6(%rsp,%rdi), %ebx
  movb %sil, 64(%rsp,%rbx)
  movzbl 7(%rsp,%rdi), %esi
  movb %r8b, 64(%rsp,%rsi)
  addq $8, %rdi
  addb $-8, %r8b
  cmpq %rdi, %r13
  jne .LBB0_24
  testb $7, %r9b
  movq %r14, %rbx
  je .LBB0_21
.LBB0_19:
  leaq (%rsp,%rdi), %r8
  subb %dil, %r15b
  addb $3, %r15b
  xorl %esi, %esi
.LBB0_20:
  movzbl (%r8,%rsi), %edi
  movb %r15b, 64(%rsp,%rdi)
  incq %rsi
  decb %r15b
  cmpq %rsi, %r11
  jne .LBB0_20
.LBB0_21:
  movl $524284, %edi
  subl %ebx, %edi
  xorl %esi, %esi
  movq 56(%rsp), %r13
.LBB0_22:
  movl %esi, %r8d
  leaq 3(%r8), %r9
  movq %r13, %r15
.LBB0_26:
  movzbl 3(%rsp,%r15), %ebx
  leal (%r9,%r15), %r14d
  cmpb (%r14,%rax), %bl
  jne .LBB0_27
  decq %r15
  leal 4(%r15), %ebx
  testl %ebx, %ebx
  jg .LBB0_26
  jmp .LBB0_29
.LBB0_27:
  movl %edx, %r9d
  addq %r8, %r9
  movzbl -1(%rax,%r9), %r8d
  movzbl 64(%rsp,%r8), %r8d
  addl %r8d, %esi
  cmpl %edi, %esi
  jbe .LBB0_22
  movq 24(%rsp), %rsi
  imulq $104729, %rsi, %rdx
  jmp .LBB0_30
.LBB0_29:
  movq 24(%rsp), %rsi
  movq %rsi, %rdx
  shlq $24, %rdx
  xorq %r8, %rdx
.LBB0_30:
  addq %rdx, %r12
  incq %rsi
  incb %r13b
  incb %r10b
  incb %r11b
  movl %r13d, %r15d
  cmpq $2048, %rsi
  jne .LBB0_4
  movq 32(%rsp), %rdi
  leal 97(%rdi), %edx
  imulq $16381, %rdi, %rsi
  movb %dl, (%rsi,%rax)
  incq %rdi
  cmpq $8, %rdi
  jne .LBB0_3
  leaq .L.str(%rip), %rdi
  movq %r12, %rsi
  xorl %eax, %eax
  vzeroupper
  callq printf@PLT
  xorl %eax, %eax
  addq $328, %rsp
  popq %rbx
  popq %r12
  popq %r13
  popq %r14
  popq %r15
  popq %rbp
  retq

.L.str:

