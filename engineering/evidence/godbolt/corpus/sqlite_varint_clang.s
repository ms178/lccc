main:
  pushq %rbp
  pushq %r15
  pushq %r14
  pushq %r13
  pushq %r12
  pushq %rbx
  subq $40, %rsp
  xorl %r12d, %r12d
  leaq check_known_values.values(%rip), %r13
  leaq 8(%rsp), %rbx
  leaq 32(%rsp), %r14
.LBB0_1:
  movq (%r12,%r13), %r15
  movq %rbx, %rdi
  movq %r15, %rsi
  callq sqlite_put_varint
  movl %eax, %ebp
  movq %rbx, %rdi
  movq %r14, %rsi
  callq sqlite_get_varint
  movzbl %al, %eax
  cmpl %eax, %ebp
  jne .LBB0_3
  cmpq %r15, 32(%rsp)
  jne .LBB0_3
  addq $8, %r12
  cmpq $88, %r12
  jne .LBB0_1
  movl $1831565813, %ebx
  xorl %eax, %eax
  movabsq $-9223372036854775808, %r14
  leaq sqlite_varint_offsets(%rip), %r15
  leaq sqlite_varint_bytes(%rip), %r12
  xorl %ebp, %ebp
  jmp .LBB0_6
.LBB0_12:
  movl %ebx, %esi
  shlq $3, %rsi
  orq $268435456, %rsi
.LBB0_17:
  movl %r13d, (%rbp,%r15)
  movl %r13d, %edi
  addq %r12, %rdi
  callq sqlite_put_varint
  addl %r13d, %eax
  incq %r14
  addq $4, %rbp
  cmpq $1048576, %rbp
  je .LBB0_18
.LBB0_6:
  movl %eax, %r13d
  movl %r14d, %eax
  imulq $954437177, %rax, %rax
  shrq $33, %rax
  leal (%rax,%rax,8), %ecx
  movl %r14d, %eax
  subl %ecx, %eax
  imull $1664525, %ebx, %ebx
  addl $1013904223, %ebx
  cmpl $7, %eax
  ja .LBB0_16
  leaq .LJTI0_0(%rip), %rcx
  movslq (%rcx,%rax,4), %rax
  addq %rcx, %rax
  jmpq *%rax
.LBB0_8:
  movl %ebx, %esi
  andl $127, %esi
  jmp .LBB0_17
.LBB0_10:
  movl %ebx, %esi
  andl $2080767, %esi
  orl $16384, %esi
  jmp .LBB0_17
.LBB0_11:
  movl %ebx, %esi
  andl $266338303, %esi
  orl $2097152, %esi
  jmp .LBB0_17
.LBB0_15:
  movl %ebx, %esi
  orq $33554432, %rsi
  shlq $24, %rsi
  jmp .LBB0_17
.LBB0_9:
  movl %ebx, %esi
  andl $16255, %esi
  orl $128, %esi
  jmp .LBB0_17
.LBB0_13:
  movl %ebx, %esi
  orq $33554432, %rsi
  shlq $10, %rsi
  jmp .LBB0_17
.LBB0_14:
  movl %ebx, %esi
  orq $33554432, %rsi
  shlq $17, %rsi
  jmp .LBB0_17
.LBB0_16:
  movl %ebx, %esi
  shlq $25, %rsi
  addq %r14, %rsi
  jmp .LBB0_17
.LBB0_3:
  movl $2, %eax
  jmp .LBB0_25
.LBB0_18:
  movq %rax, 24(%rsp)
  xorl %ebx, %ebx
  leaq 8(%rsp), %r14
  xorl %ebp, %ebp
.LBB0_19:
  xorl %r13d, %r13d
.LBB0_20:
  movl (%r15), %edi
  addq %r12, %rdi
  movq %r14, %rsi
  callq sqlite_get_varint
  movzbl %al, %eax
  movl %r13d, %ecx
  andb $31, %cl
  shlxq %rcx, %rax, %rax
  xorq 8(%rsp), %rax
  addq %rax, %rbx
  incq %r13
  addq $4, %r15
  cmpq $262144, %r13
  jne .LBB0_20
  imull $104729, %ebp, %eax
  andl $262143, %eax
  leaq sqlite_varint_offsets(%rip), %r15
  movl (%r15,%rax,4), %eax
  xorb $1, (%rax,%r12)
  incl %ebp
  cmpl $24, %ebp
  jne .LBB0_19
  cmpl $0, 24(%rsp)
  je .LBB0_23
  leaq .L.str(%rip), %rdi
  movq %rbx, %rsi
  xorl %eax, %eax
  callq printf@PLT
  xorl %eax, %eax
  jmp .LBB0_25
.LBB0_23:
  movl $3, %eax
.LBB0_25:
  addq $40, %rsp
  popq %rbx
  popq %r12
  popq %r13
  popq %r14
  popq %r15
  popq %rbp
  retq
.LJTI0_0:
  .long .LBB0_8-.LJTI0_0
  .long .LBB0_9-.LJTI0_0
  .long .LBB0_10-.LJTI0_0
  .long .LBB0_11-.LJTI0_0
  .long .LBB0_12-.LJTI0_0
  .long .LBB0_13-.LJTI0_0
  .long .LBB0_14-.LJTI0_0
  .long .LBB0_15-.LJTI0_0

sqlite_get_varint:
  movsbq (%rdi), %rcx
  testq %rcx, %rcx
  js .LBB1_2
  movb $1, %al
  movq %rcx, (%rsi)
  retq
.LBB1_2:
  movsbq 1(%rdi), %rax
  testq %rax, %rax
  js .LBB1_4
  andl $127, %ecx
  shll $7, %ecx
  orq %rax, %rcx
  movb $2, %al
  movq %rcx, (%rsi)
  retq
.LBB1_4:
  movzbl %cl, %ecx
  shll $14, %ecx
  movzbl %al, %edx
  movzbl 2(%rdi), %eax
  orl %eax, %ecx
  andl $2080895, %ecx
  testb %al, %al
  js .LBB1_6
  andl $127, %edx
  shll $7, %edx
  orl %edx, %ecx
  movb $3, %al
  movq %rcx, (%rsi)
  retq
.LBB1_6:
  shll $14, %edx
  movzbl 3(%rdi), %eax
  orl %eax, %edx
  andl $2080895, %edx
  testb %al, %al
  js .LBB1_8
  shll $7, %ecx
  orl %ecx, %edx
  movb $4, %al
  movq %rdx, (%rsi)
  retq
.LBB1_8:
  movl %ecx, %eax
  shll $14, %eax
  movzbl 4(%rdi), %r8d
  orl %r8d, %eax
  testb %r8b, %r8b
  js .LBB1_10
  shll $7, %edx
  orl %edx, %eax
  shrl $18, %ecx
  shlq $32, %rcx
  orq %rax, %rcx
  movb $5, %al
  movq %rcx, (%rsi)
  retq
.LBB1_10:
  shll $7, %ecx
  orl %edx, %ecx
  shll $14, %edx
  movzbl 5(%rdi), %r9d
  orl %r9d, %edx
  testb %r9b, %r9b
  js .LBB1_12
  shll $7, %eax
  andl $266354560, %eax
  orl %eax, %edx
  shrl $18, %ecx
  shlq $32, %rcx
  orq %rdx, %rcx
  movb $6, %al
  movq %rcx, (%rsi)
  retq
.LBB1_12:
  shll $14, %eax
  movzbl 6(%rdi), %r9d
  orl %r9d, %eax
  testb %r9b, %r9b
  js .LBB1_14
  andl $-266354561, %eax
  shll $7, %edx
  andl $266354560, %edx
  orl %eax, %edx
  shrl $11, %ecx
  shlq $32, %rcx
  orq %rdx, %rcx
  movb $7, %al
  movq %rcx, (%rsi)
  retq
.LBB1_14:
  andl $2080895, %eax
  shll $14, %edx
  movzbl 7(%rdi), %r9d
  orl %r9d, %edx
  testb %r9b, %r9b
  js .LBB1_16
  andl $-266354561, %edx
  shll $7, %eax
  orl %edx, %eax
  shrl $4, %ecx
  shlq $32, %rcx
  orq %rax, %rcx
  movb $8, %al
  movq %rcx, (%rsi)
  retq
.LBB1_16:
  shll $15, %eax
  movzbl 8(%rdi), %edi
  orl %eax, %edi
  shll $8, %edx
  andl $532709120, %edx
  orl %edi, %edx
  shll $4, %ecx
  shrl $3, %r8d
  andl $15, %r8d
  orl %ecx, %r8d
  shlq $32, %r8
  orq %r8, %rdx
  movb $9, %al
  movq %rdx, (%rsi)
  retq

.LCPI2_2:
  .byte 128
  .byte 128
  .byte 128
  .byte 128
  .byte 128
  .byte 128
  .byte 128
  .byte 128
  .zero 1
  .zero 1
  .zero 1
  .zero 1
  .zero 1
  .zero 1
  .zero 1
  .zero 1
.LCPI2_3:
  .byte 15
  .byte 14
  .byte 13
  .byte 12
  .byte 11
  .byte 10
  .byte 9
  .byte 8
  .byte 7
  .byte 6
  .byte 5
  .byte 4
  .byte 3
  .byte 2
  .byte 1
  .byte 0
.LCPI2_4:
  .byte 7
  .byte 6
  .byte 5
  .byte 4
  .byte 3
  .byte 2
  .byte 1
  .byte 0
  .byte 0
  .byte 0
  .byte 0
  .byte 0
  .byte 0
  .byte 0
  .byte 0
  .byte 0
sqlite_put_varint:
  cmpq $127, %rsi
  ja .LBB2_2
  movb %sil, (%rdi)
  movl $1, %eax
  retq
.LBB2_2:
  cmpq $16383, %rsi
  ja .LBB2_4
  movl %esi, %eax
  shrl $7, %eax
  orb $-128, %al
  movb %al, (%rdi)
  andb $127, %sil
  movb %sil, 1(%rdi)
  movl $2, %eax
  retq
.LBB2_4:
  movq %rsi, %rax
  shrq $56, %rax
  je .LBB2_5
  movb %sil, 8(%rdi)
  movl %esi, %eax
  shrl $8, %eax
  movl %esi, %ecx
  shrl $15, %ecx
  movl %esi, %edx
  shrl $22, %edx
  movq %rsi, %r8
  shrq $29, %r8
  movq %rsi, %r9
  shrq $36, %r9
  movq %rsi, %r10
  shrq $43, %r10
  movq %rsi, %r11
  shrq $50, %r11
  shrq $57, %rsi
  vmovd %esi, %xmm0
  vpinsrb $1, %r11d, %xmm0, %xmm0
  vpinsrb $2, %r10d, %xmm0, %xmm0
  vpinsrb $3, %r9d, %xmm0, %xmm0
  vpinsrb $4, %r8d, %xmm0, %xmm0
  vpinsrb $5, %edx, %xmm0, %xmm0
  vpinsrb $6, %ecx, %xmm0, %xmm0
  vpinsrb $7, %eax, %xmm0, %xmm0
  vpor .LCPI2_2(%rip), %xmm0, %xmm0
  vmovq %xmm0, (%rdi)
  movl $9, %eax
  retq
.LBB2_5:
  xorl %eax, %eax
.LBB2_6:
  movl %esi, %ecx
  orb $-128, %cl
  movb %cl, -10(%rsp,%rax)
  incq %rax
  shrq $7, %rsi
  jne .LBB2_6
  andb $127, -10(%rsp)
  cmpl $8, %eax
  jae .LBB2_9
  leaq -1(%rax), %rdx
  xorl %ecx, %ecx
  jmp .LBB2_19
.LBB2_9:
  cmpl $32, %eax
  jae .LBB2_14
  xorl %ecx, %ecx
  jmp .LBB2_11
.LBB2_14:
  leaq (%rsp,%rax), %rdx
  addq $-42, %rdx
  movl %eax, %ecx
  andl $2147483616, %ecx
  movq %rax, %rsi
  andq $-32, %rsi
  xorl %r8d, %r8d
  vbroadcasti128 .LCPI2_3(%rip), %ymm0
.LBB2_15:
  vmovdqu (%rdx), %ymm1
  vpshufb %ymm0, %ymm1, %ymm1
  vpermq $78, %ymm1, %ymm1
  vmovdqu %ymm1, (%rdi,%r8)
  addq $32, %r8
  addq $-32, %rdx
  cmpq %r8, %rsi
  jne .LBB2_15
  cmpq %rcx, %rax
  je .LBB2_22
  testb $24, %al
  je .LBB2_18
.LBB2_11:
  movq %rcx, %rsi
  movl %eax, %ecx
  andl $2147483640, %ecx
  movq %rax, %r8
  andq $-8, %r8
  movq %r8, %rdx
  notq %rdx
  addq %rax, %rdx
  movq %rsi, %r9
  negq %r9
  addq %rsp, %r9
  addq $-10, %r9
  addq %rax, %r9
  addq $-8, %r9
  vmovq .LCPI2_4(%rip), %xmm0
.LBB2_12:
  vmovq (%r9), %xmm1
  vpshufb %xmm0, %xmm1, %xmm1
  vmovq %xmm1, (%rdi,%rsi)
  addq $8, %rsi
  addq $-8, %r9
  cmpq %rsi, %r8
  jne .LBB2_12
  cmpq %rcx, %rax
  je .LBB2_22
.LBB2_19:
  addq %rsp, %rdx
  addq $-10, %rdx
.LBB2_20:
  movzbl (%rdx), %esi
  movb %sil, (%rdi,%rcx)
  incq %rcx
  decq %rdx
  cmpq %rcx, %rax
  jne .LBB2_20
.LBB2_22:
  vzeroupper
  retq
.LBB2_18:
  movq %rax, %rdx
  notq %rdx
  orq $31, %rdx
  addq %rax, %rdx
  jmp .LBB2_19

.L.str:
  .asciz "%llx\n"

check_known_values.values:
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

