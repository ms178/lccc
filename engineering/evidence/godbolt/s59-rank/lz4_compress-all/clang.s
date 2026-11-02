main:
  pushq %rbp
  pushq %r15
  pushq %r14
  pushq %r13
  pushq %r12
  pushq %rbx
  subq $56, %rsp
  movl $1511734397, %ecx
  xorl %eax, %eax
  leaq src_data(%rip), %r13
  jmp .LBB0_1
.LBB0_7:
  movl %edx, %ecx
  shrl $24, %ecx
.LBB0_9:
  movb %cl, 1(%rax,%r13)
  addq $2, %rax
  movl %edx, %ecx
  cmpq $524288, %rax
  je .LBB0_10
.LBB0_1:
  imull $1664525, %ecx, %edx
  addl $1013904223, %edx
  cmpq $128, %rax
  jb .LBB0_4
  movl %edx, %esi
  andl $14, %esi
  cmpl $5, %esi
  ja .LBB0_4
  leal (%rcx,%rcx,2), %esi
  leal (%rcx,%rsi,4), %esi
  addb $31, %sil
  movzbl %sil, %esi
  andl $63, %esi
  addl %eax, %esi
  addl $-128, %esi
  movzbl (%rsi,%r13), %esi
  jmp .LBB0_5
.LBB0_4:
  movl %edx, %esi
  shrl $24, %esi
.LBB0_5:
  movb %sil, (%rax,%r13)
  imull $1664525, %edx, %edx
  addl $1013904223, %edx
  cmpq $128, %rax
  jb .LBB0_7
  movl %edx, %esi
  andl $14, %esi
  cmpl $6, %esi
  jae .LBB0_7
  leal (%rcx,%rcx,2), %esi
  shll $3, %esi
  subl %ecx, %esi
  movb $50, %cl
  subb %sil, %cl
  movzbl %cl, %ecx
  andl $63, %ecx
  addl %eax, %ecx
  addl $-127, %ecx
  movzbl (%rcx,%r13), %ecx
  jmp .LBB0_9
.LBB0_10:
  xorl %esi, %esi
  leaq hash_table(%rip), %r10
  leaq dst_data(%rip), %rbx
  leaq src_data+524288(%rip), %rax
  addq %r13, %rax
  movq %rax, 32(%rsp)
  xorl %edi, %edi
  jmp .LBB0_11
.LBB0_52:
  movq %rdx, %rax
  leaq dst_data(%rip), %rbx
.LBB0_71:
  movq 8(%rsp), %rdi
.LBB0_72:
  subl %ebx, %eax
  movq %rax, %rcx
  shlq $32, %rcx
  movzbl dst_data(%rip), %edx
  movq 16(%rsp), %rsi
  addq %rdx, %rsi
  movq %rsi, %rdx
  addq %rcx, %rdx
  shrl %eax
  movzbl (%rax,%rbx), %esi
  imulq $4099, %rdi, %rax
  xorb $85, (%rax,%r9)
  shll $16, %esi
  addq %rdx, %rsi
  incq %rdi
  cmpq $24, %rdi
  je .LBB0_73
.LBB0_11:
  movq %rdi, 8(%rsp)
  movq %rsi, 16(%rsp)
  movl $65536, %edx
  movq %r10, %rdi
  xorl %esi, %esi
  vzeroupper
  callq memset@PLT
  leaq src_data+524276(%rip), %r8
  leaq hash_table(%rip), %r10
  leaq src_data(%rip), %r9
  leaq src_data+1(%rip), %r12
  movq %r9, %r13
  movq %rbx, %r11
  jmp .LBB0_12
.LBB0_49:
  movq %r12, %rax
  subq %r13, %rax
  sarq $6, %rax
  leaq (%r12,%rax), %rbp
  incq %rbp
.LBB0_50:
  movq %rbp, %r12
  cmpq %r8, %rbp
  jae .LBB0_51
.LBB0_12:
  movl (%r12), %eax
  imull $-1640531535, %eax, %ecx
  shrl $18, %ecx
  movl (%r10,%rcx,4), %esi
  leaq (%r9,%rsi), %rdi
  movl %r12d, %edx
  subl %r9d, %edx
  movl %edx, (%r10,%rcx,4)
  cmpq %r12, %rdi
  jae .LBB0_49
  cmpl %eax, (%rdi)
  jne .LBB0_49
  movl %r12d, %ebx
  leaq 4(%rdi), %r14
  leaq 4(%r12), %rbp
  leaq src_data+524288(%rip), %rax
  cmpq %rax, %rbp
  movq %rsi, 40(%rsp)
  movq %rdi, 48(%rsp)
  jae .LBB0_19
  movq %rsi, %rax
  subq %r12, %rax
  addq 32(%rsp), %rax
  movq %r9, %rcx
  subq %r12, %rcx
  addq $524284, %rcx
.LBB0_16:
  movzbl (%rbp), %edx
  cmpb (%r14), %dl
  jne .LBB0_19
  incq %rbp
  incq %r14
  decq %rcx
  jne .LBB0_16
  movq %rax, %r14
  leaq src_data+524288(%rip), %rbp
.LBB0_19:
  subq %r13, %r12
  leaq 1(%r11), %r15
  cmpl $15, %r12d
  jb .LBB0_23
  movb $-16, (%r11)
  leal -15(%r12), %eax
  cmpl $255, %eax
  jb .LBB0_22
  subl %r13d, %ebx
  leal -270(%rbx), %ecx
  movl $2155905153, %eax
  imulq %rax, %rcx
  shrq $39, %rcx
  movq %rcx, 24(%rsp)
  leaq 1(%rcx), %rdx
  movq %r15, %rdi
  movl $255, %esi
  movq %r11, %r15
  vzeroupper
  callq memset@PLT
  movq %r15, %r11
  leaq src_data+524276(%rip), %r8
  leaq hash_table(%rip), %r10
  leaq src_data(%rip), %r9
  movq 24(%rsp), %rcx
  addq %rcx, %r15
  addq $2, %r15
  movl %ecx, %eax
  shll $8, %eax
  subl %eax, %ecx
  leal (%rbx,%rcx), %eax
  addl $-270, %eax
.LBB0_22:
  movb %al, (%r15)
  incq %r15
  jmp .LBB0_24
.LBB0_23:
  movl %r12d, %eax
  shlb $4, %al
  movb %al, (%r11)
  testl %r12d, %r12d
  je .LBB0_43
.LBB0_24:
  movl %r12d, %eax
  cmpq $8, %rax
  jb .LBB0_25
  movq %r13, %rcx
  subq %r15, %rcx
  cmpq $-128, %rcx
  ja .LBB0_25
  cmpl $128, %eax
  jae .LBB0_29
  xorl %edx, %edx
  jmp .LBB0_33
.LBB0_25:
  xorl %edx, %edx
  movq %r15, %rcx
  jmp .LBB0_36
.LBB0_29:
  movl %r12d, %edx
  andl $-128, %edx
  leaq (%r15,%rdx), %rcx
  xorl %esi, %esi
.LBB0_30:
  vmovups (%r13,%rsi), %ymm0
  vmovups 32(%r13,%rsi), %ymm1
  vmovups 64(%r13,%rsi), %ymm2
  vmovups 96(%r13,%rsi), %ymm3
  vmovups %ymm0, (%r15,%rsi)
  vmovups %ymm1, 32(%r15,%rsi)
  vmovups %ymm2, 64(%r15,%rsi)
  vmovups %ymm3, 96(%r15,%rsi)
  subq $-128, %rsi
  cmpq %rsi, %rdx
  jne .LBB0_30
  cmpl %edx, %eax
  je .LBB0_42
  testb $120, %r12b
  je .LBB0_36
.LBB0_33:
  movq %rdx, %rsi
  movl %r12d, %edx
  andl $-8, %edx
  leaq (%r15,%rdx), %rcx
.LBB0_34:
  movq (%r13,%rsi), %rdi
  movq %rdi, (%r15,%rsi)
  addq $8, %rsi
  cmpq %rsi, %rdx
  jne .LBB0_34
  movq %rcx, %r15
  cmpl %edx, %eax
  je .LBB0_43
.LBB0_36:
  movq %rdx, %rsi
  andq $7, %r12
  je .LBB0_38
.LBB0_37:
  movzbl (%r13,%rsi), %edi
  movb %dil, (%rcx)
  incq %rcx
  incq %rsi
  decq %r12
  jne .LBB0_37
.LBB0_38:
  subq %rax, %rdx
  movq %rcx, %r15
  cmpq $-8, %rdx
  ja .LBB0_43
  leaq (%rsi,%r13), %rdx
  addq $7, %rdx
  subq %rsi, %rax
  xorl %esi, %esi
.LBB0_40:
  movzbl -7(%rdx,%rsi), %edi
  movb %dil, (%rcx,%rsi)
  movzbl -6(%rdx,%rsi), %edi
  movb %dil, 1(%rcx,%rsi)
  movzbl -5(%rdx,%rsi), %edi
  movb %dil, 2(%rcx,%rsi)
  movzbl -4(%rdx,%rsi), %edi
  movb %dil, 3(%rcx,%rsi)
  movzbl -3(%rdx,%rsi), %edi
  movb %dil, 4(%rcx,%rsi)
  movzbl -2(%rdx,%rsi), %edi
  movb %dil, 5(%rcx,%rsi)
  movzbl -1(%rdx,%rsi), %edi
  movb %dil, 6(%rcx,%rsi)
  movzbl (%rdx,%rsi), %edi
  movb %dil, 7(%rcx,%rsi)
  addq $8, %rsi
  cmpq %rsi, %rax
  jne .LBB0_40
  addq %rsi, %rcx
.LBB0_42:
  movq %rcx, %r15
.LBB0_43:
  movl %r14d, %eax
  subl 48(%rsp), %eax
  leal -4(%rax), %ecx
  movl %ebp, %edx
  subl %r14d, %edx
  movw %dx, (%r15)
  leaq 2(%r15), %rdi
  movzbl (%r11), %edx
  cmpl $15, %ecx
  jb .LBB0_48
  orb $15, %dl
  movb %dl, (%r11)
  addl $-19, %eax
  cmpl $255, %eax
  jb .LBB0_46
  movq 40(%rsp), %rax
  addl %r9d, %eax
  subl %eax, %r14d
  leal -274(%r14), %ebx
  movl $2155905153, %eax
  imulq %rax, %rbx
  shrq $39, %rbx
  leaq 1(%rbx), %rdx
  movl $255, %esi
  vzeroupper
  callq memset@PLT
  leaq src_data+524276(%rip), %r8
  leaq hash_table(%rip), %r10
  leaq src_data(%rip), %r9
  leaq (%r15,%rbx), %rdi
  addq $3, %rdi
  movl %ebx, %eax
  shll $8, %eax
  subl %eax, %ebx
  leal (%r14,%rbx), %eax
  addl $-274, %eax
.LBB0_46:
  movb %al, (%rdi)
  incq %rdi
  jmp .LBB0_47
.LBB0_48:
  orb %cl, %dl
  movb %dl, (%r11)
.LBB0_47:
  movq %rdi, %r11
  movq %rbp, %r13
  jmp .LBB0_50
.LBB0_51:
  leal 524288(%r9), %eax
  movl %eax, %ecx
  subl %r13d, %ecx
  cmpl $15, %ecx
  movl $15, %edx
  cmovael %edx, %ecx
  shlb $4, %cl
  leaq 1(%r11), %rdx
  movb %cl, (%r11)
  subl %r13d, %eax
  je .LBB0_52
  cmpl $8, %eax
  leaq dst_data(%rip), %rbx
  jb .LBB0_54
  movq %r11, %rcx
  subq %r13, %rcx
  cmpq $127, %rcx
  jb .LBB0_54
  movl %eax, %esi
  cmpl $128, %eax
  jae .LBB0_58
  xorl %ecx, %ecx
  jmp .LBB0_62
.LBB0_54:
  xorl %ecx, %ecx
  movq %rdx, %rax
  jmp .LBB0_65
.LBB0_58:
  movl %esi, %ecx
  andl $-128, %ecx
  leaq (%rdx,%rcx), %rax
  xorl %edi, %edi
.LBB0_59:
  vmovups (%r13,%rdi), %ymm0
  vmovups 32(%r13,%rdi), %ymm1
  vmovups 64(%r13,%rdi), %ymm2
  vmovups 96(%r13,%rdi), %ymm3
  vmovups %ymm0, 1(%r11,%rdi)
  vmovups %ymm1, 33(%r11,%rdi)
  vmovups %ymm2, 65(%r11,%rdi)
  vmovups %ymm3, 97(%r11,%rdi)
  subq $-128, %rdi
  cmpq %rdi, %rcx
  jne .LBB0_59
  cmpl %esi, %ecx
  je .LBB0_71
  testb $120, %sil
  je .LBB0_65
.LBB0_62:
  movq %rcx, %rdi
  movl %esi, %ecx
  andl $-8, %ecx
  leaq (%rdx,%rcx), %rax
.LBB0_63:
  movq (%r13,%rdi), %r8
  movq %r8, (%rdx,%rdi)
  addq $8, %rdi
  cmpq %rdi, %rcx
  jne .LBB0_63
  cmpl %esi, %ecx
  movq 8(%rsp), %rdi
  je .LBB0_72
.LBB0_65:
  movl %r9d, %edx
  subl %r13d, %edx
  addl $524288, %edx
  movq %rdx, %rdi
  movq %rcx, %rsi
  andq $7, %rdi
  je .LBB0_67
.LBB0_66:
  movzbl (%r13,%rsi), %r8d
  movb %r8b, (%rax)
  incq %rsi
  incq %rax
  decq %rdi
  jne .LBB0_66
.LBB0_67:
  subq %rdx, %rcx
  cmpq $-8, %rcx
  movq 8(%rsp), %rdi
  ja .LBB0_72
  leaq (%rsi,%r13), %rcx
  addq $7, %rcx
  subq %rsi, %rdx
  xorl %esi, %esi
.LBB0_69:
  movzbl -7(%rcx,%rsi), %edi
  movb %dil, (%rax,%rsi)
  movzbl -6(%rcx,%rsi), %edi
  movb %dil, 1(%rax,%rsi)
  movzbl -5(%rcx,%rsi), %edi
  movb %dil, 2(%rax,%rsi)
  movzbl -4(%rcx,%rsi), %edi
  movb %dil, 3(%rax,%rsi)
  movzbl -3(%rcx,%rsi), %edi
  movb %dil, 4(%rax,%rsi)
  movzbl -2(%rcx,%rsi), %edi
  movb %dil, 5(%rax,%rsi)
  movzbl -1(%rcx,%rsi), %edi
  movb %dil, 6(%rax,%rsi)
  movzbl (%rcx,%rsi), %edi
  movb %dil, 7(%rax,%rsi)
  addq $8, %rsi
  cmpq %rsi, %rdx
  jne .LBB0_69
  addq %rsi, %rax
  jmp .LBB0_71
.LBB0_73:
  leaq .L.str(%rip), %rdi
  xorl %eax, %eax
  vzeroupper
  callq printf@PLT
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

