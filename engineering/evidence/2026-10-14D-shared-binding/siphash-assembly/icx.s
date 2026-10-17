siphash24:
  pushq %rbp
  pushq %r15
  pushq %r14
  pushq %rbx
  subq $56, %rsp
  leaq 32(%rsp), %rax
  movq $0, 32(%rsp)
  vmovq %rcx, %xmm0
  vmovq %rdx, %xmm1
  vpunpcklqdq %xmm0, %xmm1, %xmm0
  vpermq $68, %ymm0, %ymm0
  vpxor .LCPI1_0(%rip), %ymm0, %ymm0
  vmovdqu %ymm0, (%rsp)
  leaq 40(%rsp), %rcx
  movq %rax, 40(%rsp)
  movq $0, 48(%rsp)
  leaq (%rdi,%rsi), %rdx
  movq %rdi, %r8
.LBB1_1:
  cmpq %rdx, %r8
  jae .LBB1_16
  movq %rdi, %rbx
  subq %r8, %rbx
  addq %rsi, %rbx
  movq %rax, %r11
  cmpq $2, %rbx
  jb .LBB1_8
  movq %rbx, %r9
  shrq %r9
  movl $1, %r10d
.LBB1_4:
  movq 40(%rsp), %r11
  cmpq %rcx, %r11
  jae .LBB1_12
  movzbl -1(%r8,%r10), %ebp
  leaq 1(%r11), %r14
  movq %r14, 40(%rsp)
  movb %bpl, (%r11)
  movq 40(%rsp), %r11
  cmpq %rcx, %r11
  jae .LBB1_13
  movzbl (%r8,%r10), %ebp
  leaq 1(%r11), %r14
  movq %r14, 40(%rsp)
  movb %bpl, (%r11)
  addq $2, %r10
  decq %r9
  jne .LBB1_4
  movq 40(%rsp), %r11
.LBB1_8:
  movq %rbx, %r10
  andq $-2, %r10
  movb $1, %r9b
  cmpq %rbx, %r10
  jae .LBB1_11
  cmpq %rcx, %r11
  jae .LBB1_13
  movzbl (%r8,%r10), %r10d
  leaq 1(%r11), %r14
  movq %r14, 40(%rsp)
  movb %r10b, (%r11)
  movq 40(%rsp), %r11
.LBB1_11:
  movq %rbx, %r10
  cmpq %rcx, %r11
  jae .LBB1_15
  jmp .LBB1_16
.LBB1_12:
  decq %r10
.LBB1_13:
  xorl %r9d, %r9d
  cmpq %rcx, %r11
  jb .LBB1_16
.LBB1_15:
  addq %r10, %r8
  movq 8(%rsp), %r11
  movq 32(%rsp), %r10
  movq 24(%rsp), %rbx
  xorq %r10, %rbx
  rorxq $51, %r11, %r14
  addq (%rsp), %r11
  xorq %r11, %r14
  rorxq $32, %r11, %r11
  rorxq $48, %rbx, %r15
  addq 16(%rsp), %rbx
  xorq %rbx, %r15
  addq %r15, %r11
  rorxq $43, %r15, %r15
  xorq %r11, %r15
  addq %r14, %rbx
  rorxq $47, %r14, %r14
  xorq %rbx, %r14
  rorxq $32, %rbx, %rbx
  addq %r14, %r11
  rorxq $51, %r14, %r14
  xorq %r11, %r14
  rorxq $32, %r11, %r11
  addq %r15, %rbx
  rorxq $48, %r15, %r15
  xorq %rbx, %r15
  addq %r15, %r11
  rorxq $43, %r15, %r15
  xorq %r11, %r15
  addq %r14, %rbx
  rorxq $47, %r14, %r14
  xorq %rbx, %r14
  rorxq $32, %rbx, %rbx
  movq %r14, 8(%rsp)
  movq %r15, 24(%rsp)
  movq %rbx, 16(%rsp)
  xorq %r10, %r11
  movq %r11, (%rsp)
  movq %rax, 40(%rsp)
  addq $8, 48(%rsp)
  testb %r9b, %r9b
  je .LBB1_1
.LBB1_16:
  movq %rsp, %rdi
  vzeroupper
  callq sip24_final
  addq $56, %rsp
  popq %rbx
  popq %r14
  popq %r15
  popq %rbp
  retq

.LCPI2_1:
  .quad 8388350548712186997
  .quad 7737605742629119589
  .quad 7746018054687388513
  .quad 8893317812261383291
.LCPI2_2:
  .byte 3
  .byte 7
  .byte 11
  .byte 15
