main:
  subq $40, %rsp
  vmovaps .LCPI0_0(%rip), %ymm0
  vmovaps %ymm0, guard(%rip)
  leaq guard(%rip), %rsi
  vmovaps .LCPI0_1(%rip), %ymm0
  vmovaps %ymm0, guard+32(%rip)
  vmovaps .LCPI0_2(%rip), %ymm0
  vmovaps %ymm0, guard+64(%rip)
  vmovaps .LCPI0_3(%rip), %ymm0
  vmovaps %ymm0, guard+96(%rip)
  vmovaps .LCPI0_4(%rip), %ymm0
  vmovaps %ymm0, guard+128(%rip)
  vmovaps .LCPI0_5(%rip), %ymm0
  vmovaps %ymm0, guard+160(%rip)
  vmovaps .LCPI0_6(%rip), %ymm0
  vmovaps %ymm0, guard+192(%rip)
  vmovaps .LCPI0_7(%rip), %ymm0
  vmovaps %ymm0, guard+224(%rip)
  vxorps %xmm0, %xmm0, %xmm0
  vmovups %ymm0, (%rsp)
  vbroadcastss .LCPI0_9(%rip), %ymm0
  vmovaps %ymm0, tree(%rip)
  leaq tree(%rip), %rdx
  vmovaps %ymm0, tree+32(%rip)
  vmovaps %ymm0, tree+64(%rip)
  vmovaps %ymm0, tree+96(%rip)
  vmovaps %ymm0, tree+128(%rip)
  vmovaps %ymm0, tree+160(%rip)
  vmovaps %ymm0, tree+192(%rip)
  vmovaps %ymm0, tree+224(%rip)
  vmovaps %ymm0, tree+256(%rip)
  movw $144, 16(%rsp)
  movq %rsp, %rdi
  vzeroupper
  callq check@PLT
  addq $40, %rsp
  retq
