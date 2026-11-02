sum_fields.constprop.0:
  pushq %rbx
  movl %edi, %r11d
  xorl %r8d, %r8d
  xorl %ebx, %ebx
  xorl %edx, %edx
  xorl %edi, %edi
  xorl %r10d, %r10d
  movl $buf.0, %ecx
  jmp .L3
.L7:
  cmpb $10, %al
  je .L17
  testb %al, %al
  je .L1
  movzbl 1(%rcx), %edx
  addq $1, %rcx
  leal -48(%rdx), %esi
  cmpb $9, %sil
  ja .L15
  xorl %ebx, %ebx
  cmpb $45, %al
  movl %edx, %eax
  sete %bl
  xorl %edx, %edx
.L8:
  subl $48, %eax
  leaq (%rdx,%rdx,4), %rdx
  addq $1, %rcx
  movl $1, %r8d
  movsbq %al, %rax
  leaq (%rax,%rdx,2), %rdx
.L3:
  movzbl (%rcx), %eax
  cmpl %r11d, %edi
  sete %r9b
  leal -48(%rax), %esi
  cmpb $9, %sil
  jbe .L8
  testl %r8d, %r8d
  je .L6
  testb %r9b, %r9b
  je .L6
  movq %rdx, %rsi
  negq %rsi
  testl %ebx, %ebx
  cmovne %rsi, %rdx
  addq %rdx, %r10
.L6:
  cmpb $44, %al
  jne .L7
  movzbl 1(%rcx), %eax
  addq $1, %rcx
  addl $1, %edi
  leal -48(%rax), %edx
  cmpb $9, %dl
  ja .L6
  xorl %ebx, %ebx
  xorl %edx, %edx
  jmp .L8
.L17:
  testb %al, %al
  je .L1
  movzbl 1(%rcx), %eax
  addq $1, %rcx
  leal -48(%rax), %edx
  cmpb $9, %dl
  jbe .L16
  xorl %edi, %edi
  jmp .L6
.L15:
  movl %edx, %eax
  jmp .L6
.L16:
  xorl %ebx, %ebx
  xorl %edi, %edi
  xorl %edx, %edx
  jmp .L8
.L1:
  movq %r10, %rax
  popq %rbx
  ret
.LC1:
main:
  pushq %r12
  xorl %ecx, %ecx
  movl $3435973837, %edi
  pushq %rbp
  pushq %rbx
  xorl %ebx, %ebx
  subq $16, %rsp
  vmovdqa .LC0(%rip), %xmm1
  leaq 8(%rsp), %r11
.L30:
  movl %ebx, %r9d
  xorl %r10d, %r10d
  movl $500, %ebp
.L40:
  movl %r9d, %edx
  imulq $274877907, %rdx, %rdx
  shrq $38, %rdx
  imull $1000, %edx, %eax
  movl %r9d, %edx
  subl %eax, %edx
  movl %edx, %eax
  subl $500, %eax
  js .L45
  jne .L41
  movb $48, 8(%rsp)
  movl %ecx, %r8d
  movl $1, %esi
  movl $1, %eax
.L33:
  leal -1(%rax), %r12d
  movslq %ecx, %rcx
  leaq (%r12,%r11), %rdx
  leaq buf.0(%rcx), %rax
  leaq buf.0+1(%rcx,%r12), %r12
.L37:
  movzbl (%rdx), %ecx
  addq $1, %rax
  subq $1, %rdx
  movb %cl, -1(%rax)
  cmpq %r12, %rax
  jne .L37
  addl $1, %r10d
  addl %esi, %r8d
  cmpl $6, %r10d
  je .L46
.L38:
  leal 1(%r8), %ecx
  movslq %r8d, %r8
  addl $17, %r9d
  movb $44, buf.0(%r8)
  jmp .L40
.L45:
  leal 1(%rcx), %r8d
  movslq %ecx, %rcx
  movl %ebp, %eax
  movb $45, buf.0(%rcx)
  subl %edx, %eax
.L32:
  movl $1, %ecx
.L34:
  movl %eax, %edx
  imulq %rdi, %rdx
  shrq $35, %rdx
  leal (%rdx,%rdx,4), %esi
  addl %esi, %esi
  subl %esi, %eax
  addl $48, %eax
  movb %al, 7(%rsp,%rcx)
  movl %edx, %eax
  movq %rcx, %rdx
  addq $1, %rcx
  testl %eax, %eax
  jne .L34
  movl %edx, %eax
  cmpl $8, %edx
  jne .L35
  vmovq 8(%rsp), %xmm0
  movslq %r8d, %rax
  movl $8, %esi
  addl $1, %r10d
  addl %esi, %r8d
  vpshufb %xmm1, %xmm0, %xmm0
  vmovq %xmm0, buf.0(%rax)
  cmpl $6, %r10d
  jne .L38
.L46:
  leal 1(%r8), %ecx
  addl $131, %ebx
  movslq %r8d, %r8
  movb $10, buf.0(%r8)
  cmpl $8384, %ebx
  jne .L30
  movslq %ecx, %rcx
  movl $5, %edi
  movb $0, buf.0(%rcx)
  call sum_fields.constprop.0
  movl $3, %edi
  movq %rax, %rbp
  call sum_fields.constprop.0
  xorl %edi, %edi
  movq %rax, %rbx
  call sum_fields.constprop.0
  movq %rbp, %rcx
  movq %rbx, %rdx
  movl $.LC1, %edi
  movq %rax, %rsi
  xorl %eax, %eax
  call printf
  addq $16, %rsp
  xorl %eax, %eax
  popq %rbx
  popq %rbp
  popq %r12
  ret
.L41:
  movl %ecx, %r8d
  jmp .L32
.L35:
  movl %edx, %esi
  movl %r8d, %ecx
  jmp .L33
.LC0:
