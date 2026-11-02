.LCPI0_1:
main:
  pushq %rbp
  pushq %r15
  pushq %r14
  pushq %r13
  pushq %r12
  pushq %rbx
  pushq %rax
  xorl %r8d, %r8d
  leaq -16(%rsp), %rdx
  leaq main.buf(%rip), %rax
  movl $3435973837, %esi
  vmovdqa .LCPI0_1(%rip), %xmm0
  vbroadcasti128 .LCPI0_1(%rip), %ymm1
  xorl %edi, %edi
  jmp .LBB0_1
.LBB0_63:
  movslq %r8d, %r8
  movb $10, (%r8,%rax)
  incq %r8
  incl %edi
  cmpl $64, %edi
  je .LBB0_9
.LBB0_1:
  imull $131, %edi, %r9d
  xorl %r10d, %r10d
  jmp .LBB0_2
.LBB0_62:
  incl %r10d
  cmpl $6, %r10d
  je .LBB0_63
.LBB0_2:
  testl %r10d, %r10d
  je .LBB0_4
  movslq %r8d, %rcx
  incl %r8d
  movb $44, (%rcx,%rax)
.LBB0_4:
  movl %r10d, %ebx
  shll $4, %ebx
  addl %r10d, %ebx
  addl %r9d, %ebx
  imulq $274877907, %rbx, %rcx
  shrq $38, %rcx
  imull $1000, %ecx, %ecx
  subl %ecx, %ebx
  cmpl $499, %ebx
  ja .LBB0_47
  movslq %r8d, %rcx
  incl %r8d
  movb $45, (%rcx,%rax)
  movl $500, %ecx
  subl %ebx, %ecx
  movl %ecx, %ebx
  jmp .LBB0_6
.LBB0_47:
  addl $-500, %ebx
  je .LBB0_48
.LBB0_6:
  xorl %r11d, %r11d
.LBB0_7:
  movl %ebx, %ecx
  imulq %rsi, %rcx
  shrq $35, %rcx
  leal (%rcx,%rcx), %r14d
  leal (%r14,%r14,4), %ebp
  movl %ebx, %r14d
  subl %ebp, %r14d
  orb $48, %r14b
  movb %r14b, (%rsp,%r11)
  incq %r11
  cmpl $10, %ebx
  movl %ecx, %ebx
  jae .LBB0_7
  testl %r11d, %r11d
  jg .LBB0_49
  jmp .LBB0_62
.LBB0_48:
  movb $48, (%rsp)
  movl $1, %r11d
.LBB0_49:
  movslq %r8d, %r14
  cmpq $16, %r11
  jae .LBB0_51
  movq %r14, %r8
  movq %r11, %rbx
  jmp .LBB0_60
.LBB0_51:
  cmpq $128, %r11
  jae .LBB0_53
  xorl %r15d, %r15d
  jmp .LBB0_57
.LBB0_53:
  movq %r11, %r15
  andq $-128, %r15
  leaq (%r15,%r14), %r8
  movl %r11d, %ebx
  andl $127, %ebx
  leaq (%r14,%rax), %r12
  addq $96, %r12
  leaq -32(%rsp), %rcx
  leaq (%rcx,%r11), %r13
  movq %r15, %rbp
  negq %rbp
  xorl %ecx, %ecx
.LBB0_54:
  vmovdqu -96(%r13,%rcx), %ymm2
  vmovdqu -64(%r13,%rcx), %ymm3
  vmovdqu -32(%r13,%rcx), %ymm4
  vmovdqu (%r13,%rcx), %ymm5
  vpshufb %ymm1, %ymm5, %ymm5
  vpermq $78, %ymm5, %ymm5
  vpshufb %ymm1, %ymm4, %ymm4
  vpermq $78, %ymm4, %ymm4
  vpshufb %ymm1, %ymm3, %ymm3
  vpermq $78, %ymm3, %ymm3
  vpshufb %ymm1, %ymm2, %ymm2
  vpermq $78, %ymm2, %ymm2
  vmovdqu %ymm5, -96(%r12)
  vmovdqu %ymm4, -64(%r12)
  vmovdqu %ymm3, -32(%r12)
  vmovdqu %ymm2, (%r12)
  subq $-128, %r12
  addq $-128, %rcx
  cmpq %rcx, %rbp
  jne .LBB0_54
  cmpq %r15, %r11
  je .LBB0_62
  testb $112, %r11b
  je .LBB0_60
.LBB0_57:
  movq %r11, %r12
  andq $-16, %r12
  leaq (%r12,%r14), %r8
  movl %r11d, %ebx
  andl $15, %ebx
  addq %r15, %r14
  addq %rax, %r14
  leaq (%rdx,%r11), %rcx
  negq %r15
  movq %r12, %r13
  negq %r13
.LBB0_58:
  vmovdqu (%rcx,%r15), %xmm2
  vpshufb %xmm0, %xmm2, %xmm2
  vmovdqu %xmm2, (%r14)
  addq $16, %r14
  addq $-16, %r15
  cmpq %r15, %r13
  jne .LBB0_58
  cmpq %r12, %r11
  je .LBB0_62
.LBB0_60:
  incq %rbx
.LBB0_61:
  movzbl -2(%rsp,%rbx), %ecx
  movb %cl, (%r8,%rax)
  incq %r8
  decq %rbx
  cmpq $1, %rbx
  ja .LBB0_61
  jmp .LBB0_62
.LBB0_9:
  movb $0, (%r8,%rax)
  xorl %r9d, %r9d
  xorl %ecx, %ecx
  xorl %edx, %edx
  xorl %edi, %edi
  xorl %r8d, %r8d
  jmp .LBB0_10
.LBB0_11:
  leaq (%rdx,%rdx,4), %rdx
  andl $15, %r10d
  leaq (%r10,%rdx,2), %rdx
  movl $1, %r8d
  movq %r9, %rsi
.LBB0_23:
  incq %rax
  movq %rsi, %r9
.LBB0_10:
  movzbl (%rax), %r10d
  leal -48(%r10), %esi
  cmpb $9, %sil
  jbe .LBB0_11
  movq %rdx, %rsi
  negq %rsi
  testl %edi, %edi
  cmoveq %rdx, %rsi
  xorl %edx, %edx
  testl %ecx, %ecx
  cmovneq %rdx, %rsi
  testl %r8d, %r8d
  cmoveq %rdx, %rsi
  addq %r9, %rsi
  cmpl $43, %r10d
  jle .LBB0_13
  cmpl $44, %r10d
  je .LBB0_64
  movl $1, %edi
  cmpl $45, %r10d
  je .LBB0_22
  jmp .LBB0_21
.LBB0_13:
  cmpl $10, %r10d
  jne .LBB0_14
  xorl %ecx, %ecx
  jmp .LBB0_21
.LBB0_64:
  incl %ecx
  jmp .LBB0_21
.LBB0_14:
  testl %r10d, %r10d
  je .LBB0_15
.LBB0_21:
  xorl %edx, %edx
  xorl %edi, %edi
.LBB0_22:
  xorl %r8d, %r8d
  jmp .LBB0_23
.LBB0_15:
  leaq main.buf(%rip), %rax
  xorl %r10d, %r10d
  xorl %ecx, %ecx
  xorl %edi, %edi
  xorl %r8d, %r8d
  xorl %r9d, %r9d
  jmp .LBB0_16
.LBB0_17:
  leaq (%rdi,%rdi,4), %rdx
  andl $15, %r11d
  leaq (%r11,%rdx,2), %rdi
  movl $1, %r9d
  movq %r10, %rdx
.LBB0_35:
  incq %rax
  movq %rdx, %r10
.LBB0_16:
  movzbl (%rax), %r11d
  leal -48(%r11), %edx
  cmpb $9, %dl
  jbe .LBB0_17
  movq %rdi, %rdx
  negq %rdx
  testl %r8d, %r8d
  cmoveq %rdi, %rdx
  xorl %edi, %edi
  cmpl $3, %ecx
  cmovneq %rdi, %rdx
  testl %r9d, %r9d
  cmoveq %rdi, %rdx
  addq %r10, %rdx
  cmpl $43, %r11d
  jle .LBB0_25
  cmpl $44, %r11d
  je .LBB0_65
  movl $1, %r8d
  cmpl $45, %r11d
  je .LBB0_34
  jmp .LBB0_33
.LBB0_25:
  cmpl $10, %r11d
  jne .LBB0_26
  xorl %ecx, %ecx
  jmp .LBB0_33
.LBB0_65:
  incl %ecx
  jmp .LBB0_33
.LBB0_26:
  testl %r11d, %r11d
  je .LBB0_27
.LBB0_33:
  xorl %edi, %edi
  xorl %r8d, %r8d
.LBB0_34:
  xorl %r9d, %r9d
  jmp .LBB0_35
.LBB0_27:
  leaq main.buf(%rip), %rax
  xorl %r11d, %r11d
  xorl %edi, %edi
  xorl %r8d, %r8d
  xorl %r9d, %r9d
  xorl %r10d, %r10d
  jmp .LBB0_28
.LBB0_29:
  leaq (%r8,%r8,4), %rcx
  andl $15, %ebx
  leaq (%rbx,%rcx,2), %r8
  movl $1, %r10d
  movq %r11, %rcx
.LBB0_46:
  incq %rax
  movq %rcx, %r11
.LBB0_28:
  movzbl (%rax), %ebx
  leal -48(%rbx), %ecx
  cmpb $9, %cl
  jbe .LBB0_29
  movq %r8, %rcx
  negq %rcx
  testl %r9d, %r9d
  cmoveq %r8, %rcx
  xorl %r8d, %r8d
  cmpl $5, %edi
  cmovneq %r8, %rcx
  testl %r10d, %r10d
  cmoveq %r8, %rcx
  addq %r11, %rcx
  cmpl $43, %ebx
  jle .LBB0_37
  movl $1, %r9d
  cmpl $45, %ebx
  je .LBB0_45
  cmpl $44, %ebx
  jne .LBB0_44
  incl %edi
  jmp .LBB0_44
.LBB0_37:
  cmpl $10, %ebx
  jne .LBB0_38
  xorl %edi, %edi
  jmp .LBB0_44
.LBB0_38:
  testl %ebx, %ebx
  je .LBB0_39
.LBB0_44:
  xorl %r8d, %r8d
  xorl %r9d, %r9d
.LBB0_45:
  xorl %r10d, %r10d
  jmp .LBB0_46
.LBB0_39:
  leaq .L.str(%rip), %rdi
  xorl %eax, %eax
  vzeroupper
  callq printf@PLT
  xorl %eax, %eax
  addq $8, %rsp
  popq %rbx
  popq %r12
  popq %r13
  popq %r14
  popq %r15
  popq %rbp
  retq

.L.str:

