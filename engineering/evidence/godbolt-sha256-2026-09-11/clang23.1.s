sha256_transform:
  pushq %rbp
  pushq %r15
  pushq %r14
  pushq %r13
  pushq %r12
  pushq %rbx
  subq $168, %rsp
  movups (%rsi), %xmm0
  movups 16(%rsi), %xmm1
  movups 32(%rsi), %xmm2
  movups 48(%rsi), %xmm3
  movaps %xmm3, -48(%rsp)
  movaps %xmm2, -64(%rsp)
  movaps %xmm1, -80(%rsp)
  movaps %xmm0, -96(%rsp)
  movl $16, %eax
.LBB1_1:
  movl -104(%rsp,%rax,4), %ecx
  movl %ecx, %edx
  roll $15, %edx
  movl %ecx, %esi
  roll $13, %esi
  xorl %edx, %esi
  shrl $10, %ecx
  xorl %esi, %ecx
  addl -124(%rsp,%rax,4), %ecx
  movl -156(%rsp,%rax,4), %edx
  movl %edx, %esi
  roll $25, %esi
  movl %edx, %r8d
  roll $14, %r8d
  xorl %esi, %r8d
  shrl $3, %edx
  xorl %r8d, %edx
  addl -160(%rsp,%rax,4), %ecx
  addl %edx, %ecx
  movl %ecx, -96(%rsp,%rax,4)
  incq %rax
  cmpq $64, %rax
  jne .LBB1_1
  movl (%rdi), %r11d
  movl 4(%rdi), %r12d
  movl 8(%rdi), %ecx
  movl 12(%rdi), %r9d
  movl 16(%rdi), %r8d
  movl 20(%rdi), %ebp
  movl 24(%rdi), %ebx
  movl 28(%rdi), %r14d
  xorl %r15d, %r15d
  leaq K(%rip), %r13
  movl %r11d, -128(%rsp)
  movl %r12d, -124(%rsp)
  movl %r14d, -100(%rsp)
  movl %ebx, -104(%rsp)
  movl %ebp, -108(%rsp)
  movl %r8d, -112(%rsp)
  movl %r9d, -116(%rsp)
  movl %ecx, -120(%rsp)
.LBB1_3:
  movl %ecx, %edx
  movl %r8d, %eax
  movl %ebp, %r10d
  roll $26, %r8d
  movl %ebx, %esi
  movl %eax, %ebx
  roll $21, %ebx
  movl %r12d, %ecx
  movl %eax, %ebp
  roll $7, %ebp
  xorl %r8d, %ebx
  xorl %ebx, %ebp
  movl %r10d, %ebx
  xorl %esi, %ebx
  andl %eax, %ebx
  xorl %esi, %ebx
  addl %r14d, %ebx
  addl %ebp, %ebx
  addl (%r15,%r13), %ebx
  addl -96(%rsp,%r15), %ebx
  movl %r11d, %r12d
  movl %r11d, %r8d
  roll $30, %r8d
  roll $19, %r11d
  xorl %r8d, %r11d
  movl %r12d, %r8d
  roll $10, %r8d
  xorl %r11d, %r8d
  movl %ecx, %ebp
  xorl %edx, %ebp
  andl %r12d, %ebp
  movl %ecx, %r11d
  andl %edx, %r11d
  xorl %ebp, %r11d
  addl %r8d, %r11d
  movl %r9d, %r8d
  addl %ebx, %r8d
  addl %ebx, %r11d
  addq $4, %r15
  movl %esi, %r14d
  movl %r10d, %ebx
  movl %eax, %ebp
  movl %edx, %r9d
  cmpq $256, %r15
  jne .LBB1_3
  addl -128(%rsp), %r11d
  movl %r11d, (%rdi)
  addl -124(%rsp), %r12d
  movl %r12d, 4(%rdi)
  addl -120(%rsp), %ecx
  movl %ecx, 8(%rdi)
  addl -116(%rsp), %edx
  movl %edx, 12(%rdi)
  addl -112(%rsp), %r8d
  movl %r8d, 16(%rdi)
  addl -108(%rsp), %eax
  movl %eax, 20(%rdi)
  addl -104(%rsp), %r10d
  movl %r10d, 24(%rdi)
  addl -100(%rsp), %esi
  movl %esi, 28(%rdi)
  addq $168, %rsp
  popq %rbx
  popq %r12
  popq %r13
  popq %r14
  popq %r15
  popq %rbp
  retq

.L.str:
  .asciz "%016llx\n"

.L__const.check_known_vector.state:
  .long 1779033703
  .long 3144134277
  .long 1013904242
  .long 2773480762
  .long 1359893119
  .long 2600822924
  .long 528734635
  .long 1541459225
