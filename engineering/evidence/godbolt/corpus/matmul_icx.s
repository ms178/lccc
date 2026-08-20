matmul:
  movl $C+96, %eax
  xorl %ecx, %ecx
.LBB0_1:
  movl $B+96, %edx
  movq %rcx, %rsi
  shlq $11, %rsi
  xorl %edi, %edi
.LBB0_2:
  vbroadcastsd A(%rsi,%rdi,8), %ymm0
  movq $-4, %r8
.LBB0_3:
  vmovupd -64(%rdx,%r8,8), %ymm1
  vfmadd213pd -64(%rax,%r8,8), %ymm0, %ymm1
  vmovupd %ymm1, -64(%rax,%r8,8)
  vmovupd -32(%rdx,%r8,8), %ymm1
  vfmadd213pd -32(%rax,%r8,8), %ymm0, %ymm1
  vmovupd %ymm1, -32(%rax,%r8,8)
  vmovupd (%rdx,%r8,8), %ymm1
  vfmadd213pd (%rax,%r8,8), %ymm0, %ymm1
  vmovupd %ymm1, (%rax,%r8,8)
  vmovupd 32(%rdx,%r8,8), %ymm1
  vfmadd213pd 32(%rax,%r8,8), %ymm0, %ymm1
  vmovupd %ymm1, 32(%rax,%r8,8)
  addq $16, %r8
  cmpq $252, %r8
  jb .LBB0_3
  leaq 1(%rdi), %r8
  addq $2048, %rdx
  cmpq $255, %rdi
  movq %r8, %rdi
  jne .LBB0_2
  leaq 1(%rcx), %rdx
  addq $2048, %rax
  cmpq $255, %rcx
  movq %rdx, %rcx
  jne .LBB0_1
  vzeroupper
  retq

.LCPI1_0:
  .long 0
  .long 1
  .long 2
  .long 3
.LCPI1_1:
  .quad 0x3f70000000000000
main:
  subq $24, %rsp
  vstmxcsr 12(%rsp)
  orl $32832, 12(%rsp)
  vldmxcsr 12(%rsp)
  movl $4, %eax
  xorl %ecx, %ecx
  vmovdqu .LCPI1_0(%rip), %xmm0
  vbroadcastsd .LCPI1_1(%rip), %ymm1
  vpcmpeqd %xmm2, %xmm2, %xmm2
  xorl %edx, %edx
.LBB1_1:
  vmovd %edx, %xmm3
  vpbroadcastd %xmm3, %xmm3
  xorl %esi, %esi
.LBB1_2:
  leal (%rdx,%rsi), %edi
  vmovd %edi, %xmm4
  vpbroadcastd %xmm4, %xmm4
  vpaddd %xmm0, %xmm4, %xmm4
  vcvtdq2pd %xmm4, %ymm4
  vmulpd %ymm1, %ymm4, %ymm4
  vmovupd %ymm4, A(%rcx,%rsi,8)
  vmovd %esi, %xmm4
  vpbroadcastd %xmm4, %xmm4
  vpor %xmm0, %xmm4, %xmm4
  vpmulld %xmm4, %xmm3, %xmm4
  vpsubd %xmm2, %xmm4, %xmm4
  vcvtdq2pd %xmm4, %ymm4
  vmulpd %ymm1, %ymm4, %ymm4
  vmovupd %ymm4, B(%rcx,%rsi,8)
  leal (%rax,%rsi), %edi
  vmovd %edi, %xmm4
  vpbroadcastd %xmm4, %xmm4
  vpaddd %xmm0, %xmm4, %xmm4
  vcvtdq2pd %xmm4, %ymm4
  vmulpd %ymm1, %ymm4, %ymm4
  vmovupd %ymm4, A+32(%rcx,%rsi,8)
  leaq 4(%rsi), %rdi
  vmovd %edi, %xmm4
  vpbroadcastd %xmm4, %xmm4
  vpor %xmm0, %xmm4, %xmm4
  vpmulld %xmm4, %xmm3, %xmm4
  vpsubd %xmm2, %xmm4, %xmm4
  vcvtdq2pd %xmm4, %ymm4
  vmulpd %ymm1, %ymm4, %ymm4
  vmovupd %ymm4, B+32(%rcx,%rsi,8)
  addq $8, %rsi
  cmpq $252, %rdi
  jb .LBB1_2
  leaq 1(%rdx), %rsi
  incq %rax
  addq $2048, %rcx
  cmpq $255, %rdx
  movq %rsi, %rdx
  jne .LBB1_1
  movl $C+96, %eax
  xorl %ecx, %ecx
.LBB1_5:
  movl $B+96, %edx
  movq %rcx, %rsi
  shlq $11, %rsi
  xorl %edi, %edi
.LBB1_6:
  vbroadcastsd A(%rsi,%rdi,8), %ymm0
  movq $-4, %r8
.LBB1_7:
  vmovupd -64(%rdx,%r8,8), %ymm1
  vfmadd213pd -64(%rax,%r8,8), %ymm0, %ymm1
  vmovupd %ymm1, -64(%rax,%r8,8)
  vmovupd -32(%rdx,%r8,8), %ymm1
  vfmadd213pd -32(%rax,%r8,8), %ymm0, %ymm1
  vmovupd %ymm1, -32(%rax,%r8,8)
  vmovupd (%rdx,%r8,8), %ymm1
  vfmadd213pd (%rax,%r8,8), %ymm0, %ymm1
  vmovupd %ymm1, (%rax,%r8,8)
  vmovupd 32(%rdx,%r8,8), %ymm1
  vfmadd213pd 32(%rax,%r8,8), %ymm0, %ymm1
  vmovupd %ymm1, 32(%rax,%r8,8)
  addq $16, %r8
  cmpq $252, %r8
  jb .LBB1_7
  leaq 1(%rdi), %r8
  addq $2048, %rdx
  cmpq $255, %rdi
  movq %r8, %rdi
  jne .LBB1_6
  leaq 1(%rcx), %rdx
  addq $2048, %rax
  cmpq $255, %rcx
  movq %rdx, %rcx
  jne .LBB1_5
  vmovsd C+263168(%rip), %xmm0
  vmovsd %xmm0, 16(%rsp)
  vmovsd 16(%rsp), %xmm0
  movl $.L.str, %edi
  movb $1, %al
  vzeroupper
  callq printf
  xorl %eax, %eax
  addq $24, %rsp
  retq

.L.str:
  .asciz "matmul C[128][128] = %.4f\n"

