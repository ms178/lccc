.LCPI0_0:
  .long 1634760805
  .long 857760878
  .long 2036477234
  .long 1797285236
  .long 67372036
  .long 84215045
  .long 101058054
  .long 117901063
.LCPI0_1:
  .long 134744072
  .long 151587081
  .long 168430090
  .long 185273099
main:
  pushq %rbp
  pushq %r15
  pushq %r14
  pushq %r13
  pushq %r12
  pushq %rbx
  subq $152, %rsp
  vstmxcsr 16(%rsp)
  orl $32832, 16(%rsp)
  vldmxcsr 16(%rsp)
  leaq 16(%rsp), %rdi
  movl $check_known_vector.test_in, %esi
  callq chacha20_core
  cmpl $-454561520, 16(%rsp)
  jne .LBB0_2
  cmpl $358169553, 20(%rsp)
  jne .LBB0_2
  cmpl $-394014517, 72(%rsp)
  movl $2, %ebx
  jne .LBB0_3
  cmpl $1312575650, 76(%rsp)
  jne .LBB0_3
  vmovups .LCPI0_0(%rip), %ymm0
  vmovups %ymm0, 16(%rsp)
  vmovups .LCPI0_1(%rip), %xmm0
  vmovups %xmm0, 48(%rsp)
  movabsq $81985530642677487, %rax
  movq %rax, 68(%rsp)
  movl $-1985229329, 76(%rsp)
  xorl %r15d, %r15d
  movl $67372036, %ebp
  leaq 80(%rsp), %rbx
  xorl %eax, %eax
.LBB0_7:
  movq %rax, 8(%rsp)
  xorl %r13d, %r13d
.LBB0_8:
  movl %r13d, %eax
  movq 8(%rsp), %rcx
  xorl %ecx, %eax
  movq %rcx, %r12
  movl %eax, 64(%rsp)
  movq %rbx, %rdi
  leaq 16(%rsp), %rsi
  vzeroupper
  callq chacha20_core
  movl 80(%rsp), %ebx
  movl 108(%rsp), %eax
  movl 140(%rsp), %ecx
  shlq $16, %rax
  movzbl %bl, %r14d
  shlq $32, %rbx
  orq %rcx, %rbx
  xorq %rax, %rbx
  addq %r15, %rbx
  xorl %ebp, %r14d
  movl %r14d, 32(%rsp)
  leal 1(%r13), %eax
  xorl %r12d, %eax
  movl %eax, 64(%rsp)
  leaq 80(%rsp), %rdi
  leaq 16(%rsp), %rsi
  callq chacha20_core
  movl 80(%rsp), %r15d
  movl 108(%rsp), %eax
  movl 140(%rsp), %ecx
  shlq $16, %rax
  movzbl %r15b, %ebp
  shlq $32, %r15
  orq %rcx, %r15
  xorq %rax, %r15
  xorl %r14d, %ebp
  movl %ebp, 32(%rsp)
  leal 2(%r13), %eax
  xorl %r12d, %eax
  movl %eax, 64(%rsp)
  leaq 80(%rsp), %rdi
  leaq 16(%rsp), %rsi
  callq chacha20_core
  movl 80(%rsp), %r12d
  movl 108(%rsp), %eax
  movl 140(%rsp), %ecx
  shlq $16, %rax
  movzbl %r12b, %r14d
  shlq $32, %r12
  orq %rcx, %r12
  xorq %rax, %r12
  addq %r15, %r12
  addq %rbx, %r12
  leaq 80(%rsp), %rbx
  xorl %ebp, %r14d
  movl %r14d, 32(%rsp)
  leal 3(%r13), %eax
  xorl 8(%rsp), %eax
  movl %eax, 64(%rsp)
  movq %rbx, %rdi
  leaq 16(%rsp), %rsi
  callq chacha20_core
  movl 80(%rsp), %r15d
  movl 108(%rsp), %eax
  movl 140(%rsp), %ecx
  shlq $16, %rax
  movzbl %r15b, %ebp
  shlq $32, %r15
  orq %rcx, %r15
  xorq %rax, %r15
  addq %r12, %r15
  xorl %r14d, %ebp
  movl %ebp, 32(%rsp)
  addl $4, %r13d
  cmpl $131072, %r13d
  jne .LBB0_8
  movq 8(%rsp), %rcx
  leal 1(%rcx), %eax
  cmpl $15, %ecx
  jne .LBB0_7
  xorl %ebx, %ebx
  movl $.L.str, %edi
  movq %r15, %rsi
  xorl %eax, %eax
  callq printf
  jmp .LBB0_3
.LBB0_2:
  movl $2, %ebx
.LBB0_3:
  movl %ebx, %eax
  addq $152, %rsp
  popq %rbx
  popq %r12
  popq %r13
  popq %r14
  popq %r15
  popq %rbp
  retq

.LCPI1_0:
  .byte 6
  .byte 7
  .byte 4
  .byte 5
  .byte 10
  .byte 11
  .byte 8
  .byte 9
  .byte 14
  .byte 15
  .byte 12
  .byte 13
  .byte 2
  .byte 3
  .byte 0
  .byte 1
.LCPI1_1:
  .byte 14
  .byte 15
  .byte 12
  .byte 13
  .byte 2
  .byte 3
  .byte 0
  .byte 1
  .byte 6
  .byte 7
  .byte 4
  .byte 5
  .byte 10
  .byte 11
  .byte 8
  .byte 9
.LCPI1_2:
  .byte 7
  .byte 4
  .byte 5
  .byte 6
  .byte 11
  .byte 8
  .byte 9
  .byte 10
  .byte 15
  .byte 12
  .byte 13
  .byte 14
  .byte 3
  .byte 0
  .byte 1
  .byte 2
.LCPI1_3:
  .byte 3
  .byte 0
  .byte 1
  .byte 2
  .byte 7
  .byte 4
  .byte 5
  .byte 6
  .byte 11
  .byte 8
  .byte 9
  .byte 10
  .byte 15
  .byte 12
  .byte 13
  .byte 14
.LCPI1_4:
  .byte 2
  .byte 3
  .byte 0
  .byte 1
  .byte 6
  .byte 7
  .byte 4
  .byte 5
  .byte 10
  .byte 11
  .byte 8
  .byte 9
  .byte 14
  .byte 15
  .byte 12
  .byte 13
chacha20_core:
  vmovdqu 32(%rsi), %xmm5
  vpshufd $147, 16(%rsi), %xmm6
  vpshufd $78, (%rsi), %xmm7
  vpshufd $57, 48(%rsi), %xmm8
  movl $10, %eax
  vmovdqu .LCPI1_0(%rip), %xmm0
  vmovdqu .LCPI1_1(%rip), %xmm1
  vmovdqu .LCPI1_2(%rip), %xmm2
  vmovdqu .LCPI1_3(%rip), %xmm3
  vmovdqu .LCPI1_4(%rip), %xmm4
.LBB1_1:
  vpshufb %xmm1, %xmm8, %xmm8
  vpshufd $57, %xmm7, %xmm7
  vpaddd %xmm6, %xmm7, %xmm7
  vpshufb %xmm0, %xmm7, %xmm9
  vpxor %xmm9, %xmm8, %xmm8
  vpaddd %xmm5, %xmm8, %xmm5
  vpshufd $147, %xmm5, %xmm9
  vpxor %xmm6, %xmm9, %xmm6
  vpsrld $20, %xmm6, %xmm9
  vpslld $12, %xmm6, %xmm6
  vpor %xmm6, %xmm9, %xmm6
  vpaddd %xmm7, %xmm6, %xmm7
  vpshufb %xmm2, %xmm7, %xmm9
  vpshufb %xmm3, %xmm8, %xmm8
  vpxor %xmm8, %xmm9, %xmm8
  vpaddd %xmm5, %xmm8, %xmm5
  vpshufd $147, %xmm5, %xmm9
  vpxor %xmm6, %xmm9, %xmm6
  vpsrld $25, %xmm6, %xmm9
  vpslld $7, %xmm6, %xmm6
  vpor %xmm6, %xmm9, %xmm6
  vpshufd $147, %xmm7, %xmm7
  vpaddd %xmm6, %xmm7, %xmm7
  vpshufb %xmm4, %xmm7, %xmm9
  vpshufb %xmm0, %xmm8, %xmm8
  vpxor %xmm9, %xmm8, %xmm8
  vpaddd %xmm5, %xmm8, %xmm5
  vpxor %xmm6, %xmm5, %xmm6
  vpsrld $20, %xmm6, %xmm9
  vpslld $12, %xmm6, %xmm6
  vpor %xmm6, %xmm9, %xmm6
  vpaddd %xmm7, %xmm6, %xmm7
  vpshufb %xmm3, %xmm7, %xmm9
  vpshufb %xmm3, %xmm8, %xmm8
  vpxor %xmm8, %xmm9, %xmm8
  vpaddd %xmm5, %xmm8, %xmm5
  vpxor %xmm6, %xmm5, %xmm6
  vpsrld $25, %xmm6, %xmm9
  vpslld $7, %xmm6, %xmm6
  vpor %xmm6, %xmm9, %xmm6
  decl %eax
  jne .LBB1_1
  vpshufd $78, %xmm7, %xmm0
  vpshufd $57, %xmm6, %xmm1
  vinserti128 $1, %xmm1, %ymm0, %ymm0
  vpshufd $147, %xmm8, %xmm1
  vinserti128 $1, %xmm1, %ymm5, %ymm1
  vpaddd (%rsi), %ymm0, %ymm0
  vmovdqu %ymm0, (%rdi)
  vpaddd 32(%rsi), %ymm1, %ymm0
  vmovdqu %ymm0, 32(%rdi)
  vzeroupper
  retq

.L.str:
  .asciz "%016llx\n"

check_known_vector.test_in:
  .long 1634760805
  .long 857760878
  .long 2036477234
  .long 1797285236
  .long 50462976
  .long 117835012
  .long 185207048
  .long 252579084
  .long 319951120
  .long 387323156
  .long 454695192
  .long 522067228
  .long 1
  .long 150994944
  .long 1241513984
  .long 0

