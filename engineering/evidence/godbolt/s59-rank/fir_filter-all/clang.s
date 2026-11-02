.LCPI0_0:
.LCPI0_1:
.LCPI0_2:
.LCPI0_3:
.LCPI0_4:
.LCPI0_5:
.LCPI0_6:
.LCPI0_7:
.LCPI0_8:
.LCPI0_9:
.LCPI0_10:
.LCPI0_11:
.LCPI0_12:
.LCPI0_13:
.LCPI0_14:
.LCPI0_15:
.LCPI0_16:
.LCPI0_17:
.LCPI0_18:
.LCPI0_23:
.LCPI0_24:
.LCPI0_25:
.LCPI0_26:
main:
  pushq %rax
  vmovaps .LCPI0_0(%rip), %ymm0
  vmovaps %ymm0, main.a(%rip)
  vmovaps .LCPI0_1(%rip), %ymm0
  vmovaps %ymm0, main.a+32(%rip)
  vmovaps .LCPI0_2(%rip), %ymm0
  vmovaps %ymm0, main.a+64(%rip)
  vmovaps .LCPI0_3(%rip), %ymm0
  vmovaps %ymm0, main.a+96(%rip)
  vmovaps .LCPI0_4(%rip), %ymm0
  vmovaps %ymm0, main.a+128(%rip)
  vmovaps .LCPI0_5(%rip), %ymm0
  vmovaps %ymm0, main.a+160(%rip)
  vmovaps .LCPI0_6(%rip), %ymm0
  vmovaps %ymm0, main.a+192(%rip)
  vmovaps .LCPI0_7(%rip), %ymm0
  vmovaps %ymm0, main.a+224(%rip)
  vmovaps .LCPI0_8(%rip), %ymm0
  vmovaps %ymm0, main.a+256(%rip)
  vmovaps .LCPI0_9(%rip), %ymm0
  vmovaps %ymm0, main.a+288(%rip)
  vmovaps .LCPI0_10(%rip), %ymm0
  vmovaps %ymm0, main.a+320(%rip)
  vmovaps .LCPI0_11(%rip), %ymm0
  vmovaps %ymm0, main.a+352(%rip)
  vmovaps .LCPI0_12(%rip), %ymm0
  vmovaps %ymm0, main.a+384(%rip)
  vmovaps .LCPI0_13(%rip), %ymm0
  vmovaps %ymm0, main.a+416(%rip)
  vmovaps .LCPI0_14(%rip), %ymm0
  vmovaps %ymm0, main.a+448(%rip)
  vmovaps .LCPI0_15(%rip), %ymm0
  vmovaps %ymm0, main.a+480(%rip)
  vmovaps .LCPI0_16(%rip), %xmm0
  vmovaps %xmm0, main.a+512(%rip)
  leaq main.a+14(%rip), %rcx
  vpbroadcastw main.a(%rip), %xmm7
  xorl %edx, %edx
  vpbroadcastd .LCPI0_17(%rip), %ymm0
  vpbroadcastd .LCPI0_18(%rip), %ymm1
  vpbroadcastd .LCPI0_23(%rip), %ymm2
  vpbroadcastd .LCPI0_24(%rip), %ymm3
  vpbroadcastd .LCPI0_25(%rip), %ymm4
  vpbroadcastd .LCPI0_26(%rip), %ymm5
  leaq main.d(%rip), %rax
.LBB0_1:
  vmovdqu -12(%rcx), %xmm6
  vpalignr $14, %xmm7, %xmm6, %xmm7
  vpmovzxwd %xmm7, %ymm7
  vpmaddwd %ymm0, %ymm7, %ymm7
  vpmovzxwd %xmm6, %ymm8
  vpmaddwd %ymm1, %ymm8, %ymm8
  vpmovzxwd -10(%rcx), %ymm9
  vpmaddwd %ymm1, %ymm9, %ymm9
  vpaddd %ymm9, %ymm8, %ymm8
  vpaddd %ymm7, %ymm8, %ymm7
  vpmovzxwd -8(%rcx), %ymm8
  vpmaddwd %ymm0, %ymm8, %ymm8
  vpmovzxwd -6(%rcx), %ymm9
  vpmaddwd %ymm2, %ymm9, %ymm9
  vpaddd %ymm9, %ymm8, %ymm8
  vpmovzxwd -4(%rcx), %ymm9
  vpmaddwd %ymm3, %ymm9, %ymm9
  vpaddd %ymm9, %ymm8, %ymm8
  vpaddd %ymm7, %ymm8, %ymm7
  vpmovzxwd -2(%rcx), %ymm8
  vpmaddwd %ymm4, %ymm8, %ymm8
  vpmovzxwd (%rcx), %ymm9
  vpmaddwd %ymm5, %ymm9, %ymm9
  vpaddd %ymm9, %ymm8, %ymm8
  vpaddd %ymm7, %ymm8, %ymm7
  vmovdqu %ymm7, (%rdx,%rax)
  addq $16, %rcx
  addq $32, %rdx
  vmovdqa %xmm6, %xmm7
  cmpq $1024, %rdx
  jne .LBB0_1
  movl $146959, %esi
  movabsq $1099511628211, %rcx
  xorl %edx, %edx
.LBB0_3:
  movl (%rax,%rdx,4), %r8d
  movl 4(%rax,%rdx,4), %edi
  movzwl %r8w, %r9d
  xorq %rsi, %r9
  imulq %rcx, %r9
  shrl $16, %r8d
  xorq %r9, %r8
  imulq %rcx, %r8
  movzwl %di, %esi
  xorq %r8, %rsi
  imulq %rcx, %rsi
  shrl $16, %edi
  xorq %rsi, %rdi
  imulq %rcx, %rdi
  movl 8(%rax,%rdx,4), %r8d
  movzwl %r8w, %esi
  xorq %rdi, %rsi
  imulq %rcx, %rsi
  shrl $16, %r8d
  xorq %rsi, %r8
  imulq %rcx, %r8
  movl 12(%rax,%rdx,4), %esi
  movzwl %si, %edi
  xorq %r8, %rdi
  imulq %rcx, %rdi
  shrl $16, %esi
  xorq %rdi, %rsi
  imulq %rcx, %rsi
  addq $4, %rdx
  cmpq $256, %rdx
  jne .LBB0_3
  leaq .L.str(%rip), %rdi
  xorl %eax, %eax
  vzeroupper
  callq printf@PLT
  xorl %eax, %eax
  popq %rcx
  retq

.L.str:

