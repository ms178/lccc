.LCPI0_0:
.LCPI0_1:
.LCPI0_2:
main:
  pushq %rbp
  pushq %r15
  pushq %r14
  pushq %r13
  pushq %r12
  pushq %rbx
  subq $296, %rsp
  vpxor %xmm0, %xmm0, %xmm0
  vmovdqu %ymm0, 60(%rsp)
  vmovdqu %ymm0, 36(%rsp)
  movl $1633837952, 32(%rsp)
  movl $24, 92(%rsp)
  xorl %eax, %eax
.LBB0_1:
  movl 88(%rsp,%rax,4), %ecx
  rorxl $17, %ecx, %edx
  rorxl $19, %ecx, %esi
  xorl %edx, %esi
  shrl $10, %ecx
  xorl %esi, %ecx
  movl 36(%rsp,%rax,4), %esi
  movl 40(%rsp,%rax,4), %edx
  rorxl $7, %esi, %r8d
  rorxl $18, %esi, %edi
  xorl %r8d, %edi
  rorxl $7, %edx, %r8d
  rorxl $18, %edx, %r9d
  xorl %r8d, %r9d
  addl 68(%rsp,%rax,4), %ecx
  shrl $3, %edx
  xorl %r9d, %edx
  addl %esi, %edx
  shrl $3, %esi
  xorl %edi, %esi
  addl 32(%rsp,%rax,4), %ecx
  addl %esi, %ecx
  movl %ecx, 96(%rsp,%rax,4)
  movl 92(%rsp,%rax,4), %ecx
  rorxl $17, %ecx, %esi
  rorxl $19, %ecx, %edi
  xorl %esi, %edi
  shrl $10, %ecx
  xorl %edi, %ecx
  addl 72(%rsp,%rax,4), %ecx
  addl %ecx, %edx
  movl %edx, 100(%rsp,%rax,4)
  addq $2, %rax
  cmpq $48, %rax
  jne .LBB0_1
  movl $1013904242, %r9d
  movl $-1521486534, %ecx
  movl $1359893119, %edx
  movl $-1694144372, %edi
  movl $528734635, %r15d
  movl $1541459225, %r14d
  movl $-1150833019, %esi
  movl $1779033703, %ebp
  xorl %r8d, %r8d
  leaq K(%rip), %rax
.LBB0_3:
  movl %r15d, %r10d
  movl %esi, %r11d
  movl %ebp, %esi
  rorxl $6, %edx, %ebx
  rorxl $11, %edx, %ebp
  xorl %ebx, %ebp
  rorxl $25, %edx, %r15d
  xorl %ebp, %r15d
  movl %edx, %ebp
  andl %edi, %ebp
  andnl %r10d, %edx, %ebx
  orl %ebp, %ebx
  addl %r14d, %ebx
  addl %r15d, %ebx
  addl (%r8,%rax), %ebx
  addl 32(%rsp,%r8), %ebx
  rorxl $2, %esi, %ebp
  rorxl $13, %esi, %r15d
  xorl %ebp, %r15d
  rorxl $22, %esi, %r14d
  xorl %r15d, %r14d
  movl %r9d, %ebp
  addl %ebx, %ecx
  movl %edi, %r15d
  movl %edx, %edi
  movl %ecx, %edx
  movl %r9d, %ecx
  xorl %r11d, %r9d
  andl %esi, %r9d
  andl %r11d, %ebp
  xorl %r9d, %ebp
  addl %r14d, %ebp
  addl %ebx, %ebp
  addq $4, %r8
  movl %r10d, %r14d
  movl %r11d, %r9d
  cmpq $256, %r8
  jne .LBB0_3
  movl $2, %ebx
  cmpl $-744873627, %esi
  jne .LBB0_16
  cmpl $1349398616, %ebp
  jne .LBB0_16
  cmpl $-1776334700, %r10d
  jne .LBB0_16
  xorl %ecx, %ecx
  vmovdqa .LCPI0_0(%rip), %ymm0
  vmovdqa .LCPI0_1(%rip), %ymm1
  vmovdqa .LCPI0_2(%rip), %ymm2
  xorl %esi, %esi
.LBB0_8:
  movq %rcx, 16(%rsp)
  movl %ecx, %edi
  xorl $1779033703, %edi
  vmovd %edi, %xmm3
  vpblendd $254, %ymm0, %ymm3, %ymm3
  movl $1541459225, %r9d
  movl $-1521486534, %r8d
  xorl %ecx, %ecx
.LBB0_9:
  movq %rsi, 24(%rsp)
  movl %ecx, 4(%rsp)
  imull $1664525, %ecx, %ecx
  vmovd %ecx, %xmm4
  vpbroadcastd %xmm4, %ymm4
  vpaddd %ymm1, %ymm4, %ymm5
  vpxor %ymm5, %ymm3, %ymm5
  vmovdqu %ymm5, 32(%rsp)
  movq %rdi, 8(%rsp)
  vmovd %edi, %xmm5
  vpblendd $1, %xmm5, %xmm3, %xmm5
  vpinsrd $3, %r8d, %xmm5, %xmm5
  vpaddd %ymm2, %ymm4, %ymm4
  vpblendd $15, %ymm5, %ymm3, %ymm5
  vmovd %r9d, %xmm6
  vpbroadcastd %xmm6, %ymm6
  vpblendd $128, %ymm6, %ymm5, %ymm5
  vpxor %ymm4, %ymm5, %ymm4
  vmovdqu %ymm4, 64(%rsp)
  vpextrd $1, %xmm3, %r10d
  vpextrd $2, %xmm3, %esi
  vextracti128 $1, %ymm3, %xmm4
  vpextrd $1, %xmm4, %ecx
  vmovd %xmm4, %edi
  vpextrd $2, %xmm4, %ebp
  xorl %r15d, %r15d
.LBB0_10:
  movl 88(%rsp,%r15,4), %r11d
  rorxl $17, %r11d, %ebx
  rorxl $19, %r11d, %r14d
  xorl %ebx, %r14d
  shrl $10, %r11d
  xorl %r14d, %r11d
  movl 36(%rsp,%r15,4), %r14d
  movl 40(%rsp,%r15,4), %ebx
  rorxl $7, %r14d, %r13d
  rorxl $18, %r14d, %r12d
  xorl %r13d, %r12d
  rorxl $7, %ebx, %r13d
  rorxl $18, %ebx, %edx
  xorl %r13d, %edx
  addl 68(%rsp,%r15,4), %r11d
  shrl $3, %ebx
  xorl %edx, %ebx
  addl %r14d, %ebx
  shrl $3, %r14d
  xorl %r12d, %r14d
  addl 32(%rsp,%r15,4), %r11d
  addl %r14d, %r11d
  movl %r11d, 96(%rsp,%r15,4)
  movl 92(%rsp,%r15,4), %edx
  rorxl $17, %edx, %r11d
  rorxl $19, %edx, %r14d
  xorl %r11d, %r14d
  shrl $10, %edx
  xorl %r14d, %edx
  addl 72(%rsp,%r15,4), %edx
  addl %edx, %ebx
  movl %ebx, 100(%rsp,%r15,4)
  addq $2, %r15
  cmpq $48, %r15
  jne .LBB0_10
  xorl %r15d, %r15d
  movq 8(%rsp), %rdx
  movl %edx, %r12d
.LBB0_12:
  movl %esi, %r11d
  movl %edi, %ebx
  movl %ecx, %r14d
  movl %ebp, %r13d
  movl %r10d, %esi
  movl %r12d, %r10d
  rorxl $6, %edi, %ecx
  rorxl $11, %edi, %edx
  xorl %ecx, %edx
  rorxl $25, %edi, %edi
  xorl %edx, %edi
  movl %ebx, %edx
  andl %r14d, %edx
  andnl %ebp, %ebx, %ecx
  orl %edx, %ecx
  addl %r9d, %ecx
  addl %edi, %ecx
  addl (%r15,%rax), %ecx
  addl 32(%rsp,%r15), %ecx
  rorxl $2, %r12d, %edx
  rorxl $13, %r12d, %edi
  xorl %edx, %edi
  rorxl $22, %r12d, %edx
  xorl %edi, %edx
  movl %r11d, %edi
  xorl %esi, %edi
  andl %r12d, %edi
  movl %r11d, %r12d
  andl %esi, %r12d
  xorl %edi, %r12d
  addl %edx, %r12d
  movl %r8d, %edi
  addl %ecx, %edi
  addl %ecx, %r12d
  addq $4, %r15
  movl %ebp, %r9d
  movl %r14d, %ebp
  movl %ebx, %ecx
  movl %r11d, %r8d
  cmpq $256, %r15
  jne .LBB0_12
  vmovd %edi, %xmm4
  vpinsrd $1, %ebx, %xmm4, %xmm4
  vpinsrd $2, %r14d, %xmm4, %xmm4
  vpinsrd $3, %r13d, %xmm4, %xmm4
  vmovd %r12d, %xmm5
  vpinsrd $1, %r10d, %xmm5, %xmm5
  vpinsrd $2, %esi, %xmm5, %xmm5
  vpinsrd $3, %r11d, %xmm5, %xmm5
  vinserti128 $1, %xmm4, %ymm5, %ymm4
  vpaddd %ymm3, %ymm4, %ymm3
  vextracti128 $1, %ymm3, %xmm4
  vpextrd $3, %xmm4, %r9d
  vpextrd $3, %xmm3, %r8d
  movq 8(%rsp), %rdi
  addl %r12d, %edi
  movq %rdi, %rcx
  shlq $32, %rcx
  addq %r9, %rcx
  movq %r8, %rdx
  shlq $16, %rdx
  xorq %rcx, %rdx
  movq 24(%rsp), %rsi
  addq %rdx, %rsi
  movl 4(%rsp), %ecx
  incl %ecx
  cmpl $131072, %ecx
  jne .LBB0_9
  movq 16(%rsp), %rcx
  incl %ecx
  cmpl $8, %ecx
  jne .LBB0_8
  leaq .L.str(%rip), %rdi
  xorl %ebx, %ebx
  xorl %eax, %eax
  vzeroupper
  callq printf@PLT
.LBB0_16:
  movl %ebx, %eax
  addq $296, %rsp
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

