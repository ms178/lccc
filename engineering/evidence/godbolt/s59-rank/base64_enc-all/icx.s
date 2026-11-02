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
.LCPI0_17:
main:
  pushq %rax
  vstmxcsr 4(%rsp)
  orl $32832, 4(%rsp)
  vldmxcsr 4(%rsp)
  vmovups .LCPI0_0(%rip), %xmm0
  vmovups %xmm0, main.a(%rip)
  vmovdqu .LCPI0_1(%rip), %xmm1
  vmovdqu %xmm1, main.a+16(%rip)
  vmovups .LCPI0_2(%rip), %xmm2
  vmovups %xmm2, main.a+32(%rip)
  vmovups .LCPI0_3(%rip), %xmm2
  vmovups %xmm2, main.a+48(%rip)
  vmovups .LCPI0_4(%rip), %xmm2
  vmovups %xmm2, main.a+64(%rip)
  vmovups .LCPI0_5(%rip), %xmm2
  vmovups %xmm2, main.a+80(%rip)
  vmovups .LCPI0_6(%rip), %xmm2
  vmovups %xmm2, main.a+96(%rip)
  vmovups .LCPI0_7(%rip), %xmm2
  vmovups %xmm2, main.a+112(%rip)
  vmovups .LCPI0_8(%rip), %xmm2
  vmovups %xmm2, main.a+128(%rip)
  vmovups .LCPI0_9(%rip), %xmm2
  vmovups %xmm2, main.a+144(%rip)
  vmovups .LCPI0_10(%rip), %xmm2
  vmovups %xmm2, main.a+160(%rip)
  vmovups .LCPI0_11(%rip), %xmm2
  vmovups %xmm2, main.a+176(%rip)
  vmovups .LCPI0_12(%rip), %xmm2
  vmovups %xmm2, main.a+192(%rip)
  vmovups .LCPI0_13(%rip), %xmm2
  vmovups %xmm2, main.a+208(%rip)
  vmovups .LCPI0_14(%rip), %xmm2
  vmovups %xmm2, main.a+224(%rip)
  vmovups .LCPI0_15(%rip), %xmm2
  vmovups %xmm2, main.a+240(%rip)
  vmovups %xmm0, main.a+256(%rip)
  vmovdqu %xmm1, main.a+272(%rip)
  movl $284, %eax
  movb $-96, %cl
  vpbroadcastd .LCPI0_17(%rip), %xmm0
.LBB0_1:
  vmovd %ecx, %xmm1
  vpbroadcastb %xmm1, %xmm1
  vpaddb %xmm0, %xmm1, %xmm1
  vmovd %xmm1, main.a+4(%rax)
  addq $4, %rax
  addb $-108, %cl
  cmpq $296, %rax
  jb .LBB0_1
  movq $-3, %rax
  movl $main.d+3, %ecx
.LBB0_3:
  movzbl main.a+4(%rax), %esi
  shll $8, %esi
  movzbl main.a+5(%rax), %edx
  movzbl main.a+3(%rax), %edi
  movl %edi, %r8d
  shll $16, %r8d
  orl %esi, %r8d
  orl %edx, %esi
  shrl $2, %edi
  movzbl tab(%rdi), %edi
  movb %dil, -3(%rcx)
  shrl $12, %r8d
  andl $63, %r8d
  movzbl tab(%r8), %edi
  movb %dil, -2(%rcx)
  shrl $6, %esi
  andl $63, %esi
  movzbl tab(%rsi), %esi
  movb %sil, -1(%rcx)
  andl $63, %edx
  movzbl tab(%rdx), %edx
  movb %dl, (%rcx)
  addq $3, %rax
  addq $4, %rcx
  cmpq $297, %rax
  jb .LBB0_3
  movl $400, %esi
  movq $-400, %rax
.LBB0_5:
  movzbl main.d+400(%rax), %ecx
  movq %rsi, %rdx
  shlq $5, %rdx
  addq %rsi, %rdx
  addq %rcx, %rdx
  movzbl main.d+401(%rax), %ecx
  addq %rdx, %rcx
  shlq $5, %rdx
  addq %rdx, %rcx
  movzbl main.d+402(%rax), %edx
  addq %rcx, %rdx
  shlq $5, %rcx
  addq %rcx, %rdx
  movzbl main.d+403(%rax), %ecx
  addq %rdx, %rcx
  shlq $5, %rdx
  addq %rdx, %rcx
  movzbl main.d+404(%rax), %edx
  addq %rcx, %rdx
  shlq $5, %rcx
  addq %rcx, %rdx
  movzbl main.d+405(%rax), %ecx
  addq %rdx, %rcx
  shlq $5, %rdx
  addq %rdx, %rcx
  movzbl main.d+406(%rax), %edx
  addq %rcx, %rdx
  shlq $5, %rcx
  addq %rcx, %rdx
  movzbl main.d+407(%rax), %esi
  addq %rdx, %rsi
  shlq $5, %rdx
  addq %rdx, %rsi
  addq $8, %rax
  jne .LBB0_5
  movl $.L.str, %edi
  xorl %eax, %eax
  callq printf
  xorl %eax, %eax
  popq %rcx
  retq

.L.str:

tab:

