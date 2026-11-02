.LCPI0_0:
.LCPI0_2:
.LCPI0_1:
.LCPI0_4:
main:
  pushq %rbp
  pushq %r15
  pushq %r14
  pushq %r13
  pushq %r12
  pushq %rbx
  subq $104, %rsp
  vstmxcsr 32(%rsp)
  orl $32832, 32(%rsp)
  vldmxcsr 32(%rsp)
  vmovdqu .L__const.check_known_vector.state(%rip), %ymm0
  vmovdqu %ymm0, (%rsp)
  vpxor %xmm0, %xmm0, %xmm0
  vmovdqu %ymm0, 32(%rsp)
  vmovdqu %ymm0, 64(%rsp)
  movl $1633837952, 32(%rsp)
  movl $24, 92(%rsp)
  movq %rsp, %rdi
  leaq 32(%rsp), %rsi
  vzeroupper
  callq sha256_transform
  cmpl $-1166534977, (%rsp)
  jne .LBB0_1
  cmpl $-1895706646, 4(%rsp)
  movl $2, %ebp
  jne .LBB0_9
  cmpl $-234875475, 28(%rsp)
  jne .LBB0_9
  xorl %r12d, %r12d
  vmovdqu .LCPI0_1(%rip), %ymm3
  vpbroadcastq .LCPI0_4(%rip), %xmm5
  movq %rsp, %r14
  leaq 32(%rsp), %r15
  xorl %ebx, %ebx
.LBB0_5:
  movl %r12d, %eax
  xorl $1779033703, %eax
  movl %eax, (%rsp)
  vmovups .LCPI0_0(%rip), %xmm0
  vmovups %xmm0, 4(%rsp)
  movabsq $2270897968188319884, %rcx
  movq %rcx, 20(%rsp)
  movl $1541459225, 28(%rsp)
  movl $-1521486534, %edx
  movl $1541459225, %ecx
  movl $131072, %ebp
  xorl %r13d, %r13d
.LBB0_6:
  movl %eax, %esi
  xorl %r13d, %esi
  movl %esi, 32(%rsp)
  vmovq 4(%rsp), %xmm0
  vmovd %r13d, %xmm1
  vpbroadcastd %xmm1, %ymm1
  vpinsrd $2, %edx, %xmm0, %xmm0
  vpinsrd $3, 16(%rsp), %xmm0, %xmm0
  vpaddd .LCPI0_2(%rip), %xmm1, %xmm2
  vpxor %xmm2, %xmm0, %xmm2
  vmovdqu %xmm2, 68(%rsp)
  vpaddd %ymm3, %ymm1, %ymm2
  vmovq 20(%rsp), %xmm3
  vinserti128 $1, %xmm3, %ymm0, %ymm0
  vmovd %ecx, %xmm4
  vpbroadcastd %xmm4, %ymm4
  vpblendd $64, %ymm4, %ymm0, %ymm0
  vmovd %eax, %xmm4
  vpbroadcastd %xmm4, %ymm4
  vpblendd $128, %ymm4, %ymm0, %ymm0
  vpxor %ymm2, %ymm0, %ymm0
  vmovdqu %ymm0, 36(%rsp)
  vpaddd %xmm5, %xmm1, %xmm0
  vpxor %xmm0, %xmm3, %xmm0
  vmovq %xmm0, 84(%rsp)
  leal -1971305839(%r13), %eax
  xorl %ecx, %eax
  movl %eax, 92(%rsp)
  movq %r14, %rdi
  movq %r15, %rsi
  vzeroupper
  callq sha256_transform
  vpbroadcastq .LCPI0_4(%rip), %xmm5
  vmovdqu .LCPI0_1(%rip), %ymm3
  movl (%rsp), %eax
  movl 12(%rsp), %edx
  movq %rax, %rsi
  shlq $32, %rsi
  movl 28(%rsp), %ecx
  orq %rcx, %rsi
  movq %rdx, %rdi
  shlq $16, %rdi
  xorq %rsi, %rdi
  addq %rdi, %rbx
  addl $1664525, %r13d
  decl %ebp
  jne .LBB0_6
  incl %r12d
  cmpl $8, %r12d
  jne .LBB0_5
  xorl %ebp, %ebp
  movl $.L.str, %edi
  movq %rbx, %rsi
  xorl %eax, %eax
  vzeroupper
  callq printf
  jmp .LBB0_9
.LBB0_1:
  movl $2, %ebp
.LBB0_9:
  movl %ebp, %eax
  addq $104, %rsp
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
.LCPI1_5:
.LCPI1_6:
.LCPI1_7:
sha256_transform:
  pushq %rbp
  pushq %r15
  pushq %r14
  pushq %r12
  pushq %rbx
  subq $128, %rsp
  vmovups (%rsi), %ymm0
  vmovups 32(%rsi), %ymm1
  vmovups %ymm0, -128(%rsp)
  vmovups %ymm1, -96(%rsp)
  vmovdqu -128(%rsp), %xmm9
  vpshufd $233, %xmm9, %xmm10
  vmovq -72(%rsp), %xmm11
  vmovdqu .LCPI1_0(%rip), %xmm0
  vpsrlvd %xmm0, %xmm10, %xmm4
  vmovdqu .LCPI1_1(%rip), %xmm2
  vpsllvd %xmm2, %xmm10, %xmm5
  vmovdqu .LCPI1_2(%rip), %xmm3
  vpsrlvd %xmm3, %xmm10, %xmm6
  vmovdqu .LCPI1_3(%rip), %xmm1
  vpsllvd %xmm1, %xmm10, %xmm7
  vpor %xmm4, %xmm5, %xmm4
  vpor %xmm6, %xmm7, %xmm5
  vpxor %xmm4, %xmm5, %xmm12
  vpsrlvd .LCPI1_4(%rip), %xmm11, %xmm6
  vmovdqu .LCPI1_5(%rip), %xmm5
  vpsllvd %xmm5, %xmm11, %xmm7
  vpor %xmm6, %xmm7, %xmm13
  vmovdqu .LCPI1_6(%rip), %xmm6
  vpsrlvd %xmm6, %xmm11, %xmm8
  vmovdqu .LCPI1_7(%rip), %xmm7
  vpsllvd %xmm7, %xmm11, %xmm14
  vpbroadcastq -120(%rsp), %xmm15
  vpor %xmm8, %xmm14, %xmm14
  vmovdqu -112(%rsp), %xmm8
  vpxor %xmm13, %xmm14, %xmm13
  vpsrld $3, %xmm10, %xmm10
  vpxor %xmm10, %xmm12, %xmm10
  vpsrld $10, %xmm11, %xmm11
  vpxor %xmm11, %xmm13, %xmm11
  vpblendd $2, %xmm15, %xmm8, %xmm12
  vpsrlvd %xmm0, %xmm12, %xmm13
  vmovdqa %xmm0, %xmm4
  vmovq -92(%rsp), %xmm14
  vpsllvd %xmm2, %xmm12, %xmm0
  vpaddd %xmm9, %xmm14, %xmm9
  vpsrlvd %xmm3, %xmm12, %xmm14
  vpaddd %xmm11, %xmm9, %xmm9
  vpsllvd %xmm1, %xmm12, %xmm11
  vpaddd %xmm10, %xmm9, %xmm10
  vpor %xmm0, %xmm13, %xmm0
  vpor %xmm14, %xmm11, %xmm9
  vpxor %xmm0, %xmm9, %xmm0
  vpsrld $3, %xmm12, %xmm9
  vmovdqu .LCPI1_4(%rip), %xmm13
  vpsrlvd %xmm13, %xmm10, %xmm11
  vpxor %xmm0, %xmm9, %xmm0
  vpsllvd %xmm5, %xmm10, %xmm9
  vpor %xmm11, %xmm9, %xmm9
  vpsrlvd %xmm6, %xmm10, %xmm11
  vpsllvd %xmm7, %xmm10, %xmm12
  vpor %xmm11, %xmm12, %xmm11
  vpxor %xmm9, %xmm11, %xmm9
  vmovq -84(%rsp), %xmm11
  vpsrld $10, %xmm10, %xmm12
  vpxor %xmm12, %xmm9, %xmm9
  vpshufd $225, %xmm0, %xmm0
  vpaddd %xmm15, %xmm11, %xmm11
  vpaddd %xmm0, %xmm11, %xmm0
  vpaddd %xmm0, %xmm9, %xmm0
  vpshufd $225, %xmm0, %xmm11
  vpsrlvd %xmm13, %xmm11, %xmm12
  vpsllvd %xmm5, %xmm11, %xmm13
  vpbroadcastq -104(%rsp), %xmm9
  vpsrlvd %xmm6, %xmm11, %xmm14
  vpor %xmm12, %xmm13, %xmm12
  vpsllvd %xmm7, %xmm11, %xmm13
  vpor %xmm14, %xmm13, %xmm13
  vmovq -76(%rsp), %xmm14
  vmovd %xmm10, -64(%rsp)
  vpxor %xmm12, %xmm13, %xmm12
  vpextrd $1, %xmm10, -60(%rsp)
  vpblendd $2, %xmm8, %xmm9, %xmm10
  vpsrlvd %xmm3, %xmm10, %xmm13
  vpsrld $10, %xmm11, %xmm11
  vpsllvd %xmm1, %xmm10, %xmm15
  vpxor %xmm11, %xmm12, %xmm11
  vpsrlvd %xmm4, %xmm10, %xmm12
  vpaddd %xmm8, %xmm14, %xmm8
  vpsllvd %xmm2, %xmm10, %xmm14
  vpor %xmm13, %xmm15, %xmm13
  vpor %xmm12, %xmm14, %xmm12
  vpshufd $225, %xmm8, %xmm8
  vpxor %xmm12, %xmm13, %xmm12
  vpsrld $3, %xmm10, %xmm10
  vpxor %xmm10, %xmm12, %xmm10
  vpaddd %xmm10, %xmm8, %xmm8
  vpaddd %xmm11, %xmm8, %xmm8
  vpshufd $225, %xmm8, %xmm10
  vpunpcklqdq %xmm10, %xmm0, %xmm0
  vmovdqu %xmm0, -56(%rsp)
  vpsrlvd %xmm6, %xmm8, %xmm0
  vpsllvd %xmm7, %xmm8, %xmm10
  vmovdqu .LCPI1_4(%rip), %xmm2
  vpsrlvd %xmm2, %xmm8, %xmm11
  vpor %xmm0, %xmm10, %xmm0
  vpsllvd %xmm5, %xmm8, %xmm10
  vpor %xmm11, %xmm10, %xmm10
  vpxor %xmm0, %xmm10, %xmm0
  vpsrld $10, %xmm8, %xmm8
  vpxor %xmm0, %xmm8, %xmm0
  vmovq -68(%rsp), %xmm8
  vpshufd $225, %xmm9, %xmm10
  vpshufd $225, %xmm8, %xmm8
  vmovq -96(%rsp), %xmm11
  vpblendd $2, %xmm9, %xmm11, %xmm9
  vmovdqa %xmm3, %xmm15
  vpsrlvd %xmm3, %xmm9, %xmm12
  vpaddd %xmm10, %xmm8, %xmm8
  vpsllvd %xmm1, %xmm9, %xmm10
  vmovdqa %xmm1, %xmm3
  vpor %xmm12, %xmm10, %xmm10
  vmovdqa %xmm4, %xmm14
  vpsrlvd %xmm4, %xmm9, %xmm12
  vmovdqu .LCPI1_1(%rip), %xmm1
  vpsllvd %xmm1, %xmm9, %xmm13
  vpor %xmm12, %xmm13, %xmm12
  vpxor %xmm12, %xmm10, %xmm10
  vpsrld $3, %xmm9, %xmm9
  vpxor %xmm9, %xmm10, %xmm9
  vpaddd %xmm9, %xmm8, %xmm8
  vpaddd %xmm0, %xmm8, %xmm0
  vpshufd $225, %xmm0, %xmm8
  vmovq %xmm8, -40(%rsp)
  vpsrlvd %xmm6, %xmm0, %xmm8
  vpsllvd %xmm7, %xmm0, %xmm9
  vpsrlvd %xmm2, %xmm0, %xmm10
  vmovdqa %xmm2, %xmm4
  vpsllvd %xmm5, %xmm0, %xmm12
  vpor %xmm8, %xmm9, %xmm8
  vpor %xmm10, %xmm12, %xmm9
  vpxor %xmm9, %xmm8, %xmm8
  vpsrld $10, %xmm0, %xmm0
  vpxor %xmm0, %xmm8, %xmm0
  vmovq -60(%rsp), %xmm8
  vpaddd %xmm11, %xmm8, %xmm8
  vmovq -88(%rsp), %xmm9
  vpblendd $2, %xmm11, %xmm9, %xmm10
  vpsrlvd %xmm15, %xmm10, %xmm11
  vmovdqa %xmm15, %xmm2
  vpshufd $225, %xmm8, %xmm8
  vpsllvd %xmm3, %xmm10, %xmm12
  vmovdqa %xmm3, %xmm15
  vpor %xmm11, %xmm12, %xmm11
  vpsrlvd %xmm14, %xmm10, %xmm12
  vpsllvd %xmm1, %xmm10, %xmm13
  vpor %xmm12, %xmm13, %xmm12
  vpxor %xmm12, %xmm11, %xmm11
  vpsrld $3, %xmm10, %xmm10
  vpxor %xmm10, %xmm11, %xmm10
  vpaddd %xmm10, %xmm8, %xmm8
  vpaddd %xmm0, %xmm8, %xmm0
  vpshufd $225, %xmm0, %xmm8
  vmovq %xmm8, -32(%rsp)
  vpsrlvd %xmm6, %xmm0, %xmm8
  vpsllvd %xmm7, %xmm0, %xmm10
  vmovdqa %xmm4, %xmm3
  vpsrlvd %xmm4, %xmm0, %xmm11
  vpsllvd %xmm5, %xmm0, %xmm12
  vpor %xmm8, %xmm10, %xmm8
  vpor %xmm11, %xmm12, %xmm10
  vpxor %xmm10, %xmm8, %xmm8
  vpsrld $10, %xmm0, %xmm0
  vpxor %xmm0, %xmm8, %xmm0
  vmovq -52(%rsp), %xmm8
  vpaddd %xmm9, %xmm8, %xmm8
  vmovq -80(%rsp), %xmm10
  vpblendd $2, %xmm9, %xmm10, %xmm9
  vpsrlvd %xmm2, %xmm9, %xmm11
  vpshufd $225, %xmm8, %xmm8
  vpsllvd %xmm15, %xmm9, %xmm12
  vpor %xmm11, %xmm12, %xmm11
  vpsrlvd %xmm14, %xmm9, %xmm12
  vpsllvd %xmm1, %xmm9, %xmm13
  vpor %xmm12, %xmm13, %xmm12
  vpxor %xmm12, %xmm11, %xmm11
  vpsrld $3, %xmm9, %xmm9
  vpxor %xmm9, %xmm11, %xmm9
  vpaddd %xmm9, %xmm8, %xmm8
  vpaddd %xmm0, %xmm8, %xmm0
  vpshufd $225, %xmm0, %xmm8
  vmovq %xmm8, -24(%rsp)
  vpsrlvd %xmm6, %xmm0, %xmm8
  vpsllvd %xmm7, %xmm0, %xmm9
  vpsrlvd %xmm4, %xmm0, %xmm11
  vpsllvd %xmm5, %xmm0, %xmm12
  vpor %xmm8, %xmm9, %xmm8
  vpor %xmm11, %xmm12, %xmm9
  vpxor %xmm9, %xmm8, %xmm8
  vpsrld $10, %xmm0, %xmm0
  vpxor %xmm0, %xmm8, %xmm0
  vmovq -44(%rsp), %xmm8
  vpaddd %xmm10, %xmm8, %xmm8
  vmovq -72(%rsp), %xmm9
  vpblendd $2, %xmm10, %xmm9, %xmm10
  vpsrlvd %xmm2, %xmm10, %xmm11
  vpshufd $225, %xmm8, %xmm8
  vpsllvd %xmm15, %xmm10, %xmm12
  vpor %xmm11, %xmm12, %xmm11
  vpsrlvd %xmm14, %xmm10, %xmm12
  vpsllvd %xmm1, %xmm10, %xmm13
  vpor %xmm12, %xmm13, %xmm12
  vpxor %xmm12, %xmm11, %xmm11
  vpsrld $3, %xmm10, %xmm10
  vpxor %xmm10, %xmm11, %xmm10
  vpaddd %xmm10, %xmm8, %xmm8
  vpaddd %xmm0, %xmm8, %xmm0
  vpshufd $225, %xmm0, %xmm8
  vmovq %xmm8, -16(%rsp)
  vpsrlvd %xmm6, %xmm0, %xmm8
  vpsllvd %xmm7, %xmm0, %xmm10
  vpsrlvd %xmm4, %xmm0, %xmm11
  vpsllvd %xmm5, %xmm0, %xmm12
  vpor %xmm8, %xmm10, %xmm8
  vpor %xmm11, %xmm12, %xmm10
  vpxor %xmm10, %xmm8, %xmm8
  vpsrld $10, %xmm0, %xmm0
  vpxor %xmm0, %xmm8, %xmm0
  vmovq -36(%rsp), %xmm8
  vpaddd %xmm9, %xmm8, %xmm8
  vmovq -64(%rsp), %xmm10
  vpblendd $2, %xmm9, %xmm10, %xmm9
  vpsrlvd %xmm2, %xmm9, %xmm11
  vpshufd $225, %xmm8, %xmm8
  vpsllvd %xmm15, %xmm9, %xmm12
  vpor %xmm11, %xmm12, %xmm11
  vpsrlvd %xmm14, %xmm9, %xmm12
  vpsllvd %xmm1, %xmm9, %xmm13
  vpor %xmm12, %xmm13, %xmm12
  vpxor %xmm12, %xmm11, %xmm11
  vpsrld $3, %xmm9, %xmm9
  vpxor %xmm9, %xmm11, %xmm9
  vpaddd %xmm9, %xmm8, %xmm8
  vpaddd %xmm0, %xmm8, %xmm0
  vpshufd $225, %xmm0, %xmm8
  vmovq %xmm8, -8(%rsp)
  vpsrlvd %xmm6, %xmm0, %xmm8
  vpsllvd %xmm7, %xmm0, %xmm9
  vpsrlvd %xmm4, %xmm0, %xmm11
  vpsllvd %xmm5, %xmm0, %xmm12
  vpor %xmm8, %xmm9, %xmm8
  vpor %xmm11, %xmm12, %xmm9
  vpxor %xmm9, %xmm8, %xmm8
  vpsrld $10, %xmm0, %xmm0
  vpxor %xmm0, %xmm8, %xmm0
  vmovq -28(%rsp), %xmm8
  vpaddd %xmm10, %xmm8, %xmm8
  vmovq -56(%rsp), %xmm9
  vpblendd $2, %xmm10, %xmm9, %xmm10
  vpsrlvd %xmm2, %xmm10, %xmm11
  vpshufd $225, %xmm8, %xmm8
  vpsllvd %xmm15, %xmm10, %xmm12
  vpor %xmm11, %xmm12, %xmm11
  vpsrlvd %xmm14, %xmm10, %xmm12
  vpsllvd %xmm1, %xmm10, %xmm13
  vpor %xmm12, %xmm13, %xmm12
  vpxor %xmm12, %xmm11, %xmm11
  vpsrld $3, %xmm10, %xmm10
  vpxor %xmm10, %xmm11, %xmm10
  vpaddd %xmm10, %xmm8, %xmm8
  vpaddd %xmm0, %xmm8, %xmm0
  vpshufd $225, %xmm0, %xmm8
  vmovq %xmm8, (%rsp)
  vpsrlvd %xmm6, %xmm0, %xmm8
  vpsllvd %xmm7, %xmm0, %xmm10
  vpsrlvd %xmm4, %xmm0, %xmm11
  vpsllvd %xmm5, %xmm0, %xmm12
  vpor %xmm8, %xmm10, %xmm8
  vpor %xmm11, %xmm12, %xmm10
  vpxor %xmm10, %xmm8, %xmm8
  vpsrld $10, %xmm0, %xmm0
  vpxor %xmm0, %xmm8, %xmm0
  vmovq -20(%rsp), %xmm8
  vpaddd %xmm9, %xmm8, %xmm8
  vmovq -48(%rsp), %xmm10
  vpblendd $2, %xmm9, %xmm10, %xmm9
  vpsrlvd %xmm2, %xmm9, %xmm11
  vpshufd $225, %xmm8, %xmm8
  vpsllvd %xmm15, %xmm9, %xmm12
  vpor %xmm11, %xmm12, %xmm11
  vpsrlvd %xmm14, %xmm9, %xmm12
  vpsllvd %xmm1, %xmm9, %xmm13
  vpor %xmm12, %xmm13, %xmm12
  vpxor %xmm12, %xmm11, %xmm11
  vpsrld $3, %xmm9, %xmm9
  vpxor %xmm9, %xmm11, %xmm9
  vpaddd %xmm9, %xmm8, %xmm8
  vpaddd %xmm0, %xmm8, %xmm0
  vpshufd $225, %xmm0, %xmm8
  vmovq %xmm8, 8(%rsp)
  vpsrlvd %xmm6, %xmm0, %xmm8
  vpsllvd %xmm7, %xmm0, %xmm9
  vpsrlvd %xmm4, %xmm0, %xmm11
  vpsllvd %xmm5, %xmm0, %xmm12
  vpor %xmm8, %xmm9, %xmm8
  vpor %xmm11, %xmm12, %xmm9
  vpxor %xmm9, %xmm8, %xmm8
  vpsrld $10, %xmm0, %xmm0
  vpxor %xmm0, %xmm8, %xmm0
  vmovq -12(%rsp), %xmm8
  vpaddd %xmm10, %xmm8, %xmm8
  vmovq -40(%rsp), %xmm9
  vpblendd $2, %xmm10, %xmm9, %xmm10
  vpsrlvd %xmm2, %xmm10, %xmm11
  vpshufd $225, %xmm8, %xmm8
  vpsllvd %xmm15, %xmm10, %xmm12
  vpor %xmm11, %xmm12, %xmm11
  vpsrlvd %xmm14, %xmm10, %xmm12
  vpsllvd %xmm1, %xmm10, %xmm13
  vpor %xmm12, %xmm13, %xmm12
  vpxor %xmm12, %xmm11, %xmm11
  vpsrld $3, %xmm10, %xmm10
  vpxor %xmm10, %xmm11, %xmm10
  vpaddd %xmm10, %xmm8, %xmm8
  vpaddd %xmm0, %xmm8, %xmm0
  vpshufd $225, %xmm0, %xmm8
  vmovq %xmm8, 16(%rsp)
  vpsrlvd %xmm6, %xmm0, %xmm8
  vpsllvd %xmm7, %xmm0, %xmm10
  vpsrlvd %xmm4, %xmm0, %xmm11
  vpsllvd %xmm5, %xmm0, %xmm12
  vpor %xmm8, %xmm10, %xmm8
  vpor %xmm11, %xmm12, %xmm10
  vpxor %xmm10, %xmm8, %xmm8
  vpsrld $10, %xmm0, %xmm0
  vpxor %xmm0, %xmm8, %xmm0
  vmovq -4(%rsp), %xmm8
  vpaddd %xmm9, %xmm8, %xmm8
  vmovq -32(%rsp), %xmm10
  vpblendd $2, %xmm9, %xmm10, %xmm9
  vpsrlvd %xmm2, %xmm9, %xmm11
  vpshufd $225, %xmm8, %xmm8
  vpsllvd %xmm15, %xmm9, %xmm12
  vpor %xmm11, %xmm12, %xmm11
  vpsrlvd %xmm14, %xmm9, %xmm12
  vpsllvd %xmm1, %xmm9, %xmm13
  vpor %xmm12, %xmm13, %xmm12
  vpxor %xmm12, %xmm11, %xmm11
  vpsrld $3, %xmm9, %xmm9
  vpxor %xmm9, %xmm11, %xmm9
  vpaddd %xmm9, %xmm8, %xmm8
  vpaddd %xmm0, %xmm8, %xmm0
  vpshufd $225, %xmm0, %xmm8
  vmovq %xmm8, 24(%rsp)
  vpsrlvd %xmm6, %xmm0, %xmm8
  vpsllvd %xmm7, %xmm0, %xmm9
  vpsrlvd %xmm4, %xmm0, %xmm11
  vpsllvd %xmm5, %xmm0, %xmm12
  vpor %xmm8, %xmm9, %xmm8
  vpor %xmm11, %xmm12, %xmm9
  vpxor %xmm9, %xmm8, %xmm8
  vpsrld $10, %xmm0, %xmm0
  vpxor %xmm0, %xmm8, %xmm0
  vmovq 4(%rsp), %xmm8
  vpaddd %xmm10, %xmm8, %xmm8
  vmovq -24(%rsp), %xmm9
  vpblendd $2, %xmm10, %xmm9, %xmm10
  vpsrlvd %xmm2, %xmm10, %xmm11
  vpshufd $225, %xmm8, %xmm8
  vpsllvd %xmm15, %xmm10, %xmm12
  vpor %xmm11, %xmm12, %xmm11
  vpsrlvd %xmm14, %xmm10, %xmm12
  vpsllvd %xmm1, %xmm10, %xmm13
  vpor %xmm12, %xmm13, %xmm12
  vpxor %xmm12, %xmm11, %xmm11
  vpsrld $3, %xmm10, %xmm10
  vpxor %xmm10, %xmm11, %xmm10
  vpaddd %xmm10, %xmm8, %xmm8
  vpaddd %xmm0, %xmm8, %xmm0
  vpshufd $225, %xmm0, %xmm8
  vmovq %xmm8, 32(%rsp)
  vpsrlvd %xmm6, %xmm0, %xmm8
  vpsllvd %xmm7, %xmm0, %xmm10
  vpsrlvd %xmm4, %xmm0, %xmm11
  vpsllvd %xmm5, %xmm0, %xmm12
  vpor %xmm8, %xmm10, %xmm8
  vpor %xmm11, %xmm12, %xmm10
  vpxor %xmm10, %xmm8, %xmm8
  vpsrld $10, %xmm0, %xmm0
  vpxor %xmm0, %xmm8, %xmm0
  vmovq 12(%rsp), %xmm8
  vpaddd %xmm9, %xmm8, %xmm8
  vmovq -16(%rsp), %xmm10
  vpblendd $2, %xmm9, %xmm10, %xmm9
  vpsrlvd %xmm2, %xmm9, %xmm11
  vpshufd $225, %xmm8, %xmm8
  vpsllvd %xmm15, %xmm9, %xmm12
  vpor %xmm11, %xmm12, %xmm11
  vpsrlvd %xmm14, %xmm9, %xmm12
  vpsllvd %xmm1, %xmm9, %xmm13
  vpor %xmm12, %xmm13, %xmm12
  vpxor %xmm12, %xmm11, %xmm11
  vpsrld $3, %xmm9, %xmm9
  vpxor %xmm9, %xmm11, %xmm9
  vpaddd %xmm9, %xmm8, %xmm8
  vpaddd %xmm0, %xmm8, %xmm0
  vpshufd $225, %xmm0, %xmm8
  vmovq %xmm8, 40(%rsp)
  vpsrlvd %xmm6, %xmm0, %xmm8
  vpsllvd %xmm7, %xmm0, %xmm9
  vpsrlvd %xmm4, %xmm0, %xmm11
  vpsllvd %xmm5, %xmm0, %xmm12
  vpor %xmm8, %xmm9, %xmm8
  vpor %xmm11, %xmm12, %xmm9
  vpxor %xmm9, %xmm8, %xmm8
  vpsrld $10, %xmm0, %xmm0
  vpxor %xmm0, %xmm8, %xmm0
  vmovq 20(%rsp), %xmm8
  vpaddd %xmm10, %xmm8, %xmm8
  vmovq -8(%rsp), %xmm9
  vpblendd $2, %xmm10, %xmm9, %xmm10
  vpsrlvd %xmm2, %xmm10, %xmm11
  vpshufd $225, %xmm8, %xmm8
  vpsllvd %xmm15, %xmm10, %xmm12
  vpor %xmm11, %xmm12, %xmm11
  vpsrlvd %xmm14, %xmm10, %xmm12
  vpsllvd %xmm1, %xmm10, %xmm13
  vpor %xmm12, %xmm13, %xmm12
  vpxor %xmm12, %xmm11, %xmm11
  vpsrld $3, %xmm10, %xmm10
  vpxor %xmm10, %xmm11, %xmm10
  vpaddd %xmm10, %xmm8, %xmm8
  vpaddd %xmm0, %xmm8, %xmm0
  vpshufd $225, %xmm0, %xmm8
  vmovq %xmm8, 48(%rsp)
  vpsrlvd %xmm6, %xmm0, %xmm8
  vpsllvd %xmm7, %xmm0, %xmm10
  vpsrlvd %xmm4, %xmm0, %xmm11
  vpsllvd %xmm5, %xmm0, %xmm12
  vpor %xmm8, %xmm10, %xmm8
  vpor %xmm11, %xmm12, %xmm10
  vpxor %xmm10, %xmm8, %xmm8
  vpsrld $10, %xmm0, %xmm0
  vpxor %xmm0, %xmm8, %xmm0
  vmovq 28(%rsp), %xmm8
  vpaddd %xmm9, %xmm8, %xmm8
  vmovq (%rsp), %xmm10
  vpblendd $2, %xmm9, %xmm10, %xmm9
  vpsrlvd %xmm2, %xmm9, %xmm11
  vpshufd $225, %xmm8, %xmm8
  vpsllvd %xmm15, %xmm9, %xmm12
  vpor %xmm11, %xmm12, %xmm11
  vpsrlvd %xmm14, %xmm9, %xmm12
  vpsllvd %xmm1, %xmm9, %xmm13
  vpor %xmm12, %xmm13, %xmm12
  vpxor %xmm12, %xmm11, %xmm11
  vpsrld $3, %xmm9, %xmm9
  vpxor %xmm9, %xmm11, %xmm9
  vpaddd %xmm9, %xmm8, %xmm8
  vpaddd %xmm0, %xmm8, %xmm0
  vpshufd $225, %xmm0, %xmm8
  vmovq %xmm8, 56(%rsp)
  vpsrlvd %xmm6, %xmm0, %xmm8
  vpsllvd %xmm7, %xmm0, %xmm9
  vpsrlvd %xmm4, %xmm0, %xmm11
  vpsllvd %xmm5, %xmm0, %xmm12
  vpor %xmm8, %xmm9, %xmm8
  vpor %xmm11, %xmm12, %xmm9
  vpxor %xmm9, %xmm8, %xmm8
  vpsrld $10, %xmm0, %xmm0
  vpxor %xmm0, %xmm8, %xmm0
  vmovq 36(%rsp), %xmm8
  vpaddd %xmm10, %xmm8, %xmm8
  vmovq 8(%rsp), %xmm9
  vpblendd $2, %xmm10, %xmm9, %xmm10
  vpsrlvd %xmm2, %xmm10, %xmm11
  vpshufd $225, %xmm8, %xmm8
  vpsllvd %xmm15, %xmm10, %xmm12
  vpor %xmm11, %xmm12, %xmm11
  vpsrlvd %xmm14, %xmm10, %xmm12
  vpsllvd %xmm1, %xmm10, %xmm13
  vpor %xmm12, %xmm13, %xmm12
  vpxor %xmm12, %xmm11, %xmm11
  vpsrld $3, %xmm10, %xmm10
  vpxor %xmm10, %xmm11, %xmm10
  vpaddd %xmm10, %xmm8, %xmm8
  vpaddd %xmm0, %xmm8, %xmm0
  vpshufd $225, %xmm0, %xmm8
  vmovq %xmm8, 64(%rsp)
  vpsrlvd %xmm6, %xmm0, %xmm8
  vpsllvd %xmm7, %xmm0, %xmm10
  vpsrlvd %xmm4, %xmm0, %xmm11
  vpsllvd %xmm5, %xmm0, %xmm12
  vpor %xmm8, %xmm10, %xmm8
  vpor %xmm11, %xmm12, %xmm10
  vpxor %xmm10, %xmm8, %xmm8
  vpsrld $10, %xmm0, %xmm0
  vpxor %xmm0, %xmm8, %xmm0
  vmovq 44(%rsp), %xmm8
  vpaddd %xmm9, %xmm8, %xmm8
  vmovq 16(%rsp), %xmm10
  vpblendd $2, %xmm9, %xmm10, %xmm9
  vpsrlvd %xmm2, %xmm9, %xmm11
  vpshufd $225, %xmm8, %xmm8
  vpsllvd %xmm15, %xmm9, %xmm12
  vpor %xmm11, %xmm12, %xmm11
  vpsrlvd %xmm14, %xmm9, %xmm12
  vpsllvd %xmm1, %xmm9, %xmm13
  vpor %xmm12, %xmm13, %xmm12
  vpxor %xmm12, %xmm11, %xmm11
  vpsrld $3, %xmm9, %xmm9
  vpxor %xmm9, %xmm11, %xmm9
  vpaddd %xmm9, %xmm8, %xmm8
  vpaddd %xmm0, %xmm8, %xmm0
  vpshufd $225, %xmm0, %xmm8
  vmovq %xmm8, 72(%rsp)
  vpsrlvd %xmm6, %xmm0, %xmm8
  vpsllvd %xmm7, %xmm0, %xmm9
  vpsrlvd %xmm4, %xmm0, %xmm11
  vpsllvd %xmm5, %xmm0, %xmm12
  vpor %xmm8, %xmm9, %xmm8
  vpor %xmm11, %xmm12, %xmm9
  vpxor %xmm9, %xmm8, %xmm8
  vpsrld $10, %xmm0, %xmm0
  vpxor %xmm0, %xmm8, %xmm0
  vmovq 52(%rsp), %xmm8
  vpaddd %xmm10, %xmm8, %xmm8
  vmovq 24(%rsp), %xmm9
  vpblendd $2, %xmm10, %xmm9, %xmm10
  vpsrlvd %xmm2, %xmm10, %xmm11
  vpshufd $225, %xmm8, %xmm8
  vpsllvd %xmm15, %xmm10, %xmm12
  vpor %xmm11, %xmm12, %xmm11
  vpsrlvd %xmm14, %xmm10, %xmm12
  vpsllvd %xmm1, %xmm10, %xmm13
  vpor %xmm12, %xmm13, %xmm12
  vpxor %xmm12, %xmm11, %xmm11
  vpsrld $3, %xmm10, %xmm10
  vpxor %xmm10, %xmm11, %xmm10
  vpaddd %xmm10, %xmm8, %xmm8
  vpaddd %xmm0, %xmm8, %xmm0
  vpshufd $225, %xmm0, %xmm8
  vmovq %xmm8, 80(%rsp)
  vpsrlvd %xmm6, %xmm0, %xmm8
  vpsllvd %xmm7, %xmm0, %xmm10
  vpsrlvd %xmm4, %xmm0, %xmm11
  vpsllvd %xmm5, %xmm0, %xmm12
  vpor %xmm8, %xmm10, %xmm8
  vpor %xmm11, %xmm12, %xmm10
  vpxor %xmm10, %xmm8, %xmm8
  vpsrld $10, %xmm0, %xmm0
  vpxor %xmm0, %xmm8, %xmm0
  vmovq 60(%rsp), %xmm8
  vpaddd %xmm9, %xmm8, %xmm8
  vmovq 32(%rsp), %xmm10
  vpblendd $2, %xmm9, %xmm10, %xmm9
  vpsrlvd %xmm2, %xmm9, %xmm11
  vpshufd $225, %xmm8, %xmm8
  vpsllvd %xmm15, %xmm9, %xmm12
  vpor %xmm11, %xmm12, %xmm11
  vpsrlvd %xmm14, %xmm9, %xmm12
  vpsllvd %xmm1, %xmm9, %xmm13
  vpor %xmm12, %xmm13, %xmm12
  vpxor %xmm12, %xmm11, %xmm11
  vpsrld $3, %xmm9, %xmm9
  vpxor %xmm9, %xmm11, %xmm9
  vpaddd %xmm9, %xmm8, %xmm8
  vpaddd %xmm0, %xmm8, %xmm0
  vpshufd $225, %xmm0, %xmm8
  vmovq %xmm8, 88(%rsp)
  vpsrlvd %xmm6, %xmm0, %xmm8
  vpsllvd %xmm7, %xmm0, %xmm9
  vpsrlvd %xmm4, %xmm0, %xmm11
  vpsllvd %xmm5, %xmm0, %xmm12
  vpor %xmm8, %xmm9, %xmm8
  vpor %xmm11, %xmm12, %xmm9
  vpxor %xmm9, %xmm8, %xmm8
  vpsrld $10, %xmm0, %xmm0
  vpxor %xmm0, %xmm8, %xmm0
  vmovq 68(%rsp), %xmm8
  vpaddd %xmm10, %xmm8, %xmm8
  vmovq 40(%rsp), %xmm9
  vpblendd $2, %xmm10, %xmm9, %xmm10
  vpsrlvd %xmm2, %xmm10, %xmm11
  vpshufd $225, %xmm8, %xmm8
  vpsllvd %xmm15, %xmm10, %xmm12
  vpor %xmm11, %xmm12, %xmm11
  vpsrlvd %xmm14, %xmm10, %xmm12
  vpsllvd %xmm1, %xmm10, %xmm13
  vpor %xmm12, %xmm13, %xmm12
  vpxor %xmm12, %xmm11, %xmm11
  vpsrld $3, %xmm10, %xmm10
  vpxor %xmm10, %xmm11, %xmm10
  vpaddd %xmm10, %xmm8, %xmm8
  vpaddd %xmm0, %xmm8, %xmm0
  vpshufd $225, %xmm0, %xmm8
  vmovq %xmm8, 96(%rsp)
  vpsrlvd %xmm6, %xmm0, %xmm8
  vpsllvd %xmm7, %xmm0, %xmm10
  vpsrlvd %xmm4, %xmm0, %xmm11
  vpsllvd %xmm5, %xmm0, %xmm12
  vpor %xmm8, %xmm10, %xmm8
  vpor %xmm11, %xmm12, %xmm10
  vpxor %xmm10, %xmm8, %xmm8
  vpsrld $10, %xmm0, %xmm0
  vpxor %xmm0, %xmm8, %xmm0
  vmovq 76(%rsp), %xmm8
  vpaddd %xmm9, %xmm8, %xmm8
  vmovq 48(%rsp), %xmm10
  vpblendd $2, %xmm9, %xmm10, %xmm9
  vpsrlvd %xmm2, %xmm9, %xmm11
  vpshufd $225, %xmm8, %xmm8
  vpsllvd %xmm15, %xmm9, %xmm12
  vpor %xmm11, %xmm12, %xmm11
  vpsrlvd %xmm14, %xmm9, %xmm12
  vpsllvd %xmm1, %xmm9, %xmm13
  vpor %xmm12, %xmm13, %xmm12
  vpxor %xmm12, %xmm11, %xmm11
  vpsrld $3, %xmm9, %xmm9
  vpxor %xmm9, %xmm11, %xmm9
  vpaddd %xmm9, %xmm8, %xmm8
  vpaddd %xmm0, %xmm8, %xmm0
  vpshufd $225, %xmm0, %xmm8
  vmovq %xmm8, 104(%rsp)
  vpsrlvd %xmm6, %xmm0, %xmm8
  vpsllvd %xmm7, %xmm0, %xmm9
  vpor %xmm8, %xmm9, %xmm8
  vpsrlvd %xmm4, %xmm0, %xmm9
  vpsllvd %xmm5, %xmm0, %xmm11
  vpor %xmm9, %xmm11, %xmm9
  vpxor %xmm9, %xmm8, %xmm8
  vpsrld $10, %xmm0, %xmm0
  vpxor %xmm0, %xmm8, %xmm0
  vmovq 56(%rsp), %xmm8
  vpblendd $2, %xmm10, %xmm8, %xmm9
  vpsrlvd %xmm2, %xmm9, %xmm11
  vmovq 84(%rsp), %xmm12
  vpsllvd %xmm15, %xmm9, %xmm13
  vpaddd %xmm10, %xmm12, %xmm10
  vpsrlvd %xmm14, %xmm9, %xmm12
  vpor %xmm11, %xmm13, %xmm11
  vpsllvd %xmm1, %xmm9, %xmm13
  vpor %xmm12, %xmm13, %xmm12
  vpxor %xmm12, %xmm11, %xmm11
  vpsrld $3, %xmm9, %xmm9
  vpxor %xmm9, %xmm11, %xmm9
  vpshufd $225, %xmm10, %xmm10
  vpaddd %xmm9, %xmm10, %xmm9
  vpaddd %xmm0, %xmm9, %xmm0
  vpshufd $225, %xmm0, %xmm9
  vpblendd $13, 64(%rsp), %xmm8, %xmm10
  vmovq %xmm9, 112(%rsp)
  vpsrlvd %xmm6, %xmm0, %xmm6
  vpsllvd %xmm7, %xmm0, %xmm7
  vpsrlvd %xmm4, %xmm0, %xmm4
  vpsllvd %xmm5, %xmm0, %xmm5
  vpor %xmm6, %xmm7, %xmm6
  vpsrlvd %xmm2, %xmm10, %xmm2
  vpor %xmm4, %xmm5, %xmm4
  vpsllvd %xmm15, %xmm10, %xmm3
  vpxor %xmm4, %xmm6, %xmm4
  vpsrlvd %xmm14, %xmm10, %xmm5
  vpsrld $10, %xmm0, %xmm0
  vpsllvd %xmm1, %xmm10, %xmm1
  vpxor %xmm0, %xmm4, %xmm0
  vpor %xmm2, %xmm3, %xmm2
  vpor %xmm5, %xmm1, %xmm1
  vpxor %xmm1, %xmm2, %xmm1
  vpsrld $3, %xmm10, %xmm2
  vpxor %xmm2, %xmm1, %xmm1
  vmovq 92(%rsp), %xmm2
  vpaddd %xmm2, %xmm8, %xmm2
  vpshufd $225, %xmm2, %xmm2
  vpaddd %xmm1, %xmm2, %xmm1
  vpaddd %xmm0, %xmm1, %xmm0
  vpshufd $225, %xmm0, %xmm0
  vmovq %xmm0, 120(%rsp)
  vmovdqu (%rdi), %ymm0
  movl (%rdi), %ebx
  movl 4(%rdi), %eax
  movl 8(%rdi), %ecx
  movl 12(%rdi), %r11d
  movl 16(%rdi), %ebp
  movl 20(%rdi), %r12d
  movl 24(%rdi), %r15d
  movl 28(%rdi), %r14d
  xorl %edx, %edx
.LBB1_1:
  movl %ecx, %r10d
  movl %ebp, %r9d
  movl %r12d, %r8d
  movl %r15d, %esi
  movl %eax, %ecx
  movl %ebx, %eax
  rorxl $6, %ebp, %ebx
  rorxl $11, %ebp, %ebp
  xorl %ebx, %ebp
  rorxl $25, %r9d, %ebx
  xorl %ebp, %ebx
  movl %r12d, %ebp
  andl %r9d, %ebp
  addl %r14d, %ebp
  andnl %r15d, %r9d, %r14d
  addl %ebp, %r14d
  addl %ebx, %r14d
  addl K(%rdx), %r14d
  addl -128(%rsp,%rdx), %r14d
  rorxl $2, %eax, %ebx
  rorxl $13, %eax, %ebp
  xorl %ebx, %ebp
  rorxl $22, %eax, %r15d
  xorl %ebp, %r15d
  movl %ecx, %ebp
  xorl %r10d, %ebp
  andl %eax, %ebp
  movl %ecx, %ebx
  andl %r10d, %ebx
  xorl %ebp, %ebx
  addl %r15d, %ebx
  movl %r11d, %ebp
  addl %r14d, %ebp
  addl %r14d, %ebx
  addq $4, %rdx
  movl %esi, %r14d
  movl %r12d, %r15d
  movl %r9d, %r12d
  movl %r10d, %r11d
  cmpq $256, %rdx
  jne .LBB1_1
  vmovd %ebx, %xmm1
  vpinsrd $1, %eax, %xmm1, %xmm1
  vpinsrd $2, %ecx, %xmm1, %xmm1
  vpinsrd $3, %r10d, %xmm1, %xmm1
  vpaddd %xmm0, %xmm1, %xmm1
  vmovdqu %xmm1, (%rdi)
  vmovd %ebp, %xmm1
  vpinsrd $1, %r9d, %xmm1, %xmm1
  vpinsrd $2, %r8d, %xmm1, %xmm1
  vpinsrd $3, %esi, %xmm1, %xmm1
  vextracti128 $1, %ymm0, %xmm0
  vpaddd %xmm0, %xmm1, %xmm0
  vmovdqu %xmm0, 16(%rdi)
  addq $128, %rsp
  popq %rbx
  popq %r12
  popq %r14
  popq %r15
  popq %rbp
  vzeroupper
  retq

.L.str:

.L__const.check_known_vector.state:

K:
