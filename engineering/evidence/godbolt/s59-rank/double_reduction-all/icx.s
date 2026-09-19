.LCPI0_0:
.LCPI0_1:
main:
  pushq %rbp
  pushq %rbx
  pushq %rax
  vstmxcsr 4(%rsp)
  orl $32832, 4(%rsp)
  vldmxcsr 4(%rsp)
  movl $1, %edx
  movq $-4194304, %rax
  vpbroadcastd .LCPI0_0(%rip), %xmm0
  vpbroadcastd .LCPI0_1(%rip), %xmm1
.LBB0_1:
  imull $1664525, %edx, %edi
  addl $1013904223, %edi
  imull $1664525, %edi, %edx
  addl $1013904223, %edx
  imull $1664525, %edx, %ecx
  addl $1013904223, %ecx
  imull $1664525, %ecx, %r8d
  addl $1013904223, %r8d
  imull $1664525, %r8d, %r9d
  addl $1013904223, %r9d
  imull $1664525, %r9d, %esi
  addl $1013904223, %esi
  imull $1664525, %esi, %r10d
  addl $1013904223, %r10d
  imull $1664525, %r10d, %r11d
  addl $1013904223, %r11d
  imull $1664525, %r11d, %ebx
  addl $1013904223, %ebx
  imull $1664525, %ebx, %ebp
  addl $1013904223, %ebp
  vmovd %edi, %xmm2
  vpinsrd $1, %r8d, %xmm2, %xmm2
  vpinsrd $2, %r10d, %xmm2, %xmm2
  vpinsrd $3, %ebp, %xmm2, %xmm2
  vpsrld $16, %xmm2, %xmm2
  vpand %xmm0, %xmm2, %xmm2
  vpaddd %xmm1, %xmm2, %xmm2
  vmovdqu %xmm2, a+4194304(%rax)
  imull $1664525, %ebp, %edi
  vmovd %edx, %xmm2
  vpinsrd $1, %r9d, %xmm2, %xmm2
  vpinsrd $2, %r11d, %xmm2, %xmm2
  addl $1013904223, %edi
  vpinsrd $3, %edi, %xmm2, %xmm2
  vpsrld $16, %xmm2, %xmm2
  vpand %xmm0, %xmm2, %xmm2
  vpaddd %xmm1, %xmm2, %xmm2
  vmovdqu %xmm2, b+4194304(%rax)
  imull $1664525, %edi, %edx
  addl $1013904223, %edx
  vmovd %ecx, %xmm2
  vpinsrd $1, %esi, %xmm2, %xmm2
  vpinsrd $2, %ebx, %xmm2, %xmm2
  vpinsrd $3, %edx, %xmm2, %xmm2
  vpsrld $16, %xmm2, %xmm2
  vpand %xmm0, %xmm2, %xmm2
  vpaddd %xmm1, %xmm2, %xmm2
  vmovdqu %xmm2, c+4194304(%rax)
  addq $16, %rax
  jne .LBB0_1
  xorl %eax, %eax
  xorl %esi, %esi
.LBB0_3:
  vpxor %xmm0, %xmm0, %xmm0
  movq $-8, %rcx
  vpxor %xmm1, %xmm1, %xmm1
  vpxor %xmm2, %xmm2, %xmm2
  vpxor %xmm3, %xmm3, %xmm3
.LBB0_4:
  vmovdqu a+32(,%rcx,4), %ymm4
  vmovdqu b+32(,%rcx,4), %ymm5
  vpmulld %ymm5, %ymm4, %ymm6
  vpmulld c+32(,%rcx,4), %ymm4, %ymm7
  vpaddd %ymm3, %ymm6, %ymm3
  vpaddd %ymm2, %ymm7, %ymm2
  vpaddd %ymm1, %ymm4, %ymm1
  vmovdqu a+64(,%rcx,4), %ymm4
  vmovdqu b+64(,%rcx,4), %ymm6
  vpmulld %ymm6, %ymm4, %ymm7
  vpaddd %ymm0, %ymm5, %ymm0
  vpmulld c+64(,%rcx,4), %ymm4, %ymm5
  vmovdqu a+96(,%rcx,4), %ymm8
  vmovdqu b+96(,%rcx,4), %ymm9
  vpmulld %ymm9, %ymm8, %ymm10
  vpaddd %ymm7, %ymm10, %ymm7
  vpmulld c+96(,%rcx,4), %ymm8, %ymm10
  vpaddd %ymm3, %ymm7, %ymm3
  vpaddd %ymm5, %ymm10, %ymm5
  vpaddd %ymm2, %ymm5, %ymm2
  vpaddd %ymm4, %ymm8, %ymm4
  vpaddd %ymm1, %ymm4, %ymm1
  vpaddd %ymm6, %ymm9, %ymm4
  vpaddd %ymm0, %ymm4, %ymm0
  vmovdqu a+128(,%rcx,4), %ymm4
  vmovdqu b+128(,%rcx,4), %ymm5
  vpmulld %ymm5, %ymm4, %ymm6
  vpaddd %ymm3, %ymm6, %ymm3
  vpmulld c+128(,%rcx,4), %ymm4, %ymm6
  vpaddd %ymm2, %ymm6, %ymm2
  vpaddd %ymm1, %ymm4, %ymm1
  vpaddd %ymm0, %ymm5, %ymm0
  addq $32, %rcx
  cmpq $1048568, %rcx
  jb .LBB0_4
  vextracti128 $1, %ymm3, %xmm4
  vpaddd %xmm4, %xmm3, %xmm3
  vpshufd $238, %xmm3, %xmm4
  vpaddd %xmm4, %xmm3, %xmm3
  vpshufd $85, %xmm3, %xmm4
  vpaddd %xmm4, %xmm3, %xmm3
  vmovd %xmm3, %ecx
  vextracti128 $1, %ymm2, %xmm3
  vpaddd %xmm3, %xmm2, %xmm2
  vpshufd $238, %xmm2, %xmm3
  vpaddd %xmm3, %xmm2, %xmm2
  vpshufd $85, %xmm2, %xmm3
  vpaddd %xmm3, %xmm2, %xmm2
  vmovd %xmm2, %edx
  vextracti128 $1, %ymm1, %xmm2
  vpaddd %xmm2, %xmm1, %xmm1
  vpshufd $238, %xmm1, %xmm2
  vpaddd %xmm2, %xmm1, %xmm1
  vpshufd $85, %xmm1, %xmm2
  vpaddd %xmm2, %xmm1, %xmm1
  vmovd %xmm1, %edi
  vextracti128 $1, %ymm0, %xmm1
  vpaddd %xmm1, %xmm0, %xmm0
  vpshufd $238, %xmm0, %xmm1
  vpaddd %xmm1, %xmm0, %xmm0
  vpshufd $85, %xmm0, %xmm1
  vpaddd %xmm1, %xmm0, %xmm0
  vmovd %xmm0, %r8d
  addl %edi, %r8d
  movslq %edx, %rdx
  movslq %ecx, %rcx
  addq %rdx, %rcx
  movslq %r8d, %rdx
  addq %rdx, %rsi
  addq %rcx, %rsi
  leal 1(%rax), %ecx
  cmpl $127, %eax
  movl %ecx, %eax
  jne .LBB0_3
  movl $.L.str, %edi
  xorl %eax, %eax
  vzeroupper
  callq printf
  xorl %eax, %eax
  addq $8, %rsp
  popq %rbx
  popq %rbp
  retq

.L.str:

