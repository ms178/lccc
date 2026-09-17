.LCPI0_0:
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
  movl $3435973837, %ecx
  vmovdqu .LCPI0_0(%rip), %xmm1
  xorl %edx, %edx
  jmp .LBB0_1
.LBB0_10:
  movslq %edx, %rdx
  movb $10, main.buf(%rdx)
  incq %rdx
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
  addl %r8d, %edx
.LBB0_9:
  incl %edi
  cmpl $6, %edi
  je .LBB0_10
.LBB0_2:
  testl %edi, %edi
  je .LBB0_4
  movslq %edx, %r8
  incl %edx
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
  movslq %edx, %r8
  incl %edx
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
  imulq %rcx, %r10
  shrq $35, %r10
  leal (%r10,%r10), %r11d
  leal (%r11,%r11,4), %r11d
  movl %r9d, %ebx
  subl %r11d, %ebx
  addb $48, %bl
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
  movslq %edx, %r10
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
  movl $main.buf, %eax
  movb $0, main.buf(%rdx)
  xorl %ecx, %ecx
  xorl %esi, %esi
  xorl %edx, %edx
  jmp .LBB0_12
.LBB0_13:
  xorl %r8d, %r8d
.LBB0_14:
  xorl %r9d, %r9d
  movzbl (%rax), %r10d
  leal -48(%r10), %r11d
  cmpb $9, %r11b
  ja .LBB0_17
.LBB0_16:
  leaq (%rdi,%rdi,4), %rdi
  addl $-48, %r10d
  leaq (%r10,%rdi,2), %rdi
  movl $1, %r9d
  incq %rax
  movzbl (%rax), %r10d
  leal -48(%r10), %r11d
  cmpb $9, %r11b
  jbe .LBB0_16
.LBB0_17:
  movq %rdi, %r11
  negq %r11
  testl %r8d, %r8d
  cmoveq %rdi, %r11
  testl %edx, %edx
  cmovneq %rcx, %r11
  testl %r9d, %r9d
  cmoveq %rcx, %r11
  addq %r11, %rsi
  cmpl $43, %r10d
  jle .LBB0_18
  cmpl $44, %r10d
  je .LBB0_24
  cmpl $45, %r10d
  jne .LBB0_25
  xorl %edi, %edi
  movl $1, %r8d
  incq %rax
  jmp .LBB0_14
.LBB0_18:
  cmpl $10, %r10d
  je .LBB0_26
  testl %r10d, %r10d
  jne .LBB0_25
  jmp .LBB0_20
.LBB0_24:
  incl %edx
.LBB0_25:
  xorl %edi, %edi
  incq %rax
  jmp .LBB0_13
.LBB0_26:
  xorl %edx, %edx
  incq %rax
.LBB0_12:
  xorl %edi, %edi
  jmp .LBB0_13
.LBB0_20:
  movl $main.buf, %eax
  xorl %ecx, %ecx
  xorl %edx, %edx
  xorl %edi, %edi
  jmp .LBB0_39
.LBB0_40:
  xorl %r9d, %r9d
  xorl %r10d, %r10d
  movzbl (%rax), %r11d
  leal -48(%r11), %ebx
  cmpb $9, %bl
  ja .LBB0_29
.LBB0_28:
  leaq (%r8,%r8,4), %r8
  addl $-48, %r11d
  leaq (%r11,%r8,2), %r8
  movl $1, %r10d
  incq %rax
  movzbl (%rax), %r11d
  leal -48(%r11), %ebx
  cmpb $9, %bl
  jbe .LBB0_28
.LBB0_29:
  movq %r8, %rbx
  negq %rbx
  testl %r9d, %r9d
  cmoveq %r8, %rbx
  cmpl $3, %edi
  cmovneq %rcx, %rbx
  testl %r10d, %r10d
  cmoveq %rcx, %rbx
  addq %rbx, %rdx
  cmpl $43, %r11d
  jle .LBB0_30
  cmpl $44, %r11d
  je .LBB0_36
  cmpl $45, %r11d
  jne .LBB0_37
  xorl %r8d, %r8d
  movl $1, %r9d
  incq %rax
  xorl %r10d, %r10d
  movzbl (%rax), %r11d
  leal -48(%r11), %ebx
  cmpb $9, %bl
  ja .LBB0_29
  jmp .LBB0_28
.LBB0_30:
  cmpl $10, %r11d
  je .LBB0_38
  testl %r11d, %r11d
  jne .LBB0_37
  jmp .LBB0_32
.LBB0_36:
  incl %edi
.LBB0_37:
  xorl %r8d, %r8d
  incq %rax
  jmp .LBB0_40
.LBB0_38:
  xorl %edi, %edi
  incq %rax
.LBB0_39:
  xorl %r8d, %r8d
  jmp .LBB0_40
.LBB0_32:
  movl $main.buf, %eax
  xorl %edi, %edi
  xorl %ecx, %ecx
  xorl %r8d, %r8d
  jmp .LBB0_54
.LBB0_42:
  xorl %r11d, %r11d
  movzbl (%rax), %ebx
  leal -48(%rbx), %ebp
  cmpb $9, %bpl
  ja .LBB0_45
.LBB0_44:
  leaq (%r9,%r9,4), %r9
  addl $-48, %ebx
  leaq (%rbx,%r9,2), %r9
  movl $1, %r11d
  incq %rax
  movzbl (%rax), %ebx
  leal -48(%rbx), %ebp
  cmpb $9, %bpl
  jbe .LBB0_44
.LBB0_45:
  movq %r9, %r14
  negq %r14
  testl %r10d, %r10d
  cmoveq %r9, %r14
  cmpl $5, %r8d
  cmovneq %rdi, %r14
  testl %r11d, %r11d
  cmoveq %rdi, %r14
  addq %r14, %rcx
  cmpl $43, %ebx
  jle .LBB0_46
  cmpl $45, %ebx
  je .LBB0_41
  cmpl $44, %ebx
  jne .LBB0_52
  incl %r8d
  jmp .LBB0_52
.LBB0_46:
  cmpl $10, %ebx
  je .LBB0_53
  testl %ebx, %ebx
  je .LBB0_48
.LBB0_52:
  xorl %r9d, %r9d
  incq %rax
  xorl %r10d, %r10d
  jmp .LBB0_42
.LBB0_41:
  xorl %r9d, %r9d
  movl $1, %r10d
  incq %rax
  jmp .LBB0_42
.LBB0_53:
  xorl %r8d, %r8d
  incq %rax
.LBB0_54:
  xorl %r9d, %r9d
  xorl %r10d, %r10d
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

