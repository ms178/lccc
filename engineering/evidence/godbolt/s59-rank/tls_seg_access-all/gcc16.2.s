tls_pass:
  movl %edi, %edi
  movl $2, %eax
  xorl %ecx, %ecx
  movq %fs:0, %rsi
  movq %rdi, %fs:tls_slots@tpoff+8
  leaq (%rdi,%rdi), %rdx
  leaq tls_slots@tpoff(%rsi), %r8
.L2:
  movq %rax, %rsi
  movq %rdx, (%r8,%rax,8)
  addq $1, %rax
  andl $7, %esi
  movq %fs:tls_slots@tpoff+8(,%rsi,8), %r9
  xorq %rdx, %r9
  addq %rdi, %rdx
  addq %r9, %rcx
  cmpq $65, %rax
  jne .L2
  leaq (%rdi,%rcx), %rax
  ret
.LC0:
main:
  subq $8, %rsp
  xorl %r10d, %r10d
  xorl %r11d, %r11d
.L6:
  movl %r10d, %edi
  addl $1, %r10d
  orl $1, %edi
  call tls_pass
  addq %rax, %r11
  cmpl $200000, %r10d
  jne .L6
  movq %r11, %rsi
  movl $.LC0, %edi
  xorl %eax, %eax
  call printf
  xorl %eax, %eax
  addq $8, %rsp
  ret
tls_slots:
