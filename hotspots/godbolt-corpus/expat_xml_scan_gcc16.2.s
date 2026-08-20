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
  jnc .L9
  jmp .L29
.LC3:
  .string "%lu\n"
main:
  pushq %rbp
  movl $ascii_name.2+7, %esi
  movl $ascii_name.2, %edi
  movq %rsp, %rbp
  subq $16, %rsp
  call expat_utf8_name_length
  cmpq $7, %rax
  je .L60
.L34:
  leave
  movl $2, %eax
  ret
.L60:
  movl $utf8_name.1+8, %esi
  movl $utf8_name.1, %edi
  call expat_utf8_name_length
  cmpq $8, %rax
  jne .L34
  movq %rbx, -16(%rbp)
  movl $expat_xml_data, %r9d
  vmovdqa .LC0(%rip), %ymm1
  movq %r12, -8(%rbp)
  vmovdqa .LC1(%rip), %ymm0
  movq %r9, %rax
.L37:
  vmovdqu %ymm1, (%rax)
  addq $60, %rax
  vmovdqu %ymm0, -32(%rax)
  cmpq $expat_xml_data+1048560, %rax
  jne .L37
  vmovdqa .LC4(%rip), %xmm0
  xorl %r11d, %r11d
  movabsq $1469598103934665603, %rbx
  movabsq $1099511628211, %r10
  vmovdqa %xmm0, expat_xml_data+1048560(%rip)
.L38:
  movq %rbx, %r12
  movl $expat_xml_data, %edi
  jmp .L47
.L62:
  cmpb $39, %r8b
  je .L49
  movl %r8d, %eax
  andl $-33, %eax
  subl $65, %eax
  cmpb $25, %al
  jbe .L46
  cmpb $95, %r8b
  sete %al
  cmpb $58, %r8b
  sete %dl
  orb %dl, %al
  jne .L46
  cmpb $-63, %r8b
  jbe .L61
.L46:
  movl $expat_xml_data+1048576, %esi
  call expat_utf8_name_length
  testq %rax, %rax
  je .L42
  addq %rax, %r8
  addq %rax, %rdi
  xorq %r12, %r8
  imulq %r10, %r8
  movq %r8, %r12
.L45:
  cmpq $expat_xml_data+1048576, %rdi
  jnb .L42
.L47:
  movzbl (%rdi), %r8d
  cmpb $34, %r8b
  jne .L62
.L49:
  leaq 1(%rdi), %rax
  cmpq $expat_xml_data+1048576, %rax
  jb .L41
  jmp .L42
.L44:
  addq $1, %rax
  cmpq $expat_xml_data+1048576, %rax
  je .L42
.L41:
  cmpb (%rax), %r8b
  jne .L44
  cmpq $expat_xml_data+1048576, %rax
  jnb .L42
  leaq 1(%rax), %rdi
  jmp .L45
.L61:
  addq $1, %rdi
  jmp .L45
.L42:
  xorb $1, (%r9)
  addq $8191, %r9
  xorq %r12, %r11
  cmpq $expat_xml_data+524224, %r9
  jne .L38
  xorl %eax, %eax
  movq %r11, %rsi
  movl $.LC3, %edi
  vzeroupper
  call printf
  movq -16(%rbp), %rbx
  movq -8(%rbp), %r12
  leave
  xorl %eax, %eax
  ret
utf8_name.1:
  .base64 "Y2Fmw6lUYWcA"
ascii_name.2:
  .string "alpha-9"
.LC0:
  .quad 7214953259748909884
  .quad 2467238573764605025
  .quad 7308338831271540529
  .quad 6879655750179037757
.LC1:
  .quad 4477196180081177204
  .quad 7881622743709934964
  .quad 4352010437821217648
  .quad 738214045636715311
.LC4:
  .long 538976288
  .long 538976288
  .long 538976288
  .long 538976288
