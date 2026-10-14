main:
  subq $40, %rsp
  vstmxcsr (%rsp)
  orl $32832, (%rsp)
  vldmxcsr (%rsp)
  vmovups .LCPI0_0(%rip), %xmm0
  vmovups %xmm0, guard(%rip)
  vmovups .LCPI0_1(%rip), %xmm0
  vmovups %xmm0, guard+16(%rip)
  vmovups .LCPI0_2(%rip), %xmm0
  vmovups %xmm0, guard+32(%rip)
  vmovups .LCPI0_3(%rip), %xmm0
  vmovups %xmm0, guard+48(%rip)
  vmovups .LCPI0_4(%rip), %xmm0
  vmovups %xmm0, guard+64(%rip)
  vmovups .LCPI0_5(%rip), %xmm0
  vmovups %xmm0, guard+80(%rip)
  vmovups .LCPI0_6(%rip), %xmm0
  vmovups %xmm0, guard+96(%rip)
  vmovups .LCPI0_7(%rip), %xmm0
  vmovups %xmm0, guard+112(%rip)
  vmovups .LCPI0_8(%rip), %xmm0
  vmovups %xmm0, guard+128(%rip)
  vmovups .LCPI0_9(%rip), %xmm0
  vmovups %xmm0, guard+144(%rip)
  vmovups .LCPI0_10(%rip), %xmm0
  vmovups %xmm0, guard+160(%rip)
  vmovups .LCPI0_11(%rip), %xmm0
  vmovups %xmm0, guard+176(%rip)
  vmovups .LCPI0_12(%rip), %xmm0
  vmovups %xmm0, guard+192(%rip)
  vmovups .LCPI0_13(%rip), %xmm0
  vmovups %xmm0, guard+208(%rip)
  vmovups .LCPI0_14(%rip), %xmm0
  vmovups %xmm0, guard+224(%rip)
  vmovups .LCPI0_15(%rip), %xmm0
  vmovups %xmm0, guard+240(%rip)
  vxorps %xmm0, %xmm0, %xmm0
  vmovups %xmm0, (%rsp)
  movq $0, 18(%rsp)
  movl $0, 26(%rsp)
  movw $0, 30(%rsp)
  vbroadcastss .LCPI0_17(%rip), %ymm0
  vmovups %ymm0, tree(%rip)
  vmovups %ymm0, tree+32(%rip)
  vmovups %ymm0, tree+64(%rip)
  vmovups %ymm0, tree+96(%rip)
  vmovups %ymm0, tree+128(%rip)
  vmovups %ymm0, tree+160(%rip)
  vmovups %ymm0, tree+192(%rip)
  vmovups %ymm0, tree+224(%rip)
  vmovups %ymm0, tree+256(%rip)
  movw $144, 16(%rsp)
  movq %rsp, %rdi
  movl $guard, %esi
  movl $tree, %edx
  vzeroupper
  callq check
  addq $40, %rsp
  retq
