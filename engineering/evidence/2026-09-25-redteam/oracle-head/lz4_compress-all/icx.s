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
  movl $1511734397, %ecx
  movl $3, %eax
  jmp .LBB0_1
.LBB0_43:
  movl %edx, %ecx
  shrl $24, %ecx
.LBB0_44:
  movb %cl, src_data(%rax)
  addq $4, %rax
  movl %edx, %ecx
  cmpq $524291, %rax
  je .LBB0_45
.LBB0_1:
  imull $1664525, %ecx, %edx
  addl $1013904223, %edx
  movl %edx, %esi
  andl $14, %esi
  cmpl $5, %esi
  ja .LBB0_31
  leaq -3(%rax), %rsi
  cmpq $128, %rsi
  jb .LBB0_31
  leal (%rcx,%rcx,2), %esi
  leal (%rcx,%rsi,4), %esi
  addb $31, %sil
  movzbl %sil, %esi
  andl $63, %esi
  addl %eax, %esi
  addl $-131, %esi
  movzbl src_data(%rsi), %esi
  jmp .LBB0_32
.LBB0_31:
  movl %edx, %esi
  shrl $24, %esi
.LBB0_32:
  movb %sil, src_data-3(%rax)
  imull $1664525, %edx, %edx
  addl $1013904223, %edx
  movl %edx, %esi
  andl $14, %esi
  cmpl $5, %esi
  ja .LBB0_35
  leaq -2(%rax), %rsi
  cmpq $128, %rsi
  jb .LBB0_35
  leal (%rcx,%rcx,2), %esi
  shll $3, %esi
  subl %ecx, %esi
  movb $50, %dil
  subb %sil, %dil
  movzbl %dil, %esi
  andl $63, %esi
  addl %eax, %esi
  addl $-130, %esi
  movzbl src_data(%rsi), %esi
  jmp .LBB0_36
.LBB0_35:
  movl %edx, %esi
  shrl $24, %esi
.LBB0_36:
  movb %sil, src_data-2(%rax)
  imull $1664525, %edx, %edx
  addl $1013904223, %edx
  movl %edx, %esi
  andl $14, %esi
  cmpl $5, %esi
  ja .LBB0_39
  leaq -1(%rax), %rsi
  cmpq $128, %rsi
  jb .LBB0_39
  leal (%rcx,%rcx,4), %esi
  leal (%rcx,%rsi,4), %esi
  addb $41, %sil
  movzbl %sil, %esi
  andl $63, %esi
  addl %eax, %esi
  addl $-129, %esi
  movzbl src_data(%rsi), %esi
  jmp .LBB0_40
.LBB0_39:
  movl %edx, %esi
  shrl $24, %esi
.LBB0_40:
  movb %sil, src_data-1(%rax)
  imull $1664525, %edx, %edx
  addl $1013904223, %edx
  movl %edx, %esi
  andl $14, %esi
  cmpl $5, %esi
  ja .LBB0_43
  cmpq $128, %rax
  jb .LBB0_43
  movl %ecx, %esi
  shll $4, %esi
  addl %ecx, %esi
  addb $52, %sil
  movzbl %sil, %ecx
  andl $63, %ecx
  addl %eax, %ecx
  addl $-128, %ecx
  movzbl src_data(%rcx), %ecx
  jmp .LBB0_44
.LBB0_45:
  xorl %edx, %edx
  xorl %esi, %esi
  jmp .LBB0_4
.LBB0_28:
  movq 24(%rsp), %rdx
  movq 16(%rsp), %rsi
.LBB0_29:
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
  imulq $4099, %rdx, %rax
  xorb $85, src_data(%rax)
  shlq $16, %rsi
  addq %rcx, %rsi
  incq %rdx
  cmpq $24, %rdx
  je .LBB0_30
.LBB0_4:
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
  jmp .LBB0_5
.LBB0_25:
  movq %r14, %rax
  subq %rbp, %rax
  sarq $6, %rax
  leaq (%rax,%r14), %rbx
  incq %rbx
.LBB0_26:
  movq %rbx, %r14
  cmpq $src_data+524276, %rbx
  jae .LBB0_27
.LBB0_5:
  movl (%r14), %eax
  imull $-1640531535, %eax, %ecx
  shrl $18, %ecx
  movl hash_table(,%rcx,4), %edi
  leaq src_data(%rdi), %r9
  movl %r14d, %edx
  subl %esi, %edx
  movl %edx, hash_table(,%rcx,4)
  cmpq %r14, %r9
  jae .LBB0_25
  cmpl %eax, (%r9)
  jne .LBB0_25
  movl %r14d, %r12d
  leaq 4(%r9), %r13
  leaq 4(%r14), %rbx
  cmpq $src_data+524288, %rbx
  movq %rdi, 48(%rsp)
  jae .LBB0_12
  movl $src_data+524288, %eax
  movq %rdi, %rcx
  subq %r14, %rcx
  addq %rcx, %rax
  addq $src_data, %rax
  movl $src_data+524284, %ecx
  subq %r14, %rcx
.LBB0_9:
  movzbl (%rbx), %edx
  cmpb (%r13), %dl
  jne .LBB0_12
  incq %rbx
  incq %r13
  decq %rcx
  jne .LBB0_9
  movq %rax, %r13
  movl $src_data+524288, %ebx
.LBB0_12:
  subq %rbp, %r14
  leaq 1(%r8), %r15
  cmpl $15, %r14d
  jb .LBB0_16
  movb $-16, (%r8)
  leal -15(%r14), %eax
  cmpl $255, %eax
  jb .LBB0_15
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
.LBB0_15:
  movb %al, (%r15)
  incq %r15
  jmp .LBB0_17
.LBB0_16:
  movl %r14d, %eax
  shlb $4, %al
  movb %al, (%r8)
  testl %r14d, %r14d
  je .LBB0_19
.LBB0_17:
  movl %r14d, %r12d
  cmpq $12, %r12
  jbe .LBB0_46
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
  jmp .LBB0_19
.LBB0_46:
  movl $4294967292, %eax
  andq %rax, %r14
  je .LBB0_47
  xorl %eax, %eax
.LBB0_50:
  movl (%rbp,%rax), %ecx
  movl %ecx, (%r15,%rax)
  addq $4, %rax
  cmpq %r14, %rax
  jb .LBB0_50
  jmp .LBB0_51
.LBB0_47:
  xorl %r14d, %r14d
  jmp .LBB0_48
.LBB0_19:
  movl %r13d, %eax
  subl %r9d, %eax
  leal -4(%rax), %ecx
  movl %ebx, %edx
  subl %r13d, %edx
  leaq 2(%r15), %rdi
  movw %dx, (%r15)
  movzbl (%r8), %edx
  cmpl $15, %ecx
  jb .LBB0_24
  orb $15, %dl
  movb %dl, (%r8)
  addl $-19, %eax
  cmpl $255, %eax
  jb .LBB0_22
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
.LBB0_22:
  movb %al, (%rdi)
  incq %rdi
  jmp .LBB0_23
.LBB0_24:
  orb %cl, %dl
  movb %dl, (%r8)
.LBB0_23:
  movq %rdi, %r8
  movq %rbx, %rbp
  jmp .LBB0_26
.LBB0_48:
  movzbl (%rbp,%r14), %eax
  movb %al, (%r15,%r14)
  incq %r14
.LBB0_51:
  cmpq %r14, %r12
  jne .LBB0_48
  addq %r12, %r15
  jmp .LBB0_19
.LBB0_27:
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
  je .LBB0_28
  cmpl $1, %r12d
  adcl $0, %r12d
  cmpl $12, %r12d
  jbe .LBB0_55
  movq %r15, %rdi
  movq %rbp, %rsi
  movq %r12, %rdx
  callq _intel_fast_memcpy@PLT
  movq 24(%rsp), %rdx
  movq 16(%rsp), %rsi
  addq %r12, %r15
  jmp .LBB0_29
.LBB0_55:
  movq %r12, %rax
  movl $4294967292, %ecx
  andq %rcx, %rax
  je .LBB0_56
  xorl %ecx, %ecx
.LBB0_58:
  movl (%rbp,%rcx), %edx
  movl %edx, 1(%r8,%rcx)
  addq $4, %rcx
  cmpq %rax, %rcx
  jb .LBB0_58
  cmpq %r12, %rax
  movq 24(%rsp), %rdx
  movq 16(%rsp), %rsi
  jne .LBB0_60
  jmp .LBB0_61
.LBB0_56:
  xorl %eax, %eax
  movq 24(%rsp), %rdx
  movq 16(%rsp), %rsi
.LBB0_60:
  movzbl (%rbp,%rax), %ecx
  movb %cl, 1(%r8,%rax)
  leaq 1(%rax), %rcx
  movq %rcx, %rax
  cmpq %rcx, %r12
  jne .LBB0_60
.LBB0_61:
  addq %r12, %r15
  jmp .LBB0_29
.LBB0_30:
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
