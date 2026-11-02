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
  subq $104, %rsp
  vmovdqu .L__const.check_known_vector.state(%rip), %ymm0
  vmovdqu %ymm0, (%rsp)
  vpxor %xmm0, %xmm0, %xmm0
  vmovdqu %ymm0, 32(%rsp)
  vmovdqu %ymm0, 64(%rsp)
  movl $1633837952, 32(%rsp)
  movl $24, 92(%rsp)
  movq %rsp, %rdi
  leaq 32(%rsp), %rsi
  vzeroupper
  callq sha256_transform
  cmpl $-1166534977, (%rsp)
  movl $2, %ebp
  jne .LBB0_8
  cmpl $-1895706646, 4(%rsp)
  jne .LBB0_8
  cmpl $-234875475, 28(%rsp)
  jne .LBB0_8
  xorl %r12d, %r12d
  vmovdqa .LCPI0_2(%rip), %ymm8
  movq %rsp, %r14
  leaq 32(%rsp), %r15
  xorl %ebx, %ebx
.LBB0_4:
  movl %r12d, %ecx
  xorl $1779033703, %ecx
  movl %ecx, (%rsp)
  vmovaps .LCPI0_0(%rip), %xmm0
  vmovups %xmm0, 4(%rsp)
  movabsq $2270897968188319884, %rax
  movq %rax, 20(%rsp)
  movl $1541459225, 28(%rsp)
  movl $-1521486534, %eax
  movl $1541459225, %edx
  movl $131072, %ebp
  xorl %r13d, %r13d
.LBB0_5:
  vmovsd 4(%rsp), %xmm0
  vmovd %r13d, %xmm1
  vpbroadcastd %xmm1, %ymm1
  vpaddd .LCPI0_1(%rip), %ymm1, %ymm2
  vmovsd 16(%rsp), %xmm3
  vbroadcastsd %xmm3, %ymm3
  vshufps $65, %xmm3, %xmm0, %xmm4
  vpermpd $208, %ymm4, %ymm4
  vmovd %ecx, %xmm5
  vblendps $1, %xmm5, %xmm4, %xmm6
  vpinsrd $3, %eax, %xmm6, %xmm6
  vblendps $15, %ymm6, %ymm4, %ymm4
  vbroadcastss 24(%rsp), %ymm6
  vblendps $64, %ymm6, %ymm4, %ymm4
  vmovd %edx, %xmm7
  vpbroadcastd %xmm7, %ymm7
  vblendps $128, %ymm7, %ymm4, %ymm4
  vxorps %ymm2, %ymm4, %ymm2
  vmovups %ymm2, 32(%rsp)
  vpaddd %ymm1, %ymm8, %ymm1
  vshufps $80, %xmm0, %xmm0, %xmm0
  vblendps $1, %xmm5, %xmm0, %xmm0
  vpinsrd $3, %eax, %xmm0, %xmm0
  vpblendd $64, %ymm6, %ymm0, %ymm0
  vpblendd $128, %ymm7, %ymm0, %ymm0
  vpblendd $48, %ymm3, %ymm0, %ymm0
  vpxor %ymm1, %ymm0, %ymm0
  vmovdqu %ymm0, 64(%rsp)
  movq %r14, %rdi
  movq %r15, %rsi
  vzeroupper
  callq sha256_transform
  vmovdqa .LCPI0_2(%rip), %ymm8
  movl (%rsp), %ecx
  movl 12(%rsp), %eax
  movq %rcx, %rsi
  shlq $32, %rsi
  movl 28(%rsp), %edx
  orq %rdx, %rsi
  movq %rax, %rdi
  shlq $16, %rdi
  xorq %rsi, %rdi
  addq %rdi, %rbx
  addl $1664525, %r13d
  decl %ebp
  jne .LBB0_5
  incl %r12d
  cmpl $8, %r12d
  jne .LBB0_4
  leaq .L.str(%rip), %rdi
  xorl %ebp, %ebp
  movq %rbx, %rsi
  xorl %eax, %eax
  vzeroupper
  callq printf@PLT
.LBB0_8:
  movl %ebp, %eax
  addq $104, %rsp
  popq %rbx
  popq %r12
  popq %r13
  popq %r14
  popq %r15
  popq %rbp
  retq

sha256_transform:
  pushq %rbp
  pushq %r15
  pushq %r14
  pushq %r13
  pushq %r12
  pushq %rbx
  subq $152, %rsp
  vmovups (%rsi), %ymm0
  vmovups 32(%rsi), %ymm1
  vmovups %ymm1, -80(%rsp)
  vmovups %ymm0, -112(%rsp)
  xorl %eax, %eax
.LBB1_1:
  movl -56(%rsp,%rax,4), %ecx
  rorxl $17, %ecx, %edx
  rorxl $19, %ecx, %esi
  xorl %edx, %esi
  shrl $10, %ecx
  xorl %esi, %ecx
  movl -108(%rsp,%rax,4), %esi
  movl -104(%rsp,%rax,4), %edx
  rorxl $7, %esi, %r9d
  rorxl $18, %esi, %r8d
  xorl %r9d, %r8d
  rorxl $7, %edx, %r9d
  rorxl $18, %edx, %r10d
  xorl %r9d, %r10d
  addl -76(%rsp,%rax,4), %ecx
  shrl $3, %edx
  xorl %r10d, %edx
  addl %esi, %edx
  shrl $3, %esi
  xorl %r8d, %esi
  addl -112(%rsp,%rax,4), %ecx
  addl %esi, %ecx
  movl %ecx, -48(%rsp,%rax,4)
  movl -52(%rsp,%rax,4), %ecx
  rorxl $17, %ecx, %esi
  rorxl $19, %ecx, %r8d
  xorl %esi, %r8d
  shrl $10, %ecx
  xorl %r8d, %ecx
  addl -72(%rsp,%rax,4), %ecx
  addl %ecx, %edx
  movl %edx, -44(%rsp,%rax,4)
  addq $2, %rax
  cmpq $48, %rax
  jne .LBB1_1
  vmovdqu (%rdi), %xmm0
  movl (%rdi), %r13d
  movl 4(%rdi), %r8d
  movl 8(%rdi), %r9d
  movl 12(%rdi), %ecx
  movl 16(%rdi), %r12d
  movl 20(%rdi), %ebp
  movl 24(%rdi), %edx
  movl 28(%rdi), %r10d
  xorl %r11d, %r11d
  leaq K(%rip), %r14
  movl %r10d, -116(%rsp)
  movl %edx, -120(%rsp)
  movl %ebp, -124(%rsp)
  movl %r12d, -128(%rsp)
.LBB1_3:
  movl %r9d, %r15d
  movl %r12d, %esi
  movl %ebp, %ebx
  movl %edx, %eax
  movl %r8d, %r9d
  movl %r13d, %r8d
  rorxl $6, %r12d, %edx
  rorxl $11, %r12d, %ebp
  xorl %edx, %ebp
  rorxl $25, %r12d, %r12d
  xorl %ebp, %r12d
  movl %ebx, %ebp
  andl %esi, %ebp
  andnl %eax, %esi, %edx
  orl %ebp, %edx
  addl %r10d, %edx
  addl %r12d, %edx
  addl (%r11,%r14), %edx
  addl -112(%rsp,%r11), %edx
  rorxl $2, %r13d, %r10d
  rorxl $13, %r13d, %ebp
  xorl %r10d, %ebp
  rorxl $22, %r13d, %r10d
  xorl %ebp, %r10d
  movl %r9d, %ebp
  xorl %r15d, %ebp
  andl %r13d, %ebp
  movl %r9d, %r13d
  andl %r15d, %r13d
  xorl %ebp, %r13d
  addl %r10d, %r13d
  movl %ecx, %r12d
  addl %edx, %r12d
  addl %edx, %r13d
  addq $4, %r11
  movl %eax, %r10d
  movl %ebx, %edx
  movl %esi, %ebp
  movl %r15d, %ecx
  cmpq $256, %r11
  jne .LBB1_3
  vmovd %r13d, %xmm1
  vpinsrd $1, %r8d, %xmm1, %xmm1
  vpinsrd $2, %r9d, %xmm1, %xmm1
  vpinsrd $3, %r15d, %xmm1, %xmm1
  vpaddd %xmm0, %xmm1, %xmm0
  vmovdqu %xmm0, (%rdi)
  addl -128(%rsp), %r12d
  movl %r12d, 16(%rdi)
  addl -124(%rsp), %esi
  movl %esi, 20(%rdi)
  addl -120(%rsp), %ebx
  movl %ebx, 24(%rdi)
  addl -116(%rsp), %eax
  movl %eax, 28(%rdi)
  addq $152, %rsp
  popq %rbx
  popq %r12
  popq %r13
  popq %r14
  popq %r15
  popq %rbp
  vzeroupper
  retq

.L.str:

.L__const.check_known_vector.state:

K:
