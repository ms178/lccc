copy_q4:
  vmovdqu (%rsi), %ymm0
  vmovdqu %ymm0, (%rdi)
  vzeroupper
  ret
copy_d2:
  vmovupd (%rsi), %xmm0
  vmovupd %xmm0, (%rdi)
  ret
copy_w4:
  vmovdqu (%rsi), %xmm0
  vmovdqu %xmm0, (%rdi)
  ret
copy_b16:
  vmovdqu (%rsi), %xmm0
  vmovdqu %xmm0, (%rdi)
  ret
xor_q4:
  vmovdqu (%rdx), %ymm0
  vpxor (%rsi), %ymm0, %ymm0
  vmovdqu %ymm0, (%rdi)
  vzeroupper
  ret
add_q4:
  vmovdqu (%rdx), %ymm0
  vpaddq (%rsi), %ymm0, %ymm0
  vmovdqu %ymm0, (%rdi)
  vzeroupper
  ret
sub_d2:
  vmovupd (%rsi), %xmm0
  vsubpd (%rdx), %xmm0, %xmm0
  vmovupd %xmm0, (%rdi)
  ret
mul_h8:
  vmovdqu (%rdx), %xmm0
  vpmullw (%rsi), %xmm0, %xmm0
  vmovdqu %xmm0, (%rdi)
  ret
add_b16:
  vmovdqu (%rdx), %xmm0
  vpaddb (%rsi), %xmm0, %xmm0
  vmovdqu %xmm0, (%rdi)
  ret
ksub_w4:
  vpcmpeqd %xmm0, %xmm0, %xmm0
  vpaddd (%rsi), %xmm0, %xmm0
  vmovdqu %xmm0, (%rdi)
  ret
madd_q4:
  vmovdqu (%rcx), %ymm0
  vpaddq (%rdx), %ymm0, %ymm0
  vpaddq (%rsi), %ymm0, %ymm0
  vmovdqu %ymm0, (%rdi)
  vzeroupper
  ret
mixed:
  vmovupd (%rsi), %xmm0
  movq (%rcx), %rax
  vmovupd %xmm0, (%rdi)
  movq %rax, (%rdx)
  ret
