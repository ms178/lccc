sqlite_put_varint.part.0:
  movq %rsi, %rax
  shrq $56, %rax
  jne .L17
  leaq -10(%rsp), %r9
  xorl %edx, %edx
  movq %r9, %rcx
.L5:
  movl %esi, %r8d
  movl %edx, %eax
  addq $1, %rcx
  addl $1, %edx
  orl $-128, %r8d
  movb %r8b, -1(%rcx)
  shrq $7, %rsi
  jne .L5
  andb $127, -10(%rsp)
  cmpl $6, %eax
  jle .L9
  movl %eax, %ecx
  vmovq -17(%rsp,%rcx), %xmm0
  vpshufb .LC0(%rip), %xmm0, %xmm0
  vmovq %xmm0, (%rdi)
  cmpl $7, %eax
  je .L1
  subl $8, %eax
  movl $8, %ecx
.L6:
  addq %rdi, %rcx
.L8:
  movzbl (%r9,%rax), %esi
  subq $1, %rax
  addq $1, %rcx
  movb %sil, -1(%rcx)
  cmpl $-1, %eax
  jne .L8
.L1:
  movl %edx, %eax
  ret
.L17:
  movb %sil, 8(%rdi)
  leaq 7(%rdi), %rax
  shrq $8, %rsi
  leaq -1(%rdi), %rcx
.L3:
  movl %esi, %edx
  subq $1, %rax
  shrq $7, %rsi
  orl $-128, %edx
  movb %dl, 1(%rax)
  cmpq %rcx, %rax
  jne .L3
  movl $9, %edx
  movl %edx, %eax
  ret
.L9:
  xorl %ecx, %ecx
  jmp .L6
sqlite_get_varint.constprop.0:
  movsbq (%rdi), %rax
  movl $1, %edx
  testb %al, %al
  jns .L20
  movsbl 1(%rdi), %edx
  movzbl %al, %eax
  testb %dl, %dl
  jns .L28
  movzbl 2(%rdi), %ecx
  sall $14, %eax
  orl %ecx, %eax
  andl $128, %ecx
  je .L29
  movzbl 3(%rdi), %ecx
  movzbl %dl, %edx
  sall $14, %edx
  orl %ecx, %edx
  movl %edx, %ecx
  andl $2080895, %ecx
  andl $128, %edx
  je .L30
  movl %eax, %edx
  movzbl 4(%rdi), %r9d
  sall $14, %edx
  andl $-266354688, %edx
  movl %r9d, %r8d
  orl %r9d, %edx
  andl $128, %r9d
  je .L31
  sall $7, %eax
  movzbl 5(%rdi), %r9d
  andl $266354560, %eax
  orl %ecx, %eax
  sall $14, %ecx
  orl %r9d, %ecx
  andl $128, %r9d
  je .L32
  movzbl 6(%rdi), %r9d
  sall $14, %edx
  orl %r9d, %edx
  andl $128, %r9d
  je .L33
  movzbl 7(%rdi), %r9d
  sall $14, %ecx
  andl $2080895, %edx
  orl %r9d, %ecx
  andl $128, %r9d
  je .L34
  sall $4, %eax
  andl $127, %r8d
  sall $8, %ecx
  shrl $3, %r8d
  sall $15, %edx
  andl $532709120, %ecx
  orl %eax, %r8d
  movzbl 8(%rdi), %eax
  orl %ecx, %edx
  salq $32, %r8
  orl %edx, %eax
  movl $9, %edx
  orq %r8, %rax
.L20:
  movq %rax, (%rsi)
  movl %edx, %eax
  ret
.L28:
  sall $7, %eax
  andl $16256, %eax
  orl %edx, %eax
  movl $2, %edx
  movq %rax, (%rsi)
  movl %edx, %eax
  ret
.L31:
  shrl $18, %eax
  sall $7, %ecx
  andl $7, %eax
  orl %edx, %ecx
  movl $5, %edx
  salq $32, %rax
  orq %rcx, %rax
  jmp .L20
.L29:
  andl $127, %edx
  andl $2080895, %eax
  sall $7, %edx
  orl %eax, %edx
  movl %edx, %eax
  movl $3, %edx
  jmp .L20
.L30:
  sall $7, %eax
  movl $4, %edx
  andl $266354560, %eax
  orl %ecx, %eax
  jmp .L20
.L32:
  sall $7, %edx
  shrl $18, %eax
  andl $266354560, %edx
  salq $32, %rax
  orl %ecx, %edx
  orq %rdx, %rax
  movl $6, %edx
  jmp .L20
.L34:
  sall $7, %edx
  shrl $4, %eax
  andl $-266354561, %ecx
  orl %edx, %ecx
  salq $32, %rax
  movl $8, %edx
  orq %rcx, %rax
  jmp .L20
.L33:
  sall $7, %ecx
  andl $-266354561, %edx
  shrl $11, %eax
  andl $266354560, %ecx
  salq $32, %rax
  orl %edx, %ecx
  movl $7, %edx
  orq %rcx, %rax
  jmp .L20
.LC1:
  .string "%llx\n"
main:
  subq $72, %rsp
  movl $values.0, %r11d
  movq %rbp, 40(%rsp)
  movl $values.0+88, %ebp
  movq %rbx, 32(%rsp)
  jmp .L42
.L36:
  cmpq $16383, %r10
  jbe .L64
  movq %r10, %rsi
  leaq 16(%rsp), %rdi
  call sqlite_put_varint.part.0
  movl %eax, %ebx
.L37:
  leaq 8(%rsp), %rsi
  leaq 16(%rsp), %rdi
  call sqlite_get_varint.constprop.0
  movzbl %al, %eax
  cmpl %ebx, %eax
  jne .L39
  cmpq 8(%rsp), %r10
  jne .L39
  addq $8, %r11
  cmpq %r11, %rbp
  je .L65
.L42:
  movq (%r11), %r10
  cmpq $127, %r10
  ja .L36
  movb %r10b, 16(%rsp)
  movl $1, %ebx
  jmp .L37
.L64:
  movq %r10, %rax
  movl $2, %ebx
  shrq $7, %rax
  orl $-128, %eax
  movb %al, 16(%rsp)
  movl %r10d, %eax
  andl $127, %eax
  movb %al, 17(%rsp)
  jmp .L37
.L65:
  movl $1, %ebp
  movq %r12, 48(%rsp)
  xorl %ebx, %ebx
  movabsq $-9223372036854775808, %r12
  movq %r13, 56(%rsp)
  salq $49, %rbp
  xorl %r13d, %r13d
  movabsq $4398046511104, %r11
  movabsq $34359738368, %r10
  movq %r14, 64(%rsp)
  movl $1831565813, %r14d
.L55:
  movl %r13d, %eax
  imull $1664525, %r14d, %r14d
  movl %ebx, %edx
  imulq $954437177, %rax, %rax
  leaq sqlite_varint_bytes(%rdx), %rdi
  addl $1013904223, %r14d
  shrq $33, %rax
  leal (%rax,%rax,8), %ecx
  movl %r13d, %eax
  subl %ecx, %eax
  cmpl $7, %eax
  ja .L43
  jmp *.L45(,%rax,8)
.L45:
  .quad .L52
  .quad .L51
  .quad .L50
  .quad .L49
  .quad .L48
  .quad .L47
  .quad .L46
  .quad .L44
.L52:
  movl %r14d, %eax
  movl %ebx, sqlite_varint_offsets(,%r13,4)
  andl $127, %eax
  movb %al, sqlite_varint_bytes(%rdx)
  movl $1, %eax
.L53:
  addq $1, %r13
  addl %eax, %ebx
  cmpq $262144, %r13
  jne .L55
  movl %ebx, sqlite_varint_used(%rip)
  xorl %ebp, %ebp
  xorl %r11d, %r11d
.L56:
  xorl %r10d, %r10d
.L57:
  movl sqlite_varint_offsets(,%r10,4), %edi
  leaq 16(%rsp), %rsi
  addq $sqlite_varint_bytes, %rdi
  call sqlite_get_varint.constprop.0
  movl %r10d, %edx
  addq $1, %r10
  movzbl %al, %eax
  andl $31, %edx
  shlx %rdx, %rax, %rax
  xorq 16(%rsp), %rax
  addq %rax, %r11
  cmpq $262144, %r10
  jne .L57
  movl %ebp, %eax
  addl $104729, %ebp
  andl $262143, %eax
  movl sqlite_varint_offsets(,%rax,4), %eax
  xorb $1, sqlite_varint_bytes(%rax)
  cmpl $2513496, %ebp
  jne .L56
  testl %ebx, %ebx
  je .L59
  movq %r11, %rsi
  movl $.LC1, %edi
  xorl %eax, %eax
  call printf
  movq 48(%rsp), %r12
  movq 56(%rsp), %r13
  xorl %eax, %eax
  movq 64(%rsp), %r14
.L35:
  movq 32(%rsp), %rbx
  movq 40(%rsp), %rbp
  addq $72, %rsp
  ret
.L51:
  movl %r14d, %eax
  movl %ebx, sqlite_varint_offsets(,%r13,4)
  andl $16255, %eax
  orb $-128, %al
  movl %eax, %ecx
  andl $127, %eax
  shrq $7, %rcx
  orl $-128, %ecx
  movb %cl, sqlite_varint_bytes(%rdx)
  movb %al, 1(%rdi)
  movl $2, %eax
  jmp .L53
.L44:
  movl %r14d, %esi
  salq $24, %rsi
  orq %rbp, %rsi
.L54:
  movl %ebx, sqlite_varint_offsets(,%r13,4)
  call sqlite_put_varint.part.0
  jmp .L53
.L46:
  movl %r14d, %esi
  salq $17, %rsi
  orq %r11, %rsi
  jmp .L54
.L47:
  movl %r14d, %esi
  salq $10, %rsi
  orq %r10, %rsi
  jmp .L54
.L48:
  movl %r14d, %esi
  salq $3, %rsi
  orq $268435456, %rsi
  jmp .L54
.L49:
  movl %r14d, %esi
  andl $266338303, %esi
  orl $2097152, %esi
  jmp .L54
.L50:
  movl %r14d, %esi
  andl $2080767, %esi
  orl $16384, %esi
  jmp .L54
.L43:
  movl %r14d, %esi
  salq $25, %rsi
  orq %r13, %rsi
  orq %r12, %rsi
  jmp .L54
.L59:
  movq 48(%rsp), %r12
  movq 56(%rsp), %r13
  movl $3, %eax
  movq 64(%rsp), %r14
  jmp .L35
.L39:
  movl $2, %eax
  jmp .L35
values.0:
  .quad 0
  .quad 127
  .quad 128
  .quad 16383
  .quad 16384
  .quad 2097151
  .quad 2097152
  .quad 268435455
  .quad 268435456
  .quad 9223372036854775807
  .quad -1
.LC0:
  .byte 7
  .byte 6
  .byte 5
  .byte 4
  .byte 3
  .byte 2
  .byte 1
  .byte 0
  .byte -128
  .byte -128
  .byte -128
  .byte -128
  .byte -128
  .byte -128
  .byte -128
  .byte -128
