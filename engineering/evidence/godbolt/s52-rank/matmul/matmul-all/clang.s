matmul:
  subq $1512, %rsp
  leaq A(%rip), %rax
  xorl %ecx, %ecx
  leaq C(%rip), %rdx
  leaq B+2016(%rip), %rsi
.LBB0_1:
  movq %rcx, %r10
  shlq $11, %r10
  leaq (%rdx,%r10), %rdi
  vmovupd (%r10,%rdx), %ymm15
  vmovupd 32(%r10,%rdx), %ymm14
  vmovupd 64(%r10,%rdx), %ymm13
  vmovupd 96(%r10,%rdx), %ymm12
  vmovupd 128(%r10,%rdx), %ymm11
  vmovupd 160(%r10,%rdx), %ymm10
  vmovupd 192(%r10,%rdx), %ymm9
  vmovupd 224(%r10,%rdx), %ymm8
  vmovupd 256(%r10,%rdx), %ymm7
  vmovupd 288(%r10,%rdx), %ymm6
  vmovupd 320(%r10,%rdx), %ymm5
  vmovupd 352(%r10,%rdx), %ymm4
  vmovupd 384(%r10,%rdx), %ymm3
  vmovupd 416(%r10,%rdx), %ymm2
  vmovups 448(%r10,%rdx), %ymm0
  vmovups %ymm0, 1376(%rsp)
  vmovups 480(%r10,%rdx), %ymm0
  vmovups %ymm0, 1440(%rsp)
  movq %rsi, %r8
  xorl %r9d, %r9d
  vmovups 512(%r10,%rdx), %ymm0
  vmovups %ymm0, -128(%rsp)
  vmovups 544(%r10,%rdx), %ymm0
  vmovups %ymm0, -96(%rsp)
  vmovups 576(%r10,%rdx), %ymm0
  vmovups %ymm0, -64(%rsp)
  vmovups 608(%r10,%rdx), %ymm0
  vmovups %ymm0, -32(%rsp)
  vmovups 640(%r10,%rdx), %ymm0
  vmovups %ymm0, (%rsp)
  vmovups 672(%r10,%rdx), %ymm0
  vmovups %ymm0, 32(%rsp)
  vmovups 704(%r10,%rdx), %ymm0
  vmovups %ymm0, 64(%rsp)
  vmovups 736(%r10,%rdx), %ymm0
  vmovups %ymm0, 96(%rsp)
  vmovups 768(%r10,%rdx), %ymm0
  vmovups %ymm0, 128(%rsp)
  vmovups 800(%r10,%rdx), %ymm0
  vmovups %ymm0, 160(%rsp)
  vmovups 832(%r10,%rdx), %ymm0
  vmovups %ymm0, 192(%rsp)
  vmovups 864(%r10,%rdx), %ymm0
  vmovups %ymm0, 224(%rsp)
  vmovups 896(%r10,%rdx), %ymm0
  vmovups %ymm0, 256(%rsp)
  vmovups 928(%r10,%rdx), %ymm0
  vmovups %ymm0, 288(%rsp)
  vmovups 960(%r10,%rdx), %ymm0
  vmovups %ymm0, 320(%rsp)
  vmovups 992(%r10,%rdx), %ymm0
  vmovups %ymm0, 352(%rsp)
  vmovups 1024(%r10,%rdx), %ymm0
  vmovups %ymm0, 384(%rsp)
  vmovups 1056(%r10,%rdx), %ymm0
  vmovups %ymm0, 416(%rsp)
  vmovups 1088(%r10,%rdx), %ymm0
  vmovups %ymm0, 448(%rsp)
  vmovups 1120(%r10,%rdx), %ymm0
  vmovups %ymm0, 480(%rsp)
  vmovups 1152(%r10,%rdx), %ymm0
  vmovups %ymm0, 512(%rsp)
  vmovups 1184(%r10,%rdx), %ymm0
  vmovups %ymm0, 544(%rsp)
  vmovups 1216(%r10,%rdx), %ymm0
  vmovups %ymm0, 576(%rsp)
  vmovups 1248(%r10,%rdx), %ymm0
  vmovups %ymm0, 608(%rsp)
  vmovups 1280(%r10,%rdx), %ymm0
  vmovups %ymm0, 640(%rsp)
  vmovups 1312(%r10,%rdx), %ymm0
  vmovups %ymm0, 672(%rsp)
  vmovups 1344(%r10,%rdx), %ymm0
  vmovups %ymm0, 704(%rsp)
  vmovups 1376(%r10,%rdx), %ymm0
  vmovups %ymm0, 736(%rsp)
  vmovups 1408(%r10,%rdx), %ymm0
  vmovups %ymm0, 768(%rsp)
  vmovups 1440(%r10,%rdx), %ymm0
  vmovups %ymm0, 800(%rsp)
  vmovups 1472(%r10,%rdx), %ymm0
  vmovups %ymm0, 832(%rsp)
  vmovups 1504(%r10,%rdx), %ymm0
  vmovups %ymm0, 864(%rsp)
  vmovups 1536(%r10,%rdx), %ymm0
  vmovups %ymm0, 896(%rsp)
  vmovups 1568(%r10,%rdx), %ymm0
  vmovups %ymm0, 928(%rsp)
  vmovups 1600(%r10,%rdx), %ymm0
  vmovups %ymm0, 960(%rsp)
  vmovups 1632(%r10,%rdx), %ymm0
  vmovups %ymm0, 992(%rsp)
  vmovups 1664(%r10,%rdx), %ymm0
  vmovups %ymm0, 1024(%rsp)
  vmovups 1696(%r10,%rdx), %ymm0
  vmovups %ymm0, 1056(%rsp)
  vmovups 1728(%r10,%rdx), %ymm0
  vmovups %ymm0, 1088(%rsp)
  vmovups 1760(%r10,%rdx), %ymm0
  vmovups %ymm0, 1120(%rsp)
  vmovups 1792(%r10,%rdx), %ymm0
  vmovups %ymm0, 1152(%rsp)
  vmovups 1824(%r10,%rdx), %ymm0
  vmovups %ymm0, 1184(%rsp)
  vmovups 1856(%r10,%rdx), %ymm0
  vmovups %ymm0, 1216(%rsp)
  vmovups 1888(%r10,%rdx), %ymm0
  vmovups %ymm0, 1248(%rsp)
  vmovups 1920(%r10,%rdx), %ymm0
  vmovups %ymm0, 1280(%rsp)
  vmovups 1952(%r10,%rdx), %ymm0
  vmovups %ymm0, 1312(%rsp)
  vmovups 1984(%r10,%rdx), %ymm0
  vmovups %ymm0, 1344(%rsp)
  vmovups 2016(%r10,%rdx), %ymm0
  vmovups %ymm0, 1408(%rsp)
.LBB0_2:
  vbroadcastsd (%rax,%r9,8), %ymm0
  vfmadd231pd -2016(%r8), %ymm0, %ymm15
  vmovupd %ymm15, 1472(%rsp)
  vfmadd231pd -1984(%r8), %ymm0, %ymm14
  vfmadd231pd -1952(%r8), %ymm0, %ymm13
  vfmadd231pd -1920(%r8), %ymm0, %ymm12
  vfmadd231pd -1888(%r8), %ymm0, %ymm11
  vfmadd231pd -1856(%r8), %ymm0, %ymm10
  vfmadd231pd -1824(%r8), %ymm0, %ymm9
  vfmadd231pd -1792(%r8), %ymm0, %ymm8
  vfmadd231pd -1760(%r8), %ymm0, %ymm7
  vfmadd231pd -1728(%r8), %ymm0, %ymm6
  vfmadd231pd -1696(%r8), %ymm0, %ymm5
  vfmadd231pd -1664(%r8), %ymm0, %ymm4
  vfmadd231pd -1632(%r8), %ymm0, %ymm3
  vfmadd231pd -1600(%r8), %ymm0, %ymm2
  vmovupd 1376(%rsp), %ymm1
  vfmadd231pd -1568(%r8), %ymm0, %ymm1
  vmovupd %ymm1, 1376(%rsp)
  vmovapd %ymm14, %ymm15
  vmovapd %ymm13, %ymm14
  vmovapd %ymm12, %ymm13
  vmovapd %ymm11, %ymm12
  vmovapd %ymm10, %ymm11
  vmovapd %ymm9, %ymm10
  vmovapd %ymm8, %ymm9
  vmovapd %ymm7, %ymm8
  vmovapd %ymm6, %ymm7
  vmovapd %ymm5, %ymm6
  vmovapd %ymm4, %ymm5
  vmovupd 1440(%rsp), %ymm1
  vfmadd231pd -1536(%r8), %ymm0, %ymm1
  vmovupd %ymm1, 1440(%rsp)
  vmovapd %ymm5, %ymm4
  vmovapd %ymm6, %ymm5
  vmovapd %ymm7, %ymm6
  vmovapd %ymm8, %ymm7
  vmovapd %ymm9, %ymm8
  vmovapd %ymm10, %ymm9
  vmovapd %ymm11, %ymm10
  vmovapd %ymm12, %ymm11
  vmovapd %ymm13, %ymm12
  vmovapd %ymm14, %ymm13
  vmovapd %ymm15, %ymm14
  vmovups 1472(%rsp), %ymm15
  vmovupd -128(%rsp), %ymm1
  vfmadd231pd -1504(%r8), %ymm0, %ymm1
  vmovupd %ymm1, -128(%rsp)
  vmovupd -96(%rsp), %ymm1
  vfmadd231pd -1472(%r8), %ymm0, %ymm1
  vmovupd %ymm1, -96(%rsp)
  vmovupd -64(%rsp), %ymm1
  vfmadd231pd -1440(%r8), %ymm0, %ymm1
  vmovupd %ymm1, -64(%rsp)
  vmovupd -32(%rsp), %ymm1
  vfmadd231pd -1408(%r8), %ymm0, %ymm1
  vmovupd %ymm1, -32(%rsp)
  vmovupd (%rsp), %ymm1
  vfmadd231pd -1376(%r8), %ymm0, %ymm1
  vmovupd %ymm1, (%rsp)
  vmovupd 32(%rsp), %ymm1
  vfmadd231pd -1344(%r8), %ymm0, %ymm1
  vmovupd %ymm1, 32(%rsp)
  vmovupd 64(%rsp), %ymm1
  vfmadd231pd -1312(%r8), %ymm0, %ymm1
  vmovupd %ymm1, 64(%rsp)
  vmovupd 96(%rsp), %ymm1
  vfmadd231pd -1280(%r8), %ymm0, %ymm1
  vmovupd %ymm1, 96(%rsp)
  vmovupd 128(%rsp), %ymm1
  vfmadd231pd -1248(%r8), %ymm0, %ymm1
  vmovupd %ymm1, 128(%rsp)
  vmovupd 160(%rsp), %ymm1
  vfmadd231pd -1216(%r8), %ymm0, %ymm1
  vmovupd %ymm1, 160(%rsp)
  vmovupd 192(%rsp), %ymm1
  vfmadd231pd -1184(%r8), %ymm0, %ymm1
  vmovupd %ymm1, 192(%rsp)
  vmovupd 224(%rsp), %ymm1
  vfmadd231pd -1152(%r8), %ymm0, %ymm1
  vmovupd %ymm1, 224(%rsp)
  vmovupd 256(%rsp), %ymm1
  vfmadd231pd -1120(%r8), %ymm0, %ymm1
  vmovupd %ymm1, 256(%rsp)
  vmovupd 288(%rsp), %ymm1
  vfmadd231pd -1088(%r8), %ymm0, %ymm1
  vmovupd %ymm1, 288(%rsp)
  vmovupd 320(%rsp), %ymm1
  vfmadd231pd -1056(%r8), %ymm0, %ymm1
  vmovupd %ymm1, 320(%rsp)
  vmovupd 352(%rsp), %ymm1
  vfmadd231pd -1024(%r8), %ymm0, %ymm1
  vmovupd %ymm1, 352(%rsp)
  vmovupd 384(%rsp), %ymm1
  vfmadd231pd -992(%r8), %ymm0, %ymm1
  vmovupd %ymm1, 384(%rsp)
  vmovupd 416(%rsp), %ymm1
  vfmadd231pd -960(%r8), %ymm0, %ymm1
  vmovupd %ymm1, 416(%rsp)
  vmovupd 448(%rsp), %ymm1
  vfmadd231pd -928(%r8), %ymm0, %ymm1
  vmovupd %ymm1, 448(%rsp)
  vmovupd 480(%rsp), %ymm1
  vfmadd231pd -896(%r8), %ymm0, %ymm1
  vmovupd %ymm1, 480(%rsp)
  vmovupd 512(%rsp), %ymm1
  vfmadd231pd -864(%r8), %ymm0, %ymm1
  vmovupd %ymm1, 512(%rsp)
  vmovupd 544(%rsp), %ymm1
  vfmadd231pd -832(%r8), %ymm0, %ymm1
  vmovupd %ymm1, 544(%rsp)
  vmovupd 576(%rsp), %ymm1
  vfmadd231pd -800(%r8), %ymm0, %ymm1
  vmovupd %ymm1, 576(%rsp)
  vmovupd 608(%rsp), %ymm1
  vfmadd231pd -768(%r8), %ymm0, %ymm1
  vmovupd %ymm1, 608(%rsp)
  vmovupd 640(%rsp), %ymm1
  vfmadd231pd -736(%r8), %ymm0, %ymm1
  vmovupd %ymm1, 640(%rsp)
  vmovupd 672(%rsp), %ymm1
  vfmadd231pd -704(%r8), %ymm0, %ymm1
  vmovupd %ymm1, 672(%rsp)
  vmovupd 704(%rsp), %ymm1
  vfmadd231pd -672(%r8), %ymm0, %ymm1
  vmovupd %ymm1, 704(%rsp)
  vmovupd 736(%rsp), %ymm1
  vfmadd231pd -640(%r8), %ymm0, %ymm1
  vmovupd %ymm1, 736(%rsp)
  vmovupd 768(%rsp), %ymm1
  vfmadd231pd -608(%r8), %ymm0, %ymm1
  vmovupd %ymm1, 768(%rsp)
  vmovupd 800(%rsp), %ymm1
  vfmadd231pd -576(%r8), %ymm0, %ymm1
  vmovupd %ymm1, 800(%rsp)
  vmovupd 832(%rsp), %ymm1
  vfmadd231pd -544(%r8), %ymm0, %ymm1
  vmovupd %ymm1, 832(%rsp)
  vmovupd 864(%rsp), %ymm1
  vfmadd231pd -512(%r8), %ymm0, %ymm1
  vmovupd %ymm1, 864(%rsp)
  vmovupd 896(%rsp), %ymm1
  vfmadd231pd -480(%r8), %ymm0, %ymm1
  vmovupd %ymm1, 896(%rsp)
  vmovupd 928(%rsp), %ymm1
  vfmadd231pd -448(%r8), %ymm0, %ymm1
  vmovupd %ymm1, 928(%rsp)
  vmovupd 960(%rsp), %ymm1
  vfmadd231pd -416(%r8), %ymm0, %ymm1
  vmovupd %ymm1, 960(%rsp)
  vmovupd 992(%rsp), %ymm1
  vfmadd231pd -384(%r8), %ymm0, %ymm1
  vmovupd %ymm1, 992(%rsp)
  vmovupd 1024(%rsp), %ymm1
  vfmadd231pd -352(%r8), %ymm0, %ymm1
  vmovupd %ymm1, 1024(%rsp)
  vmovupd 1056(%rsp), %ymm1
  vfmadd231pd -320(%r8), %ymm0, %ymm1
  vmovupd %ymm1, 1056(%rsp)
  vmovupd 1088(%rsp), %ymm1
  vfmadd231pd -288(%r8), %ymm0, %ymm1
  vmovupd %ymm1, 1088(%rsp)
  vmovupd 1120(%rsp), %ymm1
  vfmadd231pd -256(%r8), %ymm0, %ymm1
  vmovupd %ymm1, 1120(%rsp)
  vmovupd 1152(%rsp), %ymm1
  vfmadd231pd -224(%r8), %ymm0, %ymm1
  vmovupd %ymm1, 1152(%rsp)
  vmovupd 1184(%rsp), %ymm1
  vfmadd231pd -192(%r8), %ymm0, %ymm1
  vmovupd %ymm1, 1184(%rsp)
  vmovupd 1216(%rsp), %ymm1
  vfmadd231pd -160(%r8), %ymm0, %ymm1
  vmovupd %ymm1, 1216(%rsp)
  vmovupd 1248(%rsp), %ymm1
  vfmadd231pd -128(%r8), %ymm0, %ymm1
  vmovupd %ymm1, 1248(%rsp)
  vmovupd 1280(%rsp), %ymm1
  vfmadd231pd -96(%r8), %ymm0, %ymm1
  vmovupd %ymm1, 1280(%rsp)
  vmovupd 1312(%rsp), %ymm1
  vfmadd231pd -64(%r8), %ymm0, %ymm1
  vmovupd %ymm1, 1312(%rsp)
  vmovupd 1344(%rsp), %ymm1
  vfmadd231pd -32(%r8), %ymm0, %ymm1
  vmovupd %ymm1, 1344(%rsp)
  vmovupd 1408(%rsp), %ymm1
  vfmadd231pd (%r8), %ymm0, %ymm1
  vmovupd %ymm1, 1408(%rsp)
  incq %r9
  addq $2048, %r8
  cmpq $256, %r9
  jne .LBB0_2
  vmovups %ymm15, (%rdi)
  vmovupd %ymm14, 32(%rdi)
  vmovupd %ymm13, 64(%rdi)
  vmovupd %ymm12, 96(%rdi)
  vmovupd %ymm11, 128(%rdi)
  vmovupd %ymm10, 160(%rdi)
  vmovupd %ymm9, 192(%rdi)
  vmovupd %ymm8, 224(%rdi)
  vmovupd %ymm7, 256(%rdi)
  vmovupd %ymm6, 288(%rdi)
  vmovupd %ymm5, 320(%rdi)
  vmovupd %ymm4, 352(%rdi)
  vmovupd %ymm3, 384(%rdi)
  vmovupd %ymm2, 416(%rdi)
  vmovups 1376(%rsp), %ymm0
  vmovups %ymm0, 448(%rdi)
  vmovups 1440(%rsp), %ymm0
  vmovups %ymm0, 480(%rdi)
  vmovups -128(%rsp), %ymm0
  vmovups %ymm0, 512(%rdi)
  vmovups -96(%rsp), %ymm0
  vmovups %ymm0, 544(%rdi)
  vmovups -64(%rsp), %ymm0
  vmovups %ymm0, 576(%rdi)
  vmovups -32(%rsp), %ymm0
  vmovups %ymm0, 608(%rdi)
  vmovups (%rsp), %ymm0
  vmovups %ymm0, 640(%rdi)
  vmovups 32(%rsp), %ymm0
  vmovups %ymm0, 672(%rdi)
  vmovups 64(%rsp), %ymm0
  vmovups %ymm0, 704(%rdi)
  vmovups 96(%rsp), %ymm0
  vmovups %ymm0, 736(%rdi)
  vmovups 128(%rsp), %ymm0
  vmovups %ymm0, 768(%rdi)
  vmovups 160(%rsp), %ymm0
  vmovups %ymm0, 800(%rdi)
  vmovups 192(%rsp), %ymm0
  vmovups %ymm0, 832(%rdi)
  vmovups 224(%rsp), %ymm0
  vmovups %ymm0, 864(%rdi)
  vmovups 256(%rsp), %ymm0
  vmovups %ymm0, 896(%rdi)
  vmovups 288(%rsp), %ymm0
  vmovups %ymm0, 928(%rdi)
  vmovups 320(%rsp), %ymm0
  vmovups %ymm0, 960(%rdi)
  vmovups 352(%rsp), %ymm0
  vmovups %ymm0, 992(%rdi)
  vmovups 384(%rsp), %ymm0
  vmovups %ymm0, 1024(%rdi)
  vmovups 416(%rsp), %ymm0
  vmovups %ymm0, 1056(%rdi)
  vmovups 448(%rsp), %ymm0
  vmovups %ymm0, 1088(%rdi)
  vmovups 480(%rsp), %ymm0
  vmovups %ymm0, 1120(%rdi)
  vmovups 512(%rsp), %ymm0
  vmovups %ymm0, 1152(%rdi)
  vmovups 544(%rsp), %ymm0
  vmovups %ymm0, 1184(%rdi)
  vmovups 576(%rsp), %ymm0
  vmovups %ymm0, 1216(%rdi)
  vmovups 608(%rsp), %ymm0
  vmovups %ymm0, 1248(%rdi)
  vmovups 640(%rsp), %ymm0
  vmovups %ymm0, 1280(%rdi)
  vmovups 672(%rsp), %ymm0
  vmovups %ymm0, 1312(%rdi)
  vmovups 704(%rsp), %ymm0
  vmovups %ymm0, 1344(%rdi)
  vmovups 736(%rsp), %ymm0
  vmovups %ymm0, 1376(%rdi)
  vmovups 768(%rsp), %ymm0
  vmovups %ymm0, 1408(%rdi)
  vmovups 800(%rsp), %ymm0
  vmovups %ymm0, 1440(%rdi)
  vmovups 832(%rsp), %ymm0
  vmovups %ymm0, 1472(%rdi)
  vmovups 864(%rsp), %ymm0
  vmovups %ymm0, 1504(%rdi)
  vmovups 896(%rsp), %ymm0
  vmovups %ymm0, 1536(%rdi)
  vmovups 928(%rsp), %ymm0
  vmovups %ymm0, 1568(%rdi)
  vmovups 960(%rsp), %ymm0
  vmovups %ymm0, 1600(%rdi)
  vmovups 992(%rsp), %ymm0
  vmovups %ymm0, 1632(%rdi)
  vmovups 1024(%rsp), %ymm0
  vmovups %ymm0, 1664(%rdi)
  vmovups 1056(%rsp), %ymm0
  vmovups %ymm0, 1696(%rdi)
  vmovups 1088(%rsp), %ymm0
  vmovups %ymm0, 1728(%rdi)
  vmovups 1120(%rsp), %ymm0
  vmovups %ymm0, 1760(%rdi)
  vmovups 1152(%rsp), %ymm0
  vmovups %ymm0, 1792(%rdi)
  vmovups 1184(%rsp), %ymm0
  vmovups %ymm0, 1824(%rdi)
  vmovups 1216(%rsp), %ymm0
  vmovups %ymm0, 1856(%rdi)
  vmovups 1248(%rsp), %ymm0
  vmovups %ymm0, 1888(%rdi)
  vmovups 1280(%rsp), %ymm0
  vmovups %ymm0, 1920(%rdi)
  vmovups 1312(%rsp), %ymm0
  vmovups %ymm0, 1952(%rdi)
  vmovups 1344(%rsp), %ymm0
  vmovups %ymm0, 1984(%rdi)
  vmovupd 1408(%rsp), %ymm0
  vmovupd %ymm0, 2016(%rdi)
  incq %rcx
  addq $2048, %rax
  cmpq $256, %rcx
  jne .LBB0_1
  addq $1512, %rsp
  vzeroupper
  retq

.LCPI1_0:
.LCPI1_2:
.LCPI1_3:
.LCPI1_4:
.LCPI1_5:
.LCPI1_8:
.LCPI1_1:
.LCPI1_6:
.LCPI1_7:
main:
  subq $1656, %rsp
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
  leaq A(%rip), %rax
  xorl %ecx, %ecx
  leaq C(%rip), %rdx
  leaq B+2016(%rip), %rsi
.LBB1_5:
  movq %rcx, %r10
  shlq $11, %r10
  leaq (%rdx,%r10), %rdi
  vmovupd (%r10,%rdx), %ymm15
  vmovupd 32(%r10,%rdx), %ymm14
  vmovupd 64(%r10,%rdx), %ymm13
  vmovupd 96(%r10,%rdx), %ymm12
  vmovupd 128(%r10,%rdx), %ymm11
  vmovupd 160(%r10,%rdx), %ymm10
  vmovupd 192(%r10,%rdx), %ymm9
  vmovupd 224(%r10,%rdx), %ymm8
  vmovupd 256(%r10,%rdx), %ymm7
  vmovupd 288(%r10,%rdx), %ymm6
  vmovupd 320(%r10,%rdx), %ymm5
  vmovupd 352(%r10,%rdx), %ymm4
  vmovupd 384(%r10,%rdx), %ymm3
  vmovupd 416(%r10,%rdx), %ymm2
  vmovups 448(%r10,%rdx), %ymm0
  vmovups %ymm0, 1552(%rsp)
  vmovups 480(%r10,%rdx), %ymm0
  vmovups %ymm0, 16(%rsp)
  movq %rsi, %r8
  xorl %r9d, %r9d
  vmovups 512(%r10,%rdx), %ymm0
  vmovups %ymm0, 48(%rsp)
  vmovups 544(%r10,%rdx), %ymm0
  vmovups %ymm0, 80(%rsp)
  vmovups 576(%r10,%rdx), %ymm0
  vmovups %ymm0, 112(%rsp)
  vmovups 608(%r10,%rdx), %ymm0
  vmovups %ymm0, 144(%rsp)
  vmovups 640(%r10,%rdx), %ymm0
  vmovups %ymm0, 176(%rsp)
  vmovups 672(%r10,%rdx), %ymm0
  vmovups %ymm0, 208(%rsp)
  vmovups 704(%r10,%rdx), %ymm0
  vmovups %ymm0, 240(%rsp)
  vmovups 736(%r10,%rdx), %ymm0
  vmovups %ymm0, 272(%rsp)
  vmovups 768(%r10,%rdx), %ymm0
  vmovups %ymm0, 304(%rsp)
  vmovups 800(%r10,%rdx), %ymm0
  vmovups %ymm0, 336(%rsp)
  vmovups 832(%r10,%rdx), %ymm0
  vmovups %ymm0, 368(%rsp)
  vmovups 864(%r10,%rdx), %ymm0
  vmovups %ymm0, 400(%rsp)
  vmovups 896(%r10,%rdx), %ymm0
  vmovups %ymm0, 432(%rsp)
  vmovups 928(%r10,%rdx), %ymm0
  vmovups %ymm0, 464(%rsp)
  vmovups 960(%r10,%rdx), %ymm0
  vmovups %ymm0, 496(%rsp)
  vmovups 992(%r10,%rdx), %ymm0
  vmovups %ymm0, 528(%rsp)
  vmovups 1024(%r10,%rdx), %ymm0
  vmovups %ymm0, 560(%rsp)
  vmovups 1056(%r10,%rdx), %ymm0
  vmovups %ymm0, 592(%rsp)
  vmovups 1088(%r10,%rdx), %ymm0
  vmovups %ymm0, 624(%rsp)
  vmovups 1120(%r10,%rdx), %ymm0
  vmovups %ymm0, 656(%rsp)
  vmovups 1152(%r10,%rdx), %ymm0
  vmovups %ymm0, 688(%rsp)
  vmovups 1184(%r10,%rdx), %ymm0
  vmovups %ymm0, 720(%rsp)
  vmovups 1216(%r10,%rdx), %ymm0
  vmovups %ymm0, 752(%rsp)
  vmovups 1248(%r10,%rdx), %ymm0
  vmovups %ymm0, 784(%rsp)
  vmovups 1280(%r10,%rdx), %ymm0
  vmovups %ymm0, 816(%rsp)
  vmovups 1312(%r10,%rdx), %ymm0
  vmovups %ymm0, 848(%rsp)
  vmovups 1344(%r10,%rdx), %ymm0
  vmovups %ymm0, 880(%rsp)
  vmovups 1376(%r10,%rdx), %ymm0
  vmovups %ymm0, 912(%rsp)
  vmovups 1408(%r10,%rdx), %ymm0
  vmovups %ymm0, 944(%rsp)
  vmovups 1440(%r10,%rdx), %ymm0
  vmovups %ymm0, 976(%rsp)
  vmovups 1472(%r10,%rdx), %ymm0
  vmovups %ymm0, 1008(%rsp)
  vmovups 1504(%r10,%rdx), %ymm0
  vmovups %ymm0, 1040(%rsp)
  vmovups 1536(%r10,%rdx), %ymm0
  vmovups %ymm0, 1072(%rsp)
  vmovups 1568(%r10,%rdx), %ymm0
  vmovups %ymm0, 1104(%rsp)
  vmovups 1600(%r10,%rdx), %ymm0
  vmovups %ymm0, 1136(%rsp)
  vmovups 1632(%r10,%rdx), %ymm0
  vmovups %ymm0, 1168(%rsp)
  vmovups 1664(%r10,%rdx), %ymm0
  vmovups %ymm0, 1200(%rsp)
  vmovups 1696(%r10,%rdx), %ymm0
  vmovups %ymm0, 1232(%rsp)
  vmovups 1728(%r10,%rdx), %ymm0
  vmovups %ymm0, 1264(%rsp)
  vmovups 1760(%r10,%rdx), %ymm0
  vmovups %ymm0, 1296(%rsp)
  vmovups 1792(%r10,%rdx), %ymm0
  vmovups %ymm0, 1328(%rsp)
  vmovups 1824(%r10,%rdx), %ymm0
  vmovups %ymm0, 1360(%rsp)
  vmovups 1856(%r10,%rdx), %ymm0
  vmovups %ymm0, 1392(%rsp)
  vmovups 1888(%r10,%rdx), %ymm0
  vmovups %ymm0, 1424(%rsp)
  vmovups 1920(%r10,%rdx), %ymm0
  vmovups %ymm0, 1456(%rsp)
  vmovups 1952(%r10,%rdx), %ymm0
  vmovups %ymm0, 1488(%rsp)
  vmovups 1984(%r10,%rdx), %ymm0
  vmovups %ymm0, 1520(%rsp)
  vmovups 2016(%r10,%rdx), %ymm0
  vmovups %ymm0, 1584(%rsp)
.LBB1_6:
  vbroadcastsd (%rax,%r9,8), %ymm0
  vfmadd231pd -2016(%r8), %ymm0, %ymm15
  vmovupd %ymm15, 1616(%rsp)
  vfmadd231pd -1984(%r8), %ymm0, %ymm14
  vfmadd231pd -1952(%r8), %ymm0, %ymm13
  vfmadd231pd -1920(%r8), %ymm0, %ymm12
  vfmadd231pd -1888(%r8), %ymm0, %ymm11
  vfmadd231pd -1856(%r8), %ymm0, %ymm10
  vfmadd231pd -1824(%r8), %ymm0, %ymm9
  vfmadd231pd -1792(%r8), %ymm0, %ymm8
  vfmadd231pd -1760(%r8), %ymm0, %ymm7
  vfmadd231pd -1728(%r8), %ymm0, %ymm6
  vfmadd231pd -1696(%r8), %ymm0, %ymm5
  vfmadd231pd -1664(%r8), %ymm0, %ymm4
  vfmadd231pd -1632(%r8), %ymm0, %ymm3
  vfmadd231pd -1600(%r8), %ymm0, %ymm2
  vmovupd 1552(%rsp), %ymm1
  vfmadd231pd -1568(%r8), %ymm0, %ymm1
  vmovupd %ymm1, 1552(%rsp)
  vmovapd %ymm14, %ymm15
  vmovapd %ymm13, %ymm14
  vmovapd %ymm12, %ymm13
  vmovapd %ymm11, %ymm12
  vmovapd %ymm10, %ymm11
  vmovapd %ymm9, %ymm10
  vmovapd %ymm8, %ymm9
  vmovapd %ymm7, %ymm8
  vmovapd %ymm6, %ymm7
  vmovapd %ymm5, %ymm6
  vmovapd %ymm4, %ymm5
  vmovupd 16(%rsp), %ymm1
  vfmadd231pd -1536(%r8), %ymm0, %ymm1
  vmovupd %ymm1, 16(%rsp)
  vmovapd %ymm5, %ymm4
  vmovapd %ymm6, %ymm5
  vmovapd %ymm7, %ymm6
  vmovapd %ymm8, %ymm7
  vmovapd %ymm9, %ymm8
  vmovapd %ymm10, %ymm9
  vmovapd %ymm11, %ymm10
  vmovapd %ymm12, %ymm11
  vmovapd %ymm13, %ymm12
  vmovapd %ymm14, %ymm13
  vmovapd %ymm15, %ymm14
  vmovupd 1616(%rsp), %ymm15
  vmovupd 48(%rsp), %ymm1
  vfmadd231pd -1504(%r8), %ymm0, %ymm1
  vmovupd %ymm1, 48(%rsp)
  vmovupd 80(%rsp), %ymm1
  vfmadd231pd -1472(%r8), %ymm0, %ymm1
  vmovupd %ymm1, 80(%rsp)
  vmovupd 112(%rsp), %ymm1
  vfmadd231pd -1440(%r8), %ymm0, %ymm1
  vmovupd %ymm1, 112(%rsp)
  vmovupd 144(%rsp), %ymm1
  vfmadd231pd -1408(%r8), %ymm0, %ymm1
  vmovupd %ymm1, 144(%rsp)
  vmovupd 176(%rsp), %ymm1
  vfmadd231pd -1376(%r8), %ymm0, %ymm1
  vmovupd %ymm1, 176(%rsp)
  vmovupd 208(%rsp), %ymm1
  vfmadd231pd -1344(%r8), %ymm0, %ymm1
  vmovupd %ymm1, 208(%rsp)
  vmovupd 240(%rsp), %ymm1
  vfmadd231pd -1312(%r8), %ymm0, %ymm1
  vmovupd %ymm1, 240(%rsp)
  vmovupd 272(%rsp), %ymm1
  vfmadd231pd -1280(%r8), %ymm0, %ymm1
  vmovupd %ymm1, 272(%rsp)
  vmovupd 304(%rsp), %ymm1
  vfmadd231pd -1248(%r8), %ymm0, %ymm1
  vmovupd %ymm1, 304(%rsp)
  vmovupd 336(%rsp), %ymm1
  vfmadd231pd -1216(%r8), %ymm0, %ymm1
  vmovupd %ymm1, 336(%rsp)
  vmovupd 368(%rsp), %ymm1
  vfmadd231pd -1184(%r8), %ymm0, %ymm1
  vmovupd %ymm1, 368(%rsp)
  vmovupd 400(%rsp), %ymm1
  vfmadd231pd -1152(%r8), %ymm0, %ymm1
  vmovupd %ymm1, 400(%rsp)
  vmovupd 432(%rsp), %ymm1
  vfmadd231pd -1120(%r8), %ymm0, %ymm1
  vmovupd %ymm1, 432(%rsp)
  vmovupd 464(%rsp), %ymm1
  vfmadd231pd -1088(%r8), %ymm0, %ymm1
  vmovupd %ymm1, 464(%rsp)
  vmovupd 496(%rsp), %ymm1
  vfmadd231pd -1056(%r8), %ymm0, %ymm1
  vmovupd %ymm1, 496(%rsp)
  vmovupd 528(%rsp), %ymm1
  vfmadd231pd -1024(%r8), %ymm0, %ymm1
  vmovupd %ymm1, 528(%rsp)
  vmovupd 560(%rsp), %ymm1
  vfmadd231pd -992(%r8), %ymm0, %ymm1
  vmovupd %ymm1, 560(%rsp)
  vmovupd 592(%rsp), %ymm1
  vfmadd231pd -960(%r8), %ymm0, %ymm1
  vmovupd %ymm1, 592(%rsp)
  vmovupd 624(%rsp), %ymm1
  vfmadd231pd -928(%r8), %ymm0, %ymm1
  vmovupd %ymm1, 624(%rsp)
  vmovupd 656(%rsp), %ymm1
  vfmadd231pd -896(%r8), %ymm0, %ymm1
  vmovupd %ymm1, 656(%rsp)
  vmovupd 688(%rsp), %ymm1
  vfmadd231pd -864(%r8), %ymm0, %ymm1
  vmovupd %ymm1, 688(%rsp)
  vmovupd 720(%rsp), %ymm1
  vfmadd231pd -832(%r8), %ymm0, %ymm1
  vmovupd %ymm1, 720(%rsp)
  vmovupd 752(%rsp), %ymm1
  vfmadd231pd -800(%r8), %ymm0, %ymm1
  vmovupd %ymm1, 752(%rsp)
  vmovupd 784(%rsp), %ymm1
  vfmadd231pd -768(%r8), %ymm0, %ymm1
  vmovupd %ymm1, 784(%rsp)
  vmovupd 816(%rsp), %ymm1
  vfmadd231pd -736(%r8), %ymm0, %ymm1
  vmovupd %ymm1, 816(%rsp)
  vmovupd 848(%rsp), %ymm1
  vfmadd231pd -704(%r8), %ymm0, %ymm1
  vmovupd %ymm1, 848(%rsp)
  vmovupd 880(%rsp), %ymm1
  vfmadd231pd -672(%r8), %ymm0, %ymm1
  vmovupd %ymm1, 880(%rsp)
  vmovupd 912(%rsp), %ymm1
  vfmadd231pd -640(%r8), %ymm0, %ymm1
  vmovupd %ymm1, 912(%rsp)
  vmovupd 944(%rsp), %ymm1
  vfmadd231pd -608(%r8), %ymm0, %ymm1
  vmovupd %ymm1, 944(%rsp)
  vmovupd 976(%rsp), %ymm1
  vfmadd231pd -576(%r8), %ymm0, %ymm1
  vmovupd %ymm1, 976(%rsp)
  vmovupd 1008(%rsp), %ymm1
  vfmadd231pd -544(%r8), %ymm0, %ymm1
  vmovupd %ymm1, 1008(%rsp)
  vmovupd 1040(%rsp), %ymm1
  vfmadd231pd -512(%r8), %ymm0, %ymm1
  vmovupd %ymm1, 1040(%rsp)
  vmovupd 1072(%rsp), %ymm1
  vfmadd231pd -480(%r8), %ymm0, %ymm1
  vmovupd %ymm1, 1072(%rsp)
  vmovupd 1104(%rsp), %ymm1
  vfmadd231pd -448(%r8), %ymm0, %ymm1
  vmovupd %ymm1, 1104(%rsp)
  vmovupd 1136(%rsp), %ymm1
  vfmadd231pd -416(%r8), %ymm0, %ymm1
  vmovupd %ymm1, 1136(%rsp)
  vmovupd 1168(%rsp), %ymm1
  vfmadd231pd -384(%r8), %ymm0, %ymm1
  vmovupd %ymm1, 1168(%rsp)
  vmovupd 1200(%rsp), %ymm1
  vfmadd231pd -352(%r8), %ymm0, %ymm1
  vmovupd %ymm1, 1200(%rsp)
  vmovupd 1232(%rsp), %ymm1
  vfmadd231pd -320(%r8), %ymm0, %ymm1
  vmovupd %ymm1, 1232(%rsp)
  vmovupd 1264(%rsp), %ymm1
  vfmadd231pd -288(%r8), %ymm0, %ymm1
  vmovupd %ymm1, 1264(%rsp)
  vmovupd 1296(%rsp), %ymm1
  vfmadd231pd -256(%r8), %ymm0, %ymm1
  vmovupd %ymm1, 1296(%rsp)
  vmovupd 1328(%rsp), %ymm1
  vfmadd231pd -224(%r8), %ymm0, %ymm1
  vmovupd %ymm1, 1328(%rsp)
  vmovupd 1360(%rsp), %ymm1
  vfmadd231pd -192(%r8), %ymm0, %ymm1
  vmovupd %ymm1, 1360(%rsp)
  vmovupd 1392(%rsp), %ymm1
  vfmadd231pd -160(%r8), %ymm0, %ymm1
  vmovupd %ymm1, 1392(%rsp)
  vmovupd 1424(%rsp), %ymm1
  vfmadd231pd -128(%r8), %ymm0, %ymm1
  vmovupd %ymm1, 1424(%rsp)
  vmovupd 1456(%rsp), %ymm1
  vfmadd231pd -96(%r8), %ymm0, %ymm1
  vmovupd %ymm1, 1456(%rsp)
  vmovupd 1488(%rsp), %ymm1
  vfmadd231pd -64(%r8), %ymm0, %ymm1
  vmovupd %ymm1, 1488(%rsp)
  vmovupd 1520(%rsp), %ymm1
  vfmadd231pd -32(%r8), %ymm0, %ymm1
  vmovupd %ymm1, 1520(%rsp)
  vmovupd 1584(%rsp), %ymm1
  vfmadd231pd (%r8), %ymm0, %ymm1
  vmovupd %ymm1, 1584(%rsp)
  incq %r9
  addq $2048, %r8
  cmpq $256, %r9
  jne .LBB1_6
  vmovupd %ymm15, (%rdi)
  vmovupd %ymm14, 32(%rdi)
  vmovupd %ymm13, 64(%rdi)
  vmovupd %ymm12, 96(%rdi)
  vmovupd %ymm11, 128(%rdi)
  vmovupd %ymm10, 160(%rdi)
  vmovupd %ymm9, 192(%rdi)
  vmovupd %ymm8, 224(%rdi)
  vmovupd %ymm7, 256(%rdi)
  vmovupd %ymm6, 288(%rdi)
  vmovupd %ymm5, 320(%rdi)
  vmovupd %ymm4, 352(%rdi)
  vmovupd %ymm3, 384(%rdi)
  vmovupd %ymm2, 416(%rdi)
  vmovups 1552(%rsp), %ymm0
  vmovups %ymm0, 448(%rdi)
  vmovups 16(%rsp), %ymm0
  vmovups %ymm0, 480(%rdi)
  vmovups 48(%rsp), %ymm0
  vmovups %ymm0, 512(%rdi)
  vmovups 80(%rsp), %ymm0
  vmovups %ymm0, 544(%rdi)
  vmovups 112(%rsp), %ymm0
  vmovups %ymm0, 576(%rdi)
  vmovups 144(%rsp), %ymm0
  vmovups %ymm0, 608(%rdi)
  vmovups 176(%rsp), %ymm0
  vmovups %ymm0, 640(%rdi)
  vmovups 208(%rsp), %ymm0
  vmovups %ymm0, 672(%rdi)
  vmovups 240(%rsp), %ymm0
  vmovups %ymm0, 704(%rdi)
  vmovups 272(%rsp), %ymm0
  vmovups %ymm0, 736(%rdi)
  vmovups 304(%rsp), %ymm0
  vmovups %ymm0, 768(%rdi)
  vmovups 336(%rsp), %ymm0
  vmovups %ymm0, 800(%rdi)
  vmovups 368(%rsp), %ymm0
  vmovups %ymm0, 832(%rdi)
  vmovups 400(%rsp), %ymm0
  vmovups %ymm0, 864(%rdi)
  vmovups 432(%rsp), %ymm0
  vmovups %ymm0, 896(%rdi)
  vmovups 464(%rsp), %ymm0
  vmovups %ymm0, 928(%rdi)
  vmovups 496(%rsp), %ymm0
  vmovups %ymm0, 960(%rdi)
  vmovups 528(%rsp), %ymm0
  vmovups %ymm0, 992(%rdi)
  vmovups 560(%rsp), %ymm0
  vmovups %ymm0, 1024(%rdi)
  vmovups 592(%rsp), %ymm0
  vmovups %ymm0, 1056(%rdi)
  vmovups 624(%rsp), %ymm0
  vmovups %ymm0, 1088(%rdi)
  vmovups 656(%rsp), %ymm0
  vmovups %ymm0, 1120(%rdi)
  vmovups 688(%rsp), %ymm0
  vmovups %ymm0, 1152(%rdi)
  vmovups 720(%rsp), %ymm0
  vmovups %ymm0, 1184(%rdi)
  vmovups 752(%rsp), %ymm0
  vmovups %ymm0, 1216(%rdi)
  vmovups 784(%rsp), %ymm0
  vmovups %ymm0, 1248(%rdi)
  vmovups 816(%rsp), %ymm0
  vmovups %ymm0, 1280(%rdi)
  vmovups 848(%rsp), %ymm0
  vmovups %ymm0, 1312(%rdi)
  vmovups 880(%rsp), %ymm0
  vmovups %ymm0, 1344(%rdi)
  vmovups 912(%rsp), %ymm0
  vmovups %ymm0, 1376(%rdi)
  vmovups 944(%rsp), %ymm0
  vmovups %ymm0, 1408(%rdi)
  vmovups 976(%rsp), %ymm0
  vmovups %ymm0, 1440(%rdi)
  vmovups 1008(%rsp), %ymm0
  vmovups %ymm0, 1472(%rdi)
  vmovups 1040(%rsp), %ymm0
  vmovups %ymm0, 1504(%rdi)
  vmovups 1072(%rsp), %ymm0
  vmovups %ymm0, 1536(%rdi)
  vmovups 1104(%rsp), %ymm0
  vmovups %ymm0, 1568(%rdi)
  vmovups 1136(%rsp), %ymm0
  vmovups %ymm0, 1600(%rdi)
  vmovups 1168(%rsp), %ymm0
  vmovups %ymm0, 1632(%rdi)
  vmovups 1200(%rsp), %ymm0
  vmovups %ymm0, 1664(%rdi)
  vmovups 1232(%rsp), %ymm0
  vmovups %ymm0, 1696(%rdi)
  vmovups 1264(%rsp), %ymm0
  vmovups %ymm0, 1728(%rdi)
  vmovups 1296(%rsp), %ymm0
  vmovups %ymm0, 1760(%rdi)
  vmovups 1328(%rsp), %ymm0
  vmovups %ymm0, 1792(%rdi)
  vmovups 1360(%rsp), %ymm0
  vmovups %ymm0, 1824(%rdi)
  vmovups 1392(%rsp), %ymm0
  vmovups %ymm0, 1856(%rdi)
  vmovups 1424(%rsp), %ymm0
  vmovups %ymm0, 1888(%rdi)
  vmovups 1456(%rsp), %ymm0
  vmovups %ymm0, 1920(%rdi)
  vmovups 1488(%rsp), %ymm0
  vmovups %ymm0, 1952(%rdi)
  vmovups 1520(%rsp), %ymm0
  vmovups %ymm0, 1984(%rdi)
  vmovupd 1584(%rsp), %ymm0
  vmovupd %ymm0, 2016(%rdi)
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
  addq $1656, %rsp
  retq

.L.str:

