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
.LC0:
main:
  pushq %rbp
  movl $102, %edi
  xorl %r10d, %r10d
  movl $3435973837, %r8d
  pushq %rbx
  movl $500, %r9d
  subq $8, %rsp
.L30:
  leal -102(%rdi), %edx
  movq %rdx, %rcx
  imulq $274877907, %rdx, %rdx
  shrq $38, %rdx
  imull $1000, %edx, %esi
  subl %esi, %ecx
  movl %ecx, %edx
  subl $500, %ecx
  js .L49
  jne .L38
  movb $48, %al
  movslq %r10d, %rdx
  movl $1, %esi
  movb %al, buf.0(%rdx)
.L39:
  addl %r10d, %esi
  leal -85(%rdi), %r10d
.L36:
  movslq %esi, %rdx
  leal 1(%rsi), %ecx
  movb $44, buf.0(%rdx)
  movl %r10d, %edx
  imulq $274877907, %rdx, %rdx
  shrq $38, %rdx
  imull $1000, %edx, %r11d
  movl %r10d, %edx
  subl %r11d, %edx
  movl %edx, %r11d
  subl $500, %r11d
  js .L50
  jne .L32
  movslq %ecx, %rdx
  movb $48, %al
  movb %al, buf.0(%rdx)
  movl $1, %edx
.L33:
  addl %edx, %ecx
  addl $17, %r10d
  movl %ecx, %esi
  cmpl %r10d, %edi
  jne .L36
  leal 1(%rcx), %r10d
  addl $131, %edi
  movslq %ecx, %rcx
  movb $10, buf.0(%rcx)
  cmpl $8486, %edi
  jne .L30
  movslq %r10d, %r10
  movl $5, %edi
  movb $0, buf.0(%r10)
  call sum_fields.constprop.0
  movl $3, %edi
  movq %rax, %rbp
  call sum_fields.constprop.0
  xorl %edi, %edi
  movq %rax, %rbx
  call sum_fields.constprop.0
  movq %rbp, %rcx
  movq %rbx, %rdx
  movl $.LC0, %edi
  movq %rax, %rsi
  xorl %eax, %eax
  call printf
  addq $8, %rsp
  xorl %eax, %eax
  popq %rbx
  popq %rbp
  ret
.L50:
  movslq %ecx, %rcx
  movl %r9d, %r11d
  movb $45, buf.0(%rcx)
  subl %edx, %r11d
  leal 2(%rsi), %ecx
.L32:
  movl %r11d, %edx
  imulq %r8, %rdx
  shrq $35, %rdx
  leal (%rdx,%rdx,4), %esi
  addl %esi, %esi
  subl %esi, %r11d
  movb %r11b, %al
  addb $48, %al
  testl %edx, %edx
  je .L34
  movl %edx, %esi
  imulq %r8, %rsi
  shrq $35, %rsi
  leal (%rsi,%rsi,4), %r11d
  addl %r11d, %r11d
  subl %r11d, %edx
  addl $48, %edx
  movb %dl, %ah
  testl %esi, %esi
  je .L35
  addl $48, %esi
  andq $-16711681, %rax
  movslq %ecx, %rdx
  movzbl %sil, %esi
  salq $16, %rsi
  orq %rsi, %rax
  movl %eax, %esi
  sall $8, %esi
  sarl $24, %esi
  movb %sil, buf.0(%rdx)
  leal 1(%rcx), %edx
  movslq %edx, %rdx
  movb %ah, buf.0(%rdx)
  leal 2(%rcx), %edx
  movslq %edx, %rdx
  movb %al, buf.0(%rdx)
  movl $3, %edx
  jmp .L33
.L35:
  movslq %ecx, %rdx
  movb %ah, buf.0(%rdx)
  leal 1(%rcx), %edx
  movslq %edx, %rdx
  movb %al, buf.0(%rdx)
  movl $2, %edx
  jmp .L33
.L49:
  movslq %r10d, %rcx
  addl $1, %r10d
  movb $45, buf.0(%rcx)
  movl %r9d, %ecx
  subl %edx, %ecx
.L38:
  movl %ecx, %edx
  imulq %r8, %rdx
  shrq $35, %rdx
  leal (%rdx,%rdx,4), %esi
  addl %esi, %esi
  subl %esi, %ecx
  movb %cl, %al
  addb $48, %al
  testl %edx, %edx
  je .L51
  movl %edx, %ecx
  imulq %r8, %rcx
  shrq $35, %rcx
  leal (%rcx,%rcx,4), %esi
  addl %esi, %esi
  subl %esi, %edx
  addl $48, %edx
  movb %dl, %ah
  testl %ecx, %ecx
  je .L52
  leal 48(%rcx), %edx
  andq $-16711681, %rax
  movl $3, %esi
  movzbl %dl, %edx
  salq $16, %rdx
  orq %rdx, %rax
  movslq %r10d, %rdx
  movl %eax, %ecx
  sall $8, %ecx
  sarl $24, %ecx
  movb %cl, buf.0(%rdx)
  leal 1(%r10), %edx
  movslq %edx, %rdx
  movb %ah, buf.0(%rdx)
  leal 2(%r10), %edx
  movslq %edx, %rdx
  movb %al, buf.0(%rdx)
  jmp .L39
.L52:
  movslq %r10d, %rdx
  movl $2, %esi
  movb %ah, buf.0(%rdx)
  leal 1(%r10), %edx
  movslq %edx, %rdx
  movb %al, buf.0(%rdx)
  jmp .L39
.L51:
  movslq %r10d, %rdx
  movl $1, %esi
  movb %al, buf.0(%rdx)
  jmp .L39
.L34:
  movslq %ecx, %rdx
  movb %al, buf.0(%rdx)
  movl $1, %edx
  jmp .L33
