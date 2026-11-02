.LCPI0_0:
.LCPI0_1:
.LCPI0_2:
.LCPI0_3:
.LCPI0_4:
.LCPI0_5:
.LCPI0_6:
.LCPI0_7:
main:
  pushq %rbp
  pushq %r15
  pushq %r14
  pushq %r13
  pushq %r12
  pushq %rbx
  subq $344, %rsp
  vstmxcsr 48(%rsp)
  orl $32832, 48(%rsp)
  vldmxcsr 48(%rsp)
  vmovups .LCPI0_0(%rip), %xmm0
  vmovups %ymm0, 48(%rsp)
  vmovups .LCPI0_1(%rip), %ymm0
  vmovups %ymm0, 80(%rsp)
  vmovups .LCPI0_2(%rip), %ymm0
  vmovups %ymm0, 112(%rsp)
  vmovups .LCPI0_3(%rip), %ymm0
  vmovups %ymm0, 144(%rsp)
  vmovups .LCPI0_4(%rip), %ymm0
  vmovups %ymm0, 176(%rsp)
  vmovups .LCPI0_5(%rip), %ymm0
  vmovups %ymm0, 208(%rsp)
  vmovups .LCPI0_6(%rip), %ymm0
  vmovups %ymm0, 240(%rsp)
  vmovups .LCPI0_7(%rip), %ymm0
  vmovups %ymm0, 272(%rsp)
  movl $1013904242, %ecx
  movl $-1694144372, %r8d
  movl $-1150833019, %r11d
  movl $1359893119, %eax
  movl $1541459225, %r14d
  movl $-1521486534, %ebx
  movl $528734635, %edi
  movl $1779033703, %ebp
  xorl %edx, %edx
.LBB0_1:
  movl %r8d, %esi
  movl %r11d, %r10d
  movl %ebp, %r9d
  rorxl $6, %eax, %r8d
  rorxl $11, %eax, %r11d
  xorl %r8d, %r11d
  rorxl $25, %eax, %r8d
  xorl %r11d, %r8d
  movl %eax, %r11d
  andl %esi, %r11d
  andnl %edi, %eax, %ebp
  addl %r11d, %ebp
  addl %r8d, %ebp
  addl %r14d, %ebp
  movl K(%rdx), %r8d
  rorxl $2, %r9d, %r11d
  rorxl $13, %r9d, %r14d
  xorl %r11d, %r14d
  rorxl $22, %r9d, %r15d
  xorl %r14d, %r15d
  movl %ecx, %r14d
  xorl %r10d, %r14d
  andl %r9d, %r14d
  movl %ecx, %r11d
  andl %r10d, %r11d
  xorl %r14d, %r11d
  addl 48(%rsp,%rdx), %r8d
  addl %r15d, %r11d
  addl %ebp, %r8d
  addl %r8d, %r11d
  addl %ebx, %r8d
  rorxl $6, %r8d, %ebx
  rorxl $11, %r8d, %ebp
  xorl %ebx, %ebp
  rorxl $25, %r8d, %ebx
  xorl %ebp, %ebx
  rorxl $2, %r11d, %ebp
  rorxl $13, %r11d, %r14d
  xorl %ebp, %r14d
  rorxl $22, %r11d, %r15d
  xorl %r14d, %r15d
  movl %r10d, %ebp
  movl %r10d, %r14d
  xorl %r9d, %r14d
  andl %r11d, %r14d
  andl %r9d, %ebp
  xorl %r14d, %ebp
  andnl %esi, %r8d, %r14d
  addl %r15d, %ebp
  movl K+4(%rdx), %r15d
  addl 52(%rsp,%rdx), %r15d
  addl %r14d, %r15d
  movl %r8d, %r14d
  andl %eax, %r14d
  addl %r14d, %r15d
  addl %ebx, %r15d
  addl %edi, %r15d
  movl %r10d, %ebx
  addl %r15d, %ebp
  addl %ecx, %r15d
  addq $8, %rdx
  movl %eax, %edi
  movl %esi, %r14d
  movl %r15d, %eax
  movl %r9d, %ecx
  cmpq $256, %rdx
  jne .LBB0_1
  movl $2, %ebx
  cmpl $1349398616, %ebp
  jne .LBB0_12
  cmpl $-744873627, %r11d
  jne .LBB0_12
  cmpl $-1776334700, %esi
  jne .LBB0_12
  xorl %esi, %esi
  xorl %eax, %eax
.LBB0_6:
  movq %rax, 320(%rsp)
  movl %eax, %ecx
  xorl $1779033703, %ecx
  movl $1541459225, %edx
  movl $528734635, %r10d
  movl $-1694144372, %r8d
  movl $1359893119, %r9d
  movl $-1521486534, %edi
  movl $1013904242, %r11d
  movl $-1150833019, %ebx
  xorl %eax, %eax
.LBB0_7:
  movl %r10d, 44(%rsp)
  movl %r8d, 4(%rsp)
  movl %r9d, (%rsp)
  movq %rsi, 336(%rsp)
  movq %rax, 328(%rsp)
  imull $1664525, %eax, %r14d
  movl %ebx, %eax
  movl %ecx, %ebx
  xorl %r14d, %ebx
  movl %ecx, %r15d
  movl %ecx, 32(%rsp)
  leal 1013904223(%r14), %ecx
  xorl %eax, %ecx
  movl %eax, 40(%rsp)
  leal 2027808446(%r14), %r12d
  xorl %r11d, %r12d
  leal -1253254627(%r14), %esi
  xorl %edi, %esi
  movl %esi, 20(%rsp)
  leal -239350404(%r14), %esi
  xorl %r9d, %esi
  movl %esi, 24(%rsp)
  leal 774553819(%r14), %esi
  xorl %r8d, %esi
  movl %esi, 12(%rsp)
  leal 1788458042(%r14), %r13d
  xorl %r10d, %r13d
  movl %r13d, 312(%rsp)
  leal -1492605031(%r14), %esi
  xorl %edx, %esi
  movl %esi, 16(%rsp)
  leal -478700808(%r14), %ebp
  xorl %r15d, %ebp
  movl %ebp, 308(%rsp)
  movl %r11d, %r9d
  movl %r11d, 316(%rsp)
  leal 535203415(%r14), %esi
  xorl %eax, %esi
  movl %esi, 8(%rsp)
  leal 1309757234(%r14), %r15d
  xorl %r10d, %r15d
  rorxl $17, %r15d, %eax
  movl %edx, %r11d
  movl %edx, 36(%rsp)
  rorxl $19, %r15d, %edx
  xorl %eax, %edx
  movl %r15d, %r8d
  shrl $10, %r8d
  xorl %edx, %r8d
  rorxl $7, %ecx, %eax
  rorxl $18, %ecx, %edx
  xorl %eax, %edx
  movl %edi, %esi
  movl %edi, 28(%rsp)
  leal 1549107638(%r14), %edi
  xorl %r9d, %edi
  movl %ebx, 48(%rsp)
  movl %ecx, 52(%rsp)
  movl %r12d, 56(%rsp)
  movl 20(%rsp), %eax
  movl %eax, 60(%rsp)
  movl 24(%rsp), %eax
  movl %eax, 64(%rsp)
  movl 12(%rsp), %eax
  movl %eax, 68(%rsp)
  movl %r13d, 72(%rsp)
  movl 16(%rsp), %eax
  movl %eax, 76(%rsp)
  movl %ebp, 80(%rsp)
  movl 8(%rsp), %r13d
  movl %r13d, 84(%rsp)
  movl %edi, 88(%rsp)
  addl %ecx, %edi
  shrl $3, %ecx
  xorl %edx, %ecx
  leal -1731955435(%r14), %r9d
  addl %r13d, %ebx
  addl %ecx, %ebx
  leal -718051212(%r14), %r10d
  addl %r8d, %ebx
  leal 295853011(%r14), %r8d
  addl $-1971305839, %r14d
  xorl %r11d, %r14d
  rorxl $17, %r14d, %ecx
  rorxl $19, %r14d, %eax
  xorl %ecx, %eax
  movl %r14d, %ecx
  shrl $10, %ecx
  xorl %eax, %ecx
  rorxl $7, %r12d, %edx
  rorxl $18, %r12d, %eax
  xorl %edx, %eax
  xorl %esi, %r9d
  movl %r9d, 92(%rsp)
  addl %r12d, %r9d
  shrl $3, %r12d
  xorl %eax, %r12d
  addl %r12d, %edi
  addl %ecx, %edi
  rorxl $17, %ebx, %eax
  rorxl $19, %ebx, %ebp
  xorl %eax, %ebp
  movl 308(%rsp), %r11d
  rorxl $7, %r11d, %ecx
  rorxl $18, %r11d, %eax
  xorl %ecx, %eax
  movl %r13d, %r12d
  rorxl $7, %r13d, %ecx
  rorxl $18, %r13d, %edx
  xorl %ecx, %edx
  shrl $3, %r12d
  xorl %edx, %r12d
  addl %r11d, %r12d
  movl %r11d, %esi
  shrl $3, %esi
  xorl %eax, %esi
  xorl (%rsp), %r10d
  xorl 4(%rsp), %r8d
  movl %r10d, 96(%rsp)
  movl %r8d, 100(%rsp)
  movl %r15d, 104(%rsp)
  movl %r14d, 108(%rsp)
  movl %ebx, 112(%rsp)
  movl 16(%rsp), %r11d
  addl %r11d, %esi
  addl %ebx, %esi
  shrl $10, %ebx
  xorl %ebp, %ebx
  movl 20(%rsp), %ebp
  rorxl $7, %ebp, %eax
  rorxl $18, %ebp, %ecx
  xorl %eax, %ecx
  addl %ebp, %r10d
  shrl $3, %ebp
  xorl %ecx, %ebp
  movl %edi, 116(%rsp)
  addl %ebp, %r9d
  addl %ebx, %r9d
  rorxl $17, %edi, %eax
  rorxl $19, %edi, %ecx
  xorl %eax, %ecx
  addl %edi, %r12d
  shrl $10, %edi
  xorl %ecx, %edi
  movl 24(%rsp), %r13d
  rorxl $7, %r13d, %eax
  rorxl $18, %r13d, %ecx
  xorl %eax, %ecx
  addl %r13d, %r8d
  shrl $3, %r13d
  xorl %ecx, %r13d
  addl %r13d, %r10d
  addl %edi, %r10d
  rorxl $17, %r9d, %eax
  rorxl $19, %r9d, %ecx
  xorl %eax, %ecx
  movl %r9d, %eax
  shrl $10, %eax
  xorl %ecx, %eax
  movl 12(%rsp), %edi
  rorxl $7, %edi, %ecx
  rorxl $18, %edi, %edx
  xorl %ecx, %edx
  movl %r9d, 120(%rsp)
  movl %r10d, 124(%rsp)
  addl %edi, %r15d
  movl %edi, %ecx
  shrl $3, %ecx
  xorl %edx, %ecx
  addl %ecx, %r8d
  addl %eax, %r8d
  movl %r8d, 128(%rsp)
  rorxl $17, %r10d, %eax
  rorxl $19, %r10d, %ecx
  xorl %eax, %ecx
  movl %r10d, %eax
  shrl $10, %eax
  xorl %ecx, %eax
  movl 312(%rsp), %edi
  rorxl $7, %edi, %ecx
  rorxl $18, %edi, %edx
  xorl %ecx, %edx
  addl %edi, %r14d
  movl %edi, %ecx
  shrl $3, %ecx
  xorl %edx, %ecx
  addl %ecx, %r15d
  addl %eax, %r15d
  movl %r15d, 132(%rsp)
  rorxl $17, %r8d, %eax
  rorxl $19, %r8d, %ecx
  xorl %eax, %ecx
  shrl $10, %r8d
  xorl %ecx, %r8d
  rorxl $7, %r11d, %eax
  rorxl $18, %r11d, %ecx
  xorl %eax, %ecx
  shrl $3, %r11d
  xorl %ecx, %r11d
  addl %r11d, %r14d
  addl %r8d, %r14d
  movl %r14d, 136(%rsp)
  rorxl $17, %r15d, %eax
  rorxl $19, %r15d, %ecx
  xorl %eax, %ecx
  movl %r15d, %eax
  shrl $10, %eax
  xorl %ecx, %eax
  addl %eax, %esi
  movl %esi, 140(%rsp)
  rorxl $17, %r14d, %eax
  rorxl $19, %r14d, %ecx
  xorl %eax, %ecx
  movl %r14d, %eax
  shrl $10, %eax
  xorl %ecx, %eax
  addl %eax, %r12d
  movl %r12d, 144(%rsp)
  rorxl $17, %esi, %eax
  rorxl $19, %esi, %ecx
  xorl %eax, %ecx
  movl %esi, %edx
  shrl $10, %edx
  xorl %ecx, %edx
  movl 88(%rsp), %ecx
  movl 92(%rsp), %eax
  rorxl $7, %ecx, %edi
  rorxl $18, %ecx, %ebx
  xorl %edi, %ebx
  rorxl $7, %eax, %edi
  rorxl $18, %eax, %ebp
  xorl %edi, %ebp
  movl %eax, %r8d
  shrl $3, %r8d
  xorl %ebp, %r8d
  addl %ecx, %r8d
  shrl $3, %ecx
  xorl %ebx, %ecx
  addl 8(%rsp), %ecx
  addl %r9d, %ecx
  addl %edx, %ecx
  movl %ecx, 148(%rsp)
  rorxl $17, %r12d, %edx
  rorxl $19, %r12d, %edi
  xorl %edx, %edi
  movl %r12d, %edx
  shrl $10, %edx
  xorl %edi, %edx
  addl %r10d, %r8d
  addl %edx, %r8d
  movl %r8d, 152(%rsp)
  rorxl $17, %ecx, %edx
  rorxl $19, %ecx, %edi
  xorl %edx, %edi
  shrl $10, %ecx
  xorl %edi, %ecx
  movl 96(%rsp), %edx
  rorxl $7, %edx, %edi
  rorxl $18, %edx, %r9d
  xorl %edi, %r9d
  movl %edx, %edi
  shrl $3, %edi
  xorl %r9d, %edi
  addl 128(%rsp), %eax
  addl %edi, %eax
  addl %ecx, %eax
  movl %eax, 156(%rsp)
  rorxl $17, %r8d, %ecx
  rorxl $19, %r8d, %edi
  xorl %ecx, %edi
  shrl $10, %r8d
  xorl %edi, %r8d
  movl 100(%rsp), %r10d
  rorxl $7, %r10d, %ecx
  rorxl $18, %r10d, %r9d
  xorl %ecx, %r9d
  movl %r10d, %edi
  shrl $3, %edi
  xorl %r9d, %edi
  addl %edx, %edi
  addl %r15d, %edi
  addl %r8d, %edi
  movl %edi, 160(%rsp)
  rorxl $17, %eax, %ecx
  rorxl $19, %eax, %edx
  xorl %ecx, %edx
  shrl $10, %eax
  xorl %edx, %eax
  movl 104(%rsp), %r9d
  rorxl $7, %r9d, %ecx
  rorxl $18, %r9d, %edx
  xorl %ecx, %edx
  movl %r9d, %ecx
  shrl $3, %ecx
  xorl %edx, %ecx
  addl %r10d, %ecx
  addl %r14d, %ecx
  addl %eax, %ecx
  movl %ecx, 164(%rsp)
  rorxl $17, %edi, %eax
  rorxl $19, %edi, %edx
  xorl %eax, %edx
  shrl $10, %edi
  xorl %edx, %edi
  movl 108(%rsp), %r8d
  rorxl $7, %r8d, %eax
  rorxl $18, %r8d, %r10d
  xorl %eax, %r10d
  movl %r8d, %edx
  shrl $3, %edx
  xorl %r10d, %edx
  addl %r9d, %edx
  addl %esi, %edx
  addl %edi, %edx
  movl %edx, 168(%rsp)
  rorxl $17, %ecx, %eax
  rorxl $19, %ecx, %esi
  xorl %eax, %esi
  shrl $10, %ecx
  xorl %esi, %ecx
  movl 112(%rsp), %eax
  rorxl $7, %eax, %esi
  rorxl $18, %eax, %edi
  xorl %esi, %edi
  movl %eax, %esi
  shrl $3, %esi
  xorl %edi, %esi
  addl %r8d, %esi
  addl %r12d, %esi
  addl %ecx, %esi
  movl %esi, 172(%rsp)
  rorxl $17, %edx, %ecx
  rorxl $19, %edx, %edi
  xorl %ecx, %edi
  shrl $10, %edx
  xorl %edi, %edx
  movl 116(%rsp), %ecx
  rorxl $7, %ecx, %edi
  rorxl $18, %ecx, %r8d
  xorl %edi, %r8d
  movl %ecx, %edi
  shrl $3, %edi
  xorl %r8d, %edi
  addl 148(%rsp), %eax
  addl %edi, %eax
  addl %edx, %eax
  movl %eax, 176(%rsp)
  rorxl $17, %esi, %edx
  rorxl $19, %esi, %edi
  xorl %edx, %edi
  shrl $10, %esi
  xorl %edi, %esi
  movl 120(%rsp), %edx
  rorxl $7, %edx, %edi
  rorxl $18, %edx, %r8d
  xorl %edi, %r8d
  movl %edx, %edi
  shrl $3, %edi
  xorl %r8d, %edi
  addl 152(%rsp), %ecx
  addl %edi, %ecx
  addl %esi, %ecx
  movl %ecx, 180(%rsp)
  rorxl $17, %eax, %esi
  rorxl $19, %eax, %edi
  xorl %esi, %edi
  shrl $10, %eax
  xorl %edi, %eax
  movl 124(%rsp), %esi
  rorxl $7, %esi, %edi
  rorxl $18, %esi, %r8d
  xorl %edi, %r8d
  movl %esi, %edi
  shrl $3, %edi
  xorl %r8d, %edi
  addl 156(%rsp), %edx
  addl %edi, %edx
  addl %eax, %edx
  movl %edx, 184(%rsp)
  rorxl $17, %ecx, %eax
  rorxl $19, %ecx, %edi
  xorl %eax, %edi
  shrl $10, %ecx
  xorl %edi, %ecx
  movl 128(%rsp), %eax
  rorxl $7, %eax, %edi
  rorxl $18, %eax, %r8d
  xorl %edi, %r8d
  movl %eax, %edi
  shrl $3, %edi
  xorl %r8d, %edi
  addl 160(%rsp), %esi
  addl %edi, %esi
  addl %ecx, %esi
  movl %esi, 188(%rsp)
  rorxl $17, %edx, %ecx
  rorxl $19, %edx, %edi
  xorl %ecx, %edi
  shrl $10, %edx
  xorl %edi, %edx
  movl 132(%rsp), %ecx
  rorxl $7, %ecx, %edi
  rorxl $18, %ecx, %r8d
  xorl %edi, %r8d
  movl %ecx, %edi
  shrl $3, %edi
  xorl %r8d, %edi
  addl 164(%rsp), %eax
  addl %edi, %eax
  addl %edx, %eax
  movl %eax, 192(%rsp)
  rorxl $17, %esi, %edx
  rorxl $19, %esi, %edi
  xorl %edx, %edi
  shrl $10, %esi
  xorl %edi, %esi
  movl 136(%rsp), %edx
  rorxl $7, %edx, %edi
  rorxl $18, %edx, %r8d
  xorl %edi, %r8d
  movl %edx, %edi
  shrl $3, %edi
  xorl %r8d, %edi
  addl 168(%rsp), %ecx
  addl %edi, %ecx
  addl %esi, %ecx
  movl %ecx, 196(%rsp)
  rorxl $17, %eax, %esi
  rorxl $19, %eax, %edi
  xorl %esi, %edi
  shrl $10, %eax
  xorl %edi, %eax
  movl 140(%rsp), %esi
  rorxl $7, %esi, %edi
  rorxl $18, %esi, %r8d
  xorl %edi, %r8d
  movl %esi, %edi
  shrl $3, %edi
  xorl %r8d, %edi
  addl 172(%rsp), %edx
  addl %edi, %edx
  addl %eax, %edx
  movl %edx, 200(%rsp)
  rorxl $17, %ecx, %eax
  rorxl $19, %ecx, %edi
  xorl %eax, %edi
  shrl $10, %ecx
  xorl %edi, %ecx
  movl 144(%rsp), %eax
  rorxl $7, %eax, %edi
  rorxl $18, %eax, %r8d
  xorl %edi, %r8d
  movl %eax, %edi
  shrl $3, %edi
  xorl %r8d, %edi
  addl 176(%rsp), %esi
  addl %edi, %esi
  addl %ecx, %esi
  movl %esi, 204(%rsp)
  rorxl $17, %edx, %ecx
  rorxl $19, %edx, %edi
  xorl %ecx, %edi
  shrl $10, %edx
  xorl %edi, %edx
  movl 148(%rsp), %ecx
  rorxl $7, %ecx, %edi
  rorxl $18, %ecx, %r8d
  xorl %edi, %r8d
  movl %ecx, %edi
  shrl $3, %edi
  xorl %r8d, %edi
  addl 180(%rsp), %eax
  addl %edi, %eax
  addl %edx, %eax
  movl %eax, 208(%rsp)
  rorxl $17, %esi, %edx
  rorxl $19, %esi, %edi
  xorl %edx, %edi
  shrl $10, %esi
  xorl %edi, %esi
  movl 152(%rsp), %edx
  rorxl $7, %edx, %edi
  rorxl $18, %edx, %r8d
  xorl %edi, %r8d
  movl %edx, %edi
  shrl $3, %edi
  xorl %r8d, %edi
  addl 184(%rsp), %ecx
  addl %edi, %ecx
  addl %esi, %ecx
  movl %ecx, 212(%rsp)
  rorxl $17, %eax, %esi
  rorxl $19, %eax, %edi
  xorl %esi, %edi
  shrl $10, %eax
  xorl %edi, %eax
  movl 156(%rsp), %esi
  rorxl $7, %esi, %edi
  rorxl $18, %esi, %r8d
  xorl %edi, %r8d
  movl %esi, %edi
  shrl $3, %edi
  xorl %r8d, %edi
  addl 188(%rsp), %edx
  addl %edi, %edx
  addl %eax, %edx
  movl %edx, 216(%rsp)
  rorxl $17, %ecx, %eax
  rorxl $19, %ecx, %edi
  xorl %eax, %edi
  shrl $10, %ecx
  xorl %edi, %ecx
  movl 160(%rsp), %eax
  rorxl $7, %eax, %edi
  rorxl $18, %eax, %r8d
  xorl %edi, %r8d
  movl %eax, %edi
  shrl $3, %edi
  xorl %r8d, %edi
  addl 192(%rsp), %esi
  addl %edi, %esi
  addl %ecx, %esi
  movl %esi, 220(%rsp)
  rorxl $17, %edx, %ecx
  rorxl $19, %edx, %edi
  xorl %ecx, %edi
  shrl $10, %edx
  xorl %edi, %edx
  movl 164(%rsp), %ecx
  rorxl $7, %ecx, %edi
  rorxl $18, %ecx, %r8d
  xorl %edi, %r8d
  movl %ecx, %edi
  shrl $3, %edi
  xorl %r8d, %edi
  addl 196(%rsp), %eax
  addl %edi, %eax
  addl %edx, %eax
  movl %eax, 224(%rsp)
  rorxl $17, %esi, %edx
  rorxl $19, %esi, %edi
  xorl %edx, %edi
  shrl $10, %esi
  xorl %edi, %esi
  movl 168(%rsp), %edx
  rorxl $7, %edx, %edi
  rorxl $18, %edx, %r8d
  xorl %edi, %r8d
  movl %edx, %edi
  shrl $3, %edi
  xorl %r8d, %edi
  addl 200(%rsp), %ecx
  addl %edi, %ecx
  addl %esi, %ecx
  movl %ecx, 228(%rsp)
  rorxl $17, %eax, %esi
  rorxl $19, %eax, %edi
  xorl %esi, %edi
  shrl $10, %eax
  xorl %edi, %eax
  movl 172(%rsp), %esi
  rorxl $7, %esi, %edi
  rorxl $18, %esi, %r8d
  xorl %edi, %r8d
  movl %esi, %edi
  shrl $3, %edi
  xorl %r8d, %edi
  addl 204(%rsp), %edx
  addl %edi, %edx
  addl %eax, %edx
  movl %edx, 232(%rsp)
  rorxl $17, %ecx, %eax
  rorxl $19, %ecx, %edi
  xorl %eax, %edi
  shrl $10, %ecx
  xorl %edi, %ecx
  movl 176(%rsp), %eax
  rorxl $7, %eax, %edi
  rorxl $18, %eax, %r8d
  xorl %edi, %r8d
  movl %eax, %edi
  shrl $3, %edi
  xorl %r8d, %edi
  addl 208(%rsp), %esi
  addl %edi, %esi
  addl %ecx, %esi
  movl %esi, 236(%rsp)
  rorxl $17, %edx, %ecx
  rorxl $19, %edx, %edi
  xorl %ecx, %edi
  shrl $10, %edx
  xorl %edi, %edx
  movl 180(%rsp), %ecx
  rorxl $7, %ecx, %edi
  rorxl $18, %ecx, %r8d
  xorl %edi, %r8d
  movl %ecx, %edi
  shrl $3, %edi
  xorl %r8d, %edi
  addl 212(%rsp), %eax
  addl %edi, %eax
  addl %edx, %eax
  movl %eax, 240(%rsp)
  rorxl $17, %esi, %edx
  rorxl $19, %esi, %edi
  xorl %edx, %edi
  shrl $10, %esi
  xorl %edi, %esi
  movl 184(%rsp), %edx
  rorxl $7, %edx, %edi
  rorxl $18, %edx, %r8d
  xorl %edi, %r8d
  movl %edx, %edi
  shrl $3, %edi
  xorl %r8d, %edi
  addl 216(%rsp), %ecx
  addl %edi, %ecx
  addl %esi, %ecx
  movl %ecx, 244(%rsp)
  rorxl $17, %eax, %esi
  rorxl $19, %eax, %edi
  xorl %esi, %edi
  shrl $10, %eax
  xorl %edi, %eax
  movl 188(%rsp), %esi
  rorxl $7, %esi, %edi
  rorxl $18, %esi, %r8d
  xorl %edi, %r8d
  movl %esi, %edi
  shrl $3, %edi
  xorl %r8d, %edi
  addl 220(%rsp), %edx
  addl %edi, %edx
  addl %eax, %edx
  movl %edx, 248(%rsp)
  rorxl $17, %ecx, %eax
  rorxl $19, %ecx, %edi
  xorl %eax, %edi
  shrl $10, %ecx
  xorl %edi, %ecx
  movl 192(%rsp), %eax
  rorxl $7, %eax, %edi
  rorxl $18, %eax, %r8d
  xorl %edi, %r8d
  movl %eax, %edi
  shrl $3, %edi
  xorl %r8d, %edi
  addl 224(%rsp), %esi
  addl %edi, %esi
  addl %ecx, %esi
  movl %esi, 252(%rsp)
  rorxl $17, %edx, %ecx
  rorxl $19, %edx, %edi
  xorl %ecx, %edi
  shrl $10, %edx
  xorl %edi, %edx
  movl 196(%rsp), %ecx
  rorxl $7, %ecx, %edi
  rorxl $18, %ecx, %r8d
  xorl %edi, %r8d
  movl %ecx, %edi
  shrl $3, %edi
  xorl %r8d, %edi
  addl 228(%rsp), %eax
  addl %edi, %eax
  addl %edx, %eax
  movl %eax, 256(%rsp)
  rorxl $17, %esi, %edx
  rorxl $19, %esi, %edi
  xorl %edx, %edi
  shrl $10, %esi
  xorl %edi, %esi
  movl 200(%rsp), %edx
  rorxl $7, %edx, %edi
  rorxl $18, %edx, %r8d
  xorl %edi, %r8d
  movl %edx, %edi
  shrl $3, %edi
  xorl %r8d, %edi
  addl 232(%rsp), %ecx
  addl %edi, %ecx
  addl %esi, %ecx
  movl %ecx, 260(%rsp)
  rorxl $17, %eax, %esi
  rorxl $19, %eax, %edi
  xorl %esi, %edi
  shrl $10, %eax
  xorl %edi, %eax
  movl 204(%rsp), %esi
  rorxl $7, %esi, %edi
  rorxl $18, %esi, %r8d
  xorl %edi, %r8d
  movl %esi, %edi
  shrl $3, %edi
  xorl %r8d, %edi
  addl 236(%rsp), %edx
  addl %edi, %edx
  addl %eax, %edx
  movl %edx, 264(%rsp)
  rorxl $17, %ecx, %eax
  rorxl $19, %ecx, %edi
  xorl %eax, %edi
  shrl $10, %ecx
  xorl %edi, %ecx
  movl 208(%rsp), %eax
  rorxl $7, %eax, %edi
  rorxl $18, %eax, %r8d
  xorl %edi, %r8d
  movl %eax, %edi
  shrl $3, %edi
  xorl %r8d, %edi
  addl 240(%rsp), %esi
  addl %edi, %esi
  addl %ecx, %esi
  movl %esi, 268(%rsp)
  rorxl $17, %edx, %ecx
  rorxl $19, %edx, %edi
  xorl %ecx, %edi
  shrl $10, %edx
  xorl %edi, %edx
  movl 212(%rsp), %ecx
  rorxl $7, %ecx, %edi
  rorxl $18, %ecx, %r8d
  xorl %edi, %r8d
  movl %ecx, %edi
  shrl $3, %edi
  xorl %r8d, %edi
  addl 244(%rsp), %eax
  addl %edi, %eax
  addl %edx, %eax
  movl %eax, 272(%rsp)
  rorxl $17, %esi, %edx
  rorxl $19, %esi, %edi
  xorl %edx, %edi
  shrl $10, %esi
  xorl %edi, %esi
  movl 216(%rsp), %edx
  rorxl $7, %edx, %edi
  rorxl $18, %edx, %r8d
  xorl %edi, %r8d
  movl %edx, %edi
  shrl $3, %edi
  xorl %r8d, %edi
  addl 248(%rsp), %ecx
  addl %edi, %ecx
  addl %esi, %ecx
  movl %ecx, 276(%rsp)
  rorxl $17, %eax, %esi
  rorxl $19, %eax, %edi
  xorl %esi, %edi
  shrl $10, %eax
  xorl %edi, %eax
  movl 220(%rsp), %esi
  rorxl $7, %esi, %edi
  rorxl $18, %esi, %r8d
  xorl %edi, %r8d
  movl %esi, %edi
  shrl $3, %edi
  xorl %r8d, %edi
  addl 252(%rsp), %edx
  addl %edi, %edx
  addl %eax, %edx
  movl %edx, 280(%rsp)
  rorxl $17, %ecx, %eax
  rorxl $19, %ecx, %edi
  xorl %eax, %edi
  shrl $10, %ecx
  xorl %edi, %ecx
  movl 224(%rsp), %eax
  rorxl $7, %eax, %edi
  rorxl $18, %eax, %r8d
  xorl %edi, %r8d
  movl %eax, %edi
  shrl $3, %edi
  xorl %r8d, %edi
  addl 256(%rsp), %esi
  addl %edi, %esi
  addl %ecx, %esi
  movl %esi, 284(%rsp)
  rorxl $17, %edx, %ecx
  rorxl $19, %edx, %edi
  xorl %ecx, %edi
  shrl $10, %edx
  xorl %edi, %edx
  movl 228(%rsp), %ecx
  rorxl $7, %ecx, %edi
  rorxl $18, %ecx, %r8d
  xorl %edi, %r8d
  movl %ecx, %edi
  shrl $3, %edi
  xorl %r8d, %edi
  addl 260(%rsp), %eax
  addl %edi, %eax
  addl %edx, %eax
  movl %eax, 288(%rsp)
  rorxl $17, %esi, %edx
  rorxl $19, %esi, %edi
  xorl %edx, %edi
  shrl $10, %esi
  xorl %edi, %esi
  movl 232(%rsp), %edx
  rorxl $7, %edx, %edi
  rorxl $18, %edx, %r8d
  xorl %edi, %r8d
  movl %edx, %edi
  shrl $3, %edi
  xorl %r8d, %edi
  addl 264(%rsp), %ecx
  addl %edi, %ecx
  addl %esi, %ecx
  movl %ecx, 292(%rsp)
  rorxl $17, %eax, %esi
  rorxl $19, %eax, %edi
  xorl %esi, %edi
  shrl $10, %eax
  xorl %edi, %eax
  movl 236(%rsp), %esi
  rorxl $7, %esi, %edi
  rorxl $18, %esi, %r8d
  xorl %edi, %r8d
  movl %esi, %edi
  shrl $3, %edi
  xorl %r8d, %edi
  addl 268(%rsp), %edx
  addl %edi, %edx
  addl %eax, %edx
  movl %edx, 296(%rsp)
  rorxl $17, %ecx, %eax
  rorxl $19, %ecx, %edx
  xorl %eax, %edx
  shrl $10, %ecx
  xorl %edx, %ecx
  movl 240(%rsp), %eax
  rorxl $7, %eax, %edx
  rorxl $18, %eax, %edi
  xorl %edx, %edi
  shrl $3, %eax
  xorl %edi, %eax
  addl 272(%rsp), %esi
  addl %eax, %esi
  addl %ecx, %esi
  movl %esi, 300(%rsp)
  movl $12, %r10d
  movl 32(%rsp), %eax
  movl %eax, %r14d
  movl 40(%rsp), %ebx
  movl 36(%rsp), %eax
  movl 44(%rsp), %ecx
  movl 4(%rsp), %edx
  movl (%rsp), %esi
  movl 28(%rsp), %edi
  movl 316(%rsp), %r11d
  movl %r11d, %r13d
.LBB0_8:
  movl %r11d, %ebp
  movl %edi, %r11d
  movl %ebx, %r9d
  movl %r14d, %r8d
  rorxl $6, %esi, %edi
  rorxl $11, %esi, %ebx
  xorl %edi, %ebx
  rorxl $25, %esi, %edi
  xorl %ebx, %edi
  movl %esi, %ebx
  andl %edx, %ebx
  andnl %ecx, %esi, %r14d
  addl %ebx, %r14d
  addl %edi, %r14d
  movl K-12(%r10), %ebx
  rorxl $2, %r8d, %edi
  rorxl $13, %r8d, %r15d
  xorl %edi, %r15d
  rorxl $22, %r8d, %r12d
  xorl %r15d, %r12d
  movl %ebp, %r15d
  xorl %r9d, %r15d
  andl %r8d, %r15d
  movl %ebp, %edi
  andl %r9d, %edi
  xorl %r15d, %edi
  addl %r12d, %edi
  addl 36(%rsp,%r10), %ebx
  addl %r14d, %ebx
  addl %eax, %edi
  addl %ebx, %edi
  addl %r11d, %eax
  addl %ebx, %eax
  rorxl $6, %eax, %r11d
  rorxl $11, %eax, %r14d
  xorl %r11d, %r14d
  rorxl $25, %eax, %ebx
  xorl %r14d, %ebx
  movl %eax, %r14d
  andl %esi, %r14d
  rorxl $2, %edi, %r11d
  rorxl $13, %edi, %r15d
  xorl %r11d, %r15d
  rorxl $22, %edi, %r12d
  xorl %r15d, %r12d
  movl %r9d, %r15d
  xorl %r8d, %r15d
  andl %edi, %r15d
  movl %r9d, %r11d
  andl %r8d, %r11d
  xorl %r15d, %r11d
  movl K-8(%r10), %r15d
  addl %r12d, %r11d
  addl 40(%rsp,%r10), %r15d
  andnl %edx, %eax, %r12d
  addl %r12d, %r15d
  addl %r14d, %r15d
  addl %ebx, %r15d
  addl %r15d, %r11d
  addl %ecx, %r11d
  addl %ebp, %ecx
  addl %r15d, %ecx
  rorxl $6, %ecx, %ebx
  rorxl $11, %ecx, %ebp
  xorl %ebx, %ebp
  rorxl $25, %ecx, %r14d
  xorl %ebp, %r14d
  andnl %esi, %ecx, %ebp
  rorxl $2, %r11d, %ebx
  rorxl $13, %r11d, %r15d
  xorl %ebx, %r15d
  rorxl $22, %r11d, %ebx
  xorl %r15d, %ebx
  movl %r8d, %r15d
  xorl %edi, %r15d
  andl %r11d, %r15d
  movl %r8d, %r12d
  andl %edi, %r12d
  xorl %r15d, %r12d
  movl K-4(%r10), %r15d
  addl 44(%rsp,%r10), %r15d
  addl %ebp, %r15d
  movl %ecx, %ebp
  andl %eax, %ebp
  addl %ebp, %r15d
  addl %r14d, %r15d
  addl %r15d, %r12d
  addl %edx, %ebx
  addl %r12d, %ebx
  addl %r9d, %edx
  addl %r15d, %edx
  rorxl $6, %edx, %r9d
  rorxl $11, %edx, %ebp
  xorl %r9d, %ebp
  rorxl $25, %edx, %r15d
  xorl %ebp, %r15d
  movl K(%r10), %r9d
  rorxl $2, %ebx, %ebp
  rorxl $13, %ebx, %r12d
  xorl %ebp, %r12d
  rorxl $22, %ebx, %r14d
  xorl %r12d, %r14d
  movl %edi, %ebp
  xorl %r11d, %ebp
  andl %ebx, %ebp
  movl %edi, %r12d
  andl %r11d, %r12d
  xorl %ebp, %r12d
  addl 48(%rsp,%r10), %r9d
  andnl %eax, %edx, %ebp
  addl %ebp, %r9d
  movl %edx, %ebp
  andl %ecx, %ebp
  addl %ebp, %r9d
  addl %r15d, %r9d
  addl %r9d, %r12d
  addl %esi, %r14d
  addl %r12d, %r14d
  addl %r8d, %esi
  addl %r9d, %esi
  addq $16, %r10
  cmpq $268, %r10
  jne .LBB0_8
  addl 32(%rsp), %r14d
  movl 40(%rsp), %r8d
  addl %ebx, %r8d
  movl %r8d, %ebx
  addl %r11d, %r13d
  movl %r13d, %r11d
  addl 28(%rsp), %edi
  movl (%rsp), %r9d
  addl %esi, %r9d
  movl 4(%rsp), %r8d
  addl %edx, %r8d
  movl 44(%rsp), %r10d
  addl %ecx, %r10d
  addl 36(%rsp), %eax
  movq %rdi, %rcx
  shlq $16, %rcx
  movq %r14, %rdx
  shlq $32, %rdx
  orq %rax, %rdx
  xorq %rcx, %rdx
  movq 336(%rsp), %rsi
  addq %rdx, %rsi
  movl %r14d, %ecx
  movl %eax, %edx
  movq 328(%rsp), %rax
  cmpl $131071, %eax
  leal 1(%rax), %eax
  jne .LBB0_7
  movq 320(%rsp), %rcx
  leal 1(%rcx), %eax
  cmpl $7, %ecx
  jne .LBB0_6
  xorl %ebx, %ebx
  movl $.L.str, %edi
  xorl %eax, %eax
  vzeroupper
  callq printf
.LBB0_12:
  movl %ebx, %eax
  addq $344, %rsp
  popq %rbx
  popq %r12
  popq %r13
  popq %r14
  popq %r15
  popq %rbp
  vzeroupper
  retq

.L.str:

K:

