main:
  pushq %rbp
  pushq %r15
  pushq %r14
  pushq %r13
  pushq %r12
  pushq %rbx
  subq $24, %rsp
  vstmxcsr (%rsp)
  orl $32832, (%rsp)
  vldmxcsr (%rsp)
  leaq 8(%rsp), %rax
  vmovq %rax, %xmm0
  vpbroadcastq %xmm0, %ymm0
  xorl %eax, %eax
  movl $3435973837, %edx
  vmovdqu .LCPI0_0(%rip), %xmm1
  xorl %ecx, %ecx
  jmp .LBB0_1
.LBB0_10:
  movslq %ecx, %rcx
  movb $10, main.buf(%rcx)
  incq %rcx
  movq 16(%rsp), %rax
  incl %eax
  cmpl $64, %eax
  je .LBB0_11
.LBB0_1:
  movq %rax, 16(%rsp)
  imull $131, %eax, %esi
  xorl %edi, %edi
  movl %esi, 4(%rsp)
  jmp .LBB0_2
.LBB0_65:
  addl %r8d, %ecx
.LBB0_9:
  incl %edi
  cmpl $6, %edi
  je .LBB0_10
.LBB0_2:
  testl %edi, %edi
  je .LBB0_4
  movslq %ecx, %r8
  incl %ecx
  movb $44, main.buf(%r8)
.LBB0_4:
  movl %edi, %r9d
  shll $4, %r9d
  addl %edi, %r9d
  addl %esi, %r9d
  imulq $274877907, %r9, %r8
  shrq $38, %r8
  imull $1000, %r8d, %r8d
  subl %r8d, %r9d
  cmpl $499, %r9d
  ja .LBB0_56
  movslq %ecx, %r8
  incl %ecx
  movb $45, main.buf(%r8)
  movl $500, %r8d
  subl %r9d, %r8d
  movl %r8d, %r9d
  jmp .LBB0_6
.LBB0_56:
  addl $-500, %r9d
  je .LBB0_57
.LBB0_6:
  xorl %r8d, %r8d
.LBB0_7:
  movl %r9d, %r10d
  imulq %rdx, %r10
  shrq $35, %r10
  leal (%r10,%r10), %r11d
  leal (%r11,%r11,4), %r11d
  movl %r9d, %ebx
  subl %r11d, %ebx
  orb $48, %bl
  movb %bl, 8(%rsp,%r8)
  incq %r8
  cmpl $10, %r9d
  movl %r10d, %r9d
  jae .LBB0_7
  testl %r8d, %r8d
  jg .LBB0_58
  jmp .LBB0_9
.LBB0_57:
  movb $48, 8(%rsp)
  movl $1, %r8d
.LBB0_58:
  movl %r8d, %r11d
  movslq %ecx, %r10
  xorl %r9d, %r9d
  testl %r8d, %r8d
  setne %r9b
  movq %r11, %r14
  subq %r9, %r14
  incq %r14
  movq %r14, %rbx
  andq $-4, %rbx
  je .LBB0_59
  movl %r8d, %ebp
  xorl %r15d, %r15d
.LBB0_63:
  vmovd %ebp, %xmm2
  vpbroadcastd %xmm2, %xmm2
  vpaddd %xmm1, %xmm2, %xmm2
  vpmovzxdq %xmm2, %ymm2
  vpaddq %ymm2, %ymm0, %ymm2
  vmovq %xmm2, %r12
  vpextrq $1, %xmm2, %r13
  vextracti128 $1, %ymm2, %xmm2
  vmovq %xmm2, %rax
  vpextrq $1, %xmm2, %rsi
  movzbl (%r12), %r12d
  vmovd %r12d, %xmm2
  vpinsrb $1, (%r13), %xmm2, %xmm2
  vpinsrb $2, (%rax), %xmm2, %xmm2
  vpinsrb $3, (%rsi), %xmm2, %xmm2
  vmovd %xmm2, main.buf(%r10,%r15)
  addq $4, %r15
  addl $-4, %ebp
  cmpq %rbx, %r15
  jl .LBB0_63
  cmpq %rbx, %r14
  movl 4(%rsp), %esi
  je .LBB0_65
  jmp .LBB0_60
.LBB0_59:
  xorl %ebx, %ebx
.LBB0_60:
  negq %r9
  addq %r11, %r9
  subq %rbx, %r9
  incq %r9
  movl %ebx, %r11d
  notl %r11d
  addl %r8d, %r11d
  addq %rbx, %r10
  addq $main.buf, %r10
  xorl %ebx, %ebx
.LBB0_61:
  movl %r11d, %eax
  movzbl 8(%rsp,%rax), %eax
  movb %al, (%r10,%rbx)
  incq %rbx
  decl %r11d
  cmpq %rbx, %r9
  jne .LBB0_61
  jmp .LBB0_65
.LBB0_11:
  movl $4294967248, %eax
  movl $main.buf, %edx
  movb $0, main.buf(%rcx)
  xorl %ecx, %ecx
  xorl %esi, %esi
  xorl %edi, %edi
  jmp .LBB0_12
.LBB0_13:
  xorl %r9d, %r9d
.LBB0_14:
  xorl %r10d, %r10d
.LBB0_15:
  movzbl (%rdx), %r11d
  leal -48(%r11), %ebx
  cmpb $9, %bl
  ja .LBB0_17
  leaq (%r8,%r8,4), %r8
  addl %eax, %r11d
  leaq (%r11,%r8,2), %r8
  movl $1, %r10d
  incq %rdx
  jmp .LBB0_15
.LBB0_17:
  movq %r8, %rbx
  negq %rbx
  testl %r9d, %r9d
  cmoveq %r8, %rbx
  testl %edi, %edi
  cmovneq %rcx, %rbx
  testl %r10d, %r10d
  cmoveq %rcx, %rbx
  addq %rbx, %rsi
  cmpl $43, %r11d
  jle .LBB0_18
  cmpl $44, %r11d
  je .LBB0_24
  cmpl $45, %r11d
  jne .LBB0_25
  xorl %r8d, %r8d
  movl $1, %r9d
  incq %rdx
  jmp .LBB0_14
.LBB0_18:
  cmpl $10, %r11d
  je .LBB0_26
  testl %r11d, %r11d
  jne .LBB0_25
  jmp .LBB0_20
.LBB0_24:
  incl %edi
.LBB0_25:
  xorl %r8d, %r8d
  incq %rdx
  jmp .LBB0_13
.LBB0_26:
  xorl %edi, %edi
  incq %rdx
.LBB0_12:
  xorl %r8d, %r8d
  jmp .LBB0_13
.LBB0_20:
  movl $main.buf, %ecx
  xorl %edi, %edi
  xorl %edx, %edx
  xorl %r8d, %r8d
  jmp .LBB0_39
.LBB0_40:
  xorl %r10d, %r10d
  xorl %r11d, %r11d
  jmp .LBB0_27
.LBB0_28:
  leaq (%r9,%r9,4), %r9
  addl %eax, %ebx
  leaq (%rbx,%r9,2), %r9
  movl $1, %r11d
  incq %rcx
.LBB0_27:
  movzbl (%rcx), %ebx
  leal -48(%rbx), %ebp
  cmpb $9, %bpl
  jbe .LBB0_28
  movq %r9, %r14
  negq %r14
  testl %r10d, %r10d
  cmoveq %r9, %r14
  cmpl $3, %r8d
  cmovneq %rdi, %r14
  testl %r11d, %r11d
  cmoveq %rdi, %r14
  addq %r14, %rdx
  cmpl $43, %ebx
  jle .LBB0_30
  cmpl $44, %ebx
  je .LBB0_36
  cmpl $45, %ebx
  jne .LBB0_37
  xorl %r9d, %r9d
  movl $1, %r10d
  incq %rcx
  xorl %r11d, %r11d
  jmp .LBB0_27
.LBB0_30:
  cmpl $10, %ebx
  je .LBB0_38
  testl %ebx, %ebx
  jne .LBB0_37
  jmp .LBB0_32
.LBB0_36:
  incl %r8d
.LBB0_37:
  xorl %r9d, %r9d
  incq %rcx
  jmp .LBB0_40
.LBB0_38:
  xorl %r8d, %r8d
  incq %rcx
.LBB0_39:
  xorl %r9d, %r9d
  jmp .LBB0_40
.LBB0_32:
  movl $main.buf, %edi
  xorl %r8d, %r8d
  xorl %ecx, %ecx
  xorl %r9d, %r9d
  jmp .LBB0_54
.LBB0_42:
  xorl %ebx, %ebx
.LBB0_43:
  movzbl (%rdi), %r14d
  leal -48(%r14), %ebp
  cmpb $9, %bpl
  ja .LBB0_45
  leaq (%r10,%r10,4), %r10
  addl %eax, %r14d
  leaq (%r14,%r10,2), %r10
  movl $1, %ebx
  incq %rdi
  jmp .LBB0_43
.LBB0_45:
  movq %r10, %r15
  negq %r15
  testl %r11d, %r11d
  cmoveq %r10, %r15
  cmpl $5, %r9d
  cmovneq %r8, %r15
  testl %ebx, %ebx
  cmoveq %r8, %r15
  addq %r15, %rcx
  cmpl $43, %r14d
  jle .LBB0_46
  cmpl $45, %r14d
  je .LBB0_41
  cmpl $44, %r14d
  jne .LBB0_52
  incl %r9d
  jmp .LBB0_52
.LBB0_46:
  cmpl $10, %r14d
  je .LBB0_53
  testl %r14d, %r14d
  je .LBB0_48
.LBB0_52:
  xorl %r10d, %r10d
  incq %rdi
  xorl %r11d, %r11d
  jmp .LBB0_42
.LBB0_41:
  xorl %r10d, %r10d
  movl $1, %r11d
  incq %rdi
  jmp .LBB0_42
.LBB0_53:
  xorl %r9d, %r9d
  incq %rdi
.LBB0_54:
  xorl %r10d, %r10d
  xorl %r11d, %r11d
  jmp .LBB0_42
.LBB0_48:
  movl $.L.str, %edi
  xorl %eax, %eax
  vzeroupper
  callq printf
  xorl %eax, %eax
  addq $24, %rsp
  popq %rbx
  popq %r12
  popq %r13
  popq %r14
  popq %r15
  popq %rbp
  retq

.L.str:
  .asciz "%ld %ld %ld\n"
