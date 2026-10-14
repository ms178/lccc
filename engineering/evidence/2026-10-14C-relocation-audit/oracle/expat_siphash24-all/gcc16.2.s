sip24_update:
  pushq %rbp
  movq %rdi, %r8
  movq %rsi, %r10
  leaq (%rsi,%rdx), %r11
  pushq %rbx
  leaq 32(%rdi), %rbp
  leaq 40(%rdi), %rbx
.L2:
  movq 40(%r8), %rax
  cmpq %r11, %r10
  jnb .L3
.L12:
  cmpq %rbx, %rax
  jnb .L5
  leaq 1(%rax), %rdx
  addq $1, %r10
  movq %rdx, 40(%r8)
  movzbl -1(%r10), %edx
  movb %dl, (%rax)
  movq 40(%r8), %rax
  cmpq %r11, %r10
  jb .L12
.L3:
  cmpq %rbx, %rax
  jb .L6
.L5:
  vmovdqu 24(%r8), %xmm0
  movq (%r8), %rcx
  movl $2, %edi
  movq 8(%r8), %rdx
  movq 16(%r8), %rsi
  vpsrldq $8, %xmm0, %xmm1
  vpextrq $1, %xmm0, %r9
  vpxor %xmm1, %xmm0, %xmm0
  vmovq %xmm0, %rax
  jmp .L7
.L9:
  movl $1, %edi
.L7:
  addq %rdx, %rcx
  addq %rax, %rsi
  rorx $51, %rdx, %rdx
  rorx $48, %rax, %rax
  xorq %rcx, %rdx
  xorq %rsi, %rax
  rorx $32, %rcx, %rcx
  addq %rdx, %rsi
  addq %rax, %rcx
  rorx $47, %rdx, %rdx
  rorx $43, %rax, %rax
  xorq %rsi, %rdx
  xorq %rcx, %rax
  rorx $32, %rsi, %rsi
  cmpl $1, %edi
  jne .L9
  xorq %rcx, %r9
  addq $8, 48(%r8)
  movq %rax, 24(%r8)
  movq %rdx, 8(%r8)
  movq %rsi, 16(%r8)
  movq %r9, (%r8)
  movq %rbp, 40(%r8)
  cmpq %r11, %r10
  jb .L2
.L6:
  movq %r8, %rax
  popq %rbx
  popq %rbp
  ret
sip24_final:
  movq 40(%rdi), %rax
  movq 48(%rdi), %rsi
  leaq 32(%rdi), %rdx
  subq %rdx, %rax
  addq %rax, %rsi
  salq $56, %rsi
  cmpb $7, %al
  ja .L14
  movzbl %al, %eax
  jmp *.L16(,%rax,8)
.L16:
.L15:
  movzbl 38(%rdi), %eax
  salq $48, %rax
  orq %rax, %rsi
.L17:
  movzbl 37(%rdi), %eax
  salq $40, %rax
  orq %rax, %rsi
.L18:
  movzbl 36(%rdi), %eax
  salq $32, %rax
  orq %rax, %rsi
.L19:
  movzbl 35(%rdi), %eax
  salq $24, %rax
  orq %rax, %rsi
.L20:
  movzbl 34(%rdi), %eax
  salq $16, %rax
  orq %rax, %rsi
.L21:
  movzbl 33(%rdi), %eax
  salq $8, %rax
  orq %rax, %rsi
.L22:
  movzbl 32(%rdi), %eax
  orq %rax, %rsi
.L14:
  movq 24(%rdi), %rcx
  movq (%rdi), %r8
  movl $2, %r9d
  movq 8(%rdi), %rax
  movq 16(%rdi), %rdx
  xorq %rsi, %rcx
  jmp .L23
.L25:
  movl $1, %r9d
.L23:
  addq %rax, %r8
  addq %rcx, %rdx
  rorx $51, %rax, %rax
  rorx $48, %rcx, %rcx
  xorq %r8, %rax
  xorq %rdx, %rcx
  rorx $32, %r8, %r8
  addq %rax, %rdx
  addq %rcx, %r8
  rorx $47, %rax, %rax
  rorx $43, %rcx, %rcx
  xorq %rdx, %rax
  xorq %r8, %rcx
  rorx $32, %rdx, %rdx
  cmpl $1, %r9d
  jne .L25
  xorq %r8, %rsi
  xorb $-1, %dl
  movl $4, %r9d
.L24:
  addq %rcx, %rdx
  addq %rax, %rsi
  rorx $48, %rcx, %rcx
  rorx $51, %rax, %rax
  xorq %rdx, %rcx
  xorq %rsi, %rax
  rorx $32, %rsi, %rsi
  addq %rcx, %rsi
  addq %rax, %rdx
  rorx $43, %rcx, %r8
  rorx $47, %rax, %rax
  movq %rsi, %rcx
  xorq %rdx, %rax
  rorx $32, %rdx, %rdx
  xorq %r8, %rcx
  subl $1, %r9d
  jne .L24
  movq %rax, 8(%rdi)
  xorq %rdx, %rax
  movq %rsi, (%rdi)
  xorq %r8, %rax
  movq %rcx, 24(%rdi)
  movq %rdx, 16(%rdi)
  ret
expat_siphash24:
  pushq %rbp
  movq %rdi, %rax
  movq %rsp, %rbp
  andq $-32, %rsp
  subq $64, %rsp
  vmovq (%rdx), %xmm1
  vpinsrq $1, 8(%rdx), %xmm1, %xmm0
  leaq 32(%rsp), %rdx
  movq %rsp, %rdi
  movq $0, 32(%rsp)
  vinserti128 $1, %xmm0, %ymm0, %ymm0
  movq %rdx, 40(%rsp)
  vpxor .LC0(%rip), %ymm0, %ymm0
  movq %rsi, %rdx
  movq $0, 48(%rsp)
  movq %rax, %rsi
  vmovdqa %ymm0, (%rsp)
  call sip24_update
  movq %rax, %rdi
  call sip24_final
  vzeroupper
  leave
  ret
.LC2:
main:
  movabsq $1084818905618843912, %rax
  pushq %rbp
  movq %rsp, %rbp
  pushq %r15
  pushq %r14
  pushq %r13
  pushq %r12
  pushq %rbx
  andq $-32, %rsp
  subq $320, %rsp
  movq %rax, 24(%rsp)
  leaq 176(%rsp), %rdx
  movl $376, %eax
.L30:
  imull $1664525, %eax, %eax
  addq $1, %rdx
  leaq 320(%rsp), %rbx
  addl $1013904223, %eax
  movl %eax, %ecx
  shrl $24, %ecx
  movb %cl, -1(%rdx)
  cmpq %rdx, %rbx
  jne .L30
  xorl %r12d, %r12d
  leaq 64(%rsp), %rbx
  jmp .L32
.L48:
  addq $1, %r12
  cmpq $64, %r12
  je .L47
.L32:
  movb %r12b, 96(%rsp,%r12)
  vmovdqa .LC1(%rip), %ymm0
  movq %r12, %rdx
  leaq 96(%rsp), %rsi
  leaq 32(%rsp), %rdi
  movq %rbx, 72(%rsp)
  movq $0, 64(%rsp)
  movq $0, 80(%rsp)
  vmovdqa %ymm0, 32(%rsp)
  call sip24_update
  movq %rax, %rdi
  call sip24_final
  movzbl vectors.0+2(,%r12,8), %edx
  movq %rax, %rcx
  movzbl vectors.0+1(,%r12,8), %eax
  salq $16, %rdx
  salq $8, %rax
  orq %rdx, %rax
  movzbl vectors.0(,%r12,8), %edx
  orq %rdx, %rax
  movzbl vectors.0+3(,%r12,8), %edx
  salq $24, %rdx
  orq %rdx, %rax
  movzbl vectors.0+4(,%r12,8), %edx
  salq $32, %rdx
  orq %rdx, %rax
  movzbl vectors.0+5(,%r12,8), %edx
  salq $40, %rdx
  orq %rdx, %rax
  movzbl vectors.0+6(,%r12,8), %edx
  salq $48, %rdx
  orq %rdx, %rax
  movzbl vectors.0+7(,%r12,8), %edx
  salq $56, %rdx
  orq %rdx, %rax
  cmpq %rcx, %rax
  je .L48
.L39:
  movl $1, %eax
  vzeroupper
.L29:
  leaq -40(%rbp), %rsp
  popq %rbx
  popq %r12
  popq %r13
  popq %r14
  popq %r15
  popq %rbp
  ret
.L47:
  movl $1, %r12d
  leaq 128(%rsp), %r14
.L37:
  movabsq $506097522914230528, %rax
  leaq -1(%r12), %rbx
  leaq 16(%rsp), %rdx
  movq %rax, 16(%rsp)
  movq %rbx, %rsi
  leaq 176(%rsp), %rdi
  vzeroupper
  call expat_siphash24
  xorl %r15d, %r15d
  movq %rax, %r13
.L36:
  vmovdqa .LC1(%rip), %ymm1
  movq %r15, %rdx
  movq $0, 128(%rsp)
  leaq 176(%rsp), %rsi
  movq %r14, 136(%rsp)
  leaq 96(%rsp), %rdi
  movq $0, 144(%rsp)
  vmovdqa %ymm1, 96(%rsp)
  call sip24_update
  movq %rbx, %rdx
  leaq 96(%rsp), %rdi
  leaq 176(%rsp,%r15), %rsi
  subq %r15, %rdx
  call sip24_update
  leaq 96(%rsp), %rdi
  call sip24_final
  cmpq %rax, %r13
  jne .L39
  addq $1, %r15
  cmpq %r12, %r15
  jne .L36
  leaq 1(%r15), %r12
  cmpq $129, %r15
  jne .L37
  movabsq $506097522914230528, %r13
  xorl %r12d, %r12d
  movabsq $-7046029254386353131, %rbx
  vzeroupper
.L38:
  movq %rbx, %rax
  movq %rbx, %rsi
  movq %rbx, %rdi
  movq %rbx, %rdx
  shrq $32, %rsi
  orq $1, %rax
  andl $7, %edi
  shrq $48, %rdx
  addq %rax, %r13
  movl %r12d, %eax
  andl $127, %esi
  addl $1, %r12d
  andl $127, %eax
  leaq 176(%rsp,%rdi), %rdi
  movq %r13, 16(%rsp)
  xorb %dl, 176(%rsp,%rax)
  leaq 16(%rsp), %rdx
  call expat_siphash24
  xorq %rax, %rbx
  cmpl $2000000, %r12d
  jne .L38
  movq %rbx, %rsi
  movl $.LC2, %edi
  xorl %eax, %eax
  call printf
  xorl %eax, %eax
  jmp .L29
vectors.0:
.LC0:
.LC1:
