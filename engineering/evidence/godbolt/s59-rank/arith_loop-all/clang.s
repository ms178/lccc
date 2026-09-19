.LCPI0_0:
.LCPI0_1:
arith_loop:
  testl %edi, %edi
  jle .LBB0_1
  pushq %rbp
  pushq %r15
  pushq %r14
  pushq %r13
  pushq %r12
  pushq %rbx
  movl $32, -8(%rsp)
  movl $31, -12(%rsp)
  movl $30, -16(%rsp)
  movl $29, -20(%rsp)
  movl $28, -24(%rsp)
  movl $27, -28(%rsp)
  movl $26, -36(%rsp)
  movl $25, -44(%rsp)
  movl $24, -32(%rsp)
  movl $23, -40(%rsp)
  movl $22, %r14d
  movl $21, %r15d
  movl $20, %r12d
  movl $19, %r13d
  movl $18, %eax
  vmovdqa .LCPI0_0(%rip), %ymm0
  vmovdqa .LCPI0_1(%rip), %ymm1
  movl $17, %esi
  movl $16, %ecx
  movl $15, %edx
  movl %edi, %ebp
  movl $14, %edi
  movl $13, %r8d
  movl $12, %r9d
  movl $11, %r10d
  movl $10, %r11d
  movl $9, %ebx
.LBB0_5:
  movl %ebp, -4(%rsp)
  vmovd %ebx, %xmm2
  vpbroadcastd %xmm2, %ymm3
  vpinsrd $1, %r11d, %xmm3, %xmm2
  vmovd %r11d, %xmm5
  vpinsrd $2, %r10d, %xmm2, %xmm4
  vpinsrd $1, %r10d, %xmm5, %xmm5
  vmovd %r10d, %xmm6
  vpinsrd $3, %r9d, %xmm4, %xmm7
  vpinsrd $2, %r9d, %xmm5, %xmm4
  vpinsrd $1, %r9d, %xmm6, %xmm6
  vmovd %r8d, %xmm8
  vpinsrd $3, %r8d, %xmm4, %xmm5
  vpinsrd $2, %r8d, %xmm6, %xmm4
  vpinsrd $1, %edi, %xmm8, %xmm6
  vmovd %edi, %xmm8
  vpinsrd $3, %edi, %xmm4, %xmm4
  vpinsrd $2, %edx, %xmm6, %xmm6
  vpinsrd $3, %ecx, %xmm6, %xmm9
  vpinsrd $1, %edx, %xmm8, %xmm6
  vpinsrd $2, %ecx, %xmm6, %xmm6
  vmovd %edx, %xmm8
  vpinsrd $1, %ecx, %xmm8, %xmm10
  vmovd %esi, %xmm11
  vpinsrd $3, %esi, %xmm6, %xmm8
  vpinsrd $2, %esi, %xmm10, %xmm6
  vpinsrd $1, %eax, %xmm11, %xmm10
  vpinsrd $2, %r13d, %xmm10, %xmm10
  vpinsrd $3, %r12d, %xmm10, %xmm10
  vpermd %ymm0, %ymm1, %ymm11
  vpblendd $128, %ymm3, %ymm11, %ymm3
  vmovd %r15d, %xmm11
  vpinsrd $1, %r14d, %xmm11, %xmm11
  movl -40(%rsp), %ebx
  vpinsrd $2, %ebx, %xmm11, %xmm11
  movl -32(%rsp), %r10d
  vpinsrd $3, %r10d, %xmm11, %xmm11
  vpblendd $3, %ymm2, %ymm0, %ymm2
  vpermq $57, %ymm2, %ymm2
  vpmulld %ymm3, %ymm2, %ymm12
  movl -44(%rsp), %ebp
  vmovd %ebp, %xmm2
  movl -36(%rsp), %r11d
  vpinsrd $1, %r11d, %xmm2, %xmm2
  movl -28(%rsp), %r9d
  vpinsrd $2, %r9d, %xmm2, %xmm2
  movl -24(%rsp), %r8d
  vpinsrd $3, %r8d, %xmm2, %xmm3
  vinserti128 $1, %xmm11, %ymm10, %ymm2
  movl -20(%rsp), %edi
  vmovd %edi, %xmm10
  movl -16(%rsp), %esi
  vpinsrd $1, %esi, %xmm10, %xmm10
  movl -12(%rsp), %edx
  vpinsrd $2, %edx, %xmm10, %xmm10
  movl -8(%rsp), %ecx
  vpinsrd $3, %ecx, %xmm10, %xmm10
  vinserti128 $1, %xmm9, %ymm7, %ymm7
  vmovd %eax, %xmm9
  vpinsrd $1, %r13d, %xmm9, %xmm9
  vpinsrd $2, %r12d, %xmm9, %xmm9
  vpinsrd $3, %r15d, %xmm9, %xmm9
  vinserti128 $1, %xmm10, %ymm3, %ymm3
  vmovd %r14d, %xmm10
  vpinsrd $1, %ebx, %xmm10, %xmm10
  vpinsrd $2, %r10d, %xmm10, %xmm10
  vpinsrd $3, %ebp, %xmm10, %xmm10
  vpaddd %ymm0, %ymm12, %ymm0
  vmovd %r11d, %xmm11
  vpinsrd $1, %r9d, %xmm11, %xmm11
  vpinsrd $2, %r8d, %xmm11, %xmm11
  vpinsrd $3, %edi, %xmm11, %xmm11
  vinserti128 $1, %xmm8, %ymm5, %ymm5
  vmovd %esi, %xmm8
  vpinsrd $1, %edx, %xmm8, %xmm8
  vpinsrd $2, %ecx, %xmm8, %xmm8
  vpinsrd $3, %eax, %xmm6, %xmm6
  vinserti128 $1, %xmm10, %ymm9, %ymm9
  vinserti128 $1, %xmm6, %ymm4, %ymm4
  vpmulld %ymm4, %ymm5, %ymm4
  vinserti128 $1, %xmm8, %ymm11, %ymm5
  vmovd %r13d, %xmm6
  vpinsrd $1, %r12d, %xmm6, %xmm6
  vpinsrd $2, %r15d, %xmm6, %xmm6
  vpinsrd $3, %r14d, %xmm6, %xmm6
  vpbroadcastd %xmm0, %ymm8
  vmovd %ebx, %xmm10
  vpinsrd $1, %r10d, %xmm10, %xmm10
  vpinsrd $2, %ebp, %xmm10, %xmm10
  movl -4(%rsp), %ebp
  vpinsrd $3, %r11d, %xmm10, %xmm10
  vpblendd $128, %ymm8, %ymm5, %ymm5
  vinserti128 $1, %xmm10, %ymm6, %ymm6
  vpmulld %ymm6, %ymm9, %ymm6
  vpaddd %ymm4, %ymm7, %ymm4
  vmovd %r9d, %xmm7
  vpinsrd $1, %r8d, %xmm7, %xmm7
  vpinsrd $2, %edi, %xmm7, %xmm7
  vpinsrd $3, %esi, %xmm7, %xmm7
  vmovd %edx, %xmm8
  vpinsrd $1, %ecx, %xmm8, %xmm8
  vinserti128 $1, %xmm8, %ymm7, %ymm7
  vpbroadcastq %xmm0, %ymm8
  vpblendd $192, %ymm8, %ymm7, %ymm7
  vpmulld %ymm7, %ymm5, %ymm5
  vpaddd %ymm5, %ymm3, %ymm3
  vextracti128 $1, %ymm3, %xmm5
  vpextrd $3, %xmm5, -8(%rsp)
  vpextrd $2, %xmm5, -12(%rsp)
  vpextrd $1, %xmm5, -16(%rsp)
  vpaddd %ymm6, %ymm2, %ymm2
  vmovd %xmm5, -20(%rsp)
  vpextrd $3, %xmm3, -24(%rsp)
  vpextrd $2, %xmm3, -28(%rsp)
  vpextrd $1, %xmm3, -36(%rsp)
  vmovd %xmm3, -44(%rsp)
  vextracti128 $1, %ymm4, %xmm3
  vpextrd $3, %xmm3, %ecx
  vpextrd $2, %xmm3, %edx
  vpextrd $1, %xmm3, %edi
  vpextrd $3, %xmm4, %r9d
  vpextrd $2, %xmm4, %r10d
  vmovd %xmm3, %r8d
  vpextrd $1, %xmm4, %r11d
  vextracti128 $1, %ymm2, %xmm3
  vpextrd $3, %xmm3, -32(%rsp)
  vpextrd $2, %xmm3, -40(%rsp)
  vmovd %xmm4, %ebx
  vpextrd $1, %xmm3, %r14d
  vmovd %xmm3, %r15d
  vpextrd $3, %xmm2, %r12d
  vpextrd $2, %xmm2, %r13d
  vpextrd $1, %xmm2, %eax
  vmovd %xmm2, %esi
  decl %ebp
  jne .LBB0_5
  vmovd %ebx, %xmm1
  vpinsrd $1, %r11d, %xmm1, %xmm1
  vpinsrd $2, %r10d, %xmm1, %xmm1
  vpinsrd $3, %r9d, %xmm1, %xmm1
  vmovd %r8d, %xmm2
  vpinsrd $1, %edi, %xmm2, %xmm2
  vpinsrd $2, %edx, %xmm2, %xmm2
  vpinsrd $3, %ecx, %xmm2, %xmm2
  vmovd -44(%rsp), %xmm3
  vpinsrd $1, -36(%rsp), %xmm3, %xmm3
  vpinsrd $2, -28(%rsp), %xmm3, %xmm3
  vpinsrd $3, -24(%rsp), %xmm3, %xmm3
  vmovd -20(%rsp), %xmm4
  vpinsrd $1, -16(%rsp), %xmm4, %xmm4
  vpinsrd $2, -12(%rsp), %xmm4, %xmm4
  vpinsrd $3, -8(%rsp), %xmm4, %xmm4
  vmovd %esi, %xmm5
  vpinsrd $1, %eax, %xmm5, %xmm5
  vpinsrd $2, %r13d, %xmm5, %xmm5
  vpinsrd $3, %r12d, %xmm5, %xmm5
  vmovd %r15d, %xmm6
  vpinsrd $1, %r14d, %xmm6, %xmm6
  vinserti128 $1, %xmm2, %ymm1, %ymm1
  vinserti128 $1, %xmm4, %ymm3, %ymm2
  vpinsrd $2, -40(%rsp), %xmm6, %xmm3
  vpinsrd $3, -32(%rsp), %xmm3, %xmm3
  vinserti128 $1, %xmm3, %ymm5, %ymm3
  vpxor %ymm3, %ymm1, %ymm1
  vpxor %ymm2, %ymm1, %ymm1
  vextracti128 $1, %ymm1, %xmm2
  vpxor %xmm2, %xmm1, %xmm1
  vpshufd $238, %xmm1, %xmm2
  vpxor %xmm2, %xmm1, %xmm1
  vpshufd $85, %xmm1, %xmm2
  vpxor %xmm2, %xmm1, %xmm1
  vmovd %xmm1, %ecx
  vextracti128 $1, %ymm0, %xmm1
  vpxor %xmm1, %xmm0, %xmm0
  vpshufd $238, %xmm0, %xmm1
  vpxor %xmm1, %xmm0, %xmm0
  vpshufd $85, %xmm0, %xmm1
  vpxor %xmm1, %xmm0, %xmm0
  vmovd %xmm0, %eax
  xorl %ecx, %eax
  popq %rbx
  popq %r12
  popq %r13
  popq %r14
  popq %r15
  popq %rbp
  vzeroupper
  retq
.LBB0_1:
  movl $32, %eax
  retq

main:
  pushq %rax
  movl $10000000, %edi
  callq arith_loop
  movl %eax, 4(%rsp)
  movl 4(%rsp), %esi
  leaq .L.str(%rip), %rdi
  xorl %eax, %eax
  callq printf@PLT
  xorl %eax, %eax
  popq %rcx
  retq

.L.str:

