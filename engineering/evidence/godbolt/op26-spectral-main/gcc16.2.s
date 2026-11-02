main:
  leaq 8(%rsp), %r10
  andq $-32, %rsp
  pushq -8(%r10)
  pushq %rbp
  movq %rsp, %rbp
  pushq %r10
  leaq -48016(%rbp), %rax
  leaq -32016(%rbp), %r8
  subq $48008, %rsp
  vbroadcastsd .LC4(%rip), %ymm0
.L15:
  vmovapd %ymm0, (%rax)
  addq $64, %rax
  vmovapd %ymm0, -32(%rax)
  cmpq %r8, %rax
  jne .L15
  movl $10, %r9d
  vzeroupper
.L16:
  leaq -16016(%rbp), %rsi
  leaq -48016(%rbp), %rdi
  call mul_Av.constprop.0
  leaq -16016(%rbp), %rdi
  movq %r8, %rsi
  call mul_Atv.constprop.0
  movq %rdi, %rsi
  movq %r8, %rdi
  call mul_Av.constprop.0
  leaq -48016(%rbp), %rsi
  leaq -16016(%rbp), %rdi
  call mul_Atv.constprop.0
  subl $1, %r9d
  jne .L16
  vxorpd %xmm4, %xmm4, %xmm4
  xorl %eax, %eax
  vmovapd %xmm4, %xmm1
.L17:
  vmovapd (%r8,%rax), %ymm0
  vmulpd -48016(%rbp,%rax), %ymm0, %ymm2
  addq $32, %rax
  vmulpd %ymm0, %ymm0, %ymm0
  vaddsd %xmm2, %xmm1, %xmm1
  vunpckhpd %xmm2, %xmm2, %xmm3
  vextractf128 $0x1, %ymm2, %xmm2
  vaddsd %xmm4, %xmm0, %xmm4
  vaddsd %xmm3, %xmm1, %xmm1
  vunpckhpd %xmm0, %xmm0, %xmm3
  vextractf128 $0x1, %ymm0, %xmm0
  vaddsd %xmm4, %xmm3, %xmm3
  vaddsd %xmm2, %xmm1, %xmm1
  vunpckhpd %xmm2, %xmm2, %xmm2
  vaddsd %xmm3, %xmm0, %xmm4
  vunpckhpd %xmm0, %xmm0, %xmm0
  vaddsd %xmm2, %xmm1, %xmm1
  vaddsd %xmm0, %xmm4, %xmm4
  cmpq $16000, %rax
  jne .L17
  vdivsd %xmm4, %xmm1, %xmm0
  vxorpd %xmm1, %xmm1, %xmm1
  vucomisd %xmm0, %xmm1
  ja .L26
  vsqrtsd %xmm0, %xmm0, %xmm0
  vzeroupper
.L20:
  movl $.LC6, %edi
  movl $1, %eax
  call printf
  addq $48008, %rsp
  xorl %eax, %eax
  popq %r10
  popq %rbp
  leaq -8(%r10), %rsp
  ret
.L26:
  vzeroupper
  call sqrt
  jmp .L20
.LC0:
  .long 0
  .long 1
  .long 2
  .long 3
  .long 4
  .long 5
  .long 6
  .long 7
.LC4:
  .long 0
  .long 1072693248
