siphash24:
  pushq %rbx
  subq $64, %rsp
  leaq 40(%rsp), %rax
  movq $0, 40(%rsp)
  vbroadcastf128 (%rdx), %ymm0
  vxorps .LCPI1_0(%rip), %ymm0, %ymm0
  vmovups %ymm0, 8(%rsp)
  leaq 48(%rsp), %rcx
  movq %rax, 48(%rsp)
  movq $0, 56(%rsp)
  addq %rdi, %rsi
  jmp .LBB1_1
.LBB1_6:
  movq 16(%rsp), %r9
  movq 40(%rsp), %r8
  movq 32(%rsp), %r10
  xorq %r8, %r10
  rorxq $51, %r9, %r11
  addq 8(%rsp), %r9
  xorq %r9, %r11
  rorxq $32, %r9, %r9
  rorxq $48, %r10, %rbx
  addq 24(%rsp), %r10
  xorq %r10, %rbx
  addq %rbx, %r9
  rorxq $43, %rbx, %rbx
  xorq %r9, %rbx
  addq %r11, %r10
  rorxq $47, %r11, %r11
  xorq %r10, %r11
  rorxq $32, %r10, %r10
  addq %r11, %r9
  rorxq $51, %r11, %r11
  xorq %r9, %r11
  rorxq $32, %r9, %r9
  addq %rbx, %r10
  rorxq $48, %rbx, %rbx
  xorq %r10, %rbx
  addq %rbx, %r9
  rorxq $43, %rbx, %rbx
  xorq %r9, %rbx
  addq %r11, %r10
  rorxq $47, %r11, %r11
  xorq %r10, %r11
  addq $8, 56(%rsp)
  xorq %r8, %r9
  rorxq $32, %r10, %r8
  movq %r11, 16(%rsp)
  movq %rbx, 32(%rsp)
  movq %r8, 24(%rsp)
  movq %r9, 8(%rsp)
  movq %rax, 48(%rsp)
  cmpq %rcx, %rdx
  jb .LBB1_7
.LBB1_1:
  cmpq %rsi, %rdi
  jae .LBB1_7
  movq %rsi, %r8
  subq %rdi, %r8
.LBB1_3:
  movq 48(%rsp), %rdx
  cmpq %rcx, %rdx
  jae .LBB1_6
  movzbl (%rdi), %r9d
  incq %rdi
  leaq 1(%rdx), %r10
  movq %r10, 48(%rsp)
  movb %r9b, (%rdx)
  decq %r8
  jne .LBB1_3
  movq %rsi, %rdi
  cmpq %rcx, 48(%rsp)
  jae .LBB1_6
.LBB1_7:
  leaq 8(%rsp), %rdi
  vzeroupper
  callq sip24_final
  addq $64, %rsp
  popq %rbx
  retq

.LCPI2_0:
  .quad 8388350548712186997
  .quad 7737605742629119589
  .quad 7746018054687388513
  .quad 8893317812261383291
