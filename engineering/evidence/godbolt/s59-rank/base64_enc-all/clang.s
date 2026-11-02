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
  subq $280, %rsp
  vmovdqa .LCPI0_0(%rip), %ymm0
  vmovdqa %ymm0, main.a(%rip)
  vmovaps .LCPI0_1(%rip), %ymm1
  vmovaps %ymm1, main.a+32(%rip)
  vmovaps .LCPI0_2(%rip), %ymm1
  vmovaps %ymm1, main.a+64(%rip)
  vmovaps .LCPI0_3(%rip), %ymm1
  vmovaps %ymm1, main.a+96(%rip)
  vmovaps .LCPI0_4(%rip), %ymm1
  vmovaps %ymm1, main.a+128(%rip)
  vmovaps .LCPI0_5(%rip), %ymm1
  vmovaps %ymm1, main.a+160(%rip)
  vmovaps .LCPI0_6(%rip), %ymm1
  vmovaps %ymm1, main.a+192(%rip)
  vmovdqa .LCPI0_7(%rip), %ymm1
  vmovdqa %ymm1, main.a+224(%rip)
  vmovdqa %ymm0, main.a+256(%rip)
  movabsq $-5870050417102696277, %rax
  movq %rax, main.a+288(%rip)
  movl $1109260499, main.a+296(%rip)
  leaq main.a+95(%rip), %rdx
  xorl %eax, %eax
  leaq tab(%rip), %rcx
.LBB0_1:
  movq %rax, 272(%rsp)
  movzbl -47(%rdx), %eax
  movzbl -44(%rdx), %esi
  movzbl -41(%rdx), %r8d
  movzbl -38(%rdx), %r10d
  movzbl -35(%rdx), %r9d
  movzbl -32(%rdx), %r13d
  movzbl -29(%rdx), %ebx
  movzbl -26(%rdx), %r11d
  movzbl -23(%rdx), %r12d
  movzbl -20(%rdx), %r14d
  movzbl -17(%rdx), %r15d
  movzbl -14(%rdx), %edi
  movq %rdi, 48(%rsp)
  movl %eax, %edi
  movq %rdi, 40(%rsp)
  movl %esi, %ebp
  movq %rbp, 56(%rsp)
  movl %r8d, %ebp
  movq %rbp, 64(%rsp)
  shrl $2, %eax
  movzbl (%rax,%rcx), %eax
  vmovd %eax, %xmm0
  movl %r10d, %edi
  shrl $2, %esi
  vpinsrb $1, (%rsi,%rcx), %xmm0, %xmm0
  movl %r9d, %esi
  shrl $2, %r8d
  vpinsrb $2, (%r8,%rcx), %xmm0, %xmm0
  movl %r13d, %r8d
  shrl $2, %r10d
  shrl $2, %r9d
  shrl $2, %r13d
  vpinsrb $3, (%r10,%rcx), %xmm0, %xmm0
  vpinsrb $4, (%r9,%rcx), %xmm0, %xmm0
  vpinsrb $5, (%r13,%rcx), %xmm0, %xmm0
  movzbl -11(%rdx), %r13d
  movl %ebx, %r10d
  movl %r11d, %r9d
  shrl $2, %ebx
  vpinsrb $6, (%rbx,%rcx), %xmm0, %xmm0
  movl %r12d, %ebx
  shrl $2, %r11d
  vpinsrb $7, (%r11,%rcx), %xmm0, %xmm0
  movl %r14d, %r11d
  shrl $2, %r12d
  vpinsrb $8, (%r12,%rcx), %xmm0, %xmm0
  movl %r15d, %r12d
  shrl $2, %r14d
  shrl $2, %r15d
  vpinsrb $9, (%r14,%rcx), %xmm0, %xmm0
  vpinsrb $10, (%r15,%rcx), %xmm0, %xmm0
  movzbl -8(%rdx), %ebp
  movq 48(%rsp), %rax
  movl %eax, %r14d
  shrl $2, %eax
  vpinsrb $11, (%rax,%rcx), %xmm0, %xmm0
  movl %r13d, %r15d
  shrl $2, %r13d
  vpinsrb $12, (%r13,%rcx), %xmm0, %xmm0
  movl %ebp, %r13d
  shrl $2, %ebp
  vpinsrb $13, (%rbp,%rcx), %xmm0, %xmm0
  movzbl -5(%rdx), %ebp
  movl %ebp, %eax
  movq %rax, 32(%rsp)
  shrl $2, %ebp
  vpinsrb $14, (%rbp,%rcx), %xmm0, %xmm0
  movzbl -2(%rdx), %ebp
  movl %ebp, %eax
  movq %rax, 24(%rsp)
  shrl $2, %ebp
  vpinsrb $15, (%rbp,%rcx), %xmm0, %xmm0
  movzbl -95(%rdx), %ebp
  movl %ebp, %eax
  movq %rax, 16(%rsp)
  shrl $2, %ebp
  movzbl (%rbp,%rcx), %ebp
  vmovd %ebp, %xmm1
  movzbl -92(%rdx), %ebp
  movl %ebp, %eax
  movq %rax, 8(%rsp)
  shrl $2, %ebp
  vpinsrb $1, (%rbp,%rcx), %xmm1, %xmm1
  movzbl -89(%rdx), %ebp
  movl %ebp, %eax
  movq %rax, 72(%rsp)
  shrl $2, %ebp
  vpinsrb $2, (%rbp,%rcx), %xmm1, %xmm1
  movzbl -86(%rdx), %ebp
  movl %ebp, %eax
  movq %rax, 80(%rsp)
  shrl $2, %ebp
  vpinsrb $3, (%rbp,%rcx), %xmm1, %xmm1
  movzbl -83(%rdx), %ebp
  movl %ebp, %eax
  movq %rax, 88(%rsp)
  shrl $2, %ebp
  vpinsrb $4, (%rbp,%rcx), %xmm1, %xmm1
  movzbl -80(%rdx), %ebp
  movl %ebp, %eax
  movq %rax, 96(%rsp)
  shrl $2, %ebp
  vpinsrb $5, (%rbp,%rcx), %xmm1, %xmm1
  movzbl -77(%rdx), %ebp
  movl %ebp, %eax
  movq %rax, 104(%rsp)
  shrl $2, %ebp
  vpinsrb $6, (%rbp,%rcx), %xmm1, %xmm1
  movzbl -74(%rdx), %ebp
  movl %ebp, %eax
  movq %rax, 112(%rsp)
  shrl $2, %ebp
  vpinsrb $7, (%rbp,%rcx), %xmm1, %xmm1
  movzbl -71(%rdx), %ebp
  movl %ebp, %eax
  movq %rax, 120(%rsp)
  shrl $2, %ebp
  vpinsrb $8, (%rbp,%rcx), %xmm1, %xmm1
  movzbl -68(%rdx), %ebp
  movl %ebp, %eax
  movq %rax, 128(%rsp)
  shrl $2, %ebp
  vpinsrb $9, (%rbp,%rcx), %xmm1, %xmm1
  movzbl -65(%rdx), %ebp
  movl %ebp, %eax
  movq %rax, 136(%rsp)
  shrl $2, %ebp
  vpinsrb $10, (%rbp,%rcx), %xmm1, %xmm1
  movzbl -62(%rdx), %ebp
  movl %ebp, %eax
  movq %rax, 144(%rsp)
  shrl $2, %ebp
  vpinsrb $11, (%rbp,%rcx), %xmm1, %xmm1
  movzbl -59(%rdx), %ebp
  movl %ebp, %eax
  movq %rax, 152(%rsp)
  shrl $2, %ebp
  vpinsrb $12, (%rbp,%rcx), %xmm1, %xmm1
  movzbl -56(%rdx), %ebp
  movl %ebp, %eax
  movq %rax, 160(%rsp)
  shrl $2, %ebp
  vpinsrb $13, (%rbp,%rcx), %xmm1, %xmm1
  movzbl -53(%rdx), %ebp
  movl %ebp, %eax
  movq %rax, 168(%rsp)
  shrl $2, %ebp
  vpinsrb $14, (%rbp,%rcx), %xmm1, %xmm1
  movzbl -50(%rdx), %ebp
  movl %ebp, %eax
  movq %rax, 176(%rsp)
  shrl $2, %ebp
  vpinsrb $15, (%rbp,%rcx), %xmm1, %xmm1
  movzbl -46(%rdx), %eax
  shll $8, %eax
  movq %rax, 48(%rsp)
  movq 40(%rsp), %rbp
  shll $16, %ebp
  orl %eax, %ebp
  shrl $12, %ebp
  andl $63, %ebp
  movzbl (%rbp,%rcx), %ebp
  vmovd %ebp, %xmm2
  movzbl -43(%rdx), %eax
  shll $8, %eax
  movq %rax, 40(%rsp)
  movq 56(%rsp), %rbp
  shll $16, %ebp
  orl %eax, %ebp
  shrl $12, %ebp
  andl $63, %ebp
  vpinsrb $1, (%rbp,%rcx), %xmm2, %xmm2
  movzbl -40(%rdx), %eax
  shll $8, %eax
  movq %rax, 56(%rsp)
  movq 64(%rsp), %rbp
  shll $16, %ebp
  orl %eax, %ebp
  shrl $12, %ebp
  andl $63, %ebp
  vpinsrb $2, (%rbp,%rcx), %xmm2, %xmm2
  movzbl -37(%rdx), %ebp
  shll $8, %ebp
  shll $16, %edi
  orl %ebp, %edi
  shrl $12, %edi
  andl $63, %edi
  vpinsrb $3, (%rdi,%rcx), %xmm2, %xmm2
  movzbl -34(%rdx), %edi
  shll $8, %edi
  shll $16, %esi
  orl %edi, %esi
  shrl $12, %esi
  andl $63, %esi
  vpinsrb $4, (%rsi,%rcx), %xmm2, %xmm2
  movzbl -31(%rdx), %eax
  shll $8, %eax
  movq %rax, 64(%rsp)
  shll $16, %r8d
  orl %eax, %r8d
  shrl $12, %r8d
  andl $63, %r8d
  vpinsrb $5, (%r8,%rcx), %xmm2, %xmm2
  movzbl -28(%rdx), %r8d
  shll $8, %r8d
  shll $16, %r10d
  orl %r8d, %r10d
  shrl $12, %r10d
  andl $63, %r10d
  vpinsrb $6, (%r10,%rcx), %xmm2, %xmm2
  movzbl -25(%rdx), %r10d
  shll $8, %r10d
  shll $16, %r9d
  orl %r10d, %r9d
  shrl $12, %r9d
  andl $63, %r9d
  vpinsrb $7, (%r9,%rcx), %xmm2, %xmm2
  movzbl -22(%rdx), %r9d
  shll $8, %r9d
  shll $16, %ebx
  orl %r9d, %ebx
  shrl $12, %ebx
  andl $63, %ebx
  vpinsrb $8, (%rbx,%rcx), %xmm2, %xmm2
  movzbl -19(%rdx), %ebx
  shll $8, %ebx
  shll $16, %r11d
  orl %ebx, %r11d
  shrl $12, %r11d
  andl $63, %r11d
  vpinsrb $9, (%r11,%rcx), %xmm2, %xmm2
  movzbl -16(%rdx), %r11d
  shll $8, %r11d
  shll $16, %r12d
  orl %r11d, %r12d
  shrl $12, %r12d
  andl $63, %r12d
  vpinsrb $10, (%r12,%rcx), %xmm2, %xmm2
  movzbl -13(%rdx), %r12d
  shll $8, %r12d
  shll $16, %r14d
  orl %r12d, %r14d
  shrl $12, %r14d
  andl $63, %r14d
  vpinsrb $11, (%r14,%rcx), %xmm2, %xmm2
  movzbl -10(%rdx), %r14d
  shll $8, %r14d
  shll $16, %r15d
  orl %r14d, %r15d
  shrl $12, %r15d
  andl $63, %r15d
  vpinsrb $12, (%r15,%rcx), %xmm2, %xmm2
  movzbl -7(%rdx), %r15d
  shll $8, %r15d
  shll $16, %r13d
  orl %r15d, %r13d
  shrl $12, %r13d
  andl $63, %r13d
  vpinsrb $13, (%r13,%rcx), %xmm2, %xmm2
  movzbl -4(%rdx), %r13d
  shll $8, %r13d
  movq 32(%rsp), %rax
  shll $16, %eax
  orl %r13d, %eax
  shrl $12, %eax
  andl $63, %eax
  vpinsrb $14, (%rax,%rcx), %xmm2, %xmm2
  movzbl -1(%rdx), %eax
  shll $8, %eax
  movq %rax, 32(%rsp)
  movq 24(%rsp), %rsi
  shll $16, %esi
  orl %eax, %esi
  shrl $12, %esi
  andl $63, %esi
  vpinsrb $15, (%rsi,%rcx), %xmm2, %xmm2
  movzbl -94(%rdx), %eax
  shll $8, %eax
  movq %rax, 24(%rsp)
  movq 16(%rsp), %rsi
  shll $16, %esi
  orl %eax, %esi
  shrl $12, %esi
  andl $63, %esi
  movzbl (%rsi,%rcx), %eax
  vmovd %eax, %xmm3
  movzbl -91(%rdx), %eax
  shll $8, %eax
  movq %rax, 16(%rsp)
  movq 8(%rsp), %rsi
  shll $16, %esi
  orl %eax, %esi
  shrl $12, %esi
  andl $63, %esi
  vpinsrb $1, (%rsi,%rcx), %xmm3, %xmm3
  movzbl -88(%rdx), %eax
  shll $8, %eax
  movq %rax, 8(%rsp)
  movq 72(%rsp), %rsi
  shll $16, %esi
  orl %eax, %esi
  shrl $12, %esi
  andl $63, %esi
  vpinsrb $2, (%rsi,%rcx), %xmm3, %xmm3
  movzbl -85(%rdx), %esi
  shll $8, %esi
  movq %rsi, 72(%rsp)
  movq 80(%rsp), %rax
  shll $16, %eax
  orl %esi, %eax
  shrl $12, %eax
  andl $63, %eax
  vpinsrb $3, (%rax,%rcx), %xmm3, %xmm3
  movzbl -82(%rdx), %esi
  shll $8, %esi
  movq %rsi, 80(%rsp)
  movq 88(%rsp), %rax
  shll $16, %eax
  orl %esi, %eax
  shrl $12, %eax
  andl $63, %eax
  vpinsrb $4, (%rax,%rcx), %xmm3, %xmm3
  movzbl -79(%rdx), %esi
  shll $8, %esi
  movq %rsi, 88(%rsp)
  movq 96(%rsp), %rax
  shll $16, %eax
  orl %esi, %eax
  shrl $12, %eax
  andl $63, %eax
  vpinsrb $5, (%rax,%rcx), %xmm3, %xmm3
  movzbl -76(%rdx), %esi
  shll $8, %esi
  movq %rsi, 96(%rsp)
  movq 104(%rsp), %rax
  shll $16, %eax
  orl %esi, %eax
  shrl $12, %eax
  andl $63, %eax
  vpinsrb $6, (%rax,%rcx), %xmm3, %xmm3
  movzbl -73(%rdx), %esi
  shll $8, %esi
  movq %rsi, 104(%rsp)
  movq 112(%rsp), %rax
  shll $16, %eax
  orl %esi, %eax
  shrl $12, %eax
  andl $63, %eax
  vpinsrb $7, (%rax,%rcx), %xmm3, %xmm3
  movzbl -70(%rdx), %esi
  shll $8, %esi
  movq %rsi, 112(%rsp)
  movq 120(%rsp), %rax
  shll $16, %eax
  orl %esi, %eax
  shrl $12, %eax
  andl $63, %eax
  vpinsrb $8, (%rax,%rcx), %xmm3, %xmm3
  movzbl -67(%rdx), %esi
  shll $8, %esi
  movq %rsi, 120(%rsp)
  movq 128(%rsp), %rax
  shll $16, %eax
  orl %esi, %eax
  shrl $12, %eax
  andl $63, %eax
  vpinsrb $9, (%rax,%rcx), %xmm3, %xmm3
  movzbl -64(%rdx), %esi
  shll $8, %esi
  movq %rsi, 128(%rsp)
  movq 136(%rsp), %rax
  shll $16, %eax
  orl %esi, %eax
  shrl $12, %eax
  andl $63, %eax
  vpinsrb $10, (%rax,%rcx), %xmm3, %xmm3
  movzbl -61(%rdx), %esi
  shll $8, %esi
  movq %rsi, 136(%rsp)
  movq 144(%rsp), %rax
  shll $16, %eax
  orl %esi, %eax
  shrl $12, %eax
  andl $63, %eax
  vpinsrb $11, (%rax,%rcx), %xmm3, %xmm3
  movzbl -58(%rdx), %esi
  shll $8, %esi
  movq %rsi, 144(%rsp)
  movq 152(%rsp), %rax
  shll $16, %eax
  orl %esi, %eax
  shrl $12, %eax
  andl $63, %eax
  vpinsrb $12, (%rax,%rcx), %xmm3, %xmm3
  movzbl -55(%rdx), %esi
  shll $8, %esi
  movq %rsi, 152(%rsp)
  movq 160(%rsp), %rax
  shll $16, %eax
  orl %esi, %eax
  shrl $12, %eax
  andl $63, %eax
  vpinsrb $13, (%rax,%rcx), %xmm3, %xmm3
  movzbl -52(%rdx), %esi
  shll $8, %esi
  movq %rsi, 160(%rsp)
  movq 168(%rsp), %rax
  shll $16, %eax
  orl %esi, %eax
  shrl $12, %eax
  andl $63, %eax
  vpinsrb $14, (%rax,%rcx), %xmm3, %xmm3
  movzbl -49(%rdx), %esi
  shll $8, %esi
  movq %rsi, 168(%rsp)
  movq 176(%rsp), %rax
  shll $16, %eax
  orl %esi, %eax
  shrl $12, %eax
  andl $63, %eax
  vpinsrb $15, (%rax,%rcx), %xmm3, %xmm3
  movzbl -45(%rdx), %esi
  movq %rsi, 176(%rsp)
  movq 48(%rsp), %rax
  addl %esi, %eax
  shrl $6, %eax
  andl $63, %eax
  movzbl (%rax,%rcx), %eax
  vmovd %eax, %xmm4
  movzbl -42(%rdx), %esi
  movq %rsi, 48(%rsp)
  movq 40(%rsp), %rax
  addl %esi, %eax
  shrl $6, %eax
  andl $63, %eax
  vpinsrb $1, (%rax,%rcx), %xmm4, %xmm4
  movzbl -39(%rdx), %esi
  movq %rsi, 40(%rsp)
  movq 56(%rsp), %rax
  addl %esi, %eax
  shrl $6, %eax
  andl $63, %eax
  vpinsrb $2, (%rax,%rcx), %xmm4, %xmm4
  movzbl -36(%rdx), %eax
  movq %rax, 56(%rsp)
  addl %ebp, %eax
  shrl $6, %eax
  andl $63, %eax
  vpinsrb $3, (%rax,%rcx), %xmm4, %xmm4
  movzbl -33(%rdx), %eax
  movq %rax, 264(%rsp)
  addl %edi, %eax
  shrl $6, %eax
  andl $63, %eax
  vpinsrb $4, (%rax,%rcx), %xmm4, %xmm4
  movzbl -30(%rdx), %esi
  movq %rsi, 256(%rsp)
  movq 64(%rsp), %rax
  addl %esi, %eax
  shrl $6, %eax
  andl $63, %eax
  vpinsrb $5, (%rax,%rcx), %xmm4, %xmm4
  movzbl -27(%rdx), %eax
  movq %rax, 64(%rsp)
  addl %r8d, %eax
  shrl $6, %eax
  andl $63, %eax
  vpinsrb $6, (%rax,%rcx), %xmm4, %xmm4
  movzbl -24(%rdx), %eax
  movq %rax, 248(%rsp)
  addl %r10d, %eax
  shrl $6, %eax
  andl $63, %eax
  vpinsrb $7, (%rax,%rcx), %xmm4, %xmm4
  movzbl -21(%rdx), %eax
  movq %rax, 240(%rsp)
  addl %r9d, %eax
  shrl $6, %eax
  andl $63, %eax
  vpinsrb $8, (%rax,%rcx), %xmm4, %xmm4
  movzbl -18(%rdx), %eax
  movq %rax, 232(%rsp)
  addl %ebx, %eax
  shrl $6, %eax
  andl $63, %eax
  vpinsrb $9, (%rax,%rcx), %xmm4, %xmm4
  movzbl -15(%rdx), %eax
  movq %rax, 224(%rsp)
  addl %r11d, %eax
  shrl $6, %eax
  andl $63, %eax
  vpinsrb $10, (%rax,%rcx), %xmm4, %xmm4
  movzbl -12(%rdx), %eax
  movq %rax, 216(%rsp)
  addl %r12d, %eax
  shrl $6, %eax
  andl $63, %eax
  vpinsrb $11, (%rax,%rcx), %xmm4, %xmm4
  movzbl -9(%rdx), %eax
  movq %rax, 208(%rsp)
  addl %r14d, %eax
  shrl $6, %eax
  andl $63, %eax
  vpinsrb $12, (%rax,%rcx), %xmm4, %xmm4
  movzbl -6(%rdx), %eax
  movq %rax, 200(%rsp)
  addl %r15d, %eax
  shrl $6, %eax
  andl $63, %eax
  vpinsrb $13, (%rax,%rcx), %xmm4, %xmm4
  movzbl -3(%rdx), %eax
  movq %rax, 192(%rsp)
  addl %r13d, %eax
  shrl $6, %eax
  andl $63, %eax
  vpinsrb $14, (%rax,%rcx), %xmm4, %xmm4
  movzbl (%rdx), %esi
  movq %rsi, 184(%rsp)
  movq 32(%rsp), %rax
  addl %esi, %eax
  shrl $6, %eax
  andl $63, %eax
  vpinsrb $15, (%rax,%rcx), %xmm4, %xmm4
  movzbl -93(%rdx), %esi
  movq %rsi, 32(%rsp)
  movq 24(%rsp), %rax
  addl %esi, %eax
  shrl $6, %eax
  andl $63, %eax
  movzbl (%rax,%rcx), %eax
  vmovd %eax, %xmm5
  movzbl -90(%rdx), %esi
  movq %rsi, 24(%rsp)
  movq 16(%rsp), %rax
  addl %esi, %eax
  shrl $6, %eax
  andl $63, %eax
  vpinsrb $1, (%rax,%rcx), %xmm5, %xmm5
  movzbl -87(%rdx), %esi
  movq %rsi, 16(%rsp)
  movq 8(%rsp), %rax
  addl %esi, %eax
  shrl $6, %eax
  andl $63, %eax
  vpinsrb $2, (%rax,%rcx), %xmm5, %xmm5
  movzbl -84(%rdx), %esi
  movq %rsi, 8(%rsp)
  movq 72(%rsp), %rax
  addl %esi, %eax
  shrl $6, %eax
  andl $63, %eax
  vpinsrb $3, (%rax,%rcx), %xmm5, %xmm5
  movzbl -81(%rdx), %r13d
  movq 80(%rsp), %rax
  addl %r13d, %eax
  shrl $6, %eax
  andl $63, %eax
  vpinsrb $4, (%rax,%rcx), %xmm5, %xmm5
  movzbl -78(%rdx), %r12d
  movq 88(%rsp), %rax
  addl %r12d, %eax
  shrl $6, %eax
  andl $63, %eax
  vpinsrb $5, (%rax,%rcx), %xmm5, %xmm5
  movzbl -75(%rdx), %r15d
  movq 96(%rsp), %rax
  addl %r15d, %eax
  shrl $6, %eax
  andl $63, %eax
  vpinsrb $6, (%rax,%rcx), %xmm5, %xmm5
  movzbl -72(%rdx), %r14d
  movq 104(%rsp), %rax
  addl %r14d, %eax
  shrl $6, %eax
  andl $63, %eax
  vpinsrb $7, (%rax,%rcx), %xmm5, %xmm5
  movzbl -69(%rdx), %ebx
  movq 112(%rsp), %rax
  addl %ebx, %eax
  shrl $6, %eax
  andl $63, %eax
  vpinsrb $8, (%rax,%rcx), %xmm5, %xmm5
  movzbl -66(%rdx), %r11d
  movq 120(%rsp), %rax
  addl %r11d, %eax
  shrl $6, %eax
  andl $63, %eax
  vpinsrb $9, (%rax,%rcx), %xmm5, %xmm5
  movzbl -63(%rdx), %r10d
  movq 128(%rsp), %rax
  addl %r10d, %eax
  shrl $6, %eax
  andl $63, %eax
  vpinsrb $10, (%rax,%rcx), %xmm5, %xmm5
  movzbl -60(%rdx), %r9d
  movq 136(%rsp), %rax
  addl %r9d, %eax
  shrl $6, %eax
  andl $63, %eax
  vpinsrb $11, (%rax,%rcx), %xmm5, %xmm5
  movzbl -57(%rdx), %r8d
  movq 144(%rsp), %rax
  addl %r8d, %eax
  shrl $6, %eax
  andl $63, %eax
  vpinsrb $12, (%rax,%rcx), %xmm5, %xmm5
  movzbl -54(%rdx), %edi
  movq 152(%rsp), %rax
  addl %edi, %eax
  shrl $6, %eax
  andl $63, %eax
  vpinsrb $13, (%rax,%rcx), %xmm5, %xmm5
  movzbl -51(%rdx), %esi
  movq 160(%rsp), %rax
  addl %esi, %eax
  shrl $6, %eax
  andl $63, %eax
  vpinsrb $14, (%rax,%rcx), %xmm5, %xmm5
  movzbl -48(%rdx), %eax
  movq 168(%rsp), %rbp
  addl %eax, %ebp
  shrl $6, %ebp
  andl $63, %ebp
  vpinsrb $15, (%rbp,%rcx), %xmm5, %xmm5
  movq 176(%rsp), %rbp
  andl $63, %ebp
  movzbl (%rbp,%rcx), %ebp
  vmovd %ebp, %xmm6
  movq 48(%rsp), %rbp
  andl $63, %ebp
  vpinsrb $1, (%rbp,%rcx), %xmm6, %xmm6
  movq 40(%rsp), %rbp
  andl $63, %ebp
  vpinsrb $2, (%rbp,%rcx), %xmm6, %xmm6
  movq 56(%rsp), %rbp
  andl $63, %ebp
  vpinsrb $3, (%rbp,%rcx), %xmm6, %xmm6
  movq 264(%rsp), %rbp
  andl $63, %ebp
  vpinsrb $4, (%rbp,%rcx), %xmm6, %xmm6
  movq 256(%rsp), %rbp
  andl $63, %ebp
  vpinsrb $5, (%rbp,%rcx), %xmm6, %xmm6
  movq 64(%rsp), %rbp
  andl $63, %ebp
  vpinsrb $6, (%rbp,%rcx), %xmm6, %xmm6
  movq 248(%rsp), %rbp
  andl $63, %ebp
  vpinsrb $7, (%rbp,%rcx), %xmm6, %xmm6
  movq 240(%rsp), %rbp
  andl $63, %ebp
  vpinsrb $8, (%rbp,%rcx), %xmm6, %xmm6
  movq 232(%rsp), %rbp
  andl $63, %ebp
  vpinsrb $9, (%rbp,%rcx), %xmm6, %xmm6
  movq 224(%rsp), %rbp
  andl $63, %ebp
  vpinsrb $10, (%rbp,%rcx), %xmm6, %xmm6
  movq 216(%rsp), %rbp
  andl $63, %ebp
  vpinsrb $11, (%rbp,%rcx), %xmm6, %xmm6
  movq 208(%rsp), %rbp
  andl $63, %ebp
  vpinsrb $12, (%rbp,%rcx), %xmm6, %xmm6
  movq 200(%rsp), %rbp
  andl $63, %ebp
  vpinsrb $13, (%rbp,%rcx), %xmm6, %xmm6
  movq 192(%rsp), %rbp
  andl $63, %ebp
  vpinsrb $14, (%rbp,%rcx), %xmm6, %xmm6
  movq 184(%rsp), %rbp
  andl $63, %ebp
  vpinsrb $15, (%rbp,%rcx), %xmm6, %xmm6
  movq 32(%rsp), %rbp
  andl $63, %ebp
  movzbl (%rbp,%rcx), %ebp
  vmovd %ebp, %xmm7
  movq 24(%rsp), %rbp
  andl $63, %ebp
  vpinsrb $1, (%rbp,%rcx), %xmm7, %xmm7
  movq 16(%rsp), %rbp
  andl $63, %ebp
  vpinsrb $2, (%rbp,%rcx), %xmm7, %xmm7
  movq 8(%rsp), %rbp
  andl $63, %ebp
  vpinsrb $3, (%rbp,%rcx), %xmm7, %xmm7
  andl $63, %r13d
  vpinsrb $4, (%r13,%rcx), %xmm7, %xmm7
  andl $63, %r12d
  vpinsrb $5, (%r12,%rcx), %xmm7, %xmm7
  andl $63, %r15d
  vpinsrb $6, (%r15,%rcx), %xmm7, %xmm7
  andl $63, %r14d
  vpinsrb $7, (%r14,%rcx), %xmm7, %xmm7
  andl $63, %ebx
  vpinsrb $8, (%rbx,%rcx), %xmm7, %xmm7
  andl $63, %r11d
  vpinsrb $9, (%r11,%rcx), %xmm7, %xmm7
  andl $63, %r10d
  vpinsrb $10, (%r10,%rcx), %xmm7, %xmm7
  andl $63, %r9d
  vpinsrb $11, (%r9,%rcx), %xmm7, %xmm7
  andl $63, %r8d
  vpinsrb $12, (%r8,%rcx), %xmm7, %xmm7
  leaq main.d(%rip), %r8
  andl $63, %edi
  vpinsrb $13, (%rdi,%rcx), %xmm7, %xmm7
  andl $63, %esi
  vpinsrb $14, (%rsi,%rcx), %xmm7, %xmm7
  andl $63, %eax
  vpinsrb $15, (%rax,%rcx), %xmm7, %xmm7
  movq 272(%rsp), %rax
  vinserti128 $1, %xmm0, %ymm1, %ymm0
  vinserti128 $1, %xmm2, %ymm3, %ymm1
  vinserti128 $1, %xmm4, %ymm5, %ymm2
  vinserti128 $1, %xmm6, %ymm7, %ymm3
  vpunpcklbw %ymm1, %ymm0, %ymm4
  vpunpckhbw %ymm1, %ymm0, %ymm0
  vpunpcklbw %ymm3, %ymm2, %ymm1
  vpunpckhbw %ymm3, %ymm2, %ymm2
  vpunpcklwd %ymm1, %ymm4, %ymm3
  vpunpckhwd %ymm1, %ymm4, %ymm1
  vpunpcklwd %ymm2, %ymm0, %ymm4
  vpunpckhwd %ymm2, %ymm0, %ymm0
  vinserti128 $1, %xmm1, %ymm3, %ymm2
  vinserti128 $1, %xmm0, %ymm4, %ymm5
  vperm2i128 $49, %ymm1, %ymm3, %ymm1
  vperm2i128 $49, %ymm0, %ymm4, %ymm0
  vmovdqu %ymm0, 96(%rax,%r8)
  vmovdqu %ymm1, 64(%rax,%r8)
  vmovdqu %ymm5, 32(%rax,%r8)
  vmovdqu %ymm2, (%rax,%r8)
  subq $-128, %rax
  addq $96, %rdx
  cmpq $384, %rax
  jne .LBB0_1
  movzbl main.a+289(%rip), %edx
  shll $8, %edx
  movzbl main.a+290(%rip), %eax
  movzbl main.a+288(%rip), %esi
  movl %esi, %edi
  shll $16, %edi
  orl %edx, %edi
  orl %eax, %edx
  shrl $2, %esi
  movzbl (%rsi,%rcx), %esi
  movb %sil, main.d+384(%rip)
  shrl $12, %edi
  andl $63, %edi
  movzbl (%rdi,%rcx), %esi
  movb %sil, main.d+385(%rip)
  shrl $6, %edx
  andl $63, %edx
  movzbl (%rdx,%rcx), %edx
  movb %dl, main.d+386(%rip)
  andl $63, %eax
  movzbl (%rax,%rcx), %eax
  movb %al, main.d+387(%rip)
  movzbl main.a+292(%rip), %edx
  shll $8, %edx
  movzbl main.a+293(%rip), %eax
  movzbl main.a+291(%rip), %esi
  movl %esi, %edi
  shll $16, %edi
  orl %edx, %edi
  orl %eax, %edx
  shrl $2, %esi
  movzbl (%rsi,%rcx), %esi
  movb %sil, main.d+388(%rip)
  shrl $12, %edi
  andl $63, %edi
  movzbl (%rdi,%rcx), %esi
  movb %sil, main.d+389(%rip)
  shrl $6, %edx
  andl $63, %edx
  movzbl (%rdx,%rcx), %edx
  movb %dl, main.d+390(%rip)
  andl $63, %eax
  movzbl (%rax,%rcx), %eax
  movb %al, main.d+391(%rip)
  movzbl main.a+295(%rip), %edx
  shll $8, %edx
  movzbl main.a+296(%rip), %eax
  movzbl main.a+294(%rip), %esi
  movl %esi, %edi
  shll $16, %edi
  orl %edx, %edi
  orl %eax, %edx
  shrl $2, %esi
  movzbl (%rsi,%rcx), %esi
  movb %sil, main.d+392(%rip)
  shrl $12, %edi
  andl $63, %edi
  movzbl (%rdi,%rcx), %esi
  movb %sil, main.d+393(%rip)
  shrl $6, %edx
  andl $63, %edx
  movzbl (%rdx,%rcx), %edx
  movb %dl, main.d+394(%rip)
  andl $63, %eax
  movzbl (%rax,%rcx), %eax
  movb %al, main.d+395(%rip)
  movzbl main.a+298(%rip), %edx
  shll $8, %edx
  movzbl main.a+299(%rip), %eax
  movzbl main.a+297(%rip), %esi
  movl %esi, %edi
  shll $16, %edi
  orl %edx, %edi
  orl %eax, %edx
  shrl $2, %esi
  movzbl (%rsi,%rcx), %esi
  movb %sil, main.d+396(%rip)
  shrl $12, %edi
  andl $63, %edi
  movzbl (%rdi,%rcx), %esi
  movb %sil, main.d+397(%rip)
  shrl $6, %edx
  andl $63, %edx
  movzbl (%rdx,%rcx), %edx
  movb %dl, main.d+398(%rip)
  andl $63, %eax
  movzbl (%rax,%rcx), %eax
  movb %al, main.d+399(%rip)
  movl $400, %esi
  xorl %eax, %eax
.LBB0_3:
  movq %rsi, %rcx
  shlq $5, %rcx
  addq %rsi, %rcx
  movzbl (%rax,%r8), %edx
  addq %rcx, %rdx
  movzbl 1(%rax,%r8), %ecx
  addq %rdx, %rcx
  shlq $5, %rdx
  addq %rdx, %rcx
  movzbl 2(%rax,%r8), %edx
  addq %rcx, %rdx
  shlq $5, %rcx
  addq %rcx, %rdx
  movzbl 3(%rax,%r8), %ecx
  addq %rdx, %rcx
  shlq $5, %rdx
  addq %rdx, %rcx
  movzbl 4(%rax,%r8), %edx
  addq %rcx, %rdx
  shlq $5, %rcx
  addq %rcx, %rdx
  movzbl 5(%rax,%r8), %ecx
  addq %rdx, %rcx
  shlq $5, %rdx
  addq %rdx, %rcx
  movzbl 6(%rax,%r8), %edx
  addq %rcx, %rdx
  shlq $5, %rcx
  addq %rcx, %rdx
  movzbl 7(%rax,%r8), %ecx
  addq %rdx, %rcx
  shlq $5, %rdx
  addq %rdx, %rcx
  movzbl 8(%rax,%r8), %edx
  addq %rcx, %rdx
  shlq $5, %rcx
  addq %rcx, %rdx
  movzbl 9(%rax,%r8), %esi
  addq %rdx, %rsi
  shlq $5, %rdx
  addq %rdx, %rsi
  addq $10, %rax
  cmpq $400, %rax
  jne .LBB0_3
  leaq .L.str(%rip), %rdi
  xorl %eax, %eax
  vzeroupper
  callq printf@PLT
  xorl %eax, %eax
  addq $280, %rsp
  popq %rbx
  popq %r12
  popq %r13
  popq %r14
  popq %r15
  popq %rbp
  retq

.L.str:

tab:

