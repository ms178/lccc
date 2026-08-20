.LCPI0_0:
  .long 1
  .long 2
  .long 3
  .long 4
.LCPI0_1:
  .long 2
  .long 3
  .long 4
  .long 5
main:
  pushq %rbx
  subq $288, %rsp
  vstmxcsr 4(%rsp)
  orl $32832, 4(%rsp)
  vldmxcsr 4(%rsp)
  movl $1, %ebx
  cmpl $2, %edi
  jl .LBB0_2
  movq 8(%rsi), %rdi
  xorl %esi, %esi
  movl $10, %edx
  callq strtol
  movq %rax, %rbx
.LBB0_2:
  xorl %eax, %eax
  vmovdqu .LCPI0_0(%rip), %xmm0
  vmovdqu .LCPI0_1(%rip), %xmm1
.LBB0_3:
  vmovd %eax, %xmm2
  vpbroadcastd %xmm2, %xmm2
  vpor %xmm0, %xmm2, %xmm3
  vcvtdq2pd %xmm3, %ymm3
  vmovups %ymm3, a(,%rax,8)
  vpor %xmm1, %xmm2, %xmm2
  vcvtdq2pd %xmm2, %ymm2
  vmovups %ymm2, b(,%rax,8)
  leaq 4(%rax), %rcx
  vmovd %ecx, %xmm2
  vpbroadcastd %xmm2, %xmm2
  vpaddd %xmm0, %xmm2, %xmm3
  vcvtdq2pd %xmm3, %ymm3
  vmovupd %ymm3, a+32(,%rax,8)
  vpaddd %xmm1, %xmm2, %xmm2
  vcvtdq2pd %xmm2, %ymm2
  vmovupd %ymm2, b+32(,%rax,8)
  addq $8, %rax
  cmpq $60, %rcx
  jb .LBB0_3
  movabsq $4634274385308418048, %rax
  movq %rax, a+512(%rip)
  movabsq $4634344754052595712, %rax
  movq %rax, b+512(%rip)
  testl %ebx, %ebx
  jle .LBB0_5
  xorl %edi, %edi
  vzeroupper
  callq sum_f64
  vmovq %xmm0, 88(%rsp)
  xorl %edi, %edi
  callq dot_f64
  vmovq %xmm0, 80(%rsp)
  movl $1, %edi
  callq sum_f64
  vmovq %xmm0, 72(%rsp)
  movl $1, %edi
  callq dot_f64
  vmovq %xmm0, 64(%rsp)
  movl $2, %edi
  callq sum_f64
  vmovq %xmm0, 56(%rsp)
  movl $2, %edi
  callq dot_f64
  vmovq %xmm0, 48(%rsp)
  movl $3, %edi
  callq sum_f64
  vmovq %xmm0, 40(%rsp)
  movl $3, %edi
  callq dot_f64
  vmovq %xmm0, 32(%rsp)
  movl $4, %edi
  callq sum_f64
  vmovq %xmm0, 24(%rsp)
  movl $4, %edi
  callq dot_f64
  vmovq %xmm0, 16(%rsp)
  movl $5, %edi
  callq sum_f64
  vmovq %xmm0, 8(%rsp)
  movl $5, %edi
  callq dot_f64
  vmovq %xmm0, 280(%rsp)
  movl $7, %edi
  callq sum_f64
  vmovq %xmm0, 272(%rsp)
  movl $7, %edi
  callq dot_f64
  vmovq %xmm0, 264(%rsp)
  movl $8, %edi
  callq sum_f64
  vmovq %xmm0, 256(%rsp)
  movl $8, %edi
  callq dot_f64
  vmovq %xmm0, 248(%rsp)
  movl $9, %edi
  callq sum_f64
  vmovq %xmm0, 240(%rsp)
  movl $9, %edi
  callq dot_f64
  vmovq %xmm0, 232(%rsp)
  movl $15, %edi
  callq sum_f64
  vmovq %xmm0, 224(%rsp)
  movl $15, %edi
  callq dot_f64
  vmovq %xmm0, 216(%rsp)
  movl $16, %edi
  callq sum_f64
  vmovq %xmm0, 208(%rsp)
  movl $16, %edi
  callq dot_f64
  vmovq %xmm0, 200(%rsp)
  movl $17, %edi
  callq sum_f64
  vmovq %xmm0, 192(%rsp)
  movl $17, %edi
  callq dot_f64
  vmovq %xmm0, 184(%rsp)
  movl $31, %edi
  callq sum_f64
  vmovq %xmm0, 176(%rsp)
  movl $31, %edi
  callq dot_f64
  vmovq %xmm0, 168(%rsp)
  movl $32, %edi
  callq sum_f64
  vmovq %xmm0, 160(%rsp)
  movl $32, %edi
  callq dot_f64
  vmovq %xmm0, 152(%rsp)
  movl $33, %edi
  callq sum_f64
  vmovq %xmm0, 144(%rsp)
  movl $33, %edi
  callq dot_f64
  vmovq %xmm0, 136(%rsp)
  movl $63, %edi
  callq sum_f64
  vmovq %xmm0, 128(%rsp)
  movl $63, %edi
  callq dot_f64
  vmovq %xmm0, 120(%rsp)
  movl $64, %edi
  callq sum_f64
  vmovq %xmm0, 112(%rsp)
  movl $64, %edi
  callq dot_f64
  vmovq %xmm0, 104(%rsp)
  movl $65, %edi
  callq sum_f64
  vmovq %xmm0, 96(%rsp)
  movl $65, %edi
  callq dot_f64
  vmovsd 8(%rsp), %xmm15
  vmovsd 16(%rsp), %xmm14
  vmovsd 24(%rsp), %xmm13
  vmovsd 32(%rsp), %xmm12
  vmovsd 40(%rsp), %xmm11
  vmovsd 48(%rsp), %xmm10
  vmovsd 56(%rsp), %xmm9
  vmovsd 64(%rsp), %xmm8
  vmovsd 72(%rsp), %xmm7
  vmovsd 80(%rsp), %xmm6
  vmovsd 88(%rsp), %xmm5
  vpxor %xmm1, %xmm1, %xmm1
.LBB0_7:
  vaddsd %xmm1, %xmm5, %xmm1
  vaddsd %xmm6, %xmm7, %xmm2
  vaddsd %xmm1, %xmm2, %xmm1
  vaddsd %xmm8, %xmm9, %xmm2
  vaddsd %xmm2, %xmm10, %xmm2
  vaddsd %xmm1, %xmm2, %xmm1
  vaddsd %xmm12, %xmm11, %xmm2
  vaddsd %xmm13, %xmm14, %xmm3
  vaddsd %xmm1, %xmm2, %xmm1
  vaddsd %xmm3, %xmm1, %xmm1
  vaddsd 280(%rsp), %xmm15, %xmm2
  vmovsd 264(%rsp), %xmm3
  vaddsd 272(%rsp), %xmm3, %xmm3
  vaddsd 256(%rsp), %xmm1, %xmm1
  vaddsd %xmm2, %xmm3, %xmm2
  vaddsd %xmm1, %xmm2, %xmm1
  vmovsd 240(%rsp), %xmm2
  vaddsd 248(%rsp), %xmm2, %xmm2
  vmovsd 224(%rsp), %xmm3
  vaddsd 232(%rsp), %xmm3, %xmm3
  vmovsd 208(%rsp), %xmm4
  vaddsd 216(%rsp), %xmm4, %xmm4
  vaddsd %xmm1, %xmm2, %xmm1
  vaddsd %xmm3, %xmm4, %xmm2
  vaddsd %xmm1, %xmm2, %xmm1
  vmovsd 192(%rsp), %xmm2
  vaddsd 200(%rsp), %xmm2, %xmm2
  vmovsd 176(%rsp), %xmm3
  vaddsd 184(%rsp), %xmm3, %xmm3
  vmovsd 160(%rsp), %xmm4
  vaddsd 168(%rsp), %xmm4, %xmm4
  vaddsd 152(%rsp), %xmm1, %xmm1
  vaddsd %xmm2, %xmm3, %xmm2
  vaddsd %xmm4, %xmm1, %xmm1
  vaddsd %xmm2, %xmm1, %xmm1
  vmovsd 136(%rsp), %xmm2
  vaddsd 144(%rsp), %xmm2, %xmm2
  vmovsd 120(%rsp), %xmm3
  vaddsd 128(%rsp), %xmm3, %xmm3
  vmovsd 104(%rsp), %xmm4
  vaddsd 112(%rsp), %xmm4, %xmm4
  vaddsd %xmm2, %xmm3, %xmm2
  vaddsd 96(%rsp), %xmm0, %xmm3
  vaddsd %xmm4, %xmm3, %xmm3
  vaddsd %xmm2, %xmm3, %xmm2
  vaddsd %xmm1, %xmm2, %xmm1
  decl %ebx
  jne .LBB0_7
  jmp .LBB0_8
.LBB0_5:
  vpxor %xmm1, %xmm1, %xmm1
.LBB0_8:
  movl $.L.str, %edi
  vmovapd %xmm1, %xmm0
  movb $1, %al
  vzeroupper
  callq printf
  xorl %eax, %eax
  addq $288, %rsp
  popq %rbx
  retq

sum_f64:
  testl %edi, %edi
  jle .LBB1_1
  movl %edi, %eax
  movl $4294967292, %ecx
  andq %rax, %rcx
  je .LBB1_3
  vxorpd %xmm0, %xmm0, %xmm0
  xorl %edx, %edx
.LBB1_5:
  vaddpd a(,%rdx,8), %ymm0, %ymm0
  addq $4, %rdx
  cmpq %rcx, %rdx
  jb .LBB1_5
  vextractf128 $1, %ymm0, %xmm1
  vaddpd %xmm1, %xmm0, %xmm0
  vshufpd $1, %xmm0, %xmm0, %xmm1
  vaddsd %xmm1, %xmm0, %xmm0
  cmpq %rax, %rcx
  jne .LBB1_7
  jmp .LBB1_8
.LBB1_1:
  vxorps %xmm0, %xmm0, %xmm0
  retq
.LBB1_3:
  xorl %ecx, %ecx
  vxorpd %xmm0, %xmm0, %xmm0
.LBB1_7:
  vaddsd a(,%rcx,8), %xmm0, %xmm0
  incq %rcx
  cmpq %rcx, %rax
  jne .LBB1_7
.LBB1_8:
  vzeroupper
  retq

dot_f64:
  testl %edi, %edi
  jle .LBB2_1
  movl %edi, %eax
  movl $4294967292, %ecx
  andq %rax, %rcx
  je .LBB2_3
  vxorpd %xmm0, %xmm0, %xmm0
  xorl %edx, %edx
.LBB2_5:
  vmovupd b(,%rdx,8), %ymm1
  vfmadd231pd a(,%rdx,8), %ymm1, %ymm0
  addq $4, %rdx
  cmpq %rcx, %rdx
  jb .LBB2_5
  vextractf128 $1, %ymm0, %xmm1
  vaddpd %xmm1, %xmm0, %xmm0
  vshufpd $1, %xmm0, %xmm0, %xmm1
  vaddsd %xmm1, %xmm0, %xmm0
  cmpq %rax, %rcx
  jne .LBB2_7
  jmp .LBB2_8
.LBB2_1:
  vxorps %xmm0, %xmm0, %xmm0
  retq
.LBB2_3:
  xorl %ecx, %ecx
  vxorpd %xmm0, %xmm0, %xmm0
.LBB2_7:
  vmovsd b(,%rcx,8), %xmm1
  vfmadd231sd a(,%rcx,8), %xmm1, %xmm0
  incq %rcx
  cmpq %rcx, %rax
  jne .LBB2_7
.LBB2_8:
  vzeroupper
  retq

.L.str:
  .asciz "%.0f\n"

