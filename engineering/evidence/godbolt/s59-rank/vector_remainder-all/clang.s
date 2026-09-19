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
.LCPI0_16:
.LCPI0_17:
.LCPI0_18:
.LCPI0_19:
.LCPI0_20:
.LCPI0_21:
.LCPI0_22:
.LCPI0_23:
.LCPI0_24:
.LCPI0_25:
.LCPI0_26:
.LCPI0_27:
.LCPI0_28:
.LCPI0_29:
.LCPI0_30:
.LCPI0_31:
main:
  pushq %rbp
  pushq %r15
  pushq %r14
  pushq %r12
  pushq %rbx
  subq $16, %rsp
  movl $1, %ebx
  cmpl $2, %edi
  jl .LBB0_2
  movq 8(%rsi), %rdi
  xorl %esi, %esi
  movl $10, %edx
  callq strtol@PLT
  movq %rax, %rbx
.LBB0_2:
  vmovaps .LCPI0_0(%rip), %ymm0
  vmovaps %ymm0, a(%rip)
  vmovaps .LCPI0_1(%rip), %ymm0
  vmovaps %ymm0, a+32(%rip)
  vmovaps .LCPI0_2(%rip), %ymm0
  vmovaps %ymm0, a+64(%rip)
  vmovaps .LCPI0_3(%rip), %ymm0
  vmovaps %ymm0, a+96(%rip)
  vmovaps .LCPI0_4(%rip), %ymm0
  vmovaps %ymm0, b(%rip)
  vmovaps .LCPI0_5(%rip), %ymm0
  vmovaps %ymm0, b+32(%rip)
  vmovaps .LCPI0_6(%rip), %ymm0
  vmovaps %ymm0, b+64(%rip)
  vmovaps .LCPI0_7(%rip), %ymm0
  vmovaps %ymm0, b+96(%rip)
  vmovaps .LCPI0_8(%rip), %ymm0
  vmovaps %ymm0, a+128(%rip)
  vmovaps .LCPI0_9(%rip), %ymm0
  vmovaps %ymm0, a+160(%rip)
  vmovaps .LCPI0_10(%rip), %ymm0
  vmovaps %ymm0, a+192(%rip)
  vmovaps .LCPI0_11(%rip), %ymm0
  vmovaps %ymm0, a+224(%rip)
  vmovaps .LCPI0_12(%rip), %ymm0
  vmovaps %ymm0, b+128(%rip)
  vmovaps .LCPI0_13(%rip), %ymm0
  vmovaps %ymm0, b+160(%rip)
  vmovaps .LCPI0_14(%rip), %ymm0
  vmovaps %ymm0, b+192(%rip)
  vmovaps .LCPI0_15(%rip), %ymm0
  vmovaps %ymm0, b+224(%rip)
  vmovaps .LCPI0_16(%rip), %ymm0
  vmovaps %ymm0, a+256(%rip)
  vmovaps .LCPI0_17(%rip), %ymm0
  vmovaps %ymm0, a+288(%rip)
  vmovaps .LCPI0_18(%rip), %ymm0
  vmovaps %ymm0, a+320(%rip)
  vmovaps .LCPI0_19(%rip), %ymm0
  vmovaps %ymm0, a+352(%rip)
  vmovaps .LCPI0_20(%rip), %ymm0
  vmovaps %ymm0, b+256(%rip)
  vmovaps .LCPI0_21(%rip), %ymm0
  vmovaps %ymm0, b+288(%rip)
  vmovaps .LCPI0_22(%rip), %ymm0
  vmovaps %ymm0, b+320(%rip)
  vmovaps .LCPI0_23(%rip), %ymm0
  vmovaps %ymm0, b+352(%rip)
  vmovaps .LCPI0_24(%rip), %ymm0
  vmovaps %ymm0, a+384(%rip)
  vmovaps .LCPI0_25(%rip), %ymm0
  vmovaps %ymm0, a+416(%rip)
  vmovaps .LCPI0_26(%rip), %ymm0
  vmovaps %ymm0, a+448(%rip)
  vmovaps .LCPI0_27(%rip), %ymm0
  vmovaps %ymm0, a+480(%rip)
  vmovaps .LCPI0_28(%rip), %ymm0
  vmovaps %ymm0, b+384(%rip)
  vmovaps .LCPI0_29(%rip), %ymm0
  vmovaps %ymm0, b+416(%rip)
  vmovaps .LCPI0_30(%rip), %ymm0
  vmovaps %ymm0, b+448(%rip)
  vmovapd .LCPI0_31(%rip), %ymm0
  vmovapd %ymm0, b+480(%rip)
  movabsq $4634274385308418048, %rax
  movq %rax, a+512(%rip)
  movabsq $4634344754052595712, %rax
  movq %rax, b+512(%rip)
  vxorpd %xmm0, %xmm0, %xmm0
  testl %ebx, %ebx
  jle .LBB0_7
  xorl %r14d, %r14d
  leaq main.bounds(%rip), %r15
.LBB0_4:
  xorl %r12d, %r12d
.LBB0_5:
  vmovsd %xmm0, 8(%rsp)
  movl (%r12,%r15), %ebp
  movl %ebp, %edi
  vzeroupper
  callq sum_f64
  vaddsd 8(%rsp), %xmm0, %xmm0
  vmovsd %xmm0, 8(%rsp)
  movl %ebp, %edi
  callq dot_f64
  vaddsd 8(%rsp), %xmm0, %xmm0
  addq $4, %r12
  cmpq $72, %r12
  jne .LBB0_5
  incl %r14d
  cmpl %ebx, %r14d
  jne .LBB0_4
.LBB0_7:
  leaq .L.str(%rip), %rdi
  movb $1, %al
  vzeroupper
  callq printf@PLT
  xorl %eax, %eax
  addq $16, %rsp
  popq %rbx
  popq %r12
  popq %r14
  popq %r15
  popq %rbp
  retq

sum_f64:
  testl %edi, %edi
  jle .LBB1_1
  movl %edi, %edx
  movl %edx, %eax
  andl $7, %eax
  cmpl $8, %edi
  jae .LBB1_8
  vxorpd %xmm0, %xmm0, %xmm0
  xorl %ecx, %ecx
  jmp .LBB1_5
.LBB1_1:
  vxorps %xmm0, %xmm0, %xmm0
  retq
.LBB1_8:
  andl $2147483640, %edx
  vxorpd %xmm0, %xmm0, %xmm0
  xorl %ecx, %ecx
  leaq a(%rip), %rsi
.LBB1_9:
  vaddsd (%rsi,%rcx,8), %xmm0, %xmm0
  vaddsd 8(%rsi,%rcx,8), %xmm0, %xmm0
  vaddsd 16(%rsi,%rcx,8), %xmm0, %xmm0
  vaddsd 24(%rsi,%rcx,8), %xmm0, %xmm0
  vaddsd 32(%rsi,%rcx,8), %xmm0, %xmm0
  vaddsd 40(%rsi,%rcx,8), %xmm0, %xmm0
  vaddsd 48(%rsi,%rcx,8), %xmm0, %xmm0
  vaddsd 56(%rsi,%rcx,8), %xmm0, %xmm0
  addq $8, %rcx
  cmpq %rcx, %rdx
  jne .LBB1_9
  testq %rax, %rax
  je .LBB1_7
.LBB1_5:
  shlq $3, %rcx
  xorl %edx, %edx
  leaq a(%rip), %rsi
.LBB1_6:
  leaq (%rcx,%rdx,8), %rdi
  vaddsd (%rsi,%rdi), %xmm0, %xmm0
  incq %rdx
  cmpq %rdx, %rax
  jne .LBB1_6
.LBB1_7:
  retq

dot_f64:
  testl %edi, %edi
  jle .LBB2_1
  movl %edi, %edx
  movl %edx, %eax
  andl $7, %eax
  cmpl $8, %edi
  jae .LBB2_8
  vxorpd %xmm0, %xmm0, %xmm0
  xorl %ecx, %ecx
  jmp .LBB2_5
.LBB2_1:
  vxorps %xmm0, %xmm0, %xmm0
  retq
.LBB2_8:
  andl $2147483640, %edx
  vxorpd %xmm0, %xmm0, %xmm0
  xorl %ecx, %ecx
  leaq a(%rip), %rsi
  leaq b(%rip), %rdi
.LBB2_9:
  vmovsd (%rsi,%rcx,8), %xmm1
  vmovsd 8(%rsi,%rcx,8), %xmm2
  vfmadd231sd (%rdi,%rcx,8), %xmm1, %xmm0
  vfmadd231sd 8(%rdi,%rcx,8), %xmm2, %xmm0
  vmovsd 16(%rsi,%rcx,8), %xmm1
  vfmadd132sd 16(%rdi,%rcx,8), %xmm0, %xmm1
  vmovsd 24(%rsi,%rcx,8), %xmm0
  vfmadd132sd 24(%rdi,%rcx,8), %xmm1, %xmm0
  vmovsd 32(%rsi,%rcx,8), %xmm1
  vfmadd132sd 32(%rdi,%rcx,8), %xmm0, %xmm1
  vmovsd 40(%rsi,%rcx,8), %xmm0
  vfmadd132sd 40(%rdi,%rcx,8), %xmm1, %xmm0
  vmovsd 48(%rsi,%rcx,8), %xmm1
  vfmadd132sd 48(%rdi,%rcx,8), %xmm0, %xmm1
  vmovsd 56(%rsi,%rcx,8), %xmm0
  vfmadd132sd 56(%rdi,%rcx,8), %xmm1, %xmm0
  addq $8, %rcx
  cmpq %rcx, %rdx
  jne .LBB2_9
  testq %rax, %rax
  je .LBB2_7
.LBB2_5:
  shlq $3, %rcx
  shll $3, %eax
  xorl %edx, %edx
  leaq a(%rip), %rsi
  leaq b(%rip), %rdi
.LBB2_6:
  leaq (%rcx,%rdx), %r8
  vmovsd (%rsi,%r8), %xmm1
  vfmadd231sd (%rdi,%r8), %xmm1, %xmm0
  addq $8, %rdx
  cmpq %rdx, %rax
  jne .LBB2_6
.LBB2_7:
  retq

main.bounds:

.L.str:

