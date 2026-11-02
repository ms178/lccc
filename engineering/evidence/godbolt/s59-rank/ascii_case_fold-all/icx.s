.LCPI0_0:
.LCPI0_1:
.LCPI0_6:
.LCPI0_14:
.LCPI0_9:
.LCPI0_10:
.LCPI0_11:
.LCPI0_12:
.LCPI0_13:
main:
  vstmxcsr -4(%rsp)
  orl $32832, -4(%rsp)
  vldmxcsr -4(%rsp)
  movl $476374736, %eax
  movq $-16, %rcx
  vmovdqu .LCPI0_0(%rip), %ymm0
  vmovdqu .LCPI0_1(%rip), %ymm1
  vpbroadcastw .LCPI0_9(%rip), %ymm2
  vpbroadcastw .LCPI0_10(%rip), %ymm3
  vpbroadcastb .LCPI0_11(%rip), %xmm4
.LBB0_1:
  leal -476374736(%rax), %edx
  vmovd %edx, %xmm5
  vpbroadcastd %xmm5, %ymm5
  vpaddd %ymm0, %ymm5, %ymm6
  vpaddd %ymm1, %ymm5, %ymm5
  vpsrld $16, %ymm5, %ymm5
  vpsrld $16, %ymm6, %ymm6
  vpackusdw %ymm5, %ymm6, %ymm5
  vpermq $216, %ymm5, %ymm5
  vpmulhuw %ymm2, %ymm5, %ymm6
  vpsrlw $6, %ymm6, %ymm6
  vpmullw %ymm3, %ymm6, %ymm6
  vpsubw %ymm6, %ymm5, %ymm5
  vextracti128 $1, %ymm5, %xmm6
  vpackuswb %xmm6, %xmm5, %xmm5
  vpaddb %xmm4, %xmm5, %xmm5
  vmovdqu %xmm5, data+16(%rcx)
  vmovd %eax, %xmm5
  vpbroadcastd %xmm5, %ymm5
  vpaddd %ymm0, %ymm5, %ymm6
  vpaddd %ymm1, %ymm5, %ymm5
  vpsrld $16, %ymm5, %ymm5
  vpsrld $16, %ymm6, %ymm6
  vpackusdw %ymm5, %ymm6, %ymm5
  vpermq $216, %ymm5, %ymm5
  vpmulhuw %ymm2, %ymm5, %ymm6
  vpsrlw $6, %ymm6, %ymm6
  vpmullw %ymm3, %ymm6, %ymm6
  vpsubw %ymm6, %ymm5, %ymm5
  vextracti128 $1, %ymm5, %xmm6
  vpackuswb %xmm6, %xmm5, %xmm5
  vpaddb %xmm4, %xmm5, %xmm5
  vmovdqu %xmm5, data+32(%rcx)
  addl $952749472, %eax
  addq $32, %rcx
  cmpq $65520, %rcx
  jb .LBB0_1
  xorl %eax, %eax
  movq $-65536, %rcx
  vpbroadcastb .LCPI0_12(%rip), %xmm0
  vpbroadcastd .LCPI0_6(%rip), %ymm1
  vpbroadcastb .LCPI0_13(%rip), %xmm2
  vpcmpeqd %xmm3, %xmm3, %xmm3
  vpbroadcastd .LCPI0_14(%rip), %xmm4
.LBB0_3:
  movl %eax, %edx
  shll $5, %edx
  addl %eax, %edx
  vmovq data+65536(%rcx), %xmm5
  vpaddb %xmm0, %xmm5, %xmm6
  vpmovzxbd %xmm5, %ymm5
  vpaddd %ymm1, %ymm5, %ymm7
  vpmaxub %xmm2, %xmm6, %xmm8
  vpcmpeqb %xmm6, %xmm8, %xmm6
  vpxor %xmm3, %xmm6, %xmm6
  vpmovsxbd %xmm6, %ymm6
  vblendvps %ymm6, %ymm7, %ymm5, %ymm5
  vmovd %xmm5, %eax
  addl %edx, %eax
  vpextrd $1, %xmm5, %edx
  addl %eax, %edx
  shll $5, %eax
  vpextrd $2, %xmm5, %esi
  addl %edx, %eax
  addl %eax, %esi
  shll $5, %eax
  addl %esi, %eax
  vpextrd $3, %xmm5, %edx
  addl %eax, %edx
  shll $5, %eax
  addl %edx, %eax
  vextracti128 $1, %ymm5, %xmm6
  vmovd %xmm6, %edx
  addl %eax, %edx
  shll $5, %eax
  addl %edx, %eax
  vpextrd $1, %xmm6, %edx
  leal (%rdx,%rax), %esi
  shll $5, %eax
  vpextrd $2, %xmm6, %edi
  addl %esi, %eax
  leal (%rdi,%rax), %esi
  shll $5, %eax
  addl %esi, %eax
  vpshufb %xmm4, %xmm5, %xmm5
  vpunpckldq %xmm6, %xmm5, %xmm5
  vpinsrb $5, %edx, %xmm5, %xmm5
  vpinsrb $6, %edi, %xmm5, %xmm5
  vpextrd $3, %xmm6, %edx
  vpinsrb $7, %edx, %xmm5, %xmm5
  vmovq %xmm5, folded+65536(%rcx)
  addl %eax, %edx
  shll $5, %eax
  addl %edx, %eax
  addq $8, %rcx
  jne .LBB0_3
  movzbl folded(%rip), %ecx
  xorb $32, %cl
  movzbl folded+1(%rip), %edx
  xorb $55, %dl
  orb %cl, %dl
  movzbl folded+2(%rip), %ecx
  xorb $110, %cl
  xorl %esi, %esi
  cmpl $-1723667183, %eax
  setne %sil
  addl %esi, %esi
  orb %dl, %cl
  movl $1, %eax
  cmovel %esi, %eax
  vzeroupper
  retq

