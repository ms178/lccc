.LCPI0_0:
  .long 7
main:
  movl $42, %ecx
  movl $4, %edx
  leaq array(%rip), %rax
.LBB0_1:
  imull $1664525, %ecx, %esi
  addl $1013904223, %esi
  shrl %esi
  addl $-1000000000, %esi
  movl %esi, -16(%rax,%rdx,4)
  imull $389569705, %ecx, %esi
  addl $1196435762, %esi
  shrl %esi
  addl $-1000000000, %esi
  movl %esi, -12(%rax,%rdx,4)
  imull $-1354167659, %ecx, %esi
  addl $-775096599, %esi
  shrl %esi
  addl $-1000000000, %esi
  movl %esi, -8(%rax,%rdx,4)
  imull $158984081, %ecx, %esi
  addl $-1426500812, %esi
  shrl %esi
  addl $-1000000000, %esi
  movl %esi, -4(%rax,%rdx,4)
  imull $-1432516515, %ecx, %ecx
  addl $1649599747, %ecx
  movl %ecx, %esi
  shrl %esi
  addl $-1000000000, %esi
  movl %esi, (%rax,%rdx,4)
  addq $5, %rdx
  cmpq $10000004, %rdx
  jne .LBB0_1
  vpxor %xmm4, %xmm4, %xmm4
  movl $28, %ecx
  vpxor %xmm5, %xmm5, %xmm5
  vpxor %xmm6, %xmm6, %xmm6
  vpxor %xmm7, %xmm7, %xmm7
.LBB0_3:
  vpmovsxdq -112(%rax,%rcx,4), %ymm0
  vpaddq %ymm0, %ymm4, %ymm0
  vpmovsxdq -96(%rax,%rcx,4), %ymm1
  vpaddq %ymm1, %ymm5, %ymm1
  vpmovsxdq -80(%rax,%rcx,4), %ymm2
  vpmovsxdq -64(%rax,%rcx,4), %ymm3
  vpaddq %ymm2, %ymm6, %ymm2
  vpaddq %ymm3, %ymm7, %ymm3
  vpmovsxdq -48(%rax,%rcx,4), %ymm4
  vpaddq %ymm4, %ymm0, %ymm4
  vpmovsxdq -32(%rax,%rcx,4), %ymm0
  vpaddq %ymm0, %ymm1, %ymm5
  vpmovsxdq -16(%rax,%rcx,4), %ymm0
  vpmovsxdq (%rax,%rcx,4), %ymm1
  vpaddq %ymm0, %ymm2, %ymm6
  vpaddq %ymm1, %ymm3, %ymm7
  addq $32, %rcx
  cmpq $10000028, %rcx
  jne .LBB0_3
  subq $424, %rsp
  vmovdqu %ymm7, 288(%rsp)
  vmovdqu %ymm6, 320(%rsp)
  vmovdqu %ymm5, 352(%rsp)
  vmovdqu %ymm4, 384(%rsp)
  vpxor %xmm4, %xmm4, %xmm4
  movl $28, %ecx
  vpxor %xmm0, %xmm0, %xmm0
  vpxor %xmm5, %xmm5, %xmm5
  vpxor %xmm6, %xmm6, %xmm6
  vpxor %xmm7, %xmm7, %xmm7
.LBB0_5:
  vpmaxsd -112(%rax,%rcx,4), %xmm0, %xmm1
  vpmaxsd -96(%rax,%rcx,4), %xmm0, %xmm2
  vpmaxsd -80(%rax,%rcx,4), %xmm0, %xmm3
  vpmaxsd -64(%rax,%rcx,4), %xmm0, %xmm8
  vpmovzxdq %xmm1, %ymm1
  vpaddq %ymm1, %ymm4, %ymm1
  vpmovzxdq %xmm2, %ymm2
  vpaddq %ymm2, %ymm5, %ymm2
  vpmovzxdq %xmm3, %ymm3
  vpaddq %ymm3, %ymm6, %ymm3
  vpmovzxdq %xmm8, %ymm4
  vpaddq %ymm4, %ymm7, %ymm7
  vpmaxsd -48(%rax,%rcx,4), %xmm0, %xmm4
  vpmaxsd -32(%rax,%rcx,4), %xmm0, %xmm5
  vpmaxsd -16(%rax,%rcx,4), %xmm0, %xmm6
  vpmaxsd (%rax,%rcx,4), %xmm0, %xmm8
  vpmovzxdq %xmm4, %ymm4
  vpaddq %ymm4, %ymm1, %ymm4
  vpmovzxdq %xmm5, %ymm1
  vpaddq %ymm1, %ymm2, %ymm5
  vpmovzxdq %xmm6, %ymm1
  vpaddq %ymm1, %ymm3, %ymm6
  vpmovzxdq %xmm8, %ymm1
  vpaddq %ymm1, %ymm7, %ymm7
  addq $32, %rcx
  cmpq $10000028, %rcx
  jne .LBB0_5
  movl $121, %ecx
  vpbroadcastd array(%rip), %ymm0
  vmovdqa %ymm0, %ymm1
  vmovdqa %ymm0, %ymm2
  vmovdqa %ymm0, %ymm3
.LBB0_7:
  vpmaxsd -480(%rax,%rcx,4), %ymm0, %ymm0
  vpmaxsd -448(%rax,%rcx,4), %ymm1, %ymm1
  vpmaxsd -416(%rax,%rcx,4), %ymm2, %ymm2
  vpmaxsd -384(%rax,%rcx,4), %ymm3, %ymm3
  vpmaxsd -352(%rax,%rcx,4), %ymm0, %ymm0
  vpmaxsd -320(%rax,%rcx,4), %ymm1, %ymm1
  vpmaxsd -288(%rax,%rcx,4), %ymm2, %ymm2
  vpmaxsd -256(%rax,%rcx,4), %ymm3, %ymm3
  vpmaxsd -224(%rax,%rcx,4), %ymm0, %ymm13
  vpmaxsd -192(%rax,%rcx,4), %ymm1, %ymm14
  vpmaxsd -160(%rax,%rcx,4), %ymm2, %ymm12
  vpmaxsd -128(%rax,%rcx,4), %ymm3, %ymm3
  cmpq $9999993, %rcx
  je .LBB0_9
  vpmaxsd -96(%rax,%rcx,4), %ymm13, %ymm0
  vpmaxsd -64(%rax,%rcx,4), %ymm14, %ymm1
  vpmaxsd -32(%rax,%rcx,4), %ymm12, %ymm2
  vpmaxsd (%rax,%rcx,4), %ymm3, %ymm3
  subq $-128, %rcx
  jmp .LBB0_7
.LBB0_9:
  vmovdqu %ymm3, 128(%rsp)
  vmovdqu %ymm4, 256(%rsp)
  vmovups 39999972(%rax), %xmm0
  vmovaps %xmm0, 112(%rsp)
  vmovups 39999956(%rax), %xmm0
  vmovaps %xmm0, 96(%rsp)
  vmovups 39999940(%rax), %xmm0
  vmovaps %xmm0, 80(%rsp)
  vmovups 39999876(%rax), %xmm0
  vmovaps %xmm0, 64(%rsp)
  vmovups 39999892(%rax), %xmm0
  vmovaps %xmm0, 16(%rsp)
  vmovups 39999908(%rax), %xmm0
  vmovaps %xmm0, 32(%rsp)
  vmovups 39999924(%rax), %xmm0
  vmovaps %xmm0, 48(%rsp)
  movl 39999988(%rax), %esi
  movl 39999992(%rax), %r10d
  movl 39999996(%rax), %edi
  movl $56, %edx
  vpbroadcastd .LCPI0_0(%rip), %ymm0
  leaq main.buf(%rip), %rcx
.LBB0_10:
  vmovdqu -224(%rax,%rdx,4), %ymm1
  vmovdqu -192(%rax,%rdx,4), %ymm2
  vmovdqu -160(%rax,%rdx,4), %ymm3
  vmovdqu -128(%rax,%rdx,4), %ymm8
  vpaddd %ymm1, %ymm1, %ymm9
  vpaddd %ymm2, %ymm2, %ymm10
  vpaddd %ymm3, %ymm3, %ymm15
  vpaddd %ymm8, %ymm8, %ymm4
  vpaddd %ymm0, %ymm1, %ymm1
  vpaddd %ymm1, %ymm9, %ymm1
  vpaddd %ymm0, %ymm2, %ymm2
  vpaddd %ymm2, %ymm10, %ymm2
  vpaddd %ymm0, %ymm3, %ymm3
  vpaddd %ymm3, %ymm15, %ymm3
  vpaddd %ymm0, %ymm8, %ymm8
  vpaddd %ymm4, %ymm8, %ymm4
  vmovdqu %ymm1, -224(%rcx,%rdx,4)
  vmovdqu %ymm2, -192(%rcx,%rdx,4)
  vmovdqu %ymm3, -160(%rcx,%rdx,4)
  vmovdqu %ymm4, -128(%rcx,%rdx,4)
  vmovdqu -96(%rax,%rdx,4), %ymm1
  vmovdqu -64(%rax,%rdx,4), %ymm2
  vmovdqu -32(%rax,%rdx,4), %ymm3
  vmovdqu (%rax,%rdx,4), %ymm4
  vpaddd %ymm1, %ymm1, %ymm8
  vpaddd %ymm2, %ymm2, %ymm9
  vpaddd %ymm3, %ymm3, %ymm10
  vpaddd %ymm4, %ymm4, %ymm15
  vpaddd %ymm0, %ymm1, %ymm1
  vpaddd %ymm1, %ymm8, %ymm1
  vpaddd %ymm0, %ymm2, %ymm2
  vpaddd %ymm2, %ymm9, %ymm2
  vpaddd %ymm0, %ymm3, %ymm3
  vpaddd %ymm3, %ymm10, %ymm3
  vpaddd %ymm0, %ymm4, %ymm4
  vpaddd %ymm4, %ymm15, %ymm4
  vmovdqu %ymm1, -96(%rcx,%rdx,4)
  vmovdqu %ymm2, -64(%rcx,%rdx,4)
  vmovdqu %ymm3, -32(%rcx,%rdx,4)
  vmovdqu %ymm4, (%rcx,%rdx,4)
  addq $64, %rdx
  cmpq $10000056, %rdx
  jne .LBB0_10
  vmovdqu %ymm7, 160(%rsp)
  vmovdqu %ymm6, 192(%rsp)
  vmovdqu %ymm5, 224(%rsp)
  vpxor %xmm15, %xmm15, %xmm15
  movl $28, %edx
  vpxor %xmm8, %xmm8, %xmm8
  vpxor %xmm9, %xmm9, %xmm9
  vpxor %xmm10, %xmm10, %xmm10
.LBB0_12:
  vpmovsxdq -112(%rcx,%rdx,4), %ymm0
  vpaddq %ymm0, %ymm15, %ymm0
  vpmovsxdq -96(%rcx,%rdx,4), %ymm1
  vpaddq %ymm1, %ymm8, %ymm1
  vpmovsxdq -80(%rcx,%rdx,4), %ymm2
  vpmovsxdq -64(%rcx,%rdx,4), %ymm3
  vpaddq %ymm2, %ymm9, %ymm2
  vpaddq %ymm3, %ymm10, %ymm3
  vpmovsxdq -48(%rcx,%rdx,4), %ymm4
  vpaddq %ymm4, %ymm0, %ymm15
  vpmovsxdq -32(%rcx,%rdx,4), %ymm0
  vpaddq %ymm0, %ymm1, %ymm8
  vpmovsxdq -16(%rcx,%rdx,4), %ymm0
  vpmovsxdq (%rcx,%rdx,4), %ymm1
  vpaddq %ymm0, %ymm2, %ymm9
  vpaddq %ymm1, %ymm3, %ymm10
  addq $32, %rdx
  cmpq $10000028, %rdx
  jne .LBB0_12
  vpxor %xmm0, %xmm0, %xmm0
  movl $12, %edx
  vpxor %xmm1, %xmm1, %xmm1
  vpxor %xmm2, %xmm2, %xmm2
  vpxor %xmm3, %xmm3, %xmm3
.LBB0_14:
  vpmovzxdq -48(%rax,%rdx,4), %ymm4
  vpmovzxdq -32(%rax,%rdx,4), %ymm5
  vpmovzxdq -16(%rax,%rdx,4), %ymm6
  vpmovzxdq (%rax,%rdx,4), %ymm7
  vpmovzxdq -48(%rcx,%rdx,4), %ymm11
  vpmuldq %ymm4, %ymm11, %ymm4
  vpaddq %ymm0, %ymm4, %ymm0
  vpmovzxdq -32(%rcx,%rdx,4), %ymm4
  vpmuldq %ymm5, %ymm4, %ymm4
  vpaddq %ymm1, %ymm4, %ymm1
  vpmovzxdq -16(%rcx,%rdx,4), %ymm4
  vpmuldq %ymm6, %ymm4, %ymm4
  vpaddq %ymm2, %ymm4, %ymm2
  vpmovzxdq (%rcx,%rdx,4), %ymm4
  vpmuldq %ymm7, %ymm4, %ymm4
  vpaddq %ymm3, %ymm4, %ymm3
  addq $16, %rdx
  cmpq $1000012, %rdx
  jne .LBB0_14
  movl $42, %ecx
  movl $3, %edx
.LBB0_16:
  imull $1664525, %ecx, %r8d
  addl $1013904223, %r8d
  imulq $1374389535, %r8, %r9
  shrq $37, %r9
  imull $100, %r9d, %r9d
  subl %r9d, %r8d
  movl %r8d, -12(%rax,%rdx,4)
  imull $389569705, %ecx, %r8d
  addl $1196435762, %r8d
  imulq $1374389535, %r8, %r9
  shrq $37, %r9
  imull $100, %r9d, %r9d
  subl %r9d, %r8d
  movl %r8d, -8(%rax,%rdx,4)
  imull $-1354167659, %ecx, %r8d
  addl $-775096599, %r8d
  imulq $1374389535, %r8, %r9
  shrq $37, %r9
  imull $100, %r9d, %r9d
  subl %r9d, %r8d
  movl %r8d, -4(%rax,%rdx,4)
  imull $158984081, %ecx, %ecx
  addl $-1426500812, %ecx
  imulq $1374389535, %rcx, %r8
  shrq $37, %r8
  imull $100, %r8d, %r8d
  movl %ecx, %r9d
  subl %r8d, %r9d
  movl %r9d, (%rax,%rdx,4)
  addq $4, %rdx
  cmpq $10003, %rdx
  jne .LBB0_16
  movl $3, %ecx
  movl array(%rip), %edx
  vmovdqu 320(%rsp), %ymm6
  vmovdqu 288(%rsp), %ymm7
.LBB0_18:
  addl -8(%rax,%rcx,4), %edx
  movl %edx, -8(%rax,%rcx,4)
  addl -4(%rax,%rcx,4), %edx
  movl %edx, -4(%rax,%rcx,4)
  addl (%rax,%rcx,4), %edx
  movl %edx, (%rax,%rcx,4)
  addq $3, %rcx
  cmpq $10002, %rcx
  jne .LBB0_18
  vpmaxsd %ymm14, %ymm13, %ymm4
  vpmaxsd %ymm12, %ymm4, %ymm4
  vpmaxsd 128(%rsp), %ymm4, %ymm4
  vextracti128 $1, %ymm4, %xmm5
  vpmaxsd %xmm5, %xmm4, %xmm4
  vpshufd $238, %xmm4, %xmm5
  vpmaxsd %xmm5, %xmm4, %xmm4
  vpshufd $85, %xmm4, %xmm5
  vpbroadcastd %xmm4, %xmm4
  vpmaxsd %xmm5, %xmm4, %xmm4
  vmovdqa 16(%rsp), %xmm5
  vpmaxsd 64(%rsp), %xmm5, %xmm5
  vpmaxsd 32(%rsp), %xmm5, %xmm5
  vpmaxsd 48(%rsp), %xmm5, %xmm5
  vpmaxsd 80(%rsp), %xmm5, %xmm5
  vpmaxsd 96(%rsp), %xmm5, %xmm5
  vpmaxsd 112(%rsp), %xmm5, %xmm5
  vpmaxsd %xmm4, %xmm5, %xmm4
  vpshufd $238, %xmm4, %xmm5
  vpmaxsd %xmm5, %xmm4, %xmm4
  vpshufd $85, %xmm4, %xmm5
  vpmaxsd %xmm5, %xmm4, %xmm4
  vmovd %xmm4, %ecx
  vpaddq %ymm0, %ymm1, %ymm0
  vpaddq %ymm0, %ymm2, %ymm0
  vpaddq %ymm0, %ymm3, %ymm0
  vextracti128 $1, %ymm0, %xmm1
  vpaddq %xmm1, %xmm0, %xmm0
  vpshufd $238, %xmm0, %xmm1
  vpaddq %xmm1, %xmm0, %xmm0
  vmovq %xmm0, %r9
  vpaddq %ymm15, %ymm8, %ymm0
  vpaddq %ymm0, %ymm9, %ymm0
  vpaddq %ymm0, %ymm10, %ymm0
  vextracti128 $1, %ymm0, %xmm1
  vpaddq %xmm1, %xmm0, %xmm0
  vpshufd $238, %xmm0, %xmm1
  vpaddq %xmm1, %xmm0, %xmm0
  vmovq %xmm0, %r8
  vmovdqu 224(%rsp), %ymm0
  vpaddq 256(%rsp), %ymm0, %ymm0
  vpaddq 192(%rsp), %ymm0, %ymm0
  vpaddq 160(%rsp), %ymm0, %ymm0
  vextracti128 $1, %ymm0, %xmm1
  vpaddq %xmm1, %xmm0, %xmm0
  vpshufd $238, %xmm0, %xmm1
  vpaddq %xmm1, %xmm0, %xmm0
  vmovq %xmm0, %rdx
  vmovdqu 352(%rsp), %ymm0
  vpaddq 384(%rsp), %ymm0, %ymm0
  vpaddq %ymm0, %ymm6, %ymm0
  vpaddq %ymm0, %ymm7, %ymm0
  vextracti128 $1, %ymm0, %xmm1
  vpaddq %xmm1, %xmm0, %xmm0
  vpshufd $238, %xmm0, %xmm1
  vpaddq %xmm1, %xmm0, %xmm0
  cmpl %ecx, %esi
  cmovgl %esi, %ecx
  vmovq %xmm0, %rsi
  cmpl %ecx, %r10d
  cmovgl %r10d, %ecx
  cmpl %ecx, %edi
  cmovgl %edi, %ecx
  movl array+39996(%rip), %eax
  movl %eax, (%rsp)
  leaq .L.str(%rip), %rdi
  xorl %eax, %eax
  vzeroupper
  callq printf@PLT
  xorl %eax, %eax
  addq $424, %rsp
  retq

.L.str:
  .asciz "sum=%ld pos=%ld max=%d scaled=%ld dot=%ld prefix=%d\n"

