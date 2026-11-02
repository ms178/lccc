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
main:
  vmovdqa .LCPI0_0(%rip), %ymm0
  xorl %ecx, %ecx
  leaq table(%rip), %rax
  vpbroadcastd .LCPI0_1(%rip), %ymm1
  vpbroadcastd .LCPI0_2(%rip), %ymm2
  vpbroadcastd .LCPI0_3(%rip), %ymm3
  vpbroadcastd .LCPI0_4(%rip), %ymm4
  vpbroadcastd .LCPI0_5(%rip), %ymm5
  vpbroadcastd .LCPI0_6(%rip), %ymm6
  vpbroadcastd .LCPI0_7(%rip), %ymm7
  vpbroadcastd .LCPI0_8(%rip), %ymm8
  vpbroadcastd .LCPI0_9(%rip), %ymm9
.LBB0_1:
  vpaddd %ymm0, %ymm0, %ymm10
  vpaddd %ymm0, %ymm10, %ymm10
  vpaddd %ymm1, %ymm10, %ymm11
  vpaddd %ymm2, %ymm10, %ymm12
  vpaddd %ymm3, %ymm10, %ymm13
  vpaddd %ymm4, %ymm10, %ymm14
  vmovdqu %ymm11, (%rax,%rcx,4)
  vmovdqu %ymm12, 32(%rax,%rcx,4)
  vmovdqu %ymm13, 64(%rax,%rcx,4)
  vmovdqu %ymm14, 96(%rax,%rcx,4)
  vpaddd %ymm5, %ymm10, %ymm11
  vpaddd %ymm6, %ymm10, %ymm12
  vpaddd %ymm7, %ymm10, %ymm13
  vpaddd %ymm8, %ymm10, %ymm10
  vmovdqu %ymm11, 128(%rax,%rcx,4)
  vmovdqu %ymm12, 160(%rax,%rcx,4)
  vmovdqu %ymm13, 192(%rax,%rcx,4)
  vmovdqu %ymm10, 224(%rax,%rcx,4)
  addq $64, %rcx
  vpaddd %ymm0, %ymm9, %ymm0
  cmpq $4096, %rcx
  jne .LBB0_1
  xorl %ecx, %ecx
  movl $-1000, %edx
  jmp .LBB0_3
.LBB0_16:
  addl %ecx, %r9d
  movl %r9d, %ecx
.LBB0_17:
  cmpl $13274, %edx
  leal 14(%rdx), %edx
  jge .LBB0_18
.LBB0_3:
  movl $4095, %esi
  xorl %edi, %edi
  jmp .LBB0_4
.LBB0_7:
  decl %r8d
  movl %r8d, %esi
.LBB0_8:
  cmpl %esi, %edi
  jg .LBB0_10
.LBB0_4:
  movl %esi, %r8d
  subl %edi, %r8d
  shrl %r8d
  addl %edi, %r8d
  movl (%rax,%r8,4), %r9d
  cmpl %edx, %r9d
  je .LBB0_9
  jge .LBB0_7
  incl %r8d
  movl %r8d, %edi
  jmp .LBB0_8
.LBB0_9:
  addl %ecx, %r8d
  movl %r8d, %ecx
.LBB0_10:
  leal 7(%rdx), %esi
  movl $4095, %edi
  xorl %r8d, %r8d
  jmp .LBB0_11
.LBB0_14:
  decl %r9d
  movl %r9d, %edi
.LBB0_15:
  cmpl %edi, %r8d
  jg .LBB0_17
.LBB0_11:
  movl %edi, %r9d
  subl %r8d, %r9d
  shrl %r9d
  addl %r8d, %r9d
  movl (%rax,%r9,4), %r10d
  cmpl %esi, %r10d
  je .LBB0_16
  jge .LBB0_14
  incl %r9d
  movl %r9d, %r8d
  jmp .LBB0_15
.LBB0_18:
  xorl %eax, %eax
  cmpl $1198665, %ecx
  setne %al
  addl %eax, %eax
  vzeroupper
  retq

