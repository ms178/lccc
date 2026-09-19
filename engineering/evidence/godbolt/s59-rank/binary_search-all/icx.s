.LCPI0_0:
main:
  vstmxcsr -4(%rsp)
  orl $32832, -4(%rsp)
  vldmxcsr -4(%rsp)
  movq $-8, %rax
  movl $72, %ecx
  vmovdqu .LCPI0_0(%rip), %ymm0
.LBB0_1:
  leal -72(%rcx), %edx
  vmovd %edx, %xmm1
  vpbroadcastd %xmm1, %ymm1
  vpor %ymm0, %ymm1, %ymm1
  vmovdqu %ymm1, table+32(,%rax,4)
  leal -48(%rcx), %edx
  vmovd %edx, %xmm1
  vpbroadcastd %xmm1, %ymm1
  vpaddd %ymm0, %ymm1, %ymm1
  vmovdqu %ymm1, table+64(,%rax,4)
  leal -24(%rcx), %edx
  vmovd %edx, %xmm1
  vpbroadcastd %xmm1, %ymm1
  vpaddd %ymm0, %ymm1, %ymm1
  vmovdqu %ymm1, table+96(,%rax,4)
  vmovd %ecx, %xmm1
  vpbroadcastd %xmm1, %ymm1
  vpaddd %ymm0, %ymm1, %ymm1
  vmovdqu %ymm1, table+128(,%rax,4)
  addq $32, %rax
  addl $96, %ecx
  cmpq $4088, %rax
  jb .LBB0_1
  xorl %ecx, %ecx
  movl $-1000, %eax
  jmp .LBB0_4
.LBB0_3:
  leal 14(%rax), %edx
  cmpl $13274, %eax
  movl %edx, %eax
  jge .LBB0_22
.LBB0_4:
  movl $4095, %edx
  xorl %esi, %esi
  jmp .LBB0_7
.LBB0_5:
  decl %edi
  movl %edi, %edx
  cmpl %edx, %esi
  jg .LBB0_13
.LBB0_7:
  movl %edx, %edi
  subl %esi, %edi
  sarl %edi
  addl %esi, %edi
  movslq %edi, %r8
  movl table(,%r8,4), %r8d
  cmpl %eax, %r8d
  je .LBB0_10
  jge .LBB0_5
  incl %edi
  movl %edi, %esi
  cmpl %edx, %esi
  jle .LBB0_7
  jmp .LBB0_13
.LBB0_10:
  testl %edi, %edi
  js .LBB0_13
  movl %edi, %edx
  cmpl %eax, table(,%rdx,4)
  jne .LBB0_24
  addl %edi, %ecx
.LBB0_13:
  leal 7(%rax), %edx
  movl $4095, %esi
  xorl %edi, %edi
  jmp .LBB0_16
.LBB0_14:
  decl %r8d
  movl %r8d, %esi
  cmpl %esi, %edi
  jg .LBB0_3
.LBB0_16:
  movl %esi, %r8d
  subl %edi, %r8d
  sarl %r8d
  addl %edi, %r8d
  movslq %r8d, %r9
  movl table(,%r9,4), %r9d
  cmpl %edx, %r9d
  je .LBB0_19
  jge .LBB0_14
  incl %r8d
  movl %r8d, %edi
  cmpl %esi, %edi
  jle .LBB0_16
  jmp .LBB0_3
.LBB0_19:
  testl %r8d, %r8d
  js .LBB0_3
  movl %r8d, %esi
  cmpl %edx, table(,%rsi,4)
  jne .LBB0_24
  addl %r8d, %ecx
  jmp .LBB0_3
.LBB0_22:
  xorl %eax, %eax
  cmpl $1198665, %ecx
  setne %al
  addl %eax, %eax
  vzeroupper
  retq
.LBB0_24:
  movl $1, %eax
  vzeroupper
  retq

