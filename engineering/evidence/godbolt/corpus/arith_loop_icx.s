arith_loop:
  testl %edi, %edi
  jle .LBB0_1
  pushq %rbp
  pushq %r15
  pushq %r14
  pushq %r13
  pushq %r12
  pushq %rbx
  subq $12, %rsp
  movl %edi, %r10d
  movl $1, -56(%rsp)
  movl $2, %ebx
  movl $3, %esi
  movl $4, %ecx
  movl $5, %r12d
  movl $6, %r9d
  movl $7, %r15d
  movl $8, %r13d
  movl $9, %r11d
  movl $10, %ebp
  movl $11, %edx
  movl $12, -124(%rsp)
  movl $13, %r8d
  movl $14, %edi
  movl $15, -128(%rsp)
  movl $16, %r14d
  movl $17, -120(%rsp)
  movl $18, -104(%rsp)
  movl $19, -100(%rsp)
  movl $20, -96(%rsp)
  movl $21, -92(%rsp)
  movl $22, -88(%rsp)
  movl $23, -112(%rsp)
  movl $24, -84(%rsp)
  movl $25, -80(%rsp)
  movl $26, -76(%rsp)
  movl $27, -72(%rsp)
  movl $28, -68(%rsp)
  movl $29, -64(%rsp)
  movl $30, -60(%rsp)
  movl $31, %eax
  movl $32, -108(%rsp)
.LBB0_5:
  movl %r10d, 8(%rsp)
  movl %ebx, 4(%rsp)
  movl %esi, (%rsp)
  movl %ecx, -4(%rsp)
  movl %r12d, -8(%rsp)
  movl %r9d, -12(%rsp)
  movl %r15d, -16(%rsp)
  movl %r13d, -20(%rsp)
  movl %r11d, -24(%rsp)
  movl %ebp, -28(%rsp)
  movl %edx, -32(%rsp)
  movl -124(%rsp), %ecx
  movl %ecx, -36(%rsp)
  movl %r8d, -124(%rsp)
  movl %edi, -40(%rsp)
  movl -128(%rsp), %ecx
  movl %ecx, -44(%rsp)
  movl %r14d, -128(%rsp)
  movl -120(%rsp), %ecx
  movl %ecx, -116(%rsp)
  movl -104(%rsp), %ecx
  movl %ecx, -120(%rsp)
  movl -100(%rsp), %ebp
  movl -96(%rsp), %esi
  movl -92(%rsp), %r9d
  movl -88(%rsp), %r15d
  movl -112(%rsp), %r12d
  movl -84(%rsp), %r11d
  movl -80(%rsp), %r10d
  movl -76(%rsp), %edx
  movl -72(%rsp), %r8d
  movl -68(%rsp), %ebx
  movl -64(%rsp), %r14d
  movl -60(%rsp), %r13d
  movl -108(%rsp), %ecx
  movl %ecx, -52(%rsp)
  movl %eax, %edi
  movl %eax, -48(%rsp)
  movl %eax, %ecx
  imull %r13d, %ecx
  movl -52(%rsp), %eax
  imull %edi, %eax
  addl %r13d, %eax
  movl %eax, -60(%rsp)
  imull %r14d, %r13d
  addl %r14d, %ecx
  movl %ecx, -64(%rsp)
  imull %ebx, %r14d
  addl %ebx, %r13d
  movl %r13d, -68(%rsp)
  imull %r8d, %ebx
  addl %r8d, %r14d
  movl %r14d, -72(%rsp)
  imull %edx, %r8d
  addl %edx, %ebx
  movl %ebx, -76(%rsp)
  imull %r10d, %edx
  addl %r10d, %r8d
  movl %r8d, -80(%rsp)
  imull %r11d, %r10d
  addl %r11d, %edx
  movl %edx, -84(%rsp)
  imull %r12d, %r11d
  addl %r12d, %r10d
  movl %r10d, -112(%rsp)
  imull %r15d, %r12d
  addl %r15d, %r11d
  movl %r11d, -88(%rsp)
  imull %r9d, %r15d
  addl %r9d, %r12d
  movl %r12d, -92(%rsp)
  imull %esi, %r9d
  addl %esi, %r15d
  movl %r15d, -96(%rsp)
  imull %ebp, %esi
  addl %ebp, %r9d
  movl %r9d, -100(%rsp)
  movl -120(%rsp), %eax
  imull %eax, %ebp
  addl %eax, %esi
  movl %esi, -104(%rsp)
  movl %eax, %ecx
  movl -116(%rsp), %eax
  imull %eax, %ecx
  addl %eax, %ebp
  movl %ebp, -120(%rsp)
  movl -128(%rsp), %edi
  imull %edi, %eax
  addl %edi, %ecx
  movl %ecx, -116(%rsp)
  movl -44(%rsp), %r8d
  imull %r8d, %edi
  addl %r8d, %eax
  movl %eax, -128(%rsp)
  movl -40(%rsp), %eax
  imull %eax, %r8d
  addl %eax, %edi
  movl -124(%rsp), %edx
  imull %edx, %eax
  addl %edx, %r8d
  movl -36(%rsp), %ebp
  imull %ebp, %edx
  addl %ebp, %eax
  movl %eax, -124(%rsp)
  movl -32(%rsp), %r11d
  imull %r11d, %ebp
  addl %r11d, %edx
  movl -28(%rsp), %r13d
  imull %r13d, %r11d
  addl %r13d, %ebp
  movl -24(%rsp), %r15d
  imull %r15d, %r13d
  addl %r15d, %r11d
  movl -20(%rsp), %r9d
  imull %r9d, %r15d
  addl %r9d, %r13d
  movl -16(%rsp), %r12d
  imull %r12d, %r9d
  addl %r12d, %r15d
  movl -12(%rsp), %ecx
  imull %ecx, %r12d
  addl %ecx, %r9d
  movl -8(%rsp), %esi
  imull %esi, %ecx
  addl %esi, %r12d
  movl -4(%rsp), %ebx
  imull %ebx, %esi
  addl %ebx, %ecx
  movl (%rsp), %eax
  imull %eax, %ebx
  addl %eax, %esi
  movl 4(%rsp), %r14d
  imull %r14d, %eax
  movl -56(%rsp), %r10d
  addl %eax, %r10d
  addl %r14d, %ebx
  movl %ebx, %r14d
  imull %r10d, %r14d
  movl -52(%rsp), %eax
  addl %eax, %r14d
  movl %r14d, -108(%rsp)
  movl -116(%rsp), %r14d
  movl %r10d, -56(%rsp)
  imull %r10d, %eax
  movl 8(%rsp), %r10d
  addl -48(%rsp), %eax
  decl %r10d
  jne .LBB0_5
  movl -56(%rsp), %r10d
  xorl %ebx, %r10d
  xorl %ecx, %esi
  xorl %r10d, %esi
  xorl %r9d, %r12d
  xorl %r15d, %r12d
  xorl %esi, %r12d
  xorl %r11d, %r13d
  xorl %ebp, %r13d
  xorl %edx, %r13d
  xorl %r12d, %r13d
  movl -124(%rsp), %edx
  xorl %r8d, %edx
  xorl %edi, %edx
  xorl -128(%rsp), %edx
  xorl %r14d, %edx
  xorl %r13d, %edx
  movl -120(%rsp), %ecx
  xorl -104(%rsp), %ecx
  xorl -100(%rsp), %ecx
  xorl -96(%rsp), %ecx
  xorl -92(%rsp), %ecx
  xorl -88(%rsp), %ecx
  xorl %edx, %ecx
  movl -112(%rsp), %edx
  xorl -84(%rsp), %edx
  xorl -80(%rsp), %edx
  xorl -76(%rsp), %edx
  xorl -72(%rsp), %edx
  xorl -68(%rsp), %edx
  xorl -64(%rsp), %edx
  xorl %ecx, %edx
  movl -60(%rsp), %ecx
  xorl %eax, %ecx
  movl %ecx, %eax
  xorl -108(%rsp), %eax
  xorl %edx, %eax
  addq $12, %rsp
  popq %rbx
  popq %r12
  popq %r13
  popq %r14
  popq %r15
  popq %rbp
  retq
.LBB0_1:
  movl $32, %eax
  retq

main:
  pushq %rax
  vstmxcsr (%rsp)
  orl $32832, (%rsp)
  vldmxcsr (%rsp)
  movl $10000000, %edi
  callq arith_loop
  movl %eax, 4(%rsp)
  movl 4(%rsp), %esi
  movl $.L.str, %edi
  xorl %eax, %eax
  callq printf
  xorl %eax, %eax
  popq %rcx
  retq

.L.str:
  .asciz "arith_loop result: %d\n"

