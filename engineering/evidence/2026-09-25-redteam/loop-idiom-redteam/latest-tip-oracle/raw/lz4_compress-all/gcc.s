.LC7:
main:
  pushq %rbp
  xorl %edx, %edx
  movl $1511734397, %eax
  movq %rsp, %rbp
  pushq %r15
  pushq %r14
  pushq %r13
  pushq %r12
  pushq %rbx
  andq $-32, %rsp
  subq $96, %rsp
  jmp .L4
.L59:
  cmpq $127, %rdx
  jbe .L2
  movl %eax, %ecx
  andl $63, %ecx
  leal -128(%rcx,%rdx), %ecx
  movzbl src_data(%rcx), %ecx
.L3:
  movb %cl, src_data(%rdx)
  addq $1, %rdx
  cmpq $524288, %rdx
  je .L58
.L4:
  imull $1664525, %eax, %eax
  addl $1013904223, %eax
  movl %eax, %ecx
  andl $15, %ecx
  cmpl $5, %ecx
  jbe .L59
.L2:
  movl %eax, %esi
  shrl $24, %esi
  movl %esi, %ecx
  jmp .L3
.L58:
  movl $src_data, %r12d
  xorl %r13d, %r13d
.L5:
  xorl %esi, %esi
  movl $65536, %edx
  movl $hash_table, %edi
  movl $dst_data, %r14d
  call memset
  movl $src_data+1, %ebx
  movq %r13, %r8
  movq %r12, %r9
  movl $src_data, %esi
.L26:
  movl (%rbx), %edi
  movq %rbx, %r10
  subq $src_data, %r10
  imull $-1640531535, %edi, %eax
  shrl $18, %eax
  movl hash_table(,%rax,4), %ecx
  movl %r10d, hash_table(,%rax,4)
  movq %rbx, %rax
  subq %rsi, %rax
  leaq src_data(%rcx), %rdx
  cmpq %rbx, %rdx
  jnb .L6
  cmpq $src_data, %rdx
  jb .L6
  cmpl src_data(%rcx), %edi
  je .L60
.L6:
  sarq $6, %rax
  leaq 1(%rbx,%rax), %rbx
.L25:
  cmpq $src_data+524276, %rbx
  jb .L26
  movl $src_data+524288, %ebx
  movq %r8, %r13
  leaq 1(%r14), %rcx
  movq %r9, %r12
  subq %rsi, %rbx
  cmpl $14, %ebx
  jbe .L27
  movb $-16, (%r14)
.L28:
  movq %rcx, %rdi
  movl %ebx, %edx
  vzeroupper
  call memcpy
  movq %rax, %rcx
  leal -1(%rbx), %eax
  leaq 1(%rcx,%rax), %rcx
.L29:
  subq $dst_data, %rcx
  movzbl dst_data(%rip), %eax
  xorb $85, (%r12)
  addq $4099, %r12
  movq %rcx, %rdx
  shrl %ecx
  salq $32, %rdx
  xorq %rdx, %rax
  movzbl dst_data(%rcx), %edx
  salq $16, %rdx
  xorq %rdx, %rax
  addq %rax, %r13
  cmpq $src_data+98376, %r12
  jne .L5
  movq %r13, %rsi
  movl $.LC7, %edi
  xorl %eax, %eax
  call printf
  leaq -40(%rbp), %rsp
  xorl %eax, %eax
  popq %rbx
  popq %r12
  popq %r13
  popq %r14
  popq %r15
  popq %rbp
  ret
.L60:
  addq $4, %rbx
  leaq 4(%rdx), %r15
  cmpq $src_data+524288, %rbx
  jb .L7
  jmp .L61
.L9:
  addq $1, %rbx
  addq $1, %r15
  cmpq $src_data+524288, %rbx
  je .L56
.L7:
  movzbl (%r15), %edi
  cmpb %dil, (%rbx)
  je .L9
.L56:
  movq %r15, %r13
  subq %rdx, %r13
  movl %r13d, 76(%rsp)
  subl $4, %r13d
.L8:
  movl $-8160, %edx
  leaq 1(%r14), %rcx
  vmovd %edx, %xmm7
  vpbroadcastd %xmm7, %ymm7
  vmovdqa %ymm7, (%rsp)
  vpbroadcastq .LC8(%rip), %ymm7
  vmovdqa %ymm7, 32(%rsp)
  cmpl $14, %eax
  ja .L62
  movl %eax, %edx
  sall $4, %edx
  movb %dl, (%r14)
  testl %eax, %eax
  je .L18
.L17:
  movq %r9, 80(%rsp)
  movl %eax, %r12d
  movq %rcx, %rdi
  movq %r8, 88(%rsp)
  movq %r12, %rdx
  vzeroupper
  call memcpy
  movq 80(%rsp), %r9
  movq 88(%rsp), %r8
  leaq (%rax,%r12), %rcx
.L18:
  movq %rbx, %rdx
  leaq 2(%rcx), %rax
  subq %r15, %rdx
  movw %dx, (%rcx)
  movzbl (%r14), %edx
  cmpl $14, %r13d
  ja .L63
  orl %r13d, %edx
  movq %rbx, %rsi
  movb %dl, (%r14)
  movq %rax, %r14
  jmp .L25
.L27:
  movl %ebx, %eax
  sall $4, %eax
  movb %al, (%r14)
  testl %ebx, %ebx
  jne .L28
  vzeroupper
  jmp .L29
.L62:
  leal -15(%rax), %edi
  movb $-16, (%r14)
  cmpl $254, %edi
  jbe .L11
  leal -270(%rax), %edx
  cmpl $7904, %edx
  jbe .L31
  movl $2155905153, %r10d
  vmovq %rcx, %xmm6
  vmovd %edi, %xmm0
  vmovdqa (%rsp), %ymm3
  imulq %r10, %rdx
  vpbroadcastq %xmm6, %ymm1
  vpbroadcastd %xmm0, %ymm0
  vpaddq .LC0(%rip), %ymm1, %ymm1
  vpaddd .LC1(%rip), %ymm0, %ymm0
  vpcmpeqd %ymm5, %ymm5, %ymm5
  vmovdqa %ymm7, %ymm4
  shrq $39, %rdx
  addl $1, %edx
  movl %edx, %r11d
  movl %edx, 88(%rsp)
  movq %r14, %rdx
  shrl $5, %r11d
  movl %r11d, %r10d
  salq $5, %r10
  movq %r10, %r12
  leaq (%r14,%r10), %r10
.L13:
  vmovdqu %ymm5, 1(%rdx)
  addq $32, %rdx
  vmovdqa %ymm1, %ymm2
  vmovdqa %ymm0, %ymm6
  vpaddq %ymm4, %ymm1, %ymm1
  vpaddd %ymm3, %ymm0, %ymm0
  cmpq %rdx, %r10
  jne .L13
  movl %r11d, %edx
  sall $5, %edx
  cmpl %edx, 88(%rsp)
  je .L14
  imull $-8160, %r11d, %r11d
  leaq (%rcx,%r12), %rdx
  addl %r11d, %edi
.L15:
  movq %rdx, %rcx
  subl $255, %edi
  addq $1, %rdx
  movb $-1, -1(%rdx)
  cmpl $254, %edi
  ja .L15
.L16:
  movb %dil, (%rdx)
  addq $2, %rcx
  jmp .L17
.L63:
  movl 76(%rsp), %esi
  orl $15, %edx
  movb %dl, (%r14)
  leal -19(%rsi), %edx
  cmpl $254, %edx
  jbe .L20
  leal -274(%rsi), %r12d
  cmpl $7904, %r12d
  jbe .L24
  movl $2155905153, %esi
  vmovq %rax, %xmm7
  vmovd %edx, %xmm0
  vmovdqa 32(%rsp), %ymm3
  imulq %rsi, %r12
  vpbroadcastq %xmm7, %ymm1
  vpbroadcastd %xmm0, %ymm0
  vmovdqa (%rsp), %ymm2
  vpaddq .LC0(%rip), %ymm1, %ymm1
  vpaddd .LC1(%rip), %ymm0, %ymm0
  vpcmpeqd %ymm4, %ymm4, %ymm4
  shrq $39, %r12
  leal 1(%r12), %esi
  movl %esi, %r10d
  shrl $5, %r10d
  movl %r10d, %r11d
  salq $5, %r11
  leaq (%rcx,%r11), %rdi
.L22:
  vmovdqu %ymm4, 2(%rcx)
  addq $32, %rcx
  vmovdqa %ymm1, %ymm5
  vmovdqa %ymm0, %ymm6
  vpaddq %ymm3, %ymm1, %ymm1
  vpaddd %ymm2, %ymm0, %ymm0
  cmpq %rcx, %rdi
  jne .L22
  movl %r10d, %ecx
  sall $5, %ecx
  cmpl %esi, %ecx
  je .L64
  imull $-8160, %r10d, %r10d
  addq %r11, %rax
  addl %r10d, %edx
.L24:
  subl $255, %edx
  addq $1, %rax
  movb $-1, -1(%rax)
  cmpl $254, %edx
  ja .L24
.L20:
  movb %dl, (%rax)
  leaq 1(%rax), %r14
  movq %rbx, %rsi
  jmp .L25
.L61:
  movl $4, 76(%rsp)
  xorl %r13d, %r13d
  jmp .L8
.L11:
  movb %dil, 1(%r14)
  leaq 2(%r14), %rcx
  jmp .L17
.L31:
  movq %rcx, %rdx
  jmp .L15
.L64:
  vpaddd .LC4(%rip), %ymm6, %ymm6
  vpaddq .LC5(%rip), %ymm5, %ymm5
  vextracti128 $0x1, %ymm6, %xmm0
  vpextrd $3, %xmm0, %edx
  vextracti128 $0x1, %ymm5, %xmm0
  vpextrq $1, %xmm0, %rax
  jmp .L20
.L14:
  vpaddd .LC4(%rip), %ymm6, %ymm6
  vextracti128 $0x1, %ymm6, %xmm0
  vpextrd $3, %xmm0, %edi
  vpaddq .LC5(%rip), %ymm2, %ymm0
  vpaddq .LC6(%rip), %ymm2, %ymm2
  vextracti128 $0x1, %ymm0, %xmm0
  vpextrq $1, %xmm0, %rdx
  vextracti128 $0x1, %ymm2, %xmm0
  vpextrq $1, %xmm0, %rcx
  jmp .L16
.LC0:
.LC1:
.LC4:
.LC5:
.LC6:
.LC8:
