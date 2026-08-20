matmul:
  leaq C+224(%rip), %rax
  xorl %ecx, %ecx
  leaq A(%rip), %rdx
  leaq B+224(%rip), %rsi
.LBB0_1:
  movq %rcx, %rdi
  shlq $11, %rdi
  addq %rdx, %rdi
  movq %rsi, %r8
  xorl %r9d, %r9d
.LBB0_2:
  vbroadcastsd (%rdi,%r9,8), %ymm0
  xorl %r10d, %r10d
.LBB0_3:
  vmovupd -224(%r8,%r10,8), %ymm1
  vmovupd -192(%r8,%r10,8), %ymm2
  vmovupd -160(%r8,%r10,8), %ymm3
  vmovupd -128(%r8,%r10,8), %ymm4
  vfmadd213pd -224(%rax,%r10,8), %ymm0, %ymm1
  vfmadd213pd -192(%rax,%r10,8), %ymm0, %ymm2
  vfmadd213pd -160(%rax,%r10,8), %ymm0, %ymm3
  vfmadd213pd -128(%rax,%r10,8), %ymm0, %ymm4
  vmovupd %ymm1, -224(%rax,%r10,8)
  vmovupd %ymm2, -192(%rax,%r10,8)
  vmovupd %ymm3, -160(%rax,%r10,8)
  vmovupd %ymm4, -128(%rax,%r10,8)
  vmovupd -96(%r8,%r10,8), %ymm1
  vmovupd -64(%r8,%r10,8), %ymm2
  vmovupd -32(%r8,%r10,8), %ymm3
  vmovupd (%r8,%r10,8), %ymm4
  vfmadd213pd -96(%rax,%r10,8), %ymm0, %ymm1
  vfmadd213pd -64(%rax,%r10,8), %ymm0, %ymm2
  vfmadd213pd -32(%rax,%r10,8), %ymm0, %ymm3
  vfmadd213pd (%rax,%r10,8), %ymm0, %ymm4
  vmovupd %ymm1, -96(%rax,%r10,8)
  vmovupd %ymm2, -64(%rax,%r10,8)
  vmovupd %ymm3, -32(%rax,%r10,8)
  vmovupd %ymm4, (%rax,%r10,8)
  addq $32, %r10
  cmpq $256, %r10
  jne .LBB0_3
  incq %r9
  addq $2048, %r8
  cmpq $256, %r9
  jne .LBB0_2
  incq %rcx
  addq $2048, %rax
  cmpq $256, %rcx
  jne .LBB0_1
  vzeroupper
  retq

.LCPI1_0:
  .quad 4
.LCPI1_2:
  .quad 4841369599423283200
.LCPI1_3:
  .quad 4985484787499139072
.LCPI1_4:
  .quad 0x4530000000100000
.LCPI1_5:
  .quad 0x3f70000000000000
.LCPI1_8:
  .quad 8
.LCPI1_1:
  .quad 0
  .quad 1
  .quad 2
  .quad 3
.LCPI1_6:
  .long 0
  .long 2
  .long 4
  .long 6
  .long 4
  .long 6
  .long 6
  .long 7
.LCPI1_7:
  .long 4
main:
  subq $56, %rsp
  leaq A+32(%rip), %rax
  leaq B+32(%rip), %rcx
  xorl %edx, %edx
  vbroadcastsd .LCPI1_0(%rip), %ymm0
  vmovupd %ymm0, 16(%rsp)
  vpxor %xmm2, %xmm2, %xmm2
  vpbroadcastq .LCPI1_2(%rip), %ymm3
  vpbroadcastq .LCPI1_3(%rip), %ymm4
  vbroadcastsd .LCPI1_4(%rip), %ymm5
  vbroadcastsd .LCPI1_5(%rip), %ymm6
  vmovdqa .LCPI1_6(%rip), %ymm7
  vpcmpeqd %xmm8, %xmm8, %xmm8
  vpbroadcastd .LCPI1_7(%rip), %xmm9
  vpbroadcastq .LCPI1_8(%rip), %ymm10
.LBB1_1:
  vmovq %rdx, %xmm11
  vpbroadcastq %xmm11, %ymm11
  vpaddq 16(%rsp), %ymm11, %ymm12
  vpermd %ymm11, %ymm7, %ymm13
  xorl %esi, %esi
  vmovdqa .LCPI1_1(%rip), %ymm14
.LBB1_2:
  vpaddq %ymm11, %ymm14, %ymm15
  vpblendd $170, %ymm2, %ymm15, %ymm1
  vpor %ymm3, %ymm1, %ymm1
  vpsrlq $32, %ymm15, %ymm15
  vpor %ymm4, %ymm15, %ymm15
  vsubpd %ymm5, %ymm15, %ymm15
  vaddpd %ymm1, %ymm15, %ymm1
  vmulpd %ymm6, %ymm1, %ymm1
  vmovupd %ymm1, -32(%rax,%rsi,8)
  vpermd %ymm14, %ymm7, %ymm1
  vpmulld %xmm13, %xmm1, %xmm15
  vpsubd %xmm8, %xmm15, %xmm15
  vcvtdq2pd %xmm15, %ymm15
  vmulpd %ymm6, %ymm15, %ymm15
  vmovupd %ymm15, -32(%rcx,%rsi,8)
  vpaddq %ymm12, %ymm14, %ymm15
  vpblendd $170, %ymm2, %ymm15, %ymm0
  vpor %ymm3, %ymm0, %ymm0
  vpsrlq $32, %ymm15, %ymm15
  vpor %ymm4, %ymm15, %ymm15
  vsubpd %ymm5, %ymm15, %ymm15
  vaddpd %ymm0, %ymm15, %ymm0
  vmulpd %ymm6, %ymm0, %ymm0
  vmovupd %ymm0, (%rax,%rsi,8)
  vpaddd %xmm1, %xmm9, %xmm0
  vpmulld %xmm13, %xmm0, %xmm0
  vpsubd %xmm8, %xmm0, %xmm0
  vcvtdq2pd %xmm0, %ymm0
  vmulpd %ymm6, %ymm0, %ymm0
  vmovupd %ymm0, (%rcx,%rsi,8)
  addq $8, %rsi
  vpaddq %ymm10, %ymm14, %ymm14
  cmpq $256, %rsi
  jne .LBB1_2
  incq %rdx
  addq $2048, %rax
  addq $2048, %rcx
  cmpq $256, %rdx
  jne .LBB1_1
  leaq C+224(%rip), %rax
  xorl %ecx, %ecx
  leaq A(%rip), %rdx
  leaq B+224(%rip), %rsi
.LBB1_5:
  movq %rcx, %rdi
  shlq $11, %rdi
  addq %rdx, %rdi
  movq %rsi, %r8
  xorl %r9d, %r9d
.LBB1_6:
  vbroadcastsd (%rdi,%r9,8), %ymm0
  xorl %r10d, %r10d
.LBB1_7:
  vmovupd -224(%r8,%r10,8), %ymm1
  vmovupd -192(%r8,%r10,8), %ymm2
  vmovupd -160(%r8,%r10,8), %ymm3
  vmovupd -128(%r8,%r10,8), %ymm4
  vfmadd213pd -224(%rax,%r10,8), %ymm0, %ymm1
  vfmadd213pd -192(%rax,%r10,8), %ymm0, %ymm2
  vfmadd213pd -160(%rax,%r10,8), %ymm0, %ymm3
  vfmadd213pd -128(%rax,%r10,8), %ymm0, %ymm4
  vmovupd %ymm1, -224(%rax,%r10,8)
  vmovupd %ymm2, -192(%rax,%r10,8)
  vmovupd %ymm3, -160(%rax,%r10,8)
  vmovupd %ymm4, -128(%rax,%r10,8)
  vmovupd -96(%r8,%r10,8), %ymm1
  vmovupd -64(%r8,%r10,8), %ymm2
  vmovupd -32(%r8,%r10,8), %ymm3
  vmovupd (%r8,%r10,8), %ymm4
  vfmadd213pd -96(%rax,%r10,8), %ymm0, %ymm1
  vfmadd213pd -64(%rax,%r10,8), %ymm0, %ymm2
  vfmadd213pd -32(%rax,%r10,8), %ymm0, %ymm3
  vfmadd213pd (%rax,%r10,8), %ymm0, %ymm4
  vmovupd %ymm1, -96(%rax,%r10,8)
  vmovupd %ymm2, -64(%rax,%r10,8)
  vmovupd %ymm3, -32(%rax,%r10,8)
  vmovupd %ymm4, (%rax,%r10,8)
  addq $32, %r10
  cmpq $256, %r10
  jne .LBB1_7
  incq %r9
  addq $2048, %r8
  cmpq $256, %r9
  jne .LBB1_6
  incq %rcx
  addq $2048, %rax
  cmpq $256, %rcx
  jne .LBB1_5
  vmovsd C+263168(%rip), %xmm0
  vmovsd %xmm0, 8(%rsp)
  vmovsd 8(%rsp), %xmm0
  leaq .L.str(%rip), %rdi
  movb $1, %al
  vzeroupper
  callq printf@PLT
  xorl %eax, %eax
  addq $56, %rsp
  retq

.L.str:
  .asciz "matmul C[128][128] = %.4f\n"

