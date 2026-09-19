main:
  pushq %r14
  pushq %rbx
  pushq %rax
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
  leaq .L.str(%rip), %rdi
  movq %rbx, %rsi
  xorl %eax, %eax
  callq printf@PLT
  xorl %eax, %eax
  addq $8, %rsp
  popq %rbx
  popq %r14
  retq

tls_pass:
  movq %fs:0, %rax
  leaq tls_slots@TPOFF+8(%rax), %rax
  movq %rax, %fs:tls_indirect@TPOFF
  movl %edi, %ecx
  movq %rcx, %fs:tls_slots@TPOFF+8
  movl $4, %edx
  xorl %eax, %eax
.LBB1_1:
  leal -2(%rdx), %esi
  movq %fs:tls_slots@TPOFF-24(,%rdx,8), %rdi
  addq %rcx, %rdi
  movq %rdi, %fs:tls_slots@TPOFF-16(,%rdx,8)
  andl $7, %esi
  xorq %fs:tls_slots@TPOFF+8(,%rsi,8), %rdi
  movq %rax, %rsi
  addq %rdi, %rsi
  leal -1(%rdx), %eax
  movq %fs:tls_slots@TPOFF-16(,%rdx,8), %rdi
  addq %rcx, %rdi
  movq %rdi, %fs:tls_slots@TPOFF-8(,%rdx,8)
  andl $7, %eax
  xorq %fs:tls_slots@TPOFF+8(,%rax,8), %rdi
  movq %fs:tls_slots@TPOFF-8(,%rdx,8), %rax
  addq %rcx, %rax
  movq %rax, %fs:tls_slots@TPOFF(,%rdx,8)
  movl %edx, %r8d
  andl $7, %r8d
  xorq %fs:tls_slots@TPOFF+8(,%r8,8), %rax
  addq %rdi, %rax
  addq %rsi, %rax
  addq $3, %rdx
  cmpq $67, %rdx
  jne .LBB1_1
  addq %fs:tls_slots@TPOFF+8, %rax
  retq

.L.str:

tls_slots:

tls_indirect:

