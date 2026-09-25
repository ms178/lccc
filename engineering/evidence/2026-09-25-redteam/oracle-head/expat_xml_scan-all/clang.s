.LCPI0_1:
main:
  xorl %ecx, %ecx
  leaq check_name_kernel.utf8_name(%rip), %rax
  jmp .LBB0_2
.LBB0_1:
  incq %rcx
  cmpq $8, %rcx
  jge .LBB0_14
.LBB0_2:
  leaq (%rax,%rcx), %rdx
  movzbl (%rcx,%rax), %esi
  leaq -5(%rcx), %rdi
  cmpq $-3, %rdi
  ja .LBB0_7
  movl %esi, %edi
  andb $95, %dil
  addb $-65, %dil
  cmpb $26, %dil
  jb .LBB0_1
  cmpl $58, %esi
  je .LBB0_1
  cmpl $95, %esi
  je .LBB0_1
  addb $-48, %sil
  cmpb $9, %sil
  jbe .LBB0_1
  jmp .LBB0_15
.LBB0_7:
  leal 32(%rsi), %edi
  cmpb $-31, %dil
  ja .LBB0_12
  addb $16, %sil
  cmpb $4, %sil
  ja .LBB0_15
  cmpb $-64, 1(%rdx)
  jge .LBB0_15
  cmpb $-65, 2(%rdx)
  jg .LBB0_15
  movl $4, %esi
  cmpb $-65, 3(%rdx)
  jle .LBB0_13
  jmp .LBB0_15
.LBB0_12:
  movl $2, %esi
  cmpb $-64, 1(%rdx)
  jge .LBB0_15
.LBB0_13:
  addq %rsi, %rcx
  cmpq $8, %rcx
  jl .LBB0_2
.LBB0_14:
  addq %rax, %rcx
  movq %rcx, %rdx
.LBB0_15:
  pushq %rbp
  pushq %r15
  pushq %r14
  pushq %r13
  pushq %r12
  pushq %rbx
  pushq %rax
  subq %rax, %rdx
  movl $2, %ebx
  cmpq $8, %rdx
  jne .LBB0_64
  movq $-240, %rcx
  leaq expat_xml_data(%rip), %rax
  vmovups make_xml_corpus.fragment+28(%rip), %ymm0
  vmovups make_xml_corpus.fragment(%rip), %ymm1
.LBB0_17:
  vmovups %ymm0, 268(%rcx,%rax)
  vmovups %ymm1, 240(%rcx,%rax)
  vmovups %ymm1, 300(%rcx,%rax)
  vmovups %ymm0, 328(%rcx,%rax)
  vmovups %ymm1, 360(%rcx,%rax)
  vmovups %ymm0, 388(%rcx,%rax)
  vmovups %ymm1, 420(%rcx,%rax)
  vmovups %ymm0, 448(%rcx,%rax)
  addq $240, %rcx
  cmpq $1048276, %rcx
  jb .LBB0_17
  vbroadcastss .LCPI0_1(%rip), %xmm0
  vmovaps %xmm0, expat_xml_data+1048560(%rip)
  xorl %ecx, %ecx
  movabsq $1469598103934665603, %rdx
  movabsq $1099511628211, %rdi
  xorl %esi, %esi
.LBB0_19:
  movq %rax, %r10
  xorl %r8d, %r8d
  movq %rdx, %r9
  jmp .LBB0_23
.LBB0_20:
  movq %rbx, %r8
.LBB0_21:
  leaq 2(%r8), %r11
  cmpq $1048575, %r8
  cmovaeq %r10, %r11
  movq %r11, %r8
.LBB0_22:
  leaq (%rax,%r11), %r10
  cmpq $1048576, %r11
  jge .LBB0_61
.LBB0_23:
  movzbl (%r8,%rax), %r11d
  cmpl $39, %r11d
  je .LBB0_25
  cmpl $34, %r11d
  jne .LBB0_29
.LBB0_25:
  cmpq $1048576, %r8
  movl $1048575, %ebx
  cmovaeq %r8, %rbx
  leaq 1(%rbx), %r10
  cmpq $1048574, %r8
  ja .LBB0_20
.LBB0_26:
  cmpb %r11b, 1(%r8,%rax)
  je .LBB0_34
  incq %r8
  cmpq %r8, %rbx
  jne .LBB0_26
  jmp .LBB0_20
.LBB0_29:
  movl %r11d, %ebx
  andb $-33, %bl
  addb $-65, %bl
  cmpb $26, %bl
  jb .LBB0_35
  cmpq $58, %r11
  je .LBB0_35
  cmpl $95, %r11d
  je .LBB0_35
  cmpb $-62, %r11b
  jae .LBB0_35
  incq %r8
  movq %r8, %r11
  jmp .LBB0_22
.LBB0_34:
  leaq 1(%r8), %r10
  jmp .LBB0_21
.LBB0_35:
  movq %r8, %rbx
  jmp .LBB0_38
.LBB0_36:
  incq %rbx
  cmpq $1048576, %rbx
  jge .LBB0_58
.LBB0_38:
  leaq (%rax,%rbx), %r14
  movzbl (%rbx,%rax), %r15d
  testb %r15b, %r15b
  js .LBB0_45
  movl %r15d, %ebp
  andb $95, %bpl
  addb $-65, %bpl
  cmpb $26, %bpl
  jb .LBB0_36
  cmpl $58, %r15d
  je .LBB0_36
  cmpl $95, %r15d
  je .LBB0_36
  leal -48(%r15), %ebp
  cmpb $10, %bpl
  setae %bpl
  cmpl $45, %r15d
  setne %r12b
  testb %bpl, %r12b
  je .LBB0_36
  cmpl $46, %r15d
  je .LBB0_36
  jmp .LBB0_59
.LBB0_45:
  leal 32(%r15), %ebp
  cmpb $-31, %bpl
  ja .LBB0_48
  movl %r15d, %r12d
  andb $-16, %r12b
  cmpb $-32, %r12b
  jne .LBB0_49
  movl $3, %r15d
  xorl %r12d, %r12d
  jmp .LBB0_51
.LBB0_48:
  movl $2, %r15d
  xorl %r12d, %r12d
  jmp .LBB0_51
.LBB0_49:
  addb $16, %r15b
  cmpb $4, %r15b
  ja .LBB0_59
  movl $4, %r15d
  movb $1, %r12b
.LBB0_51:
  movl $1048576, %r13d
  subq %rbx, %r13
  cmpq %r15, %r13
  jb .LBB0_59
  cmpb $-65, 1(%r14)
  jg .LBB0_59
  cmpb $-31, %bpl
  ja .LBB0_55
  cmpb $-65, 2(%r14)
  jg .LBB0_59
.LBB0_55:
  testb %r12b, %r12b
  je .LBB0_57
  cmpb $-65, 3(%r14)
  jg .LBB0_59
.LBB0_57:
  addq %r15, %rbx
  cmpq $1048576, %rbx
  jl .LBB0_38
.LBB0_58:
  addq %rax, %rbx
  movq %rbx, %r14
.LBB0_59:
  subq %r10, %r14
  je .LBB0_61
  addq %r14, %r11
  xorq %r9, %r11
  imulq %rdi, %r11
  addq %r14, %r8
  leaq (%rax,%r8), %r10
  movq %r11, %r9
  cmpq $1048576, %r8
  jl .LBB0_23
  jmp .LBB0_62
.LBB0_61:
  movq %r9, %r11
.LBB0_62:
  movq %rcx, %r8
  shlq $13, %r8
  subq %rcx, %r8
  xorb $1, (%r8,%rax)
  xorq %r11, %rsi
  incq %rcx
  cmpq $64, %rcx
  jne .LBB0_19
  leaq .L.str(%rip), %rdi
  xorl %ebx, %ebx
  xorl %eax, %eax
  vzeroupper
  callq printf@PLT
.LBB0_64:
  movl %ebx, %eax
  addq $8, %rsp
  popq %rbx
  popq %r12
  popq %r13
  popq %r14
  popq %r15
  popq %rbp
  retq

.L.str:

check_name_kernel.utf8_name:

make_xml_corpus.fragment:
