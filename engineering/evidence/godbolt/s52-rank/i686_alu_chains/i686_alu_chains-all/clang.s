k_urem7:
  pushq %rbp
  pushq %r15
  pushq %r14
  pushq %r13
  pushq %r12
  pushq %rbx
  movl %edi, %eax
  movl %esi, -124(%rsp)
  testl %esi, %esi
  je .LBB0_8
  movl -124(%rsp), %ecx
  movl %ecx, %r8d
  andl $7, %r8d
  xorl %r11d, %r11d
  movabsq $2635249153617166336, %r10
  cmpl $8, %ecx
  jb .LBB0_5
  movl %r8d, -76(%rsp)
  andl $-8, -124(%rsp)
  xorl %r11d, %r11d
  movl $-36, %r14d
  movl $-35, %ecx
  movq %rcx, -120(%rsp)
  movl $-33, %r15d
  movl $-30, %r12d
  movl $-26, %r9d
  movl $-21, %ebp
  movl $-15, %esi
  movl $-8, %ecx
  movq %rcx, -112(%rsp)
  movl $1, %edi
  movl $21, %r8d
  movl $15, %ebx
  movl $10, %ecx
  movq %rcx, -88(%rsp)
  movl $6, %ecx
  movq %rcx, -96(%rsp)
  movl $3, %ecx
  movq %rcx, -104(%rsp)
.LBB0_3:
  movq %rsi, -40(%rsp)
  movq %rbp, -32(%rsp)
  movq %r9, -24(%rsp)
  movq %r12, -16(%rsp)
  movq %r15, -8(%rsp)
  movl %r14d, -60(%rsp)
  movl %eax, %r13d
  leal (%r13,%r11), %eax
  movl %r13d, %edx
  movq %rdx, -56(%rsp)
  mulxq %r10, %r9, %r9
  leal (,%r9,8), %ecx
  movl %ecx, -68(%rsp)
  subl %r9d, %ecx
  subl %ecx, %eax
  movq %rax, %rdx
  mulxq %r10, %rax, %rax
  leal (,%rax,8), %esi
  movl %eax, %edx
  subl %esi, %edx
  movl %edx, -64(%rsp)
  leal (%rdi,%r13), %edx
  subl %ecx, %edx
  subl %eax, %esi
  subl %esi, %edx
  mulxq %r10, %r14, %r14
  leal (,%r14,8), %r10d
  movl %r14d, %eax
  subl %r10d, %eax
  movl %eax, -72(%rsp)
  leal (%r8,%r13), %eax
  subl %ecx, %eax
  movq %rbx, -48(%rsp)
  leal (%rbx,%r13), %r15d
  movq -96(%rsp), %rdx
  leal (%rdx,%r13), %r12d
  subl %ecx, %r12d
  movq -104(%rsp), %rdx
  addl %r13d, %edx
  subl %ecx, %edx
  subl %r14d, %r10d
  subl %r10d, %edx
  subl %esi, %edx
  movabsq $2635249153617166336, %r14
  mulxq %r14, %rdx, %rdx
  leal (,%rdx,8), %r14d
  subl %edx, %r14d
  subl %r14d, %r12d
  subl %r10d, %r12d
  subl %esi, %r12d
  movq %r12, %rdx
  movabsq $2635249153617166336, %r12
  mulxq %r12, %rdx, %rdx
  leal (,%rdx,8), %r12d
  subl %edx, %r12d
  movq -88(%rsp), %rdx
  addl %r13d, %edx
  subl %ecx, %edx
  subl %r12d, %edx
  subl %r14d, %edx
  subl %r10d, %edx
  subl %esi, %edx
  movq %r11, %rbp
  movabsq $2635249153617166336, %rbx
  mulxq %rbx, %rdx, %rdx
  subl %ecx, %r15d
  leal (,%rdx,8), %ecx
  subl %edx, %ecx
  subl %ecx, %r15d
  subl %r12d, %r15d
  subl %r14d, %r15d
  subl %r10d, %r15d
  subl %esi, %r15d
  movq %r15, %rdx
  mulxq %rbx, %rdx, %rdx
  movq -120(%rsp), %rbx
  movq %rbp, %r11
  leal (,%rdx,8), %r15d
  subl %edx, %r15d
  subl %r15d, %eax
  subl %ecx, %eax
  subl %r12d, %eax
  subl %r14d, %eax
  subl %r10d, %eax
  movabsq $2635249153617166336, %r10
  subl %esi, %eax
  movq -40(%rsp), %rsi
  movq %rax, %rdx
  mulxq %r10, %rax, %rax
  subl -68(%rsp), %r9d
  leal (,%rax,8), %edx
  subl %edx, %eax
  addl %r13d, %eax
  subl %r15d, %eax
  movq -8(%rsp), %r15
  subl %ecx, %eax
  subl %r12d, %eax
  movq -16(%rsp), %r12
  subl %r14d, %eax
  movl -60(%rsp), %r14d
  addl %r14d, %r9d
  addl -64(%rsp), %r9d
  addl -72(%rsp), %r9d
  movq -32(%rsp), %rbp
  addl %r9d, %eax
  addl $64, %eax
  movq -24(%rsp), %r9
  addl $8, %r11d
  addl $64, %r14d
  addl $56, %ebx
  movq %rbx, -120(%rsp)
  addl $48, %r15d
  addl $40, %r12d
  addl $32, %r9d
  addl $24, %ebp
  addl $16, %esi
  movq -112(%rsp), %rcx
  addl $8, %ecx
  movq %rcx, -112(%rsp)
  addl $16, %edi
  addl $56, %r8d
  movq -48(%rsp), %rbx
  addl $48, %ebx
  movq -88(%rsp), %rcx
  addl $40, %ecx
  movq %rcx, -88(%rsp)
  movq -96(%rsp), %rcx
  addl $32, %ecx
  movq %rcx, -96(%rsp)
  movq -104(%rsp), %rcx
  addl $24, %ecx
  movq %rcx, -104(%rsp)
  cmpl %r11d, -124(%rsp)
  jne .LBB0_3
  movq -56(%rsp), %rdx
  mulxq %r10, %rcx, %rcx
  leal (,%rcx,8), %edi
  movl %ecx, %eax
  subl %edi, %eax
  movq -120(%rsp), %rbx
  addl %r13d, %ebx
  subl %ecx, %edi
  subl %edi, %ebx
  addl %r13d, %r15d
  addl %r13d, %r12d
  subl %edi, %r12d
  addl %r13d, %r9d
  addl %r13d, %ebp
  subl %edi, %ebp
  addl %r13d, %esi
  movq -112(%rsp), %rdx
  addl %r13d, %edx
  subl %edi, %edx
  mulxq %r10, %rdx, %rdx
  subl %edi, %esi
  leal (,%rdx,8), %ecx
  subl %edx, %ecx
  subl %ecx, %esi
  movq %rsi, %rdx
  mulxq %r10, %rdx, %rdx
  leal (,%rdx,8), %esi
  subl %edx, %esi
  subl %esi, %ebp
  subl %ecx, %ebp
  movq %rbp, %rdx
  mulxq %r10, %rdx, %rdx
  subl %edi, %r9d
  leal (,%rdx,8), %r8d
  subl %edx, %r8d
  subl %r8d, %r9d
  subl %esi, %r9d
  subl %ecx, %r9d
  movq %r9, %rdx
  mulxq %r10, %rdx, %rdx
  leal (,%rdx,8), %r9d
  subl %edx, %r9d
  subl %r9d, %r12d
  subl %r8d, %r12d
  subl %esi, %r12d
  subl %ecx, %r12d
  movq %r12, %rdx
  mulxq %r10, %rdx, %rdx
  subl %edi, %r15d
  leal (,%rdx,8), %edi
  subl %edx, %edi
  subl %edi, %r15d
  subl %r9d, %r15d
  subl %r8d, %r15d
  subl %esi, %r15d
  subl %ecx, %r15d
  movq %r15, %rdx
  mulxq %r10, %rdx, %rdx
  leal (,%rdx,8), %r10d
  subl %edx, %r10d
  subl %r10d, %ebx
  subl %edi, %ebx
  subl %r9d, %ebx
  subl %r8d, %ebx
  subl %esi, %ebx
  subl %ecx, %ebx
  movq %rbx, %rdx
  movabsq $2635249153617166336, %rbx
  mulxq %rbx, %rdx, %rdx
  leal (,%rdx,8), %ebx
  subl %ebx, %edx
  addl %r13d, %edx
  subl %r10d, %edx
  movabsq $2635249153617166336, %r10
  subl %edi, %edx
  subl %r9d, %edx
  subl %r8d, %edx
  subl %esi, %edx
  subl %ecx, %edx
  addl %r14d, %eax
  addl %edx, %eax
  movl -76(%rsp), %r8d
  testl %r8d, %r8d
  je .LBB0_8
.LBB0_5:
  movl $1, %ecx
  subl %r11d, %ecx
.LBB0_6:
  movl %eax, %esi
  movl %eax, %edx
  mulxq %r10, %rax, %rax
  leal (,%rax,8), %edi
  subl %edi, %eax
  addl %r11d, %eax
  addl %esi, %eax
  incl %r11d
  decl %ecx
  decl %r8d
  jne .LBB0_6
  mulxq %r10, %rax, %rax
  leal (,%rax,8), %edx
  subl %edx, %eax
  addl %eax, %esi
  subl %ecx, %esi
  movl %esi, %eax
.LBB0_8:
  popq %rbx
  popq %r12
  popq %r13
  popq %r14
  popq %r15
  popq %rbp
  retq

k_urem10:
  movl %edi, %eax
  testl %esi, %esi
  je .LBB1_7
  movl %esi, %ecx
  andl $3, %ecx
  xorl %edx, %edx
  cmpl $4, %esi
  jb .LBB1_5
  andl $-4, %esi
  xorl %edx, %edx
  movl $3435973837, %edi
.LBB1_3:
  movl %eax, %r8d
  imulq %rdi, %r8
  shrq $35, %r8
  addl %r8d, %r8d
  leal (%r8,%r8,4), %r8d
  movl %edx, %r9d
  xorl %eax, %r9d
  subl %r8d, %eax
  addl %r9d, %eax
  movq %rax, %r8
  imulq %rdi, %r8
  shrq $35, %r8
  addl %r8d, %r8d
  leal (%r8,%r8,4), %r8d
  leal 1(%rdx), %r9d
  xorl %eax, %r9d
  subl %r8d, %eax
  addl %r9d, %eax
  movq %rax, %r8
  imulq %rdi, %r8
  shrq $35, %r8
  addl %r8d, %r8d
  leal (%r8,%r8,4), %r8d
  leal 2(%rdx), %r9d
  xorl %eax, %r9d
  subl %r8d, %eax
  addl %r9d, %eax
  movq %rax, %r8
  imulq %rdi, %r8
  shrq $35, %r8
  addl %r8d, %r8d
  leal (%r8,%r8,4), %r8d
  leal 3(%rdx), %r9d
  xorl %eax, %r9d
  subl %r8d, %eax
  addl %r9d, %eax
  addl $4, %edx
  cmpl %esi, %edx
  jne .LBB1_3
  testl %ecx, %ecx
  je .LBB1_7
.LBB1_5:
  movl $3435973837, %esi
.LBB1_6:
  movl %eax, %edi
  imulq %rsi, %rdi
  shrq $35, %rdi
  addl %edi, %edi
  leal (%rdi,%rdi,4), %edi
  movl %edx, %r8d
  xorl %eax, %r8d
  subl %edi, %eax
  addl %r8d, %eax
  incl %edx
  decl %ecx
  jne .LBB1_6
.LBB1_7:
  retq

k_udiv7:
  movl %edi, %eax
  testl %esi, %esi
  je .LBB2_9
  pushq %rbp
  pushq %r15
  pushq %r14
  pushq %r12
  pushq %rbx
  movl %esi, %r10d
  andl $3, %r10d
  xorl %ebx, %ebx
  movabsq $2635249153617166336, %r11
  cmpl $4, %esi
  jb .LBB2_5
  andl $-4, %esi
  movl $-6, %ecx
  movl $-9, %edi
  movl $-12, %r8d
  xorl %ebx, %ebx
  movl $6, %r14d
  movl $3, %r15d
  xorl %r12d, %r12d
  movl $3, %ebp
.LBB2_3:
  movl %eax, %r9d
  movq %r9, %rdx
  mulxq %r11, %rdx, %rdx
  addl %r12d, %edx
  mulxq %r11, %rdx, %rdx
  addl %r15d, %edx
  mulxq %r11, %rdx, %rdx
  addl %r14d, %edx
  mulxq %r11, %rax, %rax
  addl %r12d, %eax
  addl $9, %eax
  addl $4, %ebx
  addl $12, %ecx
  addl $12, %edi
  addl $12, %r8d
  addl $-12, %ebp
  addl $12, %r12d
  addl $12, %r14d
  addl $12, %r15d
  cmpl %ebx, %esi
  jne .LBB2_3
  movq %r9, %rdx
  mulxq %r11, %rax, %rax
  addl %eax, %r8d
  movq %r8, %rdx
  mulxq %r11, %rax, %rax
  addl %eax, %edi
  movq %rdi, %rdx
  mulxq %r11, %rax, %rax
  addl %eax, %ecx
  movq %rcx, %rdx
  mulxq %r11, %rax, %rax
  subl %ebp, %eax
  testl %r10d, %r10d
  je .LBB2_8
.LBB2_5:
  leal (%rbx,%rbx,2), %esi
  movl $3, %ecx
  subl %esi, %ecx
.LBB2_6:
  movl %eax, %edx
  mulxq %r11, %rax, %rax
  addl %esi, %eax
  addl $-3, %ecx
  addl $3, %esi
  decl %r10d
  jne .LBB2_6
  mulxq %r11, %rax, %rax
  subl %ecx, %eax
.LBB2_8:
  popq %rbx
  popq %r12
  popq %r14
  popq %r15
  popq %rbp
.LBB2_9:
  retq

k_udr10:
  pushq %rbp
  pushq %r15
  pushq %r14
  pushq %r13
  pushq %r12
  pushq %rbx
  movl %edi, %eax
  movl %esi, -12(%rsp)
  testl %esi, %esi
  je .LBB3_8
  movl -12(%rsp), %esi
  movl %esi, %ecx
  andl $3, %ecx
  xorl %edx, %edx
  cmpl $4, %esi
  jb .LBB3_5
  andl $-4, -12(%rsp)
  xorl %edx, %edx
  movl $-47, %r10d
  movl $-15, %r9d
  movl $-4, %r8d
  movl $142, %edi
  movl $5, %ebx
  movl $1, %ebp
  movl $3435973837, %r11d
.LBB3_3:
  movl %eax, %r15d
  movl %eax, %eax
  movq %rax, -8(%rsp)
  imulq %r11, %rax
  shrq $35, %rax
  leal (%r15,%r15,8), %r14d
  leal (%r15,%r15,2), %r12d
  addl %edx, %r12d
  leal (%rax,%rax,8), %r13d
  leal (%r13,%r13,2), %r13d
  addl %eax, %r13d
  addl %eax, %r13d
  subl %r13d, %r12d
  imulq %r11, %r12
  shrq $35, %r12
  leal (%r12,%r12,8), %esi
  leal (%rsi,%rsi,2), %esi
  addl %r12d, %esi
  addl %r12d, %esi
  leal (%r14,%r14,2), %r13d
  addl %ebp, %r14d
  subl %esi, %r14d
  imull $87, %eax, %esi
  subl %esi, %r14d
  imulq %r11, %r14
  shrq $35, %r14
  leal (%r14,%r14,8), %esi
  leal (%rsi,%rsi,2), %esi
  addl %r14d, %r14d
  addl %r14d, %esi
  leal (%rbx,%r13), %r14d
  subl %esi, %r13d
  subl %esi, %r14d
  imull $87, %r12d, %esi
  subl %esi, %r13d
  subl %esi, %r14d
  imull $261, %eax, %eax
  subl %eax, %r13d
  addl %ebx, %r13d
  subl %eax, %r14d
  movq %r13, %rax
  imulq %r11, %r14
  shrq $35, %r14
  addl %r14d, %r14d
  leal (%r14,%r14,4), %esi
  subl %esi, %r13d
  imulq %r11, %rax
  shrq $35, %rax
  leal (%r13,%r13,2), %esi
  addl %edx, %eax
  addl %esi, %eax
  addl $3, %eax
  addl $4, %edx
  addl $52, %r10d
  addl $16, %r9d
  addl $4, %r8d
  addl $-160, %edi
  addl $52, %ebx
  addl $16, %ebp
  cmpl %edx, -12(%rsp)
  jne .LBB3_3
  leal (%r15,%r15,8), %esi
  leal (%rsi,%rsi,8), %eax
  leal (%rsi,%rsi,2), %ebx
  addl %ebx, %r10d
  addl %esi, %r9d
  leal (%r15,%r15,2), %esi
  addl %esi, %r8d
  movl $3435973837, %esi
  movq -8(%rsp), %r11
  imulq %rsi, %r11
  shrq $35, %r11
  leal (%r11,%r11,8), %ebx
  leal (%rbx,%rbx,2), %ebx
  addl %r11d, %ebx
  addl %r11d, %ebx
  subl %ebx, %r8d
  imulq %rsi, %r8
  shrq $35, %r8
  leal (%r8,%r8,8), %ebx
  leal (%rbx,%rbx,2), %ebx
  addl %r8d, %ebx
  addl %r8d, %ebx
  subl %ebx, %r9d
  imull $87, %r11d, %ebx
  subl %ebx, %r9d
  imulq %rsi, %r9
  shrq $35, %r9
  leal (%r9,%r9,8), %ebx
  leal (%rbx,%rbx,2), %ebx
  addl %r9d, %ebx
  addl %r9d, %ebx
  subl %ebx, %r10d
  imull $87, %r8d, %ebx
  subl %ebx, %r10d
  imull $261, %r11d, %ebx
  subl %ebx, %r10d
  imulq %rsi, %r10
  shrq $35, %r10
  leal (%r10,%r10,8), %esi
  leal (%rsi,%rsi,2), %esi
  addl %r10d, %r10d
  addl %r10d, %esi
  subl %esi, %eax
  imull $87, %r9d, %esi
  subl %esi, %eax
  imull $261, %r8d, %esi
  subl %esi, %eax
  imull $783, %r11d, %esi
  subl %esi, %eax
  subl %edi, %eax
  testl %ecx, %ecx
  je .LBB3_8
.LBB3_5:
  movl $1, %esi
  subl %edx, %esi
  movl $3435973837, %edi
.LBB3_6:
  movl %eax, %r9d
  movl %eax, %r8d
  movq %r8, %rax
  imulq %rdi, %rax
  shrq $35, %rax
  leal (%rax,%rax), %r10d
  leal (%r10,%r10,4), %r10d
  movl %r9d, %r11d
  subl %r10d, %r11d
  leal (%r11,%r11,2), %r10d
  addl %edx, %eax
  addl %r10d, %eax
  incl %edx
  decl %esi
  decl %ecx
  jne .LBB3_6
  leal (%r9,%r9,2), %eax
  movl $3435973837, %ecx
  imulq %r8, %rcx
  shrq $35, %rcx
  leal (%rcx,%rcx,8), %edx
  leal (%rdx,%rdx,2), %edx
  addl %ecx, %ecx
  addl %ecx, %edx
  subl %edx, %eax
  subl %esi, %eax
.LBB3_8:
  popq %rbx
  popq %r12
  popq %r13
  popq %r14
  popq %r15
  popq %rbp
  retq

k_sdr7:
  movl %edi, %eax
  testl %esi, %esi
  je .LBB4_9
  movl %esi, %ecx
  andl $3, %ecx
  xorl %edx, %edx
  cmpl $4, %esi
  jb .LBB4_6
  andl $-4, %esi
  negl %esi
  xorl %edx, %edx
  xorl %edi, %edi
.LBB4_3:
  cltq
  imulq $-1840700269, %rax, %r8
  shrq $32, %r8
  addl %eax, %r8d
  movl %r8d, %r9d
  shrl $31, %r9d
  sarl $2, %r8d
  addl %r9d, %r8d
  leal (,%r8,8), %r9d
  leal (%rdx,%r8), %r10d
  subl %r9d, %r8d
  addl %eax, %r8d
  leal (%r8,%r8,4), %eax
  addl %r10d, %eax
  cltq
  imulq $-1840700269, %rax, %r8
  shrq $32, %r8
  addl %eax, %r8d
  movl %r8d, %r9d
  shrl $31, %r9d
  sarl $2, %r8d
  addl %r9d, %r8d
  leal (,%r8,8), %r9d
  movl %r8d, %r10d
  subl %r9d, %r10d
  addl %eax, %r10d
  leal (%r10,%r10,4), %eax
  addl %r8d, %eax
  leal (%rdx,%rax), %r8d
  addl %edx, %eax
  decl %eax
  cltq
  imulq $-1840700269, %rax, %rax
  shrq $32, %rax
  addl %r8d, %eax
  decl %eax
  movl %eax, %r9d
  shrl $31, %r9d
  sarl $2, %eax
  addl %r9d, %eax
  leal (,%rax,8), %r9d
  movl %eax, %r10d
  subl %r9d, %r10d
  addl %r10d, %r8d
  decl %r8d
  leal (%r8,%r8,4), %r8d
  addl %eax, %r8d
  leal (%rdx,%r8), %eax
  addl %edx, %r8d
  addl $-2, %r8d
  leal 3(%rdi), %r10d
  movslq %r8d, %r8
  imulq $-1840700269, %r8, %r8
  shrq $32, %r8
  addl %eax, %r8d
  addl $-2, %r8d
  movl %r8d, %r9d
  shrl $31, %r9d
  sarl $2, %r8d
  addl %r9d, %r8d
  leal (,%r8,8), %r9d
  movl %r8d, %r11d
  subl %r9d, %r11d
  leal (%r11,%rax), %r9d
  addl $-2, %r9d
  leal (%r9,%r9,4), %r11d
  movl %r8d, %eax
  subl %r10d, %eax
  addl %r11d, %eax
  addl $4, %edi
  addl $-4, %edx
  cmpl %edx, %esi
  jne .LBB4_3
  leal (%r9,%r9,4), %eax
  addl %eax, %r8d
  leal (%rdx,%r8), %eax
  incl %eax
  testl %ecx, %ecx
  je .LBB4_9
  negl %edx
.LBB4_6:
  leal -1(%rdx), %esi
.LBB4_7:
  cltq
  imulq $-1840700269, %rax, %rdi
  shrq $32, %rdi
  addl %eax, %edi
  movl %edi, %r8d
  shrl $31, %r8d
  sarl $2, %edi
  addl %r8d, %edi
  leal (,%rdi,8), %r9d
  movl %edi, %r8d
  subl %r9d, %r8d
  addl %eax, %r8d
  leal (%r8,%r8,4), %r9d
  movl %edi, %eax
  subl %edx, %eax
  addl %r9d, %eax
  incl %edx
  incl %esi
  decl %ecx
  jne .LBB4_7
  leal (%r8,%r8,4), %eax
  addl %eax, %edi
  subl %esi, %edi
  movl %edi, %eax
.LBB4_9:
  retq

k_srem7:
  movl %edi, %eax
  testl %esi, %esi
  je .LBB5_6
  movl %esi, %ecx
  andl $3, %ecx
  xorl %edx, %edx
  cmpl $4, %esi
  jb .LBB5_5
  andl $-4, %esi
  xorl %edx, %edx
.LBB5_3:
  cltq
  imulq $-1840700269, %rax, %r8
  shrq $32, %r8
  addl %eax, %r8d
  movl %r8d, %edi
  shrl $31, %edi
  sarl $2, %r8d
  addl %edi, %r8d
  leal (,%r8,8), %edi
  subl %edi, %r8d
  addl %eax, %r8d
  movl %edx, %eax
  andl $65532, %eax
  movl %r8d, %r9d
  addl %eax, %r8d
  movl %eax, %edi
  addl %edi, %r9d
  addl $-30000, %r9d
  movslq %r9d, %r9
  imulq $-1840700269, %r9, %r9
  shrq $32, %r9
  addl %r8d, %r9d
  addl $-30000, %r9d
  movl %r9d, %r10d
  shrl $31, %r10d
  sarl $2, %r9d
  addl %r10d, %r9d
  leal (,%r9,8), %r10d
  subl %r10d, %r9d
  addl %r9d, %r8d
  addl $-30000, %r8d
  movl %r8d, %r9d
  addl %eax, %r8d
  addl %edi, %r9d
  addl $-29999, %r9d
  movslq %r9d, %r9
  imulq $-1840700269, %r9, %r9
  shrq $32, %r9
  addl %r8d, %r9d
  addl $-29999, %r9d
  movl %r9d, %r10d
  shrl $31, %r10d
  sarl $2, %r9d
  addl %r10d, %r9d
  leal (,%r9,8), %r10d
  subl %r10d, %r9d
  addl %r9d, %r8d
  addl $-29999, %r8d
  addl %r8d, %eax
  addl %edi, %r8d
  addl $-29998, %r8d
  movslq %r8d, %r8
  imulq $-1840700269, %r8, %r8
  shrq $32, %r8
  addl %eax, %r8d
  addl $-29998, %r8d
  movl %r8d, %r9d
  shrl $31, %r9d
  sarl $2, %r8d
  addl %r9d, %r8d
  leal (,%r8,8), %r9d
  subl %r9d, %r8d
  addl %r8d, %eax
  addl $-29998, %eax
  addl %edi, %eax
  addl $-29997, %eax
  addl $4, %edx
  cmpl %edx, %esi
  jne .LBB5_3
  testl %ecx, %ecx
  je .LBB5_6
.LBB5_5:
  cltq
  imulq $-1840700269, %rax, %rsi
  shrq $32, %rsi
  addl %eax, %esi
  movl %esi, %edi
  shrl $31, %edi
  sarl $2, %esi
  addl %edi, %esi
  leal (,%rsi,8), %edi
  subl %edi, %esi
  addl %eax, %esi
  movzwl %dx, %eax
  addl %esi, %eax
  addl $-30000, %eax
  incl %edx
  decl %ecx
  jne .LBB5_5
.LBB5_6:
  retq

k_srem16:
  movl %edi, %eax
  testl %esi, %esi
  je .LBB6_6
  movl %esi, %ecx
  andl $3, %ecx
  xorl %edx, %edx
  cmpl $4, %esi
  jb .LBB6_5
  andl $-4, %esi
  xorl %edx, %edx
.LBB6_3:
  leal 15(%rax), %edi
  testl %eax, %eax
  cmovnsl %eax, %edi
  andl $-16, %edi
  subl %edi, %eax
  leal (%rax,%rax,2), %edi
  movl %edx, %eax
  andl $252, %eax
  leal (%rax,%rdi), %r8d
  addl $-100, %r8d
  addl %eax, %edi
  addl $-85, %edi
  testl %r8d, %r8d
  cmovnsl %r8d, %edi
  andl $-16, %edi
  subl %edi, %r8d
  leal (%r8,%r8,2), %edi
  leal (%rax,%rdi), %r8d
  addl $-99, %r8d
  addl %eax, %edi
  addl $-84, %edi
  testl %r8d, %r8d
  cmovnsl %r8d, %edi
  andl $-16, %edi
  subl %edi, %r8d
  leal (%r8,%r8,2), %edi
  leal (%rax,%rdi), %r8d
  addl $-98, %r8d
  addl %eax, %edi
  addl $-83, %edi
  testl %r8d, %r8d
  cmovnsl %r8d, %edi
  andl $-16, %edi
  subl %edi, %r8d
  leal (%r8,%r8,2), %edi
  addl %edi, %eax
  addl $-97, %eax
  addl $4, %edx
  cmpl %edx, %esi
  jne .LBB6_3
  testl %ecx, %ecx
  je .LBB6_6
.LBB6_5:
  leal 15(%rax), %esi
  testl %eax, %eax
  cmovnsl %eax, %esi
  andl $-16, %esi
  subl %esi, %eax
  leal (%rax,%rax,2), %eax
  movzbl %dl, %esi
  addl %esi, %eax
  addl $-100, %eax
  incl %edx
  decl %ecx
  jne .LBB6_5
.LBB6_6:
  retq

k_sdr16:
  movl %edi, %eax
  testl %esi, %esi
  je .LBB7_6
  xorl %ecx, %ecx
  cmpl $1, %esi
  je .LBB7_5
  movl %esi, %edx
  andl $-2, %edx
  xorl %ecx, %ecx
.LBB7_3:
  leal 15(%rax), %edi
  testl %eax, %eax
  cmovnsl %eax, %edi
  movl %edi, %r8d
  sarl $4, %r8d
  andl $536870896, %edi
  subl %edi, %eax
  shll $3, %eax
  xorl %ecx, %r8d
  xorl %eax, %r8d
  leal 15(%r8), %eax
  testl %r8d, %r8d
  cmovnsl %r8d, %eax
  movl %eax, %edi
  sarl $4, %edi
  andl $536870896, %eax
  subl %eax, %r8d
  shll $3, %r8d
  leal 1(%rcx), %eax
  xorl %edi, %eax
  xorl %r8d, %eax
  addl $2, %ecx
  cmpl %edx, %ecx
  jne .LBB7_3
  testb $1, %sil
  je .LBB7_6
.LBB7_5:
  leal 15(%rax), %edx
  testl %eax, %eax
  cmovnsl %eax, %edx
  movl %edx, %esi
  sarl $4, %esi
  xorl %ecx, %esi
  andl $536870896, %edx
  subl %edx, %eax
  shll $3, %eax
  xorl %esi, %eax
.LBB7_6:
  retq

k_mul7:
  movl %edi, %eax
  testl %esi, %esi
  je .LBB8_7
  movl %esi, %ecx
  andl $7, %ecx
  xorl %edx, %edx
  cmpl $8, %esi
  jb .LBB8_5
  andl $-8, %esi
  xorl %edx, %edx
.LBB8_3:
  leal (,%rax,8), %edi
  subl %eax, %edi
  xorl %edx, %edi
  leal (,%rdi,8), %eax
  subl %edi, %eax
  leal 1(%rdx), %edi
  xorl %eax, %edi
  leal (,%rdi,8), %eax
  subl %edi, %eax
  leal 2(%rdx), %edi
  xorl %eax, %edi
  leal (,%rdi,8), %eax
  subl %edi, %eax
  leal 3(%rdx), %edi
  xorl %eax, %edi
  leal (,%rdi,8), %eax
  subl %edi, %eax
  leal 4(%rdx), %edi
  xorl %eax, %edi
  leal (,%rdi,8), %eax
  subl %edi, %eax
  leal 5(%rdx), %edi
  xorl %eax, %edi
  leal (,%rdi,8), %eax
  subl %edi, %eax
  leal 6(%rdx), %edi
  xorl %eax, %edi
  leal (,%rdi,8), %r8d
  subl %edi, %r8d
  leal 7(%rdx), %eax
  xorl %r8d, %eax
  addl $8, %edx
  cmpl %esi, %edx
  jne .LBB8_3
  testl %ecx, %ecx
  je .LBB8_7
.LBB8_5:
  movl %eax, %esi
.LBB8_6:
  leal (,%rsi,8), %eax
  subl %esi, %eax
  xorl %edx, %eax
  incl %edx
  movl %eax, %esi
  decl %ecx
  jne .LBB8_6
.LBB8_7:
  retq

k_mul11:
  movl %edi, %eax
  testl %esi, %esi
  je .LBB9_6
  movl %esi, %ecx
  andl $7, %ecx
  xorl %edx, %edx
  cmpl $8, %esi
  jb .LBB9_5
  andl $-8, %esi
  xorl %edx, %edx
.LBB9_3:
  leal (%rax,%rax,4), %edi
  leal (%rax,%rdi,2), %eax
  xorl %edx, %eax
  leal (%rax,%rax,4), %edi
  leal (%rax,%rdi,2), %eax
  leal 1(%rdx), %edi
  xorl %eax, %edi
  leal (%rdi,%rdi,4), %eax
  leal (%rdi,%rax,2), %eax
  leal 2(%rdx), %edi
  xorl %eax, %edi
  leal (%rdi,%rdi,4), %eax
  leal (%rdi,%rax,2), %eax
  leal 3(%rdx), %edi
  xorl %eax, %edi
  leal (%rdi,%rdi,4), %eax
  leal (%rdi,%rax,2), %eax
  leal 4(%rdx), %edi
  xorl %eax, %edi
  leal (%rdi,%rdi,4), %eax
  leal (%rdi,%rax,2), %eax
  leal 5(%rdx), %edi
  xorl %eax, %edi
  leal (%rdi,%rdi,4), %eax
  leal (%rdi,%rax,2), %eax
  leal 6(%rdx), %edi
  xorl %eax, %edi
  leal (%rdi,%rdi,4), %eax
  leal (%rdi,%rax,2), %edi
  leal 7(%rdx), %eax
  xorl %edi, %eax
  addl $8, %edx
  cmpl %esi, %edx
  jne .LBB9_3
  testl %ecx, %ecx
  je .LBB9_6
.LBB9_5:
  leal (%rax,%rax,4), %esi
  leal (%rax,%rsi,2), %eax
  xorl %edx, %eax
  incl %edx
  decl %ecx
  jne .LBB9_5
.LBB9_6:
  retq

k_mul17:
  movl %edi, %eax
  testl %esi, %esi
  je .LBB10_7
  movl %esi, %ecx
  andl $7, %ecx
  xorl %edx, %edx
  cmpl $8, %esi
  jb .LBB10_5
  andl $-8, %esi
  xorl %edx, %edx
.LBB10_3:
  movl %eax, %edi
  shll $4, %edi
  addl %eax, %edi
  xorl %edx, %edi
  movl %edi, %eax
  shll $4, %eax
  addl %edi, %eax
  leal 1(%rdx), %edi
  xorl %eax, %edi
  movl %edi, %eax
  shll $4, %eax
  addl %edi, %eax
  leal 2(%rdx), %edi
  xorl %eax, %edi
  movl %edi, %eax
  shll $4, %eax
  addl %edi, %eax
  leal 3(%rdx), %edi
  xorl %eax, %edi
  movl %edi, %eax
  shll $4, %eax
  addl %edi, %eax
  leal 4(%rdx), %edi
  xorl %eax, %edi
  movl %edi, %eax
  shll $4, %eax
  addl %edi, %eax
  leal 5(%rdx), %edi
  xorl %eax, %edi
  movl %edi, %eax
  shll $4, %eax
  addl %edi, %eax
  leal 6(%rdx), %edi
  xorl %eax, %edi
  movl %edi, %r8d
  shll $4, %r8d
  addl %edi, %r8d
  leal 7(%rdx), %eax
  xorl %r8d, %eax
  addl $8, %edx
  cmpl %esi, %edx
  jne .LBB10_3
  testl %ecx, %ecx
  je .LBB10_7
.LBB10_5:
  movl %eax, %esi
.LBB10_6:
  shll $4, %eax
  addl %esi, %eax
  xorl %edx, %eax
  incl %edx
  movl %eax, %esi
  decl %ecx
  jne .LBB10_6
.LBB10_7:
  retq

k_mul24:
  movl %edi, %eax
  testl %esi, %esi
  je .LBB11_6
  movl %esi, %ecx
  andl $7, %ecx
  xorl %edx, %edx
  cmpl $8, %esi
  jb .LBB11_5
  andl $-8, %esi
  xorl %edx, %edx
.LBB11_3:
  shll $3, %eax
  leal (%rax,%rax,2), %eax
  xorl %edx, %eax
  shll $3, %eax
  leal (%rax,%rax,2), %eax
  leal 1(%rdx), %edi
  xorl %eax, %edi
  shll $3, %edi
  leal (%rdi,%rdi,2), %eax
  leal 2(%rdx), %edi
  xorl %eax, %edi
  shll $3, %edi
  leal (%rdi,%rdi,2), %eax
  leal 3(%rdx), %edi
  xorl %eax, %edi
  shll $3, %edi
  leal (%rdi,%rdi,2), %eax
  leal 4(%rdx), %edi
  xorl %eax, %edi
  shll $3, %edi
  leal (%rdi,%rdi,2), %eax
  leal 5(%rdx), %edi
  xorl %eax, %edi
  shll $3, %edi
  leal (%rdi,%rdi,2), %eax
  leal 6(%rdx), %edi
  xorl %eax, %edi
  shll $3, %edi
  leal (%rdi,%rdi,2), %edi
  leal 7(%rdx), %eax
  xorl %edi, %eax
  addl $8, %edx
  cmpl %esi, %edx
  jne .LBB11_3
  testl %ecx, %ecx
  je .LBB11_6
.LBB11_5:
  shll $3, %eax
  leal (%rax,%rax,2), %eax
  xorl %edx, %eax
  incl %edx
  decl %ecx
  jne .LBB11_5
.LBB11_6:
  retq

k_mul45:
  movl %edi, %eax
  testl %esi, %esi
  je .LBB12_6
  movl %esi, %ecx
  andl $7, %ecx
  xorl %edx, %edx
  cmpl $8, %esi
  jb .LBB12_5
  andl $-8, %esi
  xorl %edx, %edx
.LBB12_3:
  leal (%rax,%rax,8), %eax
  leal (%rax,%rax,4), %eax
  xorl %edx, %eax
  leal (%rax,%rax,8), %eax
  leal (%rax,%rax,4), %eax
  leal 1(%rdx), %edi
  xorl %eax, %edi
  leal (%rdi,%rdi,8), %eax
  leal (%rax,%rax,4), %eax
  leal 2(%rdx), %edi
  xorl %eax, %edi
  leal (%rdi,%rdi,8), %eax
  leal (%rax,%rax,4), %eax
  leal 3(%rdx), %edi
  xorl %eax, %edi
  leal (%rdi,%rdi,8), %eax
  leal (%rax,%rax,4), %eax
  leal 4(%rdx), %edi
  xorl %eax, %edi
  leal (%rdi,%rdi,8), %eax
  leal (%rax,%rax,4), %eax
  leal 5(%rdx), %edi
  xorl %eax, %edi
  leal (%rdi,%rdi,8), %eax
  leal (%rax,%rax,4), %eax
  leal 6(%rdx), %edi
  xorl %eax, %edi
  leal (%rdi,%rdi,8), %eax
  leal (%rax,%rax,4), %edi
  leal 7(%rdx), %eax
  xorl %edi, %eax
  addl $8, %edx
  cmpl %esi, %edx
  jne .LBB12_3
  testl %ecx, %ecx
  je .LBB12_6
.LBB12_5:
  leal (%rax,%rax,8), %eax
  leal (%rax,%rax,4), %eax
  xorl %edx, %eax
  incl %edx
  decl %ecx
  jne .LBB12_5
.LBB12_6:
  retq

k_mulm3:
  movl %edi, %eax
  testl %esi, %esi
  je .LBB13_6
  movl %esi, %ecx
  andl $7, %ecx
  xorl %edx, %edx
  cmpl $8, %esi
  jb .LBB13_5
  andl $-8, %esi
  xorl %edx, %edx
.LBB13_3:
  leal (%rax,%rax,2), %eax
  negl %eax
  xorl %edx, %eax
  leal (%rax,%rax,2), %eax
  negl %eax
  leal 1(%rdx), %edi
  xorl %eax, %edi
  leal (%rdi,%rdi,2), %eax
  negl %eax
  leal 2(%rdx), %edi
  xorl %eax, %edi
  leal (%rdi,%rdi,2), %eax
  negl %eax
  leal 3(%rdx), %edi
  xorl %eax, %edi
  leal (%rdi,%rdi,2), %eax
  negl %eax
  leal 4(%rdx), %edi
  xorl %eax, %edi
  leal (%rdi,%rdi,2), %eax
  negl %eax
  leal 5(%rdx), %edi
  xorl %eax, %edi
  leal (%rdi,%rdi,2), %eax
  negl %eax
  leal 6(%rdx), %edi
  xorl %eax, %edi
  leal (%rdi,%rdi,2), %edi
  negl %edi
  leal 7(%rdx), %eax
  xorl %edi, %eax
  addl $8, %edx
  cmpl %esi, %edx
  jne .LBB13_3
  testl %ecx, %ecx
  je .LBB13_6
.LBB13_5:
  leal (%rax,%rax,2), %eax
  negl %eax
  xorl %edx, %eax
  incl %edx
  decl %ecx
  jne .LBB13_5
.LBB13_6:
  retq

k_mul1000:
  movl %edi, %eax
  testl %esi, %esi
  je .LBB14_6
  movl %esi, %ecx
  andl $7, %ecx
  xorl %edx, %edx
  cmpl $8, %esi
  jb .LBB14_5
  andl $-8, %esi
  xorl %edx, %edx
.LBB14_3:
  imull $1000, %eax, %eax
  xorl %edx, %eax
  imull $1000, %eax, %eax
  leal 1(%rdx), %edi
  xorl %eax, %edi
  imull $1000, %edi, %eax
  leal 2(%rdx), %edi
  xorl %eax, %edi
  imull $1000, %edi, %eax
  leal 3(%rdx), %edi
  xorl %eax, %edi
  imull $1000, %edi, %eax
  leal 4(%rdx), %edi
  xorl %eax, %edi
  imull $1000, %edi, %eax
  leal 5(%rdx), %edi
  xorl %eax, %edi
  imull $1000, %edi, %eax
  leal 6(%rdx), %edi
  xorl %eax, %edi
  imull $1000, %edi, %edi
  leal 7(%rdx), %eax
  xorl %edi, %eax
  addl $8, %edx
  cmpl %esi, %edx
  jne .LBB14_3
  testl %ecx, %ecx
  je .LBB14_6
.LBB14_5:
  imull $1000, %eax, %eax
  xorl %edx, %eax
  incl %edx
  decl %ecx
  jne .LBB14_5
.LBB14_6:
  retq

k_fnv:
  movl %edi, %eax
  testl %esi, %esi
  je .LBB15_6
  movl %esi, %ecx
  andl $7, %ecx
  xorl %edx, %edx
  cmpl $8, %esi
  jb .LBB15_5
  andl $-8, %esi
  xorl %edx, %edx
.LBB15_3:
  movl %edx, %edi
  andl $248, %edi
  xorl %edi, %eax
  imull $16777619, %eax, %eax
  leal 1(%rdi), %r8d
  xorl %eax, %r8d
  imull $16777619, %r8d, %eax
  leal 2(%rdi), %r8d
  xorl %eax, %r8d
  imull $16777619, %r8d, %eax
  leal 3(%rdi), %r8d
  xorl %eax, %r8d
  imull $16777619, %r8d, %eax
  leal 4(%rdi), %r8d
  xorl %eax, %r8d
  imull $16777619, %r8d, %eax
  leal 5(%rdi), %r8d
  xorl %eax, %r8d
  imull $16777619, %r8d, %eax
  leal 6(%rdi), %r8d
  xorl %eax, %r8d
  imull $16777619, %r8d, %eax
  orl $7, %edi
  xorl %eax, %edi
  imull $16777619, %edi, %eax
  addl $8, %edx
  cmpl %edx, %esi
  jne .LBB15_3
  testl %ecx, %ecx
  je .LBB15_6
.LBB15_5:
  movzbl %dl, %esi
  xorl %eax, %esi
  imull $16777619, %esi, %eax
  incl %edx
  decl %ecx
  jne .LBB15_5
.LBB15_6:
  retq

k_digits:
  testl %esi, %esi
  je .LBB16_1
  xorl %ecx, %ecx
  movl $0, %eax
  cmpl $1, %esi
  je .LBB16_10
  pushq %rbx
  movl %esi, %edx
  andl $-2, %edx
  xorl %ecx, %ecx
  movl $3435973837, %r8d
  xorl %eax, %eax
  jmp .LBB16_4
.LBB16_8:
  addl $2, %ecx
  cmpl %edx, %ecx
  je .LBB16_9
.LBB16_4:
  movl %ecx, %r9d
  addl %edi, %r9d
  je .LBB16_6
.LBB16_5:
  movl %r9d, %r10d
  imulq %r8, %r10
  shrq $35, %r10
  leal (%r10,%r10), %r11d
  leal (%r11,%r11,4), %r11d
  movl %r9d, %ebx
  subl %r11d, %ebx
  addl %ebx, %eax
  cmpl $9, %r9d
  movl %r10d, %r9d
  ja .LBB16_5
.LBB16_6:
  leal (%rcx,%rdi), %r9d
  incl %r9d
  je .LBB16_8
.LBB16_7:
  movl %r9d, %r10d
  imulq %r8, %r10
  shrq $35, %r10
  leal (%r10,%r10), %r11d
  leal (%r11,%r11,4), %r11d
  movl %r9d, %ebx
  subl %r11d, %ebx
  addl %ebx, %eax
  cmpl $9, %r9d
  movl %r10d, %r9d
  ja .LBB16_7
  jmp .LBB16_8
.LBB16_1:
  xorl %eax, %eax
  retq
.LBB16_9:
  testb $1, %sil
  popq %rbx
  je .LBB16_13
.LBB16_10:
  addl %edi, %ecx
  je .LBB16_13
  movl $3435973837, %edx
.LBB16_12:
  movl %ecx, %esi
  imulq %rdx, %rsi
  shrq $35, %rsi
  leal (%rsi,%rsi), %edi
  leal (%rdi,%rdi,4), %edi
  movl %ecx, %r8d
  subl %edi, %r8d
  addl %r8d, %eax
  cmpl $9, %ecx
  movl %esi, %ecx
  ja .LBB16_12
.LBB16_13:
  retq

.LCPI17_0:
main:
  pushq %rbp
  pushq %r15
  pushq %r14
  pushq %r13
  pushq %r12
  pushq %rbx
  subq $40, %rsp
  movl $20000000, %eax
  movq %rax, 16(%rsp)
  cmpl $2, %edi
  jl .LBB17_2
  movq 8(%rsi), %rdi
  xorl %esi, %esi
  xorl %edx, %edx
  callq strtoul@PLT
  movq %rax, 16(%rsp)
.LBB17_2:
  movl 16(%rsp), %eax
  vcvtsi2sd %rax, %xmm15, %xmm0
  vmovsd %xmm0, 32(%rsp)
  leaq .L__const.main.ks(%rip), %r12
  movq %rsp, %r15
  xorl %ebx, %ebx
  xorl %r14d, %r14d
.LBB17_3:
  movl $1, %edi
  movq %r15, %rsi
  callq clock_gettime@PLT
  vcvtsi2sdq (%rsp), %xmm15, %xmm0
  vcvtsi2sdq 8(%rsp), %xmm15, %xmm2
  vmovsd .LCPI17_0(%rip), %xmm1
  vfmadd231sd %xmm0, %xmm1, %xmm2
  vmovsd %xmm2, 24(%rsp)
  movl 16(%rbx,%r12), %edi
  movq 16(%rsp), %rsi
  callq *8(%rbx,%r12)
  movl %eax, %ebp
  movl $1, %edi
  movq %r15, %rsi
  callq clock_gettime@PLT
  vcvtsi2sdq (%rsp), %xmm15, %xmm0
  vcvtsi2sdq 8(%rsp), %xmm15, %xmm1
  vfmadd231sd .LCPI17_0(%rip), %xmm0, %xmm1
  vsubsd 24(%rsp), %xmm1, %xmm0
  vdivsd 32(%rsp), %xmm0, %xmm0
  movl %r14d, %eax
  shll $5, %eax
  subl %r14d, %eax
  movl %eax, %r14d
  addl %ebp, %r14d
  movq stderr@GOTPCREL(%rip), %rax
  movq (%rax), %rdi
  movq (%rbx,%r12), %r13
  leaq .L.str.17(%rip), %rsi
  movq %r13, %rdx
  movb $1, %al
  callq fprintf@PLT
  leaq .L.str.18(%rip), %rdi
  movq %r13, %rsi
  movl %ebp, %edx
  xorl %eax, %eax
  callq printf@PLT
  addq $24, %rbx
  cmpq $384, %rbx
  jne .LBB17_3
  movq 16(%rsp), %rbx
  shrl $4, %ebx
  movq %rsp, %rsi
  movl $1, %edi
  callq clock_gettime@PLT
  vcvtsi2sdq (%rsp), %xmm15, %xmm0
  vcvtsi2sdq 8(%rsp), %xmm15, %xmm2
  vmovsd .LCPI17_0(%rip), %xmm1
  vfmadd231sd %xmm0, %xmm1, %xmm2
  vmovsd %xmm2, 24(%rsp)
  movl $1234567, %edi
  movl %ebx, %esi
  callq k_digits
  movl %eax, %ebp
  movq %rsp, %rsi
  movl $1, %edi
  callq clock_gettime@PLT
  vcvtsi2sdq (%rsp), %xmm15, %xmm0
  vcvtsi2sdq 8(%rsp), %xmm15, %xmm1
  vfmadd231sd .LCPI17_0(%rip), %xmm0, %xmm1
  vsubsd 24(%rsp), %xmm1, %xmm0
  movl %r14d, %r15d
  shll $5, %r15d
  subl %r14d, %r15d
  addl %ebp, %r15d
  movq stderr@GOTPCREL(%rip), %rax
  movq (%rax), %rdi
  vcvtsi2sd %ebx, %xmm15, %xmm1
  vdivsd %xmm1, %xmm0, %xmm0
  leaq .L.str.17(%rip), %rsi
  leaq .L.str.16(%rip), %rbx
  movq %rbx, %rdx
  movb $1, %al
  callq fprintf@PLT
  leaq .L.str.18(%rip), %rdi
  movq %rbx, %rsi
  movl %ebp, %edx
  xorl %eax, %eax
  callq printf@PLT
  leaq .L.str.19(%rip), %rdi
  movl %r15d, %esi
  xorl %eax, %eax
  callq printf@PLT
  xorl %eax, %eax
  addq $40, %rsp
  popq %rbx
  popq %r12
  popq %r13
  popq %r14
  popq %r15
  popq %rbp
  retq

.L.str:

.L.str.1:

.L.str.2:

.L.str.3:

.L.str.4:

.L.str.5:

.L.str.6:

.L.str.7:

.L.str.8:

.L.str.9:

.L.str.10:

.L.str.11:

.L.str.12:

.L.str.13:

.L.str.14:

.L.str.15:

.L.str.16:

.L__const.main.ks:

.L.str.17:

.L.str.18:

.L.str.19:

