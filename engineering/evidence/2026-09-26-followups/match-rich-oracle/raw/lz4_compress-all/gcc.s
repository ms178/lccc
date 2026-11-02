.LC7:
main:
  pushq %rbp
  movl $-128, %ecx
  xorl %eax, %eax
  movl $1511734397, %edx
  movq %rsp, %rbp
  pushq %r15
  pushq %r14
  pushq %r13
  pushq %r12
  pushq %rbx
  andq $-32, %rsp
  subq $96, %rsp
  jmp .L8
.L55:
  movl %ecx, %esi
  movzbl src_data(%rsi), %esi
.L4:
  movb %sil, src_data(%rax)
  addq $1, %rax
  cmpq $524288, %rax
  je .L34
  addl $1, %ecx
.L8:
  imull $1664525, %edx, %edx
  addl $1013904223, %edx
  cmpq $127, %rax
  jbe .L2
  cmpb $95, %al
  jbe .L55
  movl %edx, %edi
  movl %edx, %esi
  andl $15, %edi
  shrl $24, %esi
  cmpl $5, %edi
  ja .L4
  movl %edx, %esi
  andl $63, %esi
  addl %ecx, %esi
  movl %esi, %esi
  movzbl src_data(%rsi), %esi
  jmp .L4
.L34:
  xorl %r12d, %r12d
  xorl %r13d, %r13d
.L7:
  xorl %esi, %esi
  movl $65536, %edx
  movl $hash_table, %edi
  movl $dst_data, %r14d
  call memset
  movl $src_data+1, %ebx
  movq %r13, %r8
  movl %r12d, %r9d
  movl $src_data, %esi
.L29:
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
  jnb .L9
  cmpq $src_data, %rdx
  jb .L9
  cmpl src_data(%rcx), %edi
  je .L56
.L9:
  sarq $6, %rax
  leaq 1(%rbx,%rax), %rbx
.L28:
  cmpq $src_data+524276, %rbx
  jb .L29
  movl $src_data+524288, %ebx
  movq %r8, %r13
  leaq 1(%r14), %rcx
  movl %r9d, %r12d
  subq %rsi, %rbx
  cmpl $14, %ebx
  jbe .L30
  movb $-16, (%r14)
.L31:
  movq %rcx, %rdi
  movl %ebx, %edx
  vzeroupper
  call memcpy
  movq %rax, %rcx
  leal -1(%rbx), %eax
  leaq 1(%rcx,%rax), %rcx
.L32:
  subq $dst_data, %rcx
  movzbl dst_data(%rip), %eax
  movq %rcx, %rdx
  shrl %ecx
  salq $32, %rdx
  xorq %rdx, %rax
  movzbl dst_data(%rcx), %edx
  salq $16, %rdx
  xorq %rdx, %rax
  addq %rax, %r13
  movl %r12d, %eax
  addl $4099, %r12d
  andl $524287, %eax
  xorb $85, src_data(%rax)
  cmpl $8394752, %r12d
  jne .L7
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
.L56:
  addq $4, %rbx
  leaq 4(%rdx), %r15
  cmpq $src_data+524288, %rbx
  jb .L10
  jmp .L57
.L12:
  addq $1, %rbx
  addq $1, %r15
  cmpq $src_data+524288, %rbx
  je .L53
.L10:
  movzbl (%r15), %edi
  cmpb %dil, (%rbx)
  je .L12
.L53:
  movq %r15, %r13
  subq %rdx, %r13
  movl %r13d, 80(%rsp)
  subl $4, %r13d
.L11:
  movl $-8160, %edx
  leaq 1(%r14), %rcx
  vmovd %edx, %xmm7
  vpbroadcastd %xmm7, %ymm7
  vmovdqa %ymm7, (%rsp)
  vpbroadcastq .LC8(%rip), %ymm7
  vmovdqa %ymm7, 32(%rsp)
  cmpl $14, %eax
  ja .L58
  movl %eax, %edx
  sall $4, %edx
  movb %dl, (%r14)
  testl %eax, %eax
  je .L21
.L20:
  movl %r9d, 84(%rsp)
  movl %eax, %r12d
  movq %rcx, %rdi
  movq %r8, 88(%rsp)
  movq %r12, %rdx
  vzeroupper
  call memcpy
  movl 84(%rsp), %r9d
  movq 88(%rsp), %r8
  leaq (%rax,%r12), %rcx
.L21:
  movq %rbx, %rdx
  leaq 2(%rcx), %rax
  subq %r15, %rdx
  movw %dx, (%rcx)
  movzbl (%r14), %edx
  cmpl $14, %r13d
  ja .L59
  orl %r13d, %edx
  movq %rbx, %rsi
  movb %dl, (%r14)
  movq %rax, %r14
  jmp .L28
.L30:
  movl %ebx, %eax
  sall $4, %eax
  movb %al, (%r14)
  testl %ebx, %ebx
  jne .L31
  vzeroupper
  jmp .L32
.L58:
  leal -15(%rax), %edi
  movb $-16, (%r14)
  cmpl $254, %edi
  jbe .L14
  leal -270(%rax), %edx
  cmpl $7904, %edx
  jbe .L35
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
  leaq (%r10,%r14), %r10
.L16:
  vmovdqu %ymm5, 1(%rdx)
  addq $32, %rdx
  vmovdqa %ymm1, %ymm2
  vmovdqa %ymm0, %ymm6
  vpaddq %ymm4, %ymm1, %ymm1
  vpaddd %ymm3, %ymm0, %ymm0
  cmpq %rdx, %r10
  jne .L16
  movl %r11d, %edx
  sall $5, %edx
  cmpl %edx, 88(%rsp)
  je .L17
  imull $-8160, %r11d, %r11d
  leaq (%rcx,%r12), %rdx
  addl %r11d, %edi
.L18:
  movq %rdx, %rcx
  subl $255, %edi
  addq $1, %rdx
  movb $-1, -1(%rdx)
  cmpl $254, %edi
  ja .L18
.L19:
  movb %dil, (%rdx)
  addq $2, %rcx
  jmp .L20
.L59:
  movl 80(%rsp), %esi
  orl $15, %edx
  movb %dl, (%r14)
  leal -19(%rsi), %edx
  cmpl $254, %edx
  jbe .L23
  leal -274(%rsi), %r12d
  cmpl $7904, %r12d
  jbe .L27
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
.L25:
  vmovdqu %ymm4, 2(%rcx)
  addq $32, %rcx
  vmovdqa %ymm1, %ymm5
  vmovdqa %ymm0, %ymm6
  vpaddq %ymm3, %ymm1, %ymm1
  vpaddd %ymm2, %ymm0, %ymm0
  cmpq %rcx, %rdi
  jne .L25
  movl %r10d, %ecx
  sall $5, %ecx
  cmpl %ecx, %esi
  je .L60
  imull $-8160, %r10d, %r10d
  addq %r11, %rax
  addl %r10d, %edx
.L27:
  subl $255, %edx
  addq $1, %rax
  movb $-1, -1(%rax)
  cmpl $254, %edx
  ja .L27
.L23:
  movb %dl, (%rax)
  leaq 1(%rax), %r14
  movq %rbx, %rsi
  jmp .L28
.L57:
  movl $4, 80(%rsp)
  xorl %r13d, %r13d
  jmp .L11
.L14:
  movb %dil, 1(%r14)
  leaq 2(%r14), %rcx
  jmp .L20
.L35:
  movq %rcx, %rdx
  jmp .L18
.L17:
  vpaddd .LC4(%rip), %ymm6, %ymm6
  vextracti128 $0x1, %ymm6, %xmm0
  vpextrd $3, %xmm0, %edi
  vpaddq .LC5(%rip), %ymm2, %ymm0
  vpaddq .LC6(%rip), %ymm2, %ymm2
  vextracti128 $0x1, %ymm0, %xmm0
  vpextrq $1, %xmm0, %rdx
  vextracti128 $0x1, %ymm2, %xmm0
  vpextrq $1, %xmm0, %rcx
  jmp .L19
.L60:
  vpaddd .LC4(%rip), %ymm6, %ymm6
  vpaddq .LC5(%rip), %ymm5, %ymm5
  vextracti128 $0x1, %ymm6, %xmm0
  vpextrd $3, %xmm0, %edx
  vextracti128 $0x1, %ymm5, %xmm0
  vpextrq $1, %xmm0, %rax
  jmp .L23
.L2:
  movl %edx, %esi
  addq $1, %rax
  addl $1, %ecx
  shrl $24, %esi
  movb %sil, src_data-1(%rax)
  jmp .L8
.LC0:
.LC1:
.LC4:
.LC5:
.LC6:
.LC8:
