main:
  pushq %r14
  pushq %rbx
  pushq %rax
  vstmxcsr 4(%rsp)
  orl $32832, 4(%rsp)
  vldmxcsr 4(%rsp)
  xorl %r14d, %r14d
  xorl %ebx, %ebx
.LBB0_1:
  movl %r14d, %edi
  orl $1, %edi
  callq tls_pass
  addq %rax, %rbx
  incl %r14d
  cmpl $200000, %r14d
  jne .LBB0_1
  movl $.L.str, %edi
  movq %rbx, %rsi
  xorl %eax, %eax
  callq printf
  xorl %eax, %eax
  addq $8, %rsp
  popq %rbx
  popq %r14
  retq

tls_pass:
  pushq %rbx
  movq %fs:0, %rax
  leaq tls_slots@TPOFF(%rax), %rdx
  leaq tls_slots@TPOFF+8(%rax), %rcx
  movq %rcx, %fs:tls_indirect@TPOFF
  movl %edi, %esi
  movq %rsi, %fs:tls_slots@TPOFF+8
  leaq (,%rsi,4), %rdi
  leaq (%rsi,%rsi,2), %r8
  addq %rsi, %rsi
  movl $4, %r9d
  xorl %eax, %eax
  movq %r8, %r10
.LBB1_1:
  leal -2(%r9), %r11d
  movq %rsi, -16(%rdx,%r9,8)
  andl $7, %r11d
  movq 8(%rdx,%r11,8), %r11
  xorq %rsi, %r11
  addq %rax, %r11
  leal -1(%r9), %eax
  movq %r10, -8(%rdx,%r9,8)
  andl $7, %eax
  movq 8(%rdx,%rax,8), %rbx
  xorq %r10, %rbx
  movq %rdi, (%rdx,%r9,8)
  movl %r9d, %eax
  andl $7, %eax
  movq 8(%rdx,%rax,8), %rax
  xorq %rdi, %rax
  addq %rbx, %rax
  addq %r11, %rax
  addq %r8, %rdi
  addq %r8, %r10
  addq %r8, %rsi
  addq $3, %r9
  cmpq $67, %r9
  jne .LBB1_1
  addq (%rcx), %rax
  popq %rbx
  retq

.L.str:

tls_slots:

tls_indirect:

