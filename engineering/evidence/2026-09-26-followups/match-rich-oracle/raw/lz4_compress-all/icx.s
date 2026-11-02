main:
  pushq %rbp
  pushq %r15
  pushq %r14
  pushq %r13
  pushq %r12
  pushq %rbx
  subq $56, %rsp
  vstmxcsr 12(%rsp)
  orl $32832, 12(%rsp)
  vldmxcsr 12(%rsp)
  movl $1511734397, %esi
  movl $4294967168, %eax
  xorl %ecx, %ecx
  jmp .LBB0_1
.LBB0_10:
  movl %esi, %edx
  shrl $24, %edx
.LBB0_14:
  movb %dl, src_data+1(%rcx)
  incq %rdi
  movq %rdi, %rcx
  cmpq $524288, %rdi
  je .LBB0_15
.LBB0_1:
  movl %esi, %edx
  imull $1664525, %esi, %esi
  addl $1013904223, %esi
  cmpq $128, %rcx
  jb .LBB0_6
  movl %ecx, %edi
  andl $224, %edi
  cmpl $95, %edi
  ja .LBB0_4
  leal (%rcx,%rax), %edi
  andl $-2, %edi
  movzbl src_data(%rdi), %edi
  jmp .LBB0_7
.LBB0_4:
  movl %esi, %edi
  andl $14, %edi
  cmpl $5, %edi
  ja .LBB0_6
  leal (%rdx,%rdx,2), %edi
  leal (%rdx,%rdi,4), %edi
  addb $31, %dil
  movzbl %dil, %edi
  andl $63, %edi
  addl %ecx, %edi
  addl $-128, %edi
  movzbl src_data(%rdi), %edi
  jmp .LBB0_7
.LBB0_6:
  movl %esi, %edi
  shrl $24, %edi
.LBB0_7:
  movb %dil, src_data(%rcx)
  imull $1664525, %esi, %esi
  addl $1013904223, %esi
  leaq 1(%rcx), %rdi
  cmpq $128, %rdi
  jb .LBB0_10
  movl %ecx, %r8d
  andl $224, %r8d
  cmpl $96, %r8d
  jae .LBB0_9
  leal (%rax,%rcx), %edx
  incl %edx
  jmp .LBB0_13
.LBB0_9:
  movl %esi, %r8d
  andl $14, %r8d
  cmpl $6, %r8d
  jae .LBB0_10
  leal (%rdx,%rdx,2), %r8d
  shll $3, %r8d
  subl %edx, %r8d
  movb $50, %dl
  subb %r8b, %dl
  movzbl %dl, %edx
  andl $63, %edx
  addl %ecx, %edx
  addl $-127, %edx
.LBB0_13:
  movzbl src_data(%rdx), %edx
  jmp .LBB0_14
.LBB0_15:
  xorl %edx, %edx
  xorl %esi, %esi
  jmp .LBB0_16
.LBB0_40:
  movq 24(%rsp), %rdx
  movq 16(%rsp), %rsi
.LBB0_41:
  movl $dst_data, %eax
  subl %eax, %r15d
  movq %r15, %rax
  shlq $32, %rax
  movzbl dst_data(%rip), %ecx
  addq %rcx, %rsi
  movq %rsi, %rcx
  addq %rax, %rcx
  shrl %r15d
  movzbl dst_data(%r15), %esi
  imull $4099, %edx, %eax
  andl $524287, %eax
  xorb $85, src_data(%rax)
  shlq $16, %rsi
  addq %rcx, %rsi
  incl %edx
  cmpl $2048, %edx
  je .LBB0_42
.LBB0_16:
  movq %rsi, 16(%rsp)
  movq %rdx, 24(%rsp)
  movl $hash_table, %edi
  movl $65536, %edx
  xorl %esi, %esi
  callq _intel_fast_memset@PLT
  movl $src_data, %esi
  movl $src_data+1, %r14d
  movl $src_data, %ebp
  movl $dst_data, %r8d
  jmp .LBB0_17
.LBB0_37:
  movq %r14, %rax
  subq %rbp, %rax
  sarq $6, %rax
  leaq (%rax,%r14), %rbx
  incq %rbx
.LBB0_38:
  movq %rbx, %r14
  cmpq $src_data+524276, %rbx
  jae .LBB0_39
.LBB0_17:
  movl (%r14), %eax
  imull $-1640531535, %eax, %ecx
  shrl $18, %ecx
  movl hash_table(,%rcx,4), %edi
  leaq src_data(%rdi), %r9
  movl %r14d, %edx
  subl %esi, %edx
  movl %edx, hash_table(,%rcx,4)
  cmpq %r14, %r9
  jae .LBB0_37
  cmpl %eax, (%r9)
  jne .LBB0_37
  movl %r14d, %r12d
  leaq 4(%r9), %r13
  leaq 4(%r14), %rbx
  cmpq $src_data+524288, %rbx
  movq %rdi, 48(%rsp)
  jae .LBB0_24
  movl $src_data+524288, %eax
  movq %rdi, %rcx
  subq %r14, %rcx
  addq %rcx, %rax
  addq $src_data, %rax
  movl $src_data+524284, %ecx
  subq %r14, %rcx
.LBB0_21:
  movzbl (%rbx), %edx
  cmpb (%r13), %dl
  jne .LBB0_24
  incq %rbx
  incq %r13
  decq %rcx
  jne .LBB0_21
  movq %rax, %r13
  movl $src_data+524288, %ebx
.LBB0_24:
  subq %rbp, %r14
  leaq 1(%r8), %r15
  cmpl $15, %r14d
  jb .LBB0_28
  movb $-16, (%r8)
  leal -15(%r14), %eax
  cmpl $255, %eax
  jb .LBB0_27
  subl %ebp, %r12d
  addl $-270, %r12d
  movq %r12, %rax
  movl $2155905153, %ecx
  imulq %rcx, %rax
  shrq $39, %rax
  movq %rax, 32(%rsp)
  leaq 1(%rax), %rdx
  movq %r15, %rdi
  movl $255, %esi
  movq %r8, 40(%rsp)
  movq %r9, %r15
  callq _intel_fast_memset@PLT
  movq %r15, %r9
  movl $src_data, %esi
  movq 40(%rsp), %r8
  movl %r12d, %eax
  movl $2155905153, %ecx
  imulq %rcx, %rax
  shrq $39, %rax
  movl %eax, %ecx
  shll $8, %ecx
  subl %ecx, %eax
  addl %r12d, %eax
  movq 32(%rsp), %rcx
  leaq (%r8,%rcx), %r15
  addq $2, %r15
.LBB0_27:
  movb %al, (%r15)
  incq %r15
  jmp .LBB0_29
.LBB0_28:
  movl %r14d, %eax
  shlb $4, %al
  movb %al, (%r8)
  testl %r14d, %r14d
  je .LBB0_31
.LBB0_29:
  movl %r14d, %r12d
  cmpq $12, %r12
  jbe .LBB0_43
  movq %r15, %rdi
  movq %rbp, %rsi
  movq %r12, %rdx
  movq %r8, %r14
  movq %r9, %rbp
  callq _intel_fast_memcpy@PLT
  movq %rbp, %r9
  movl $src_data, %esi
  movq %r14, %r8
  addq %r12, %r15
  jmp .LBB0_31
.LBB0_43:
  movl $4294967292, %eax
  andq %rax, %r14
  je .LBB0_44
  xorl %eax, %eax
.LBB0_47:
  movl (%rbp,%rax), %ecx
  movl %ecx, (%r15,%rax)
  addq $4, %rax
  cmpq %r14, %rax
  jb .LBB0_47
  jmp .LBB0_48
.LBB0_44:
  xorl %r14d, %r14d
  jmp .LBB0_45
.LBB0_31:
  movl %r13d, %eax
  subl %r9d, %eax
  leal -4(%rax), %ecx
  movl %ebx, %edx
  subl %r13d, %edx
  leaq 2(%r15), %rdi
  movw %dx, (%r15)
  movzbl (%r8), %edx
  cmpl $15, %ecx
  jb .LBB0_36
  orb $15, %dl
  movb %dl, (%r8)
  addl $-19, %eax
  cmpl $255, %eax
  jb .LBB0_34
  movq 48(%rsp), %rax
  addl %esi, %eax
  subl %eax, %r13d
  addl $-274, %r13d
  movq %r13, %r14
  movl $2155905153, %r12d
  imulq %r12, %r14
  shrq $39, %r14
  leaq 1(%r14), %rdx
  movl $255, %esi
  callq _intel_fast_memset@PLT
  movl $src_data, %esi
  movl %r13d, %eax
  imulq %r12, %rax
  shrq $39, %rax
  movl %eax, %ecx
  shll $8, %ecx
  subl %ecx, %eax
  addl %r13d, %eax
  leaq (%r15,%r14), %rdi
  addq $3, %rdi
.LBB0_34:
  movb %al, (%rdi)
  incq %rdi
  jmp .LBB0_35
.LBB0_36:
  orb %cl, %dl
  movb %dl, (%r8)
.LBB0_35:
  movq %rdi, %r8
  movq %rbx, %rbp
  jmp .LBB0_38
.LBB0_45:
  movzbl (%rbp,%r14), %eax
  movb %al, (%r15,%r14)
  incq %r14
.LBB0_48:
  cmpq %r14, %r12
  jne .LBB0_45
  addq %r12, %r15
  jmp .LBB0_31
.LBB0_39:
  movl $src_data+524288, %eax
  subl %ebp, %eax
  movl %eax, %ecx
  shlb $4, %cl
  cmpl $15, %eax
  movzbl %cl, %eax
  movl $240, %ecx
  cmovael %ecx, %eax
  leaq 1(%r8), %r15
  movl $src_data+524288, %r12d
  movb %al, (%r8)
  subl %ebp, %r12d
  je .LBB0_40
  cmpl $1, %r12d
  adcl $0, %r12d
  cmpl $12, %r12d
  jbe .LBB0_52
  movq %r15, %rdi
  movq %rbp, %rsi
  movq %r12, %rdx
  callq _intel_fast_memcpy@PLT
  movq 24(%rsp), %rdx
  movq 16(%rsp), %rsi
  addq %r12, %r15
  jmp .LBB0_41
.LBB0_52:
  movq %r12, %rax
  movl $4294967292, %ecx
  andq %rcx, %rax
  je .LBB0_53
  xorl %ecx, %ecx
.LBB0_55:
  movl (%rbp,%rcx), %edx
  movl %edx, 1(%r8,%rcx)
  addq $4, %rcx
  cmpq %rax, %rcx
  jb .LBB0_55
  cmpq %r12, %rax
  movq 24(%rsp), %rdx
  movq 16(%rsp), %rsi
  jne .LBB0_57
  jmp .LBB0_58
.LBB0_53:
  xorl %eax, %eax
  movq 24(%rsp), %rdx
  movq 16(%rsp), %rsi
.LBB0_57:
  movzbl (%rbp,%rax), %ecx
  movb %cl, 1(%r8,%rax)
  leaq 1(%rax), %rcx
  movq %rcx, %rax
  cmpq %rcx, %r12
  jne .LBB0_57
.LBB0_58:
  addq %r12, %r15
  jmp .LBB0_41
.LBB0_42:
  movl $.L.str, %edi
  xorl %eax, %eax
  callq printf
  xorl %eax, %eax
  addq $56, %rsp
  popq %rbx
  popq %r12
  popq %r13
  popq %r14
  popq %r15
  popq %rbp
  retq

.L.str:
