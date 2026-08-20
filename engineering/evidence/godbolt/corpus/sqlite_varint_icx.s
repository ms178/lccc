.LCPI0_0:
  .quad 57
  .quad 50
  .quad 43
  .quad 36
.LCPI0_1:
  .quad 29
  .quad 22
  .quad 15
  .quad 8
.LCPI0_2:
  .byte 255
  .byte 0
  .byte 0
  .byte 0
  .byte 0
  .byte 0
  .byte 0
  .byte 0
.LCPI0_6:
  .byte 7
  .byte 6
  .byte 5
  .byte 4
  .byte 3
  .byte 2
  .byte 1
  .byte 0
.LCPI0_7:
  .byte 192
  .byte 128
  .byte 128
  .byte 128
  .byte 128
  .byte 128
  .byte 128
  .byte 128
.LCPI0_3:
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
main:
  pushq %rbp
  pushq %r15
  pushq %r14
  pushq %r13
  pushq %r12
  pushq %rbx
  subq $88, %rsp
  vstmxcsr 8(%rsp)
  orl $32832, 8(%rsp)
  vldmxcsr 8(%rsp)
  leaq 7(%rsp), %r15
  xorl %r13d, %r13d
  leaq 24(%rsp), %rbx
  leaq 40(%rsp), %r14
  vpbroadcastq .LCPI0_6(%rip), %xmm4
  vpbroadcastq .LCPI0_2(%rip), %ymm0
  vmovdqu %ymm0, 48(%rsp)
.LBB0_1:
  movq $0, 40(%rsp)
  movq check_known_values.values(,%r13,8), %rbp
  cmpq $1, %r13
  ja .LBB0_3
  movb %bpl, 24(%rsp)
  movl $1, %r12d
  jmp .LBB0_13
.LBB0_3:
  cmpq $3, %r13
  ja .LBB0_5
  movl %ebp, %eax
  shrl $7, %eax
  orb $-128, %al
  movb %al, 24(%rsp)
  movl %ebp, %eax
  andb $127, %al
  movb %al, 25(%rsp)
  movl $2, %r12d
  jmp .LBB0_13
.LBB0_5:
  cmpq $8, %r13
  ja .LBB0_12
  movl $1, %r8d
  leaq 1(%rsp), %r9
  xorl %r12d, %r12d
  movq %rbp, %rdi
  movq %rbp, %rcx
.LBB0_7:
  movq %r12, %rax
  movl %r8d, %esi
  movq %r9, %rdx
  movl %edi, %r8d
  orb $-128, %r8b
  incq %r12
  movb %r8b, 8(%rsp,%rax)
  shrq $7, %rcx
  leal 1(%rsi), %r8d
  incq %r9
  cmpq $127, %rdi
  movq %rcx, %rdi
  ja .LBB0_7
  andb $127, 8(%rsp)
  movl %esi, %esi
  movq %rsi, %rcx
  andq $2147483640, %rcx
  je .LBB0_9
  xorl %edi, %edi
.LBB0_45:
  vmovq (%rdx), %xmm0
  vpshufb %xmm4, %xmm0, %xmm0
  vmovq %xmm0, 24(%rsp,%rdi)
  addq $8, %rdi
  addq $-8, %rdx
  cmpq %rcx, %rdi
  jb .LBB0_45
  cmpq %rcx, %rsi
  jne .LBB0_10
  jmp .LBB0_13
.LBB0_12:
  movb %bpl, 32(%rsp)
  vmovq %rbp, %xmm0
  vpbroadcastq %xmm0, %ymm0
  vpsrlvq .LCPI0_0(%rip), %ymm0, %ymm1
  vpsrlvq .LCPI0_1(%rip), %ymm0, %ymm0
  vmovdqu 48(%rsp), %ymm2
  vpand %ymm2, %ymm0, %ymm0
  vpand %ymm2, %ymm1, %ymm1
  vpackusdw %ymm0, %ymm1, %ymm0
  vextracti128 $1, %ymm0, %xmm1
  vpackusdw %xmm1, %xmm0, %xmm0
  vpshufd $216, %xmm0, %xmm0
  vpackuswb %xmm0, %xmm0, %xmm0
  vpor .LCPI0_3(%rip), %xmm0, %xmm0
  vmovq %xmm0, 24(%rsp)
  movl $9, %r12d
  jmp .LBB0_13
.LBB0_9:
  xorl %ecx, %ecx
.LBB0_10:
  incq %rax
  movq %r15, %rdx
  subq %rcx, %rdx
.LBB0_11:
  movzbl (%rdx,%r12), %esi
  movb %sil, 24(%rsp,%rcx)
  incq %rcx
  decq %rdx
  cmpq %rcx, %rax
  jne .LBB0_11
.LBB0_13:
  movq %rbx, %rdi
  movq %r14, %rsi
  vzeroupper
  callq sqlite_get_varint
  movzbl %al, %eax
  cmpl %eax, %r12d
  jne .LBB0_15
  cmpq %rbp, 40(%rsp)
  jne .LBB0_15
  incq %r13
  cmpq $11, %r13
  vpbroadcastq .LCPI0_6(%rip), %xmm4
  jne .LBB0_1
  movl $0, sqlite_varint_offsets(%rip)
  movb $80, sqlite_varint_bytes(%rip)
  leaq 1(%rsp), %rax
  movl $53002960, %ecx
  movl $1, %esi
  movl $262143, %edx
  vpbroadcastq .LCPI0_7(%rip), %xmm0
  movl $1, %ebp
  vmovdqu .LCPI0_0(%rip), %ymm5
  vmovdqu .LCPI0_1(%rip), %ymm6
.LBB0_26:
  movl %esi, %edi
  cmpl $262144, %esi
  cmovbl %edx, %esi
  incl %esi
  jmp .LBB0_18
.LBB0_27:
  movl %ebp, sqlite_varint_offsets(,%rdi,4)
  movl %ebp, %r8d
  movl %ecx, %r9d
  shrl $7, %r9d
  orb $-127, %r9b
  movb %r9b, sqlite_varint_bytes(%r8)
  movl %ecx, %r9d
  andb $127, %r9b
  movb %r9b, sqlite_varint_bytes+1(%r8)
  movl $2, %r8d
.LBB0_28:
  addl %r8d, %ebp
  incq %rdi
  cmpl %edi, %esi
  je .LBB0_29
.LBB0_18:
  imull $1664525, %ecx, %ecx
  addl $1013904223, %ecx
  movl %edi, %r8d
  imulq $954437177, %r8, %r8
  shrq $33, %r8
  leal (%r8,%r8,8), %r9d
  movl %edi, %r8d
  subl %r9d, %r8d
  cmpl $7, %r8d
  ja .LBB0_50
  jmpq *.LJTI0_0(,%r8,8)
.LBB0_35:
  movl %ecx, %r10d
  andl $2080767, %r10d
  orl $16384, %r10d
  jmp .LBB0_36
.LBB0_50:
  movl %ecx, %r8d
  shlq $25, %r8
  orq %rdi, %r8
  movl %ebp, sqlite_varint_offsets(,%rdi,4)
  movl %ebp, %r9d
  movb %dil, sqlite_varint_bytes+8(%r9)
  vmovq %r8, %xmm1
  vpbroadcastq %xmm1, %xmm1
  vmovq %rdi, %xmm2
  vpbroadcastq %xmm2, %xmm2
  vpunpcklqdq %xmm1, %xmm2, %xmm3
  vinserti128 $1, %xmm1, %ymm3, %ymm3
  vinserti128 $1, %xmm2, %ymm1, %ymm1
  vpsrlvq %ymm5, %ymm3, %ymm2
  vpsrlvq %ymm6, %ymm1, %ymm1
  vmovdqu 48(%rsp), %ymm3
  vpand %ymm3, %ymm2, %ymm2
  vpand %ymm3, %ymm1, %ymm1
  vpackusdw %ymm1, %ymm2, %ymm1
  vextracti128 $1, %ymm1, %xmm2
  vpackusdw %xmm2, %xmm1, %xmm1
  vpshufd $216, %xmm1, %xmm1
  vpackuswb %xmm1, %xmm1, %xmm1
  vpor %xmm0, %xmm1, %xmm1
  vmovq %xmm1, sqlite_varint_bytes(%r9)
  movl $9, %r8d
  jmp .LBB0_28
.LBB0_20:
  movl %ecx, %r10d
  andl $266338303, %r10d
  orl $2097152, %r10d
  jmp .LBB0_36
.LBB0_21:
  movl %ecx, %r10d
  shlq $3, %r10
  orq $268435456, %r10
  jmp .LBB0_36
.LBB0_22:
  movl %ecx, %r10d
  orq $33554432, %r10
  shlq $10, %r10
  jmp .LBB0_36
.LBB0_23:
  movl %ecx, %r10d
  orq $33554432, %r10
  shlq $17, %r10
  jmp .LBB0_36
.LBB0_24:
  movl %ecx, %r10d
  orq $33554432, %r10
  shlq $24, %r10
.LBB0_36:
  movl %ebp, sqlite_varint_offsets(,%rdi,4)
  movl $1, %r12d
  movq %rax, %r13
  xorl %r8d, %r8d
  movq %r10, %r11
.LBB0_37:
  movq %r8, %r9
  movl %r12d, %r14d
  movq %r13, %rbx
  movl %r10d, %r12d
  orb $-128, %r12b
  incq %r8
  movb %r12b, 8(%rsp,%r9)
  shrq $7, %r11
  leal 1(%r14), %r12d
  incq %r13
  cmpq $127, %r10
  movq %r11, %r10
  ja .LBB0_37
  andb $127, 8(%rsp)
  movl %ebp, %r10d
  movl %r14d, %r14d
  movq %r14, %r11
  andq $2147483640, %r11
  je .LBB0_39
  xorl %r12d, %r12d
.LBB0_48:
  vmovq (%rbx), %xmm1
  vpshufb %xmm4, %xmm1, %xmm1
  vmovq %xmm1, sqlite_varint_bytes(%r10,%r12)
  addq $8, %r12
  addq $-8, %rbx
  cmpq %r11, %r12
  jb .LBB0_48
  cmpq %r11, %r14
  je .LBB0_28
  jmp .LBB0_40
.LBB0_39:
  xorl %r11d, %r11d
.LBB0_40:
  incq %r9
  movq %r15, %rbx
  subq %r11, %rbx
.LBB0_41:
  movzbl (%rbx,%r8), %r14d
  movb %r14b, sqlite_varint_bytes(%r10,%r11)
  incq %r11
  decq %rbx
  cmpq %r11, %r9
  jne .LBB0_41
  jmp .LBB0_28
.LBB0_25:
  movl %edi, %esi
  movl %ebp, sqlite_varint_offsets(,%rsi,4)
  movl %ebp, %esi
  movl %ecx, %r8d
  andb $127, %r8b
  movb %r8b, sqlite_varint_bytes(%rsi)
  incl %ebp
  leal 1(%rdi), %esi
  cmpl $262142, %edi
  jbe .LBB0_26
.LBB0_29:
  xorl %ebx, %ebx
  leaq 8(%rsp), %r14
  xorl %r15d, %r15d
.LBB0_30:
  xorl %r12d, %r12d
.LBB0_31:
  movq $0, 8(%rsp)
  movl sqlite_varint_offsets(,%r12,4), %eax
  leaq sqlite_varint_bytes(%rax), %rdi
  movq %r14, %rsi
  vzeroupper
  callq sqlite_get_varint
  movzbl %al, %eax
  movl %r12d, %ecx
  andb $31, %cl
  shlxq %rcx, %rax, %rax
  xorq 8(%rsp), %rax
  addq %rax, %rbx
  incq %r12
  cmpq $262144, %r12
  jne .LBB0_31
  imull $104729, %r15d, %eax
  andl $262143, %eax
  movl sqlite_varint_offsets(,%rax,4), %eax
  xorb $1, sqlite_varint_bytes(%rax)
  incl %r15d
  cmpl $24, %r15d
  jne .LBB0_30
  testl %ebp, %ebp
  je .LBB0_34
  xorl %ebp, %ebp
  movl $.L.str, %edi
  movq %rbx, %rsi
  xorl %eax, %eax
  callq printf
  jmp .LBB0_43
.LBB0_15:
  movl $2, %ebp
  jmp .LBB0_43
.LBB0_34:
  movl $3, %ebp
.LBB0_43:
  movl %ebp, %eax
  addq $88, %rsp
  popq %rbx
  popq %r12
  popq %r13
  popq %r14
  popq %r15
  popq %rbp
  retq
.LJTI0_0:
  .quad .LBB0_25
  .quad .LBB0_27
  .quad .LBB0_35
  .quad .LBB0_20
  .quad .LBB0_21
  .quad .LBB0_22
  .quad .LBB0_23
  .quad .LBB0_24

sqlite_get_varint:
  movzbl (%rdi), %ecx
  testb %cl, %cl
  js .LBB1_2
  movb $1, %al
  movq %rcx, (%rsi)
  retq
.LBB1_2:
  movzbl 1(%rdi), %eax
  testb %al, %al
  js .LBB1_4
  andl $127, %ecx
  shlq $7, %rcx
  orq %rax, %rcx
  movb $2, %al
  movq %rcx, (%rsi)
  retq
.LBB1_4:
  shll $14, %ecx
  movzbl 2(%rdi), %edx
  orl %edx, %ecx
  andl $2080895, %ecx
  movzbl %al, %eax
  testb %dl, %dl
  js .LBB1_6
  andl $127, %eax
  shll $7, %eax
  orq %rax, %rcx
  movb $3, %al
  movq %rcx, (%rsi)
  retq
.LBB1_6:
  shll $14, %eax
  movzbl 3(%rdi), %edx
  orl %edx, %eax
  andl $2080895, %eax
  testb %dl, %dl
  js .LBB1_8
  movl %ecx, %ecx
  shlq $7, %rcx
  orq %rax, %rcx
  movb $4, %al
  movq %rcx, (%rsi)
  retq
.LBB1_8:
  movl %ecx, %edx
  shll $14, %edx
  movzbl 4(%rdi), %r8d
  orq %r8, %rdx
  testb %r8b, %r8b
  js .LBB1_10
  movl %eax, %eax
  shlq $7, %rax
  orq %rax, %rdx
  shrl $18, %ecx
  shlq $32, %rcx
  orq %rdx, %rcx
  movb $5, %al
  movq %rcx, (%rsi)
  retq
.LBB1_10:
  movl %ecx, %r9d
  shlq $7, %r9
  movl %eax, %ecx
  orq %r9, %rcx
  shll $14, %eax
  movzbl 5(%rdi), %r9d
  orq %r9, %rax
  testb %r9b, %r9b
  js .LBB1_12
  shll $7, %edx
  andl $266354560, %edx
  orq %rdx, %rax
  shrl $18, %ecx
  shlq $32, %rcx
  orq %rax, %rcx
  movb $6, %al
  movq %rcx, (%rsi)
  retq
.LBB1_12:
  shll $14, %edx
  movzbl 6(%rdi), %r9d
  orq %r9, %rdx
  testb %r9b, %r9b
  js .LBB1_14
  andl $-266354561, %edx
  shll $7, %eax
  andl $266354560, %eax
  orq %rdx, %rax
  shrl $11, %ecx
  shlq $32, %rcx
  orq %rax, %rcx
  movb $7, %al
  movq %rcx, (%rsi)
  retq
.LBB1_14:
  andl $2080895, %edx
  shll $14, %eax
  movzbl 7(%rdi), %r9d
  orq %r9, %rax
  testb %r9b, %r9b
  js .LBB1_16
  andl $-266354561, %eax
  shlq $7, %rdx
  orq %rax, %rdx
  shrl $4, %ecx
  shlq $32, %rcx
  orq %rdx, %rcx
  movb $8, %al
  movq %rcx, (%rsi)
  retq
.LBB1_16:
  shll $15, %edx
  movzbl 8(%rdi), %edi
  orq %rdi, %rdx
  shll $8, %eax
  andl $532709120, %eax
  orq %rdx, %rax
  shll $4, %ecx
  shrl $3, %r8d
  andl $15, %r8d
  orl %ecx, %r8d
  shlq $32, %r8
  orq %rax, %r8
  movb $9, %al
  movq %r8, (%rsi)
  retq

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

