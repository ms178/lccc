.LCPI0_0:
.LCPI0_1:
main:
  vstmxcsr -4(%rsp)
  orl $32832, -4(%rsp)
  vldmxcsr -4(%rsp)
  movq $-16, %rax
  movl $-1436102352, %ecx
  vmovdqu .LCPI0_0(%rip), %ymm0
  vmovdqu .LCPI0_1(%rip), %ymm1
.LBB0_1:
  leal 1436102352(%rcx), %edx
  vmovd %edx, %xmm2
  vpbroadcastd %xmm2, %ymm2
  vpaddd %ymm0, %ymm2, %ymm3
  vpaddd %ymm1, %ymm2, %ymm2
  vpsrld $24, %ymm2, %ymm2
  vpsrld $24, %ymm3, %ymm3
  vpackusdw %ymm2, %ymm3, %ymm2
  vextracti128 $1, %ymm2, %xmm3
  vpackuswb %xmm3, %xmm2, %xmm2
  vpshufd $216, %xmm2, %xmm2
  vmovdqu %xmm2, bytes+16(%rax)
  leal 957401568(%rcx), %edx
  vmovd %edx, %xmm2
  vpbroadcastd %xmm2, %ymm2
  vpaddd %ymm0, %ymm2, %ymm3
  vpaddd %ymm1, %ymm2, %ymm2
  vpsrld $24, %ymm2, %ymm2
  vpsrld $24, %ymm3, %ymm3
  vpackusdw %ymm2, %ymm3, %ymm2
  vextracti128 $1, %ymm2, %xmm3
  vpackuswb %xmm3, %xmm2, %xmm2
  vpshufd $216, %xmm2, %xmm2
  vmovdqu %xmm2, bytes+32(%rax)
  leal 478700784(%rcx), %edx
  vmovd %edx, %xmm2
  vpbroadcastd %xmm2, %ymm2
  vpaddd %ymm0, %ymm2, %ymm3
  vpaddd %ymm1, %ymm2, %ymm2
  vpsrld $24, %ymm2, %ymm2
  vpsrld $24, %ymm3, %ymm3
  vpackusdw %ymm2, %ymm3, %ymm2
  vextracti128 $1, %ymm2, %xmm3
  vpackuswb %xmm3, %xmm2, %xmm2
  vpshufd $216, %xmm2, %xmm2
  vmovdqu %xmm2, bytes+48(%rax)
  vmovd %ecx, %xmm2
  vpbroadcastd %xmm2, %ymm2
  vpaddd %ymm0, %ymm2, %ymm3
  vpaddd %ymm1, %ymm2, %ymm2
  vpsrld $24, %ymm2, %ymm2
  vpsrld $24, %ymm3, %ymm3
  vpackusdw %ymm2, %ymm3, %ymm2
  vextracti128 $1, %ymm2, %xmm3
  vpackuswb %xmm3, %xmm2, %xmm2
  vpshufd $216, %xmm2, %xmm2
  vmovdqu %xmm2, bytes+64(%rax)
  addq $64, %rax
  addl $-1914803136, %ecx
  cmpq $262128, %rax
  jb .LBB0_1
  movq $-262144, %rax
.LBB0_3:
  movzbl bytes+262144(%rax), %ecx
  incq bins(,%rcx,8)
  movzbl bytes+262145(%rax), %ecx
  incq bins(,%rcx,8)
  movzbl bytes+262146(%rax), %ecx
  incq bins(,%rcx,8)
  movzbl bytes+262147(%rax), %ecx
  incq bins(,%rcx,8)
  movzbl bytes+262148(%rax), %ecx
  incq bins(,%rcx,8)
  movzbl bytes+262149(%rax), %ecx
  incq bins(,%rcx,8)
  movzbl bytes+262150(%rax), %ecx
  incq bins(,%rcx,8)
  movzbl bytes+262151(%rax), %ecx
  incq bins(,%rcx,8)
  addq $8, %rax
  jne .LBB0_3
  movq $-2048, %rax
  xorl %edx, %edx
  xorl %ecx, %ecx
.LBB0_5:
  movq bins+2048(%rax), %rsi
  addq %rsi, %rcx
  imulq $131, %rdx, %rdx
  addq %rsi, %rdx
  movq bins+2056(%rax), %rsi
  imulq $131, %rdx, %rdx
  addq %rsi, %rdx
  movq bins+2064(%rax), %rdi
  addq %rdi, %rsi
  addq %rcx, %rsi
  imulq $131, %rdx, %rcx
  addq %rdi, %rcx
  movq bins+2072(%rax), %rdx
  imulq $131, %rcx, %rcx
  addq %rdx, %rcx
  movq bins+2080(%rax), %rdi
  addq %rdi, %rdx
  imulq $131, %rcx, %rcx
  addq %rdi, %rcx
  movq bins+2088(%rax), %rdi
  addq %rdi, %rdx
  addq %rsi, %rdx
  imulq $131, %rcx, %rsi
  addq %rdi, %rsi
  movq bins+2096(%rax), %rcx
  imulq $131, %rsi, %rsi
  addq %rcx, %rsi
  movq bins+2104(%rax), %rdi
  addq %rdi, %rcx
  addq %rdx, %rcx
  imulq $131, %rsi, %rdx
  addq %rdi, %rdx
  addq $64, %rax
  jne .LBB0_5
  movabsq $-7388273516099151790, %rax
  xorl %esi, %esi
  cmpq %rax, %rdx
  setne %sil
  addl %esi, %esi
  cmpq $262144, %rcx
  movl $1, %eax
  cmovel %esi, %eax
  vzeroupper
  retq

