.LCPI0_0:
.LCPI0_1:
main:
  pushq %rbp
  pushq %r15
  pushq %r14
  pushq %r13
  pushq %r12
  pushq %rbx
  subq $136, %rsp
  vstmxcsr (%rsp)
  orl $32832, (%rsp)
  vldmxcsr (%rsp)
  movq %rsp, %rdi
  movl $check_known_vector.test_in, %esi
  callq chacha20_core
  cmpl $-454561520, (%rsp)
  jne .LBB0_2
  cmpl $358169553, 4(%rsp)
  jne .LBB0_2
  cmpl $-394014517, 56(%rsp)
  movl $2, %ebp
  jne .LBB0_10
  cmpl $1312575650, 60(%rsp)
  jne .LBB0_10
  vmovups .LCPI0_0(%rip), %ymm0
  vmovups %ymm0, (%rsp)
  vmovups .LCPI0_1(%rip), %xmm0
  vmovups %xmm0, 32(%rsp)
  movabsq $81985530642677487, %rax
  movq %rax, 52(%rsp)
  movl $-1985229329, 60(%rsp)
  movl $67372036, %ebp
  xorl %r12d, %r12d
  leaq 64(%rsp), %r14
  movq %rsp, %r15
  xorl %ebx, %ebx
.LBB0_6:
  xorl %r13d, %r13d
.LBB0_7:
  movl %r13d, %eax
  xorl %r12d, %eax
  movl %eax, 48(%rsp)
  movq %r14, %rdi
  movq %r15, %rsi
  vzeroupper
  callq chacha20_core
  movl 64(%rsp), %eax
  movl 92(%rsp), %ecx
  movzbl %al, %edx
  shlq $32, %rax
  movl 124(%rsp), %esi
  orq %rax, %rsi
  shlq $16, %rcx
  xorq %rsi, %rcx
  addq %rcx, %rbx
  xorl %edx, %ebp
  movl %ebp, 16(%rsp)
  incl %r13d
  cmpl $131072, %r13d
  jne .LBB0_7
  incl %r12d
  cmpl $16, %r12d
  jne .LBB0_6
  xorl %ebp, %ebp
  movl $.L.str, %edi
  movq %rbx, %rsi
  xorl %eax, %eax
  callq printf
  jmp .LBB0_10
.LBB0_2:
  movl $2, %ebp
.LBB0_10:
  movl %ebp, %eax
  addq $136, %rsp
  popq %rbx
  popq %r12
  popq %r13
  popq %r14
  popq %r15
  popq %rbp
  retq

.LCPI1_0:
.LCPI1_1:
.LCPI1_2:
.LCPI1_3:
.LCPI1_4:
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

check_known_vector.test_in:

