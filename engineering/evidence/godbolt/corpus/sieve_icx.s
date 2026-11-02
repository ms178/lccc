.LCPI0_0:
  .long 1
count_primes:
  pushq %rbp
  pushq %r15
  pushq %r14
  pushq %r13
  pushq %r12
  pushq %rbx
  subq $24, %rsp
  movl $sieve, %edi
  movl $10000001, %edx
  movl $1, %esi
  callq _intel_fast_memset@PLT
  movw $0, sieve(%rip)
  movl $4, %eax
  movl $2, %esi
  movq $-2, %rdi
  movl $16, %r8d
  xorl %r9d, %r9d
  xorl %r10d, %r10d
  xorl %r11d, %r11d
  xorl %r14d, %r14d
  xorl %r15d, %r15d
  xorl %r12d, %r12d
  xorl %r13d, %r13d
  jmp .LBB0_1
.LBB0_18:
  incq %rsi
  movl %esi, %eax
  imull %esi, %eax
  leaq 1(%r13), %rcx
  decq %rdi
  addq $7, %r12
  addq $8, %r8
  addq $6, %r15
  addq $5, %r14
  addq $4, %r11
  addq $3, %r10
  addq $2, %r9
  cmpq $3160, %r13
  movq %rcx, %r13
  je .LBB0_19
.LBB0_1:
  cmpb $0, sieve+2(%r13)
  je .LBB0_18
  cmpl $10000000, %eax
  ja .LBB0_18
  movl %eax, %edx
  leaq (%rsi,%rdx), %rcx
  cmpq $10000002, %rcx
  movl $10000001, %eax
  cmovaeq %rcx, %rax
  subq %rsi, %rax
  xorl %ebx, %ebx
  cmpq %rdx, %rax
  setne %bl
  movq %rdx, 16(%rsp)
  addq %rbx, %rdx
  movq %rdx, 8(%rsp)
  subq %rdx, %rax
  movq %rax, %rdx
  orq %rsi, %rdx
  shrq $32, %rdx
  je .LBB0_4
  xorl %edx, %edx
  divq %rsi
  jmp .LBB0_6
.LBB0_4:
  xorl %edx, %edx
  divl %esi
.LBB0_6:
  leaq (%rax,%rbx), %rbp
  incq %rbp
  cmpq $8, %rbp
  jb .LBB0_12
  cmpq $10000002, %rcx
  movl $10000001, %eax
  cmovaeq %rcx, %rax
  addq %rdi, %rax
  subq 8(%rsp), %rax
  movq %rax, %rdx
  orq %rsi, %rdx
  shrq $32, %rdx
  je .LBB0_8
  xorl %edx, %edx
  divq %rsi
  jmp .LBB0_10
.LBB0_8:
  xorl %edx, %edx
  divl %esi
.LBB0_10:
  addq %rbx, %rax
  incq %rax
  shrq $3, %rax
  movq 16(%rsp), %rdx
  leaq sieve(%rdx), %rdx
.LBB0_11:
  movb $0, (%rdx)
  movb $0, 2(%rdx,%r13)
  movb $0, 4(%rdx,%r9)
  movb $0, 6(%rdx,%r10)
  movb $0, 8(%rdx,%r11)
  movb $0, 10(%rdx,%r14)
  movb $0, 12(%rdx,%r15)
  movb $0, 14(%rdx,%r12)
  addq %r8, %rdx
  decq %rax
  jne .LBB0_11
.LBB0_12:
  movq %rbp, %rax
  andq $-8, %rax
  cmpq %rbp, %rax
  jae .LBB0_18
  cmpq $10000002, %rcx
  movl $10000001, %eax
  cmovbq %rax, %rcx
  addq %rdi, %rcx
  subq 8(%rsp), %rcx
  movq %rcx, %rax
  orq %rsi, %rax
  shrq $32, %rax
  je .LBB0_14
  movq %rcx, %rax
  xorl %edx, %edx
  divq %rsi
  jmp .LBB0_16
.LBB0_14:
  movl %ecx, %eax
  xorl %edx, %edx
  divl %esi
.LBB0_16:
  leaq (%rax,%rbx), %rcx
  incq %rcx
  movq %rcx, %rdx
  shrq $3, %rdx
  andq $-8, %rcx
  notq %rax
  addq %rcx, %rax
  subq %rbx, %rax
  imulq %r8, %rdx
  movq 16(%rsp), %rcx
  addq %rdx, %rcx
  addq $sieve, %rcx
.LBB0_17:
  movb $0, (%rcx)
  addq %rsi, %rcx
  incq %rax
  jne .LBB0_17
  jmp .LBB0_18
.LBB0_19:
  xorl %eax, %eax
  movq $-14, %rcx
.LBB0_20:
  cmpb $1, sieve+16(%rcx)
  sbbl $-1, %eax
  incq %rcx
  jne .LBB0_20
  vmovd %eax, %xmm0
  vpxor %xmm1, %xmm1, %xmm1
  movq $-2, %rax
  vpxor %xmm2, %xmm2, %xmm2
  vpcmpeqd %xmm3, %xmm3, %xmm3
  vpbroadcastd .LCPI0_0(%rip), %ymm4
.LBB0_22:
  vpcmpeqb sieve+18(%rax), %xmm2, %xmm5
  vpxor %xmm3, %xmm5, %xmm5
  vpmovzxbw %xmm5, %ymm5
  vpmovzxwd %xmm5, %ymm6
  vpand %ymm4, %ymm6, %ymm6
  vpaddd %ymm6, %ymm0, %ymm0
  vextracti128 $1, %ymm5, %xmm5
  vpmovzxwd %xmm5, %ymm5
  vpand %ymm4, %ymm5, %ymm5
  vpaddd %ymm5, %ymm1, %ymm1
  addq $16, %rax
  cmpq $9999982, %rax
  jb .LBB0_22
  vpaddd %ymm1, %ymm0, %ymm0
  vextracti128 $1, %ymm0, %xmm1
  vpaddd %xmm1, %xmm0, %xmm0
  vpshufd $238, %xmm0, %xmm1
  vpaddd %xmm1, %xmm0, %xmm0
  vpshufd $85, %xmm0, %xmm1
  vpaddd %xmm1, %xmm0, %xmm0
  vmovd %xmm0, %eax
  movq $-1, %rcx
.LBB0_24:
  cmpb $1, sieve+10000001(%rcx)
  sbbl $-1, %eax
  incq %rcx
  jne .LBB0_24
  addq $24, %rsp
  popq %rbx
  popq %r12
  popq %r13
  popq %r14
  popq %r15
  popq %rbp
  vzeroupper
  retq

.LCPI1_0:
  .long 1
main:
  pushq %rbp
  pushq %r15
  pushq %r14
  pushq %r13
  pushq %r12
  pushq %rbx
  subq $24, %rsp
  vstmxcsr (%rsp)
  orl $32832, (%rsp)
  vldmxcsr (%rsp)
  movl $sieve, %edi
  movl $10000001, %edx
  movl $1, %esi
  callq _intel_fast_memset@PLT
  movw $0, sieve(%rip)
  movl $4, %eax
  movl $2, %esi
  movq $-2, %rdi
  movl $16, %r8d
  xorl %r9d, %r9d
  xorl %r10d, %r10d
  xorl %r11d, %r11d
  xorl %r14d, %r14d
  xorl %r15d, %r15d
  xorl %r12d, %r12d
  xorl %r13d, %r13d
  jmp .LBB1_1
.LBB1_18:
  incq %rsi
  movl %esi, %eax
  imull %esi, %eax
  leaq 1(%r13), %rcx
  decq %rdi
  addq $7, %r12
  addq $8, %r8
  addq $6, %r15
  addq $5, %r14
  addq $4, %r11
  addq $3, %r10
  addq $2, %r9
  cmpq $3160, %r13
  movq %rcx, %r13
  je .LBB1_19
.LBB1_1:
  cmpb $0, sieve+2(%r13)
  je .LBB1_18
  cmpl $10000000, %eax
  ja .LBB1_18
  movl %eax, %edx
  leaq (%rsi,%rdx), %rcx
  cmpq $10000002, %rcx
  movl $10000001, %eax
  cmovaeq %rcx, %rax
  subq %rsi, %rax
  xorl %ebx, %ebx
  cmpq %rdx, %rax
  setne %bl
  movq %rdx, 16(%rsp)
  addq %rbx, %rdx
  movq %rdx, 8(%rsp)
  subq %rdx, %rax
  movq %rax, %rdx
  orq %rsi, %rdx
  shrq $32, %rdx
  je .LBB1_4
  xorl %edx, %edx
  divq %rsi
  jmp .LBB1_6
.LBB1_4:
  xorl %edx, %edx
  divl %esi
.LBB1_6:
  leaq (%rax,%rbx), %rbp
  incq %rbp
  cmpq $8, %rbp
  jb .LBB1_12
  cmpq $10000002, %rcx
  movl $10000001, %eax
  cmovaeq %rcx, %rax
  addq %rdi, %rax
  subq 8(%rsp), %rax
  movq %rax, %rdx
  orq %rsi, %rdx
  shrq $32, %rdx
  je .LBB1_8
  xorl %edx, %edx
  divq %rsi
  jmp .LBB1_10
.LBB1_8:
  xorl %edx, %edx
  divl %esi
.LBB1_10:
  addq %rbx, %rax
  incq %rax
  shrq $3, %rax
  movq 16(%rsp), %rdx
  leaq sieve(%rdx), %rdx
.LBB1_11:
  movb $0, (%rdx)
  movb $0, 2(%rdx,%r13)
  movb $0, 4(%rdx,%r9)
  movb $0, 6(%rdx,%r10)
  movb $0, 8(%rdx,%r11)
  movb $0, 10(%rdx,%r14)
  movb $0, 12(%rdx,%r15)
  movb $0, 14(%rdx,%r12)
  addq %r8, %rdx
  decq %rax
  jne .LBB1_11
.LBB1_12:
  movq %rbp, %rax
  andq $-8, %rax
  cmpq %rbp, %rax
  jae .LBB1_18
  cmpq $10000002, %rcx
  movl $10000001, %eax
  cmovbq %rax, %rcx
  addq %rdi, %rcx
  subq 8(%rsp), %rcx
  movq %rcx, %rax
  orq %rsi, %rax
  shrq $32, %rax
  je .LBB1_14
  movq %rcx, %rax
  xorl %edx, %edx
  divq %rsi
  jmp .LBB1_16
.LBB1_14:
  movl %ecx, %eax
  xorl %edx, %edx
  divl %esi
.LBB1_16:
  leaq (%rax,%rbx), %rcx
  incq %rcx
  movq %rcx, %rdx
  shrq $3, %rdx
  andq $-8, %rcx
  notq %rax
  addq %rcx, %rax
  subq %rbx, %rax
  imulq %r8, %rdx
  movq 16(%rsp), %rcx
  addq %rdx, %rcx
  addq $sieve, %rcx
.LBB1_17:
  movb $0, (%rcx)
  addq %rsi, %rcx
  incq %rax
  jne .LBB1_17
  jmp .LBB1_18
.LBB1_19:
  xorl %eax, %eax
  movq $-14, %rcx
.LBB1_20:
  cmpb $1, sieve+16(%rcx)
  sbbl $-1, %eax
  incq %rcx
  jne .LBB1_20
  vmovd %eax, %xmm0
  vpxor %xmm1, %xmm1, %xmm1
  movq $-2, %rax
  vpxor %xmm2, %xmm2, %xmm2
  vpcmpeqd %xmm3, %xmm3, %xmm3
  vpbroadcastd .LCPI1_0(%rip), %ymm4
.LBB1_22:
  vpcmpeqb sieve+18(%rax), %xmm2, %xmm5
  vpxor %xmm3, %xmm5, %xmm5
  vpmovzxbw %xmm5, %ymm5
  vpmovzxwd %xmm5, %ymm6
  vpand %ymm4, %ymm6, %ymm6
  vpaddd %ymm6, %ymm0, %ymm0
  vextracti128 $1, %ymm5, %xmm5
  vpmovzxwd %xmm5, %ymm5
  vpand %ymm4, %ymm5, %ymm5
  vpaddd %ymm5, %ymm1, %ymm1
  addq $16, %rax
  cmpq $9999982, %rax
  jb .LBB1_22
  vpaddd %ymm1, %ymm0, %ymm0
  vextracti128 $1, %ymm0, %xmm1
  vpaddd %xmm1, %xmm0, %xmm0
  vpshufd $238, %xmm0, %xmm1
  vpaddd %xmm1, %xmm0, %xmm0
  vpshufd $85, %xmm0, %xmm1
  vpaddd %xmm1, %xmm0, %xmm0
  vmovd %xmm0, %eax
  movq $-1, %rcx
.LBB1_24:
  cmpb $1, sieve+10000001(%rcx)
  sbbl $-1, %eax
  incq %rcx
  jne .LBB1_24
  movl %eax, 4(%rsp)
  movl 4(%rsp), %edx
  movl $.L.str, %edi
  movl $10000000, %esi
  xorl %eax, %eax
  vzeroupper
  callq printf
  xorl %eax, %eax
  addq $24, %rsp
  popq %rbx
  popq %r12
  popq %r13
  popq %r14
  popq %r15
  popq %rbp
  retq

.L.str:
  .asciz "primes up to %d: %d\n"

