.LCPI0_0:
  .long 1
count_primes:
  pushq %rbx
  leaq sieve(%rip), %rbx
  movl $10000001, %edx
  movq %rbx, %rdi
  movl $1, %esi
  callq memset@PLT
  movw $0, sieve(%rip)
  movl $4, %r8d
  movl $2, %eax
  movl $16, %edi
  movl $9, %ecx
  movl $3, %edx
  leaq sieve+9(%rip), %rsi
  jmp .LBB0_1
.LBB0_12:
  addq $2, %rax
  movl %eax, %r8d
  imull %eax, %r8d
  addq %rdi, %rcx
  addq %rdi, %rsi
  addq $8, %rdi
  addq $2, %rdx
.LBB0_1:
  cmpb $0, (%rax,%rbx)
  setne %r9b
  cmpl $10000001, %r8d
  setb %r10b
  andb %r9b, %r10b
  cmpb $1, %r10b
  jne .LBB0_4
  movl %r8d, %r8d
.LBB0_3:
  movb $0, (%r8,%rbx)
  addq %rax, %r8
  cmpq $10000001, %r8
  jb .LBB0_3
.LBB0_4:
  cmpq $3162, %rax
  je .LBB0_5
  movq %rax, %r8
  orq $1, %r8
  cmpb $0, (%r8,%rbx)
  je .LBB0_12
  imulq %r8, %r8
  cmpq $10000000, %r8
  ja .LBB0_12
  xorl %r8d, %r8d
.LBB0_11:
  movb $0, (%rsi,%r8)
  addq %rdx, %r8
  leaq (%rcx,%r8), %r9
  cmpq $10000001, %r9
  jb .LBB0_11
  jmp .LBB0_12
.LBB0_5:
  vpxor %xmm3, %xmm3, %xmm3
  movl $58, %eax
  vpxor %xmm0, %xmm0, %xmm0
  vpcmpeqd %xmm1, %xmm1, %xmm1
  vpbroadcastd .LCPI0_0(%rip), %ymm2
  vpxor %xmm5, %xmm5, %xmm5
  vpxor %xmm6, %xmm6, %xmm6
  vpxor %xmm4, %xmm4, %xmm4
.LBB0_6:
  vmovq -56(%rax,%rbx), %xmm7
  vmovq -48(%rax,%rbx), %xmm8
  vmovq -40(%rax,%rbx), %xmm9
  vmovq -32(%rax,%rbx), %xmm10
  vpcmpeqb %xmm0, %xmm7, %xmm7
  vpxor %xmm1, %xmm7, %xmm7
  vpmovzxbd %xmm7, %ymm7
  vpand %ymm2, %ymm7, %ymm7
  vpaddd %ymm7, %ymm3, %ymm3
  vpcmpeqb %xmm0, %xmm8, %xmm7
  vpxor %xmm1, %xmm7, %xmm7
  vpmovzxbd %xmm7, %ymm7
  vpand %ymm2, %ymm7, %ymm7
  vpaddd %ymm7, %ymm5, %ymm5
  vpcmpeqb %xmm0, %xmm9, %xmm7
  vpxor %xmm1, %xmm7, %xmm7
  vpmovzxbd %xmm7, %ymm7
  vpand %ymm2, %ymm7, %ymm7
  vpaddd %ymm7, %ymm6, %ymm6
  vpcmpeqb %xmm0, %xmm10, %xmm7
  vpxor %xmm1, %xmm7, %xmm7
  vpmovzxbd %xmm7, %ymm7
  vpand %ymm2, %ymm7, %ymm7
  vpaddd %ymm7, %ymm4, %ymm4
  cmpq $9999994, %rax
  je .LBB0_13
  vmovq -24(%rax,%rbx), %xmm7
  vmovq -16(%rax,%rbx), %xmm8
  vmovq -8(%rax,%rbx), %xmm9
  vmovq (%rax,%rbx), %xmm10
  vpcmpeqb %xmm0, %xmm7, %xmm7
  vpxor %xmm1, %xmm7, %xmm7
  vpmovzxbd %xmm7, %ymm7
  vpand %ymm2, %ymm7, %ymm7
  vpcmpeqb %xmm0, %xmm8, %xmm8
  vpxor %xmm1, %xmm8, %xmm8
  vpmovzxbd %xmm8, %ymm8
  vpand %ymm2, %ymm8, %ymm8
  vpcmpeqb %xmm0, %xmm9, %xmm9
  vpxor %xmm1, %xmm9, %xmm9
  vpmovzxbd %xmm9, %ymm9
  vpand %ymm2, %ymm9, %ymm9
  vpcmpeqb %xmm0, %xmm10, %xmm10
  vpxor %xmm1, %xmm10, %xmm10
  vpmovzxbd %xmm10, %ymm10
  vpand %ymm2, %ymm10, %ymm10
  vpaddd %ymm7, %ymm3, %ymm3
  vpaddd %ymm5, %ymm8, %ymm5
  vpaddd %ymm6, %ymm9, %ymm6
  vpaddd %ymm4, %ymm10, %ymm4
  addq $64, %rax
  jmp .LBB0_6
.LBB0_13:
  vpaddd %ymm3, %ymm5, %ymm0
  vpaddd %ymm0, %ymm6, %ymm0
  vpaddd %ymm0, %ymm4, %ymm0
  vextracti128 $1, %ymm0, %xmm1
  vpaddd %xmm1, %xmm0, %xmm0
  vpshufd $238, %xmm0, %xmm1
  vpaddd %xmm1, %xmm0, %xmm0
  vpshufd $85, %xmm0, %xmm1
  vpaddd %xmm1, %xmm0, %xmm0
  vpxor %xmm1, %xmm1, %xmm1
  vpblendd $1, %xmm0, %xmm1, %xmm0
  vmovd sieve+9999970(%rip), %xmm2
  vpcmpeqb %xmm1, %xmm2, %xmm3
  vpcmpeqd %xmm2, %xmm2, %xmm2
  vpxor %xmm2, %xmm3, %xmm3
  vpmovzxbd %xmm3, %xmm4
  vpbroadcastd .LCPI0_0(%rip), %xmm3
  vpand %xmm3, %xmm4, %xmm4
  vmovd sieve+9999974(%rip), %xmm5
  vpcmpeqb %xmm1, %xmm5, %xmm5
  vpxor %xmm2, %xmm5, %xmm5
  vpmovzxbd %xmm5, %xmm5
  vpand %xmm3, %xmm5, %xmm5
  vpaddd %xmm5, %xmm4, %xmm4
  vmovd sieve+9999978(%rip), %xmm5
  vpcmpeqb %xmm1, %xmm5, %xmm5
  vpxor %xmm2, %xmm5, %xmm5
  vpmovzxbd %xmm5, %xmm5
  vpand %xmm3, %xmm5, %xmm5
  vpaddd %xmm5, %xmm4, %xmm4
  vmovd sieve+9999982(%rip), %xmm5
  vpcmpeqb %xmm1, %xmm5, %xmm5
  vpxor %xmm2, %xmm5, %xmm5
  vpmovzxbd %xmm5, %xmm5
  vpand %xmm3, %xmm5, %xmm5
  vpaddd %xmm5, %xmm4, %xmm4
  vmovd sieve+9999986(%rip), %xmm5
  vpcmpeqb %xmm1, %xmm5, %xmm5
  vpxor %xmm2, %xmm5, %xmm5
  vpmovzxbd %xmm5, %xmm5
  vpand %xmm3, %xmm5, %xmm5
  vpaddd %xmm5, %xmm4, %xmm4
  vmovd sieve+9999990(%rip), %xmm5
  vpcmpeqb %xmm1, %xmm5, %xmm5
  vpxor %xmm2, %xmm5, %xmm5
  vpmovzxbd %xmm5, %xmm5
  vpand %xmm3, %xmm5, %xmm5
  vpaddd %xmm5, %xmm4, %xmm4
  vmovd sieve+9999994(%rip), %xmm5
  vpcmpeqb %xmm1, %xmm5, %xmm1
  vpxor %xmm2, %xmm1, %xmm1
  vpmovzxbd %xmm1, %xmm1
  vpand %xmm3, %xmm1, %xmm1
  vpaddd %xmm1, %xmm4, %xmm1
  vpaddd %xmm1, %xmm0, %xmm0
  vpshufd $238, %xmm0, %xmm1
  vpaddd %xmm1, %xmm0, %xmm0
  vpshufd $85, %xmm0, %xmm1
  vpaddd %xmm1, %xmm0, %xmm0
  vmovd %xmm0, %eax
  cmpb $1, sieve+9999998(%rip)
  sbbl $-1, %eax
  cmpb $1, sieve+9999999(%rip)
  sbbl $-1, %eax
  cmpb $1, sieve+10000000(%rip)
  sbbl $-1, %eax
  popq %rbx
  vzeroupper
  retq

.LCPI1_0:
  .long 1
main:
  pushq %rbx
  subq $16, %rsp
  leaq sieve(%rip), %rbx
  movl $10000001, %edx
  movq %rbx, %rdi
  movl $1, %esi
  callq memset@PLT
  movw $0, sieve(%rip)
  movl $4, %r8d
  movl $2, %eax
  movl $16, %edi
  movl $9, %ecx
  movl $3, %edx
  leaq sieve+9(%rip), %rsi
  jmp .LBB1_1
.LBB1_12:
  addq $2, %rax
  movl %eax, %r8d
  imull %eax, %r8d
  addq %rdi, %rcx
  addq %rdi, %rsi
  addq $8, %rdi
  addq $2, %rdx
.LBB1_1:
  cmpb $0, (%rax,%rbx)
  setne %r9b
  cmpl $10000001, %r8d
  setb %r10b
  andb %r9b, %r10b
  cmpb $1, %r10b
  jne .LBB1_4
  movl %r8d, %r8d
.LBB1_3:
  movb $0, (%r8,%rbx)
  addq %rax, %r8
  cmpq $10000001, %r8
  jb .LBB1_3
.LBB1_4:
  cmpq $3162, %rax
  je .LBB1_5
  movq %rax, %r8
  orq $1, %r8
  cmpb $0, (%r8,%rbx)
  je .LBB1_12
  imulq %r8, %r8
  cmpq $10000000, %r8
  ja .LBB1_12
  xorl %r8d, %r8d
.LBB1_11:
  movb $0, (%rsi,%r8)
  addq %rdx, %r8
  leaq (%rcx,%r8), %r9
  cmpq $10000001, %r9
  jb .LBB1_11
  jmp .LBB1_12
.LBB1_5:
  vpxor %xmm3, %xmm3, %xmm3
  movl $58, %eax
  vpxor %xmm0, %xmm0, %xmm0
  vpcmpeqd %xmm1, %xmm1, %xmm1
  vpbroadcastd .LCPI1_0(%rip), %ymm2
  vpxor %xmm5, %xmm5, %xmm5
  vpxor %xmm6, %xmm6, %xmm6
  vpxor %xmm4, %xmm4, %xmm4
.LBB1_6:
  vmovq -56(%rax,%rbx), %xmm7
  vmovq -48(%rax,%rbx), %xmm8
  vmovq -40(%rax,%rbx), %xmm9
  vmovq -32(%rax,%rbx), %xmm10
  vpcmpeqb %xmm0, %xmm7, %xmm7
  vpxor %xmm1, %xmm7, %xmm7
  vpmovzxbd %xmm7, %ymm7
  vpand %ymm2, %ymm7, %ymm7
  vpaddd %ymm7, %ymm3, %ymm3
  vpcmpeqb %xmm0, %xmm8, %xmm7
  vpxor %xmm1, %xmm7, %xmm7
  vpmovzxbd %xmm7, %ymm7
  vpand %ymm2, %ymm7, %ymm7
  vpaddd %ymm7, %ymm5, %ymm5
  vpcmpeqb %xmm0, %xmm9, %xmm7
  vpxor %xmm1, %xmm7, %xmm7
  vpmovzxbd %xmm7, %ymm7
  vpand %ymm2, %ymm7, %ymm7
  vpaddd %ymm7, %ymm6, %ymm6
  vpcmpeqb %xmm0, %xmm10, %xmm7
  vpxor %xmm1, %xmm7, %xmm7
  vpmovzxbd %xmm7, %ymm7
  vpand %ymm2, %ymm7, %ymm7
  vpaddd %ymm7, %ymm4, %ymm4
  cmpq $9999994, %rax
  je .LBB1_13
  vmovq -24(%rax,%rbx), %xmm7
  vmovq -16(%rax,%rbx), %xmm8
  vmovq -8(%rax,%rbx), %xmm9
  vmovq (%rax,%rbx), %xmm10
  vpcmpeqb %xmm0, %xmm7, %xmm7
  vpxor %xmm1, %xmm7, %xmm7
  vpmovzxbd %xmm7, %ymm7
  vpand %ymm2, %ymm7, %ymm7
  vpcmpeqb %xmm0, %xmm8, %xmm8
  vpxor %xmm1, %xmm8, %xmm8
  vpmovzxbd %xmm8, %ymm8
  vpand %ymm2, %ymm8, %ymm8
  vpcmpeqb %xmm0, %xmm9, %xmm9
  vpxor %xmm1, %xmm9, %xmm9
  vpmovzxbd %xmm9, %ymm9
  vpand %ymm2, %ymm9, %ymm9
  vpcmpeqb %xmm0, %xmm10, %xmm10
  vpxor %xmm1, %xmm10, %xmm10
  vpmovzxbd %xmm10, %ymm10
  vpand %ymm2, %ymm10, %ymm10
  vpaddd %ymm7, %ymm3, %ymm3
  vpaddd %ymm5, %ymm8, %ymm5
  vpaddd %ymm6, %ymm9, %ymm6
  vpaddd %ymm4, %ymm10, %ymm4
  addq $64, %rax
  jmp .LBB1_6
.LBB1_13:
  vpaddd %ymm3, %ymm5, %ymm0
  vpaddd %ymm0, %ymm6, %ymm0
  vpaddd %ymm0, %ymm4, %ymm0
  vextracti128 $1, %ymm0, %xmm1
  vpaddd %xmm1, %xmm0, %xmm0
  vpshufd $238, %xmm0, %xmm1
  vpaddd %xmm1, %xmm0, %xmm0
  vpshufd $85, %xmm0, %xmm1
  vpaddd %xmm1, %xmm0, %xmm0
  vpxor %xmm1, %xmm1, %xmm1
  vpblendd $1, %xmm0, %xmm1, %xmm0
  vmovd sieve+9999970(%rip), %xmm2
  vpcmpeqb %xmm1, %xmm2, %xmm3
  vpcmpeqd %xmm2, %xmm2, %xmm2
  vpxor %xmm2, %xmm3, %xmm3
  vpmovzxbd %xmm3, %xmm4
  vpbroadcastd .LCPI1_0(%rip), %xmm3
  vpand %xmm3, %xmm4, %xmm4
  vmovd sieve+9999974(%rip), %xmm5
  vpcmpeqb %xmm1, %xmm5, %xmm5
  vpxor %xmm2, %xmm5, %xmm5
  vpmovzxbd %xmm5, %xmm5
  vpand %xmm3, %xmm5, %xmm5
  vpaddd %xmm5, %xmm4, %xmm4
  vmovd sieve+9999978(%rip), %xmm5
  vpcmpeqb %xmm1, %xmm5, %xmm5
  vpxor %xmm2, %xmm5, %xmm5
  vpmovzxbd %xmm5, %xmm5
  vpand %xmm3, %xmm5, %xmm5
  vpaddd %xmm5, %xmm4, %xmm4
  vmovd sieve+9999982(%rip), %xmm5
  vpcmpeqb %xmm1, %xmm5, %xmm5
  vpxor %xmm2, %xmm5, %xmm5
  vpmovzxbd %xmm5, %xmm5
  vpand %xmm3, %xmm5, %xmm5
  vpaddd %xmm5, %xmm4, %xmm4
  vmovd sieve+9999986(%rip), %xmm5
  vpcmpeqb %xmm1, %xmm5, %xmm5
  vpxor %xmm2, %xmm5, %xmm5
  vpmovzxbd %xmm5, %xmm5
  vpand %xmm3, %xmm5, %xmm5
  vpaddd %xmm5, %xmm4, %xmm4
  vmovd sieve+9999990(%rip), %xmm5
  vpcmpeqb %xmm1, %xmm5, %xmm5
  vpxor %xmm2, %xmm5, %xmm5
  vpmovzxbd %xmm5, %xmm5
  vpand %xmm3, %xmm5, %xmm5
  vpaddd %xmm5, %xmm4, %xmm4
  vmovd sieve+9999994(%rip), %xmm5
  vpcmpeqb %xmm1, %xmm5, %xmm1
  vpxor %xmm2, %xmm1, %xmm1
  vpmovzxbd %xmm1, %xmm1
  vpand %xmm3, %xmm1, %xmm1
  vpaddd %xmm1, %xmm4, %xmm1
  vpaddd %xmm1, %xmm0, %xmm0
  vpshufd $238, %xmm0, %xmm1
  vpaddd %xmm1, %xmm0, %xmm0
  vpshufd $85, %xmm0, %xmm1
  vpaddd %xmm1, %xmm0, %xmm0
  vmovd %xmm0, %eax
  cmpb $1, sieve+9999998(%rip)
  sbbl $-1, %eax
  cmpb $1, sieve+9999999(%rip)
  sbbl $-1, %eax
  cmpb $1, sieve+10000000(%rip)
  sbbl $-1, %eax
  movl %eax, 12(%rsp)
  movl 12(%rsp), %edx
  leaq .L.str(%rip), %rdi
  movl $10000000, %esi
  xorl %eax, %eax
  vzeroupper
  callq printf@PLT
  xorl %eax, %eax
  addq $16, %rsp
  popq %rbx
  retq

.L.str:
  .asciz "primes up to %d: %d\n"

