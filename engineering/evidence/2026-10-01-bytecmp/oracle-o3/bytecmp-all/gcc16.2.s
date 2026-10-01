bc_scan:
  movq %rdi, %r9
  movq %rsi, %r8
  movq %rdx, %rcx
  cmpq %rsi, %rdi
  jnb .L13
  leaq -1(%rsi), %rsi
  subq %rdi, %rsi
  cmpq $61, %rsi
  jbe .L3
  movq %rdi, %rax
  xorq %rdx, %rax
  testb $31, %al
  jne .L3
  movq %rdx, %rax
  negq %rax
  andl $31, %eax
  movq %rax, %rdx
  leaq 32(%rax), %rax
  cmpq %rax, %rsi
  jb .L15
  testq %rdx, %rdx
  je .L16
  leaq (%rdi,%rdx), %rdi
  movq %rcx, %rsi
  movq %r9, %rax
  movq %rdi, %r10
  jmp .L11
.L23:
  addq $1, %rax
  addq $1, %rsi
  cmpq %rdi, %rax
  je .L10
.L11:
  movzbl (%rsi), %r11d
  cmpb %r11b, (%rax)
  je .L23
  ret
.L16:
  movq %rcx, %rsi
  movq %rdi, %r10
.L10:
  movq %r8, %rax
  vmovq %r10, %xmm4
  vpxor %xmm3, %xmm3, %xmm3
  vpbroadcastq .LC4(%rip), %ymm2
  subq %r9, %rax
  vpbroadcastq %xmm4, %ymm1
  vpaddq .LC2(%rip), %ymm1, %ymm1
  subq %rdx, %rax
  movq %rax, %r9
  leaq (%rcx,%rdx), %rax
  xorl %edx, %edx
  movq %r9, %rcx
  andq $-32, %rcx
  jmp .L5
.L12:
  addq $32, %rdx
  cmpq %rcx, %rdx
  je .L24
  vpaddq %ymm2, %ymm1, %ymm1
.L5:
  vmovdqa (%rdi,%rdx), %ymm0
  vpcmpeqb (%rax,%rdx), %ymm0, %ymm0
  vpcmpeqb %ymm3, %ymm0, %ymm0
  vptest %ymm0, %ymm0
  je .L12
  leaq (%r10,%rdx), %rax
  leaq (%rsi,%rdx), %rcx
  vzeroupper
  jmp .L7
.L26:
  addq $1, %rax
  addq $1, %rcx
  cmpq %r8, %rax
  jnb .L25
.L7:
  movzbl (%rcx), %edi
  cmpb %dil, (%rax)
  je .L26
  ret
.L25:
  ret
.L24:
  leaq (%r10,%rdx), %rax
  leaq (%rsi,%rdx), %rcx
  cmpq %rdx, %r9
  je .L27
  vzeroupper
  jmp .L7
.L3:
  movq %r9, %rax
  jmp .L8
.L29:
  addq $1, %rax
  addq $1, %rcx
  cmpq %rax, %r8
  je .L28
.L8:
  movzbl (%rcx), %esi
  cmpb %sil, (%rax)
  je .L29
  ret
.L28:
  ret
.L15:
  movq %r9, %rax
  jmp .L7
.L13:
  movq %rdi, %rax
  ret
.L27:
  vpbroadcastq .LC3(%rip), %ymm0
  vpaddq %ymm0, %ymm1, %ymm1
  vextracti128 $0x1, %ymm1, %xmm0
  vpextrq $1, %xmm0, %rax
  vzeroupper
  ret
bc_scan_signed:
  movq %rdi, %r9
  movq %rsi, %r8
  movq %rdx, %rcx
  cmpq %rsi, %rdi
  jnb .L42
  leaq -1(%rsi), %rsi
  subq %rdi, %rsi
  cmpq $61, %rsi
  jbe .L32
  movq %rdi, %rax
  xorq %rdx, %rax
  testb $31, %al
  jne .L32
  movq %rdx, %rax
  negq %rax
  andl $31, %eax
  movq %rax, %rdx
  leaq 32(%rax), %rax
  cmpq %rax, %rsi
  jb .L44
  testq %rdx, %rdx
  je .L45
  leaq (%rdi,%rdx), %rdi
  movq %rcx, %rsi
  movq %r9, %rax
  movq %rdi, %r10
  jmp .L40
.L51:
  addq $1, %rax
  addq $1, %rsi
  cmpq %rdi, %rax
  je .L39
.L40:
  movzbl (%rsi), %r11d
  cmpb %r11b, (%rax)
  je .L51
  ret
.L45:
  movq %rcx, %rsi
  movq %rdi, %r10
.L39:
  movq %r8, %rax
  vmovq %r10, %xmm4
  vpxor %xmm3, %xmm3, %xmm3
  vpbroadcastq .LC4(%rip), %ymm2
  subq %r9, %rax
  vpbroadcastq %xmm4, %ymm1
  vpaddq .LC2(%rip), %ymm1, %ymm1
  subq %rdx, %rax
  movq %rax, %r9
  leaq (%rcx,%rdx), %rax
  xorl %edx, %edx
  movq %r9, %rcx
  andq $-32, %rcx
  jmp .L34
.L41:
  addq $32, %rdx
  cmpq %rcx, %rdx
  je .L52
  vpaddq %ymm2, %ymm1, %ymm1
.L34:
  vmovdqa (%rdi,%rdx), %ymm0
  vpcmpeqb (%rax,%rdx), %ymm0, %ymm0
  vpcmpeqb %ymm3, %ymm0, %ymm0
  vptest %ymm0, %ymm0
  je .L41
  leaq (%r10,%rdx), %rax
  leaq (%rsi,%rdx), %rcx
  vzeroupper
  jmp .L36
.L54:
  addq $1, %rax
  addq $1, %rcx
  cmpq %r8, %rax
  jnb .L53
.L36:
  movzbl (%rcx), %edi
  cmpb %dil, (%rax)
  je .L54
  ret
.L53:
  ret
.L52:
  leaq (%r10,%rdx), %rax
  leaq (%rsi,%rdx), %rcx
  cmpq %rdx, %r9
  je .L55
  vzeroupper
  jmp .L36
.L32:
  movq %r9, %rax
  jmp .L37
.L57:
  addq $1, %rax
  addq $1, %rcx
  cmpq %rax, %r8
  je .L56
.L37:
  movzbl (%rcx), %esi
  cmpb %sil, (%rax)
  je .L57
  ret
.L56:
  ret
.L44:
  movq %r9, %rax
  jmp .L36
.L42:
  movq %rdi, %rax
  ret
.L55:
  vpbroadcastq .LC3(%rip), %ymm0
  vpaddq %ymm0, %ymm1, %ymm1
  vextracti128 $0x1, %ymm1, %xmm0
  vpextrq $1, %xmm0, %rax
  vzeroupper
  ret
match_extend:
  pushq %rbp
  leaq 8(%rdi), %rax
  movq %rsi, %rcx
  vpxor %xmm5, %xmm5, %xmm5
  movq %rdi, %r8
  movq %rdx, %rsi
  movq %rsp, %rbp
  subq $16, %rsp
  vpbroadcastq .LC4(%rip), %ymm4
  movq %rbx, -16(%rbp)
  cmpq %rax, %rdx
  jb .L59
  leaq -8(%rdx), %rdi
  subq %r8, %rdi
  cmpq $79, %rdi
  jbe .L60
  movq %r8, %rdx
  xorq %rcx, %rdx
  andl $31, %edx
  jne .L60
  movq %rcx, %r10
  shrq $3, %r10
  negq %r10
  movq %r10, %rdx
  andl $3, %edx
  je .L96
  movq (%rcx), %rbx
  cmpq %rbx, (%r8)
  jne .L59
  andl $2, %r10d
  leaq 8(%rcx), %r9
  je .L97
  movq 8(%rcx), %rbx
  cmpq %rbx, 8(%r8)
  jne .L98
  leaq 16(%rcx), %r9
  leaq 24(%r8), %r10
  cmpq $3, %rdx
  jne .L99
  movq 16(%rcx), %rax
  cmpq %rax, 16(%r8)
  jne .L100
  movq %r14, -8(%rbp)
  leaq 24(%rcx), %r9
  leaq 32(%r8), %rax
.L80:
  shrq $3, %rdi
  vmovq %r9, %xmm7
  vmovdqa .LC6(%rip), %ymm0
  vmovdqa %ymm4, %ymm3
  subq %rdx, %rdi
  vpbroadcastq %xmm7, %ymm1
  vmovq %rax, %xmm7
  salq $3, %rdx
  addq $1, %rdi
  vpbroadcastq %xmm7, %ymm2
  leaq (%r8,%rdx), %r11
  addq %rcx, %rdx
  movq %rdi, %rbx
  vpaddq %ymm0, %ymm1, %ymm1
  vpaddq %ymm0, %ymm2, %ymm2
  xorl %ecx, %ecx
  shrq $2, %rbx
  vmovdqa %ymm5, %ymm7
  movq %rbx, %r14
  movq %rdi, %rbx
  andq $-4, %rbx
  jmp .L62
.L82:
  addq $4, %rcx
  cmpq %rbx, %rcx
  je .L112
  vpaddq %ymm3, %ymm2, %ymm2
  vpaddq %ymm3, %ymm1, %ymm1
.L62:
  vmovdqa (%r11,%rcx,8), %ymm0
  vpcmpeqq (%rdx,%rcx,8), %ymm0, %ymm0
  vpcmpeqq %ymm7, %ymm0, %ymm0
  vptest %ymm0, %ymm0
  je .L82
  salq $3, %rcx
  leaq (%rax,%rcx), %rdx
  leaq (%r10,%rcx), %rax
  addq %r9, %rcx
.L83:
  movq (%rax), %rdi
  cmpq %rdi, (%rcx)
  jne .L110
  leaq 8(%rdx), %rax
  leaq 8(%rcx), %rdi
  cmpq %rax, %rsi
  jb .L65
  movq (%rdx), %rbx
  cmpq %rbx, 8(%rcx)
  jne .L65
  leaq 16(%rdx), %rdi
  leaq 16(%rcx), %r9
  cmpq %rdi, %rsi
  jb .L71
  movq 16(%rcx), %rbx
  cmpq %rbx, 8(%rdx)
  jne .L71
  leaq 24(%rdx), %rax
  leaq 24(%rcx), %r9
  cmpq %rax, %rsi
  jb .L77
  movq 16(%rdx), %rbx
  cmpq %rbx, 24(%rcx)
  jne .L77
  leaq 32(%rdx), %rdi
  leaq 32(%rcx), %r9
  cmpq %rdi, %rsi
  jb .L71
  movq 32(%rcx), %rbx
  cmpq %rbx, 24(%rdx)
  jne .L71
  leaq 40(%rdx), %rax
  leaq 40(%rcx), %r9
  cmpq %rax, %rsi
  jb .L77
  movq 40(%rcx), %rbx
  cmpq %rbx, 32(%rdx)
  jne .L77
  leaq 48(%rdx), %rdi
  leaq 48(%rcx), %r9
  cmpq %rdi, %rsi
  jb .L71
  movq 40(%rdx), %rbx
  cmpq %rbx, 48(%rcx)
  jne .L71
  leaq 56(%rdx), %rax
  leaq 56(%rcx), %r9
  cmpq %rax, %rsi
  jb .L77
  movq 56(%rcx), %rbx
  cmpq %rbx, 48(%rdx)
  jne .L77
  movq -8(%rbp), %r14
  addq $64, %rcx
.L64:
  cmpq %rsi, %rax
  jnb .L84
  movq %rsi, %r10
  subq %rax, %r10
  leaq -1(%r10), %rdx
  cmpq $61, %rdx
  jbe .L90
  movq %rcx, %rdi
  xorq %rax, %rdi
  andl $31, %edi
  jne .L90
  movq %rax, %r9
  negq %r9
  andl $31, %r9d
  leaq 32(%r9), %rdi
  cmpq %rdi, %rdx
  jb .L89
  testq %r9, %r9
  je .L104
  leaq (%rax,%r9), %rdi
  movq %rcx, %rdx
  movq %rdi, %r11
  jmp .L93
.L113:
  addq $1, %rax
  addq $1, %rdx
  cmpq %rdi, %rax
  je .L92
.L93:
  movzbl (%rdx), %ebx
  cmpb %bl, (%rax)
  je .L113
.L84:
  subq %r8, %rax
  vzeroupper
  movq -16(%rbp), %rbx
  leave
  ret
.L114:
  addq $1, %rax
  addq $1, %rcx
  cmpq %rsi, %rax
  jnb .L84
.L89:
  movzbl (%rcx), %ebx
  cmpb %bl, (%rax)
  je .L114
  subq %r8, %rax
  vzeroupper
  movq -16(%rbp), %rbx
  leave
  ret
.L115:
  addq $1, %rax
  addq $1, %rcx
  cmpq %rax, %rsi
  je .L84
.L90:
  movzbl (%rcx), %ebx
  cmpb %bl, (%rax)
  je .L115
  jmp .L84
.L104:
  movq %rcx, %rdx
  movq %rax, %r11
  movq %rax, %rdi
.L92:
  subq %r9, %r10
  leaq (%rcx,%r9), %rax
  vmovq %r11, %xmm7
  xorl %ecx, %ecx
  movq %r10, %r9
  vpbroadcastq %xmm7, %ymm1
  vpaddq .LC2(%rip), %ymm1, %ymm1
  andq $-32, %r9
  jmp .L87
.L94:
  addq $32, %rcx
  cmpq %rcx, %r9
  je .L116
  vpaddq %ymm4, %ymm1, %ymm1
.L87:
  vmovdqa (%rdi,%rcx), %ymm0
  vpcmpeqb (%rax,%rcx), %ymm0, %ymm0
  vpcmpeqb %ymm5, %ymm0, %ymm0
  vptest %ymm0, %ymm0
  je .L94
  leaq (%r11,%rcx), %rax
  addq %rdx, %rcx
  jmp .L89
.L112:
  cmpq %rdi, %rcx
  je .L117
  movq %r14, %rcx
  salq $5, %rcx
  leaq (%rax,%rcx), %rdx
  leaq (%r10,%rcx), %rax
  addq %r9, %rcx
  jmp .L83
.L116:
  leaq (%r11,%r9), %rax
  leaq (%rdx,%r9), %rcx
  cmpq %r10, %r9
  jne .L89
  vpbroadcastq .LC3(%rip), %ymm0
  vpaddq %ymm0, %ymm1, %ymm1
  vextracti128 $0x1, %ymm1, %xmm0
  vpextrq $1, %xmm0, %rax
  jmp .L84
.L59:
  movq %r8, %rax
  jmp .L64
.L60:
  leaq 8(%r8), %rdx
  jmp .L79
.L78:
  addq $8, %rdx
  addq $8, %rcx
  cmpq %rdx, %rsi
  jb .L64
.L79:
  movq (%rcx), %rdi
  movq %rdx, %rax
  cmpq %rdi, -8(%rdx)
  je .L78
  subq $8, %rax
  jmp .L64
.L96:
  movq %r14, -8(%rbp)
  movq %rcx, %r9
  movq %r8, %r10
  jmp .L80
.L71:
  movq -8(%rbp), %r14
  movq %r9, %rcx
  jmp .L64
.L77:
  movq -8(%rbp), %r14
  movq %r9, %rcx
  movq %rdi, %rax
  jmp .L64
.L117:
  vpbroadcastq .LC7(%rip), %ymm0
  vextracti128 $0x1, %ymm2, %xmm2
  movq -8(%rbp), %r14
  vpextrq $1, %xmm2, %rax
  vpaddq %ymm0, %ymm1, %ymm1
  vextracti128 $0x1, %ymm1, %xmm0
  vpextrq $1, %xmm0, %rcx
  jmp .L64
.L65:
  movq -8(%rbp), %r14
  movq %rdi, %rcx
  movq %rdx, %rax
  jmp .L64
.L110:
  movq -8(%rbp), %r14
  jmp .L64
.L97:
  movq %rax, %r10
  movq %r14, -8(%rbp)
  leaq 16(%r8), %rax
  jmp .L80
.L98:
  movq %r9, %rcx
  jmp .L64
.L99:
  movq %r10, %rax
  movq %r14, -8(%rbp)
  leaq 16(%r8), %r10
  jmp .L80
.L100:
  movq %r9, %rcx
  leaq 16(%r8), %rax
  jmp .L64
.LC2:
.LC3:
.LC4:
.LC6:
