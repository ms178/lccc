copy_q4:
  vmovups (%rsi), %ymm0
  vmovups %ymm0, (%rdi)
  vzeroupper
  retq

copy_d2:
  vmovups (%rsi), %xmm0
  vmovups %xmm0, (%rdi)
  retq

copy_w4:
  vmovups (%rsi), %xmm0
  vmovups %xmm0, (%rdi)
  retq

copy_b16:
  vmovups (%rsi), %xmm0
  vmovups %xmm0, (%rdi)
  retq

xor_q4:
  vmovups (%rdx), %ymm0
  vxorps (%rsi), %ymm0, %ymm0
  vmovups %ymm0, (%rdi)
  vzeroupper
  retq

add_q4:
  vmovdqu (%rdx), %ymm0
  vpaddq (%rsi), %ymm0, %ymm0
  vmovdqu %ymm0, (%rdi)
  vzeroupper
  retq

sub_d2:
  vmovupd (%rsi), %xmm0
  vsubpd (%rdx), %xmm0, %xmm0
  vmovupd %xmm0, (%rdi)
  retq

mul_h8:
  vmovdqu (%rdx), %xmm0
  vpmullw (%rsi), %xmm0, %xmm0
  vmovdqu %xmm0, (%rdi)
  retq

add_b16:
  vmovdqu (%rdx), %xmm0
  vpaddb (%rsi), %xmm0, %xmm0
  vmovdqu %xmm0, (%rdi)
  retq

ksub_w4:
  vpcmpeqd %xmm0, %xmm0, %xmm0
  vpaddd (%rsi), %xmm0, %xmm0
  vmovdqu %xmm0, (%rdi)
  retq

madd_q4:
  vmovdqu (%rdx), %ymm0
  vpaddq (%rsi), %ymm0, %ymm0
  vpaddq (%rcx), %ymm0, %ymm0
  vmovdqu %ymm0, (%rdi)
  vzeroupper
  retq

mixed:
  vmovups (%rsi), %xmm0
  vmovups %xmm0, (%rdi)
  movq (%rcx), %rax
  movq %rax, (%rdx)
  retq

