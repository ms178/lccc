main:
  pushq %rbp
  pushq %r15
  pushq %r14
  pushq %r13
  pushq %r12
  pushq %rbx
  subq $232, %rsp
  movl $42, %eax
  leaq strings+3(%rip), %rcx
  leaq strings+4(%rip), %rdx
  xorl %esi, %esi
  leaq strings(%rip), %r14
  jmp .LBB0_1
.LBB0_5:
  movb $0, (%r8,%rdi)
  incq %rsi
  addq $200, %rcx
  addq $200, %rdx
  cmpq $100000, %rsi
  je .LBB0_6
.LBB0_1:
  imull $1664525, %eax, %eax
  addl $1013904223, %eax
  movl %eax, %edi
  shrl $2, %edi
  imulq $381774871, %rdi, %rdi
  shrq $34, %rdi
  imull $180, %edi, %r8d
  movl %eax, %edi
  subl %r8d, %edi
  addl $10, %edi
  imulq $200, %rsi, %r8
  addq %r14, %r8
  movl %edi, %r9d
  andl $3, %r9d
  movl %edi, %r11d
  andl $508, %r11d
  movq %rdx, %r15
  xorl %ebx, %ebx
.LBB0_2:
  movq %r15, %r10
  imull $1664525, %eax, %r15d
  addl $1013904223, %r15d
  imulq $1321528399, %r15, %r12
  shrq $35, %r12
  leal (%r12,%r12,4), %r13d
  leal (%r13,%r13,4), %ebp
  addl %r12d, %ebp
  subl %ebp, %r15d
  addb $97, %r15b
  movb %r15b, -3(%rcx,%rbx)
  imull $389569705, %eax, %r15d
  addl $1196435762, %r15d
  imulq $1321528399, %r15, %r12
  shrq $35, %r12
  leal (%r12,%r12,4), %r13d
  leal (%r13,%r13,4), %ebp
  addl %r12d, %ebp
  subl %ebp, %r15d
  addb $97, %r15b
  movb %r15b, -2(%rcx,%rbx)
  imull $-1354167659, %eax, %r15d
  addl $-775096599, %r15d
  imulq $1321528399, %r15, %r12
  shrq $35, %r12
  leal (%r12,%r12,4), %r13d
  leal (%r13,%r13,4), %ebp
  addl %r12d, %ebp
  subl %ebp, %r15d
  addb $97, %r15b
  movb %r15b, -1(%rcx,%rbx)
  imull $158984081, %eax, %eax
  addl $-1426500812, %eax
  imulq $1321528399, %rax, %r15
  shrq $35, %r15
  leal (%r15,%r15,4), %r12d
  leal (%r12,%r12,4), %ebp
  addl %r15d, %ebp
  movl %eax, %r15d
  subl %ebp, %r15d
  addb $97, %r15b
  movb %r15b, (%rcx,%rbx)
  addq $4, %rbx
  leaq 4(%r10), %r15
  cmpq %rbx, %r11
  jne .LBB0_2
  testq %r9, %r9
  je .LBB0_5
.LBB0_4:
  imull $1664525, %eax, %eax
  addl $1013904223, %eax
  imulq $1321528399, %rax, %r11
  shrq $35, %r11
  leal (%r11,%r11,4), %ebx
  leal (%rbx,%rbx,4), %ebx
  addl %r11d, %ebx
  movl %eax, %r11d
  subl %ebx, %r11d
  addb $97, %r11b
  movb %r11b, (%r10)
  incq %r10
  decq %r9
  jne .LBB0_4
  jmp .LBB0_5
.LBB0_6:
  xorl %ebx, %ebx
  xorl %ebp, %ebp
.LBB0_7:
  xorl %r15d, %r15d
.LBB0_8:
  leaq (%r14,%r15), %rdi
  callq strlen@PLT
  addq %rax, %rbx
  addq $200, %r15
  cmpq $20000000, %r15
  jne .LBB0_8
  incl %ebp
  cmpl $50, %ebp
  jne .LBB0_7
  movl $99999, %r15d
  xorl %r13d, %r13d
.LBB0_11:
  leaq 200(%r14), %r12
  movq %r14, %rdi
  movq %r12, %rsi
  callq strcmp@PLT
  cltq
  addq %rax, %r13
  movq %r12, %r14
  decq %r15
  jne .LBB0_11
  movq %r13, 24(%rsp)
  movl $6513249, 8(%rsp)
  leaq strings+201(%rip), %rax
  xorl %r11d, %r11d
  leaq strings(%rip), %rbp
  xorl %ecx, %ecx
  jmp .LBB0_13
.LBB0_32:
  xorl %esi, %esi
.LBB0_33:
  addq %rdx, %r11
  addq %rsi, %r11
  addq $2, %rcx
  addq $400, %rax
  cmpq $100000, %rcx
  je .LBB0_34
.LBB0_13:
  imulq $200, %rcx, %rsi
  movzbl (%rsi,%rbp), %edi
  testb %dil, %dil
  je .LBB0_22
  leaq (%rsi,%rbp), %rdx
.LBB0_15:
  xorl %r8d, %r8d
.LBB0_16:
  movzbl 8(%rsp,%r8), %r9d
  cmpb %r9b, %dil
  jne .LBB0_19
  movzbl 1(%rdx,%r8), %edi
  incq %r8
  testb %dil, %dil
  jne .LBB0_16
  movzbl 8(%rsp,%r8), %r9d
.LBB0_19:
  testb %r9b, %r9b
  je .LBB0_20
  movzbl 1(%rdx), %edi
  incq %rdx
  testb %dil, %dil
  jne .LBB0_15
.LBB0_22:
  xorl %edx, %edx
  jmp .LBB0_23
.LBB0_20:
  movl $1, %edx
.LBB0_23:
  movzbl 200(%rsi,%rbp), %r8d
  testb %r8b, %r8b
  je .LBB0_32
  addq %rbp, %rsi
  addq $200, %rsi
  movq %rax, %rdi
.LBB0_25:
  xorl %r9d, %r9d
.LBB0_26:
  movzbl 8(%rsp,%r9), %r10d
  cmpb %r10b, %r8b
  jne .LBB0_29
  movzbl (%rdi,%r9), %r8d
  incq %r9
  testb %r8b, %r8b
  jne .LBB0_26
  movzbl 8(%rsp,%r9), %r10d
.LBB0_29:
  testb %r10b, %r10b
  je .LBB0_30
  movzbl 1(%rsi), %r8d
  incq %rsi
  incq %rdi
  testb %r8b, %r8b
  jne .LBB0_25
  jmp .LBB0_32
.LBB0_30:
  movl $1, %esi
  jmp .LBB0_33
.LBB0_34:
  movq %r11, 16(%rsp)
  xorl %r12d, %r12d
  leaq 32(%rsp), %r13
  xorl %eax, %eax
.LBB0_35:
  movl %eax, 12(%rsp)
  xorl %r15d, %r15d
.LBB0_36:
  leaq (%r15,%rbp), %r14
  movq %r14, %rdi
  callq strlen@PLT
  incl %eax
  movslq %eax, %rdx
  movq %r13, %rdi
  movq %r14, %rsi
  callq memcpy@PLT
  movsbq 32(%rsp), %rax
  addq %rax, %r12
  addq $200, %r15
  cmpq $20000000, %r15
  jne .LBB0_36
  movl 12(%rsp), %eax
  incl %eax
  cmpl $50, %eax
  jne .LBB0_35
  leaq .L.str(%rip), %rdi
  movq %rbx, %rsi
  movq 24(%rsp), %rdx
  movq 16(%rsp), %rcx
  movq %r12, %r8
  xorl %eax, %eax
  callq printf@PLT
  xorl %eax, %eax
  addq $232, %rsp
  popq %rbx
  popq %r12
  popq %r13
  popq %r14
  popq %r15
  popq %rbp
  retq

.L.str:
  .asciz "strlen total: %ld, cmp_sum: %ld, found: %ld, copy_sum: %ld\n"

