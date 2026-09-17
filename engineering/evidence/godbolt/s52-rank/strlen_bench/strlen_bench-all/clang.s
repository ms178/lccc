main:
  pushq %rbp
  pushq %r15
  pushq %r14
  pushq %r13
  pushq %r12
  pushq %rbx
  subq $232, %rsp
  movl $42, %edx
  leaq strings+3(%rip), %rax
  leaq strings+4(%rip), %rcx
  xorl %esi, %esi
  movabsq $102481911535894528, %rdi
  leaq strings(%rip), %r14
  jmp .LBB0_1
.LBB0_5:
  movb $0, (%r9,%r8)
  incq %rsi
  addq $200, %rax
  addq $200, %rcx
  cmpq $100000, %rsi
  je .LBB0_6
.LBB0_1:
  imull $1664525, %edx, %edx
  addl $1013904223, %edx
  mulxq %rdi, %r8, %r8
  imull $180, %r8d, %r9d
  movl %edx, %r8d
  subl %r9d, %r8d
  addl $10, %r8d
  imulq $200, %rsi, %r9
  addq %r14, %r9
  movl %r8d, %r10d
  andl $3, %r10d
  movl %r8d, %ebx
  andl $508, %ebx
  movq %rcx, %r12
  xorl %r15d, %r15d
.LBB0_2:
  movq %r12, %r11
  imull $1664525, %edx, %r12d
  addl $1013904223, %r12d
  imulq $1321528399, %r12, %r13
  shrq $35, %r13
  leal (%r13,%r13,4), %ebp
  leal (%rbp,%rbp,4), %ebp
  addl %r13d, %ebp
  subl %ebp, %r12d
  addb $97, %r12b
  movb %r12b, -3(%rax,%r15)
  imull $389569705, %edx, %r12d
  addl $1196435762, %r12d
  imulq $1321528399, %r12, %r13
  shrq $35, %r13
  leal (%r13,%r13,4), %ebp
  leal (%rbp,%rbp,4), %ebp
  addl %r13d, %ebp
  subl %ebp, %r12d
  addb $97, %r12b
  movb %r12b, -2(%rax,%r15)
  imull $-1354167659, %edx, %r12d
  addl $-775096599, %r12d
  imulq $1321528399, %r12, %r13
  shrq $35, %r13
  leal (%r13,%r13,4), %ebp
  leal (%rbp,%rbp,4), %ebp
  addl %r13d, %ebp
  subl %ebp, %r12d
  addb $97, %r12b
  movb %r12b, -1(%rax,%r15)
  imull $158984081, %edx, %edx
  addl $-1426500812, %edx
  imulq $1321528399, %rdx, %r12
  shrq $35, %r12
  leal (%r12,%r12,4), %r13d
  leal (%r13,%r13,4), %ebp
  addl %r12d, %ebp
  movl %edx, %r12d
  subl %ebp, %r12d
  addb $97, %r12b
  movb %r12b, (%rax,%r15)
  addq $4, %r15
  leaq 4(%r11), %r12
  cmpq %r15, %rbx
  jne .LBB0_2
  testq %r10, %r10
  je .LBB0_5
.LBB0_4:
  imull $1664525, %edx, %edx
  addl $1013904223, %edx
  imulq $1321528399, %rdx, %rbx
  shrq $35, %rbx
  leal (%rbx,%rbx,4), %r15d
  leal (%r15,%r15,4), %ebp
  addl %ebx, %ebp
  movl %edx, %ebx
  subl %ebp, %ebx
  addb $97, %bl
  movb %bl, (%r11)
  incq %r11
  decq %r10
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
  testl %eax, %eax
  sets %al
  setg %cl
  subb %al, %cl
  movsbq %cl, %rax
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
  movzbl 200(%rsi,%rbp), %r8d
  testb %r8b, %r8b
  jne .LBB0_24
  jmp .LBB0_32
.LBB0_20:
  movl $1, %edx
  movzbl 200(%rsi,%rbp), %r8d
  testb %r8b, %r8b
  je .LBB0_32
.LBB0_24:
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

