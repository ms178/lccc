expat_utf8_name_length:
  cmpq %rsi, %rdi
  jnb .L20
  movq %rdi, %rax
.L19:
  movzbl (%rax), %edx
  cmpb $-17, %dl
  ja .L3
  cmpb $-33, %dl
  ja .L4
  testb %dl, %dl
  js .L31
  movl %edx, %ecx
  andl $-33, %ecx
  subl $65, %ecx
  cmpb $25, %cl
  ja .L32
.L9:
  addq $1, %rax
.L11:
  cmpq %rsi, %rax
  jb .L19
.L29:
  subq %rdi, %rax
  ret
.L3:
  addl $16, %edx
  cmpb $4, %dl
  ja .L29
  movq %rsi, %rdx
  subq %rax, %rdx
  cmpq $3, %rdx
  jle .L29
  movzbl 1(%rax), %edx
  andl $-64, %edx
  cmpb $-128, %dl
  jne .L29
  movzbl 2(%rax), %edx
  andl $-64, %edx
  cmpb $-128, %dl
  jne .L29
  movzbl 3(%rax), %edx
  andl $-64, %edx
  cmpb $-128, %dl
  jne .L29
  movl $4, %edx
  jmp .L17
.L20:
  xorl %eax, %eax
  ret
.L31:
  addl $62, %edx
  cmpb $29, %dl
  ja .L29
  movq %rsi, %rdx
  subq %rax, %rdx
  cmpq $1, %rdx
  je .L29
  movzbl 1(%rax), %edx
  andl $-64, %edx
  cmpb $-128, %dl
  jne .L29
  movl $2, %edx
.L17:
  addq %rdx, %rax
  jmp .L11
.L4:
  movq %rsi, %rdx
  subq %rax, %rdx
  cmpq $2, %rdx
  jle .L29
  movzbl 1(%rax), %edx
  andl $-64, %edx
  cmpb $-128, %dl
  jne .L29
  movzbl 2(%rax), %edx
  andl $-64, %edx
  cmpb $-128, %dl
  jne .L29
  movl $3, %edx
  jmp .L17
.L32:
  subl $45, %edx
  cmpb $50, %dl
  ja .L29
  movabsq $-1125899906859004, %rcx
  btq %rdx, %rcx
  jc .L29
  jmp .L9
.LC6:
main:
  pushq %rbp
  movl $ascii_name.2+7, %esi
  movl $ascii_name.2, %edi
  movq %rsp, %rbp
  pushq %r14
  pushq %r13
  pushq %r12
  pushq %rbx
  andq $-32, %rsp
  call expat_utf8_name_length
  cmpq $7, %rax
  je .L71
.L34:
  movl $2, %eax
.L33:
  leaq -32(%rbp), %rsp
  popq %rbx
  popq %r12
  popq %r13
  popq %r14
  popq %rbp
  ret
.L71:
  movl $utf8_name.1+8, %esi
  movl $utf8_name.1, %edi
  call expat_utf8_name_length
  cmpq $8, %rax
  jne .L34
  movl $expat_xml_data, %r10d
  vmovdqa .LC0(%rip), %ymm1
  vmovdqa .LC1(%rip), %ymm0
  movq %r10, %rax
.L37:
  vmovdqu %ymm1, (%rax)
  addq $60, %rax
  vmovdqu %ymm0, -32(%rax)
  cmpq $expat_xml_data+1048560, %rax
  jne .L37
  vmovdqa .LC7(%rip), %xmm0
  vmovdqa .LC3(%rip), %ymm4
  movl $expat_xml_data+524224, %r12d
  xorl %r11d, %r11d
  movabsq $1469598103934665603, %rbx
  movabsq $1099511628211, %r9
  vmovdqa %xmm0, expat_xml_data+1048560(%rip)
.L38:
  vpbroadcastq .LC9(%rip), %ymm3
  movq %rbx, %r13
  movl $expat_xml_data, %edi
  jmp .L57
.L72:
  cmpb $39, %r8b
  je .L61
  movl %r8d, %eax
  andl $-33, %eax
  subl $65, %eax
  cmpb $25, %al
  jbe .L53
  cmpb $95, %r8b
  sete %al
  cmpb $58, %r8b
  sete %dl
  orb %dl, %al
  jne .L53
  cmpb $-63, %r8b
  jbe .L54
.L53:
  movl $expat_xml_data+1048576, %esi
  call expat_utf8_name_length
  testq %rax, %rax
  je .L42
  addq %rax, %r8
  addq %rax, %rdi
  xorq %r13, %r8
  imulq %r9, %r8
  movq %r8, %r13
.L52:
  cmpq $expat_xml_data+1048576, %rdi
  jnb .L42
.L57:
  movzbl (%rdi), %r8d
  cmpb $34, %r8b
  jne .L72
.L61:
  leaq 1(%rdi), %rax
  cmpq $expat_xml_data+1048576, %rax
  jnb .L42
  movq %rax, %r14
  movl $47, %ecx
  negq %r14
  andl $31, %r14d
  leaq 32(%r14), %rdx
  cmpq %rcx, %rdx
  cmovb %rcx, %rdx
  movl $expat_xml_data+1048574, %ecx
  subq %rdi, %rcx
  cmpq %rdx, %rcx
  jb .L49
  testq %r14, %r14
  je .L59
  leaq 1(%r14), %rdx
  leaq (%rdi,%rdx), %rcx
  jmp .L46
.L73:
  addq $1, %rax
  cmpq %rcx, %rax
  je .L44
.L46:
  cmpb (%rax), %r8b
  jne .L73
.L45:
  cmpq $expat_xml_data+1048576, %rax
  jnb .L42
  leaq 1(%rax), %rdi
  jmp .L52
.L74:
  addq $1, %rax
  cmpq $expat_xml_data+1048576, %rax
  jnb .L42
.L49:
  cmpb (%rax), %r8b
  jne .L74
  jmp .L45
.L59:
  movq %rax, %rcx
  movl $1, %edx
.L44:
  movl $expat_xml_data+1048575, %esi
  vmovq %rcx, %xmm5
  leaq (%rdi,%rdx), %rax
  xorl %edx, %edx
  subq %rdi, %rsi
  vpbroadcastq %xmm5, %ymm1
  vmovd %r8d, %xmm2
  subq %r14, %rsi
  vpaddq %ymm4, %ymm1, %ymm1
  vpbroadcastb %xmm2, %ymm2
  movq %rsi, %rdi
  andq $-32, %rdi
  jmp .L47
.L51:
  addq $32, %rdx
  cmpq %rdi, %rdx
  je .L75
  vpaddq %ymm3, %ymm1, %ymm1
.L47:
  vpcmpeqb (%rax,%rdx), %ymm2, %ymm0
  vptest %ymm0, %ymm0
  je .L51
  leaq (%rcx,%rdx), %rax
  jmp .L49
.L75:
  leaq (%rcx,%rdx), %rax
  cmpq %rdx, %rsi
  jne .L49
  vpbroadcastq .LC8(%rip), %ymm0
  vpaddq %ymm0, %ymm1, %ymm1
  vextracti128 $0x1, %ymm1, %xmm0
  vpextrq $1, %xmm0, %rax
  jmp .L45
.L42:
  xorb $1, (%r10)
  addq $8191, %r10
  xorq %r13, %r11
  cmpq %r10, %r12
  jne .L38
  xorl %eax, %eax
  movq %r11, %rsi
  movl $.LC6, %edi
  vzeroupper
  call printf
  xorl %eax, %eax
  jmp .L33
.L54:
  addq $1, %rdi
  jmp .L52
utf8_name.1:
ascii_name.2:
.LC0:
.LC1:
.LC3:
.LC7:
.LC8:
.LC9:
