k_urem7:
  movl %edi, %eax
  testl %esi, %esi
  je .LBB0_8
  cmpl $8, %esi
  jb .LBB0_5
  pushq %rbp
  pushq %r15
  pushq %r14
  pushq %r13
  pushq %r12
  pushq %rbx
  movl %esi, -80(%rsp)
  movl %esi, %r13d
  shrl $3, %r13d
  movl $-36, -108(%rsp)
  movl $-35, %ebx
  movl $-33, %r11d
  movl $-30, %r10d
  movl $-26, %r9d
  movl $-21, %r14d
  movl $-15, %r15d
  movl $-8, %r12d
  xorl %edx, %edx
  movl $21, %esi
  movl $15, %ecx
  movl $10, %ebp
  movl $6, %r8d
  movq %r8, -88(%rsp)
  movl $3, %r8d
  movq %r8, -96(%rsp)
  movl $1, %r8d
  movq %r8, -104(%rsp)
.LBB0_3:
  movl %r13d, -76(%rsp)
  movq %r12, -56(%rsp)
  movq %r15, -48(%rsp)
  movq %r14, -40(%rsp)
  movq %r9, -32(%rsp)
  movq %r10, -24(%rsp)
  movq %r11, -16(%rsp)
  movq %rbx, -8(%rsp)
  movl %eax, %edi
  leal (%rdi,%rdx), %ebx
  movl %eax, %eax
  movq %rax, -72(%rsp)
  imulq $613566757, %rax, %r9
  shrq $32, %r9
  movl %edi, %eax
  subl %r9d, %eax
  shrl %eax
  addl %r9d, %eax
  shrl $2, %eax
  leal (,%rax,8), %r9d
  movl %r9d, %r11d
  subl %eax, %r11d
  subl %r11d, %ebx
  imulq $613566757, %rbx, %r10
  shrq $32, %r10
  subl %r10d, %ebx
  shrl %ebx
  addl %r10d, %ebx
  shrl $2, %ebx
  leal (,%rbx,8), %r10d
  subl %r9d, %eax
  leal (%rdi,%rsi), %r9d
  subl %r11d, %r9d
  leal (%rdi,%rcx), %r14d
  subl %r11d, %r14d
  movq %rbp, -64(%rsp)
  leal (%rdi,%rbp), %r15d
  subl %r11d, %r15d
  movq -88(%rsp), %r8
  leal (%rdi,%r8), %r13d
  subl %r11d, %r13d
  movq -96(%rsp), %r8
  leal (%rdi,%r8), %r12d
  subl %r11d, %r12d
  movq -104(%rsp), %r8
  leal (%rdi,%r8), %ebp
  subl %r11d, %ebp
  movl %ebx, %r11d
  subl %r10d, %r11d
  subl %ebx, %r10d
  subl %r10d, %ebp
  imulq $613566757, %rbp, %rbx
  shrq $32, %rbx
  subl %ebx, %ebp
  shrl %ebp
  addl %ebx, %ebp
  shrl $2, %ebp
  leal (,%rbp,8), %ebx
  subl %ebp, %ebx
  subl %ebx, %r12d
  subl %r10d, %r12d
  imulq $613566757, %r12, %rbp
  shrq $32, %rbp
  subl %ebp, %r12d
  shrl %r12d
  addl %ebp, %r12d
  shrl $2, %r12d
  leal (,%r12,8), %ebp
  subl %r12d, %ebp
  subl %ebp, %r13d
  subl %ebx, %r13d
  subl %r10d, %r13d
  imulq $613566757, %r13, %r12
  shrq $32, %r12
  subl %r12d, %r13d
  shrl %r13d
  addl %r12d, %r13d
  shrl $2, %r13d
  leal (,%r13,8), %r12d
  subl %r13d, %r12d
  subl %r12d, %r15d
  subl %ebp, %r15d
  subl %ebx, %r15d
  subl %r10d, %r15d
  imulq $613566757, %r15, %r13
  shrq $32, %r13
  subl %r13d, %r15d
  shrl %r15d
  addl %r13d, %r15d
  shrl $2, %r15d
  leal (,%r15,8), %r13d
  subl %r15d, %r13d
  subl %r13d, %r14d
  subl %r12d, %r14d
  subl %ebp, %r14d
  subl %ebx, %r14d
  subl %r10d, %r14d
  imulq $613566757, %r14, %r15
  shrq $32, %r15
  subl %r15d, %r14d
  shrl %r14d
  addl %r15d, %r14d
  shrl $2, %r14d
  leal (,%r14,8), %r15d
  subl %r14d, %r15d
  movq -40(%rsp), %r14
  subl %r15d, %r9d
  subl %r13d, %r9d
  subl %r12d, %r9d
  subl %ebp, %r9d
  subl %ebx, %r9d
  subl %r10d, %r9d
  imulq $613566757, %r9, %r10
  shrq $32, %r10
  subl %r10d, %r9d
  shrl %r9d
  addl %r10d, %r9d
  shrl $2, %r9d
  leal (,%r9,8), %r10d
  subl %r10d, %r9d
  movq -24(%rsp), %r10
  movq %rdi, %r8
  addl %edi, %r9d
  subl %r15d, %r9d
  movq -48(%rsp), %r15
  subl %r13d, %r9d
  movl -76(%rsp), %r13d
  subl %r12d, %r9d
  movq -56(%rsp), %r12
  subl %ebp, %r9d
  movl -108(%rsp), %edi
  subl %ebx, %r9d
  movq -8(%rsp), %rbx
  addl %edi, %eax
  addl %r11d, %eax
  movq -16(%rsp), %r11
  addl %r9d, %eax
  addl $64, %eax
  movq -32(%rsp), %r9
  addl $64, %edi
  movl %edi, -108(%rsp)
  addl $56, %ebx
  addl $48, %r11d
  addl $40, %r10d
  addl $32, %r9d
  addl $24, %r14d
  addl $16, %r15d
  addl $8, %r12d
  addl $8, %edx
  addl $56, %esi
  addl $48, %ecx
  movq -64(%rsp), %rbp
  addl $40, %ebp
  movq -88(%rsp), %rdi
  addl $32, %edi
  movq %rdi, -88(%rsp)
  movq -96(%rsp), %rdi
  addl $24, %edi
  movq %rdi, -96(%rsp)
  movq -104(%rsp), %rdi
  addl $16, %edi
  movq %rdi, -104(%rsp)
  decl %r13d
  jne .LBB0_3
  imulq $613566757, -72(%rsp), %rax
  shrq $32, %rax
  movq %r8, %r13
  movl %r13d, %edx
  subl %eax, %edx
  shrl %edx
  addl %eax, %edx
  shrl $2, %edx
  leal (,%rdx,8), %ecx
  movl %edx, %eax
  subl %ecx, %eax
  addl %r13d, %ebx
  subl %edx, %ecx
  subl %ecx, %ebx
  addl %r13d, %r11d
  subl %ecx, %r11d
  addl %r13d, %r10d
  subl %ecx, %r10d
  addl %r13d, %r9d
  subl %ecx, %r9d
  addl %r13d, %r14d
  subl %ecx, %r14d
  addl %r13d, %r15d
  subl %ecx, %r15d
  addl %r13d, %r12d
  subl %ecx, %r12d
  imulq $613566757, %r12, %rcx
  shrq $32, %rcx
  subl %ecx, %r12d
  shrl %r12d
  addl %ecx, %r12d
  shrl $2, %r12d
  leal (,%r12,8), %ecx
  subl %r12d, %ecx
  subl %ecx, %r15d
  imulq $613566757, %r15, %rdx
  shrq $32, %rdx
  subl %edx, %r15d
  shrl %r15d
  addl %edx, %r15d
  shrl $2, %r15d
  leal (,%r15,8), %edx
  subl %r15d, %edx
  subl %edx, %r14d
  subl %ecx, %r14d
  imulq $613566757, %r14, %rsi
  shrq $32, %rsi
  subl %esi, %r14d
  shrl %r14d
  addl %esi, %r14d
  shrl $2, %r14d
  leal (,%r14,8), %esi
  subl %r14d, %esi
  subl %esi, %r9d
  subl %edx, %r9d
  subl %ecx, %r9d
  imulq $613566757, %r9, %r8
  shrq $32, %r8
  subl %r8d, %r9d
  shrl %r9d
  addl %r8d, %r9d
  shrl $2, %r9d
  leal (,%r9,8), %r8d
  subl %r9d, %r8d
  subl %r8d, %r10d
  subl %esi, %r10d
  subl %edx, %r10d
  subl %ecx, %r10d
  imulq $613566757, %r10, %r9
  shrq $32, %r9
  subl %r9d, %r10d
  shrl %r10d
  addl %r9d, %r10d
  shrl $2, %r10d
  leal (,%r10,8), %r9d
  subl %r10d, %r9d
  subl %r9d, %r11d
  subl %r8d, %r11d
  subl %esi, %r11d
  subl %edx, %r11d
  subl %ecx, %r11d
  imulq $613566757, %r11, %r10
  shrq $32, %r10
  subl %r10d, %r11d
  shrl %r11d
  addl %r10d, %r11d
  shrl $2, %r11d
  leal (,%r11,8), %r10d
  subl %r11d, %r10d
  subl %r10d, %ebx
  subl %r9d, %ebx
  subl %r8d, %ebx
  subl %esi, %ebx
  subl %edx, %ebx
  subl %ecx, %ebx
  imulq $613566757, %rbx, %r11
  shrq $32, %r11
  subl %r11d, %ebx
  shrl %ebx
  addl %r11d, %ebx
  shrl $2, %ebx
  leal (,%rbx,8), %r11d
  subl %r11d, %ebx
  addl %r13d, %ebx
  subl %r10d, %ebx
  subl %r9d, %ebx
  subl %r8d, %ebx
  subl %esi, %ebx
  subl %edx, %ebx
  subl %ecx, %ebx
  addl -108(%rsp), %eax
  addl %ebx, %eax
  movl -80(%rsp), %esi
  popq %rbx
  popq %r12
  popq %r13
  popq %r14
  popq %r15
  popq %rbp
.LBB0_5:
  movl %esi, %ecx
  andl $-8, %ecx
  cmpl %esi, %ecx
  jae .LBB0_8
.LBB0_6:
  movl %eax, %edx
  movl %eax, %r8d
  imulq $613566757, %r8, %rdi
  shrq $32, %rdi
  subl %edi, %eax
  shrl %eax
  addl %edi, %eax
  shrl $2, %eax
  leal (,%rax,8), %edi
  subl %edi, %eax
  addl %ecx, %eax
  addl %edx, %eax
  incl %ecx
  cmpl %ecx, %esi
  jne .LBB0_6
  imulq $613566757, %r8, %rax
  shrq $32, %rax
  movl %edx, %esi
  subl %eax, %esi
  shrl %esi
  addl %eax, %esi
  shrl $2, %esi
  leal (,%rsi,8), %eax
  subl %eax, %esi
  addl %edx, %esi
  leal (%rcx,%rsi), %eax
  decl %eax
.LBB0_8:
  retq

k_urem10:
  movl %edi, %eax
  testl %esi, %esi
  je .LBB1_7
  cmpl $8, %esi
  jb .LBB1_4
  movl %esi, %ecx
  shrl $3, %ecx
  movl $7, %edx
  movl $3435973837, %edi
.LBB1_3:
  movl %eax, %r8d
  imulq %rdi, %r8
  shrq $35, %r8
  addl %r8d, %r8d
  leal (%r8,%r8,4), %r8d
  leal -7(%rdx), %r9d
  xorl %eax, %r9d
  subl %r8d, %eax
  addl %r9d, %eax
  movq %rax, %r8
  imulq %rdi, %r8
  shrq $35, %r8
  addl %r8d, %r8d
  leal (%r8,%r8,4), %r8d
  leal -6(%rdx), %r9d
  xorl %eax, %r9d
  subl %r8d, %eax
  addl %r9d, %eax
  movq %rax, %r8
  imulq %rdi, %r8
  shrq $35, %r8
  addl %r8d, %r8d
  leal (%r8,%r8,4), %r8d
  leal -5(%rdx), %r9d
  xorl %eax, %r9d
  subl %r8d, %eax
  addl %r9d, %eax
  movq %rax, %r8
  imulq %rdi, %r8
  shrq $35, %r8
  addl %r8d, %r8d
  leal (%r8,%r8,4), %r8d
  leal -4(%rdx), %r9d
  xorl %eax, %r9d
  subl %r8d, %eax
  addl %r9d, %eax
  movq %rax, %r8
  imulq %rdi, %r8
  shrq $35, %r8
  addl %r8d, %r8d
  leal (%r8,%r8,4), %r8d
  leal -3(%rdx), %r9d
  xorl %eax, %r9d
  subl %r8d, %eax
  addl %r9d, %eax
  movq %rax, %r8
  imulq %rdi, %r8
  shrq $35, %r8
  addl %r8d, %r8d
  leal (%r8,%r8,4), %r8d
  leal -2(%rdx), %r9d
  xorl %eax, %r9d
  subl %r8d, %eax
  addl %r9d, %eax
  movq %rax, %r8
  imulq %rdi, %r8
  shrq $35, %r8
  addl %r8d, %r8d
  leal (%r8,%r8,4), %r8d
  leal -1(%rdx), %r9d
  xorl %eax, %r9d
  subl %r8d, %eax
  addl %r9d, %eax
  movq %rax, %r8
  imulq %rdi, %r8
  shrq $35, %r8
  addl %r8d, %r8d
  leal (%r8,%r8,4), %r8d
  movl %edx, %r9d
  xorl %eax, %r9d
  subl %r8d, %eax
  addl %r9d, %eax
  addl $8, %edx
  decl %ecx
  jne .LBB1_3
.LBB1_4:
  movl %esi, %ecx
  andl $-8, %ecx
  cmpl %esi, %ecx
  jae .LBB1_7
  movl $3435973837, %edx
.LBB1_6:
  movl %eax, %edi
  imulq %rdx, %rdi
  shrq $35, %rdi
  addl %edi, %edi
  leal (%rdi,%rdi,4), %edi
  movl %ecx, %r8d
  xorl %eax, %r8d
  subl %edi, %eax
  addl %r8d, %eax
  incl %ecx
  cmpl %ecx, %esi
  jne .LBB1_6
.LBB1_7:
  retq

k_udiv7:
  movl %edi, %eax
  testl %esi, %esi
  je .LBB2_9
  cmpl $8, %esi
  jb .LBB2_5
  pushq %rbp
  pushq %r15
  pushq %r14
  pushq %r13
  pushq %r12
  pushq %rbx
  movl %esi, -16(%rsp)
  movl %esi, %ebp
  shrl $3, %ebp
  xorl %r14d, %r14d
  movl $-6, %ecx
  movq %rcx, -24(%rsp)
  movl $-9, %ecx
  movq %rcx, -32(%rsp)
  movl $-12, %r8d
  movl $-15, %r10d
  movl $-18, %r11d
  movl $-21, %ecx
  movl $-24, %ebx
  movl $3, -36(%rsp)
  movl $18, %r15d
  movl $15, %r12d
  movl $12, %r13d
  movl $9, %edx
  movl $6, %esi
  movl $3, %edi
.LBB2_3:
  movl %eax, %r9d
  movq %r9, -8(%rsp)
  imulq $613566757, %r9, %r9
  shrq $32, %r9
  movl %eax, -12(%rsp)
  subl %r9d, %eax
  shrl %eax
  addl %r9d, %eax
  shrl $2, %eax
  addl %r14d, %eax
  imulq $613566757, %rax, %r9
  shrq $32, %r9
  subl %r9d, %eax
  shrl %eax
  addl %r9d, %eax
  shrl $2, %eax
  addl %edi, %eax
  imulq $613566757, %rax, %r9
  shrq $32, %r9
  subl %r9d, %eax
  shrl %eax
  addl %r9d, %eax
  shrl $2, %eax
  addl %esi, %eax
  imulq $613566757, %rax, %r9
  shrq $32, %r9
  subl %r9d, %eax
  shrl %eax
  addl %r9d, %eax
  shrl $2, %eax
  addl %edx, %eax
  imulq $613566757, %rax, %r9
  shrq $32, %r9
  subl %r9d, %eax
  shrl %eax
  addl %r9d, %eax
  shrl $2, %eax
  addl %r13d, %eax
  imulq $613566757, %rax, %r9
  shrq $32, %r9
  subl %r9d, %eax
  shrl %eax
  addl %r9d, %eax
  shrl $2, %eax
  addl %r12d, %eax
  imulq $613566757, %rax, %r9
  shrq $32, %r9
  subl %r9d, %eax
  shrl %eax
  addl %r9d, %eax
  shrl $2, %eax
  addl %r15d, %eax
  imulq $613566757, %rax, %r9
  shrq $32, %r9
  subl %r9d, %eax
  shrl %eax
  addl %r9d, %eax
  shrl $2, %eax
  addl %r14d, %eax
  addl $21, %eax
  addl $24, %r14d
  addl $24, %edi
  movq -24(%rsp), %r9
  addl $24, %r9d
  movq %r9, -24(%rsp)
  movq %r14, %r9
  movl %edi, %r14d
  movl %esi, %edi
  movl %edx, %esi
  movl %r13d, %edx
  movl %r12d, %r13d
  movl %r15d, %r12d
  movl %ebp, %r15d
  movq %rbx, %rbp
  movq %rcx, %rbx
  movq %r11, %rcx
  movq %r10, %r11
  movq %r8, %r10
  movq -32(%rsp), %r8
  addl $24, %r8d
  movq %r8, -32(%rsp)
  movq %r10, %r8
  movq %r11, %r10
  movq %rcx, %r11
  movq %rbx, %rcx
  movq %rbp, %rbx
  movl %r15d, %ebp
  movl %r12d, %r15d
  movl %r13d, %r12d
  movl %edx, %r13d
  movl %esi, %edx
  movl %edi, %esi
  movl %r14d, %edi
  movq %r9, %r14
  addl $24, %r8d
  addl $24, %r10d
  addl $24, %r11d
  addl $24, %ecx
  addl $24, %ebx
  addl $-24, -36(%rsp)
  addl $24, %r15d
  addl $24, %r12d
  addl $24, %r13d
  addl $24, %edx
  addl $24, %esi
  decl %ebp
  jne .LBB2_3
  imulq $613566757, -8(%rsp), %rax
  shrq $32, %rax
  movl -12(%rsp), %edx
  subl %eax, %edx
  shrl %edx
  addl %eax, %edx
  shrl $2, %edx
  addl %edx, %ebx
  imulq $613566757, %rbx, %rax
  shrq $32, %rax
  subl %eax, %ebx
  shrl %ebx
  addl %eax, %ebx
  shrl $2, %ebx
  addl %ebx, %ecx
  imulq $613566757, %rcx, %rax
  shrq $32, %rax
  subl %eax, %ecx
  shrl %ecx
  addl %eax, %ecx
  shrl $2, %ecx
  addl %ecx, %r11d
  imulq $613566757, %r11, %rax
  shrq $32, %rax
  subl %eax, %r11d
  shrl %r11d
  addl %eax, %r11d
  shrl $2, %r11d
  addl %r11d, %r10d
  imulq $613566757, %r10, %rax
  shrq $32, %rax
  subl %eax, %r10d
  shrl %r10d
  addl %eax, %r10d
  shrl $2, %r10d
  addl %r10d, %r8d
  imulq $613566757, %r8, %rax
  shrq $32, %rax
  subl %eax, %r8d
  shrl %r8d
  addl %eax, %r8d
  shrl $2, %r8d
  movq -32(%rsp), %rcx
  addl %r8d, %ecx
  imulq $613566757, %rcx, %rax
  shrq $32, %rax
  subl %eax, %ecx
  shrl %ecx
  addl %eax, %ecx
  shrl $2, %ecx
  movq -24(%rsp), %rdx
  addl %ecx, %edx
  imulq $613566757, %rdx, %rax
  shrq $32, %rax
  subl %eax, %edx
  shrl %edx
  addl %eax, %edx
  shrl $2, %edx
  subl -36(%rsp), %edx
  movl %edx, %eax
  movl -16(%rsp), %esi
  popq %rbx
  popq %r12
  popq %r13
  popq %r14
  popq %r15
  popq %rbp
.LBB2_5:
  movl %esi, %ecx
  andl $-8, %ecx
  subl %ecx, %esi
  jbe .LBB2_9
  leal (%rcx,%rcx,2), %r9d
  movl $3, %ecx
  subl %r9d, %ecx
.LBB2_7:
  movl %eax, %edx
  movl %eax, %edi
  imulq $613566757, %rdi, %r8
  shrq $32, %r8
  subl %r8d, %eax
  shrl %eax
  addl %r8d, %eax
  shrl $2, %eax
  addl %r9d, %eax
  addl $-3, %ecx
  addl $3, %r9d
  decl %esi
  jne .LBB2_7
  imulq $613566757, %rdi, %rax
  shrq $32, %rax
  subl %eax, %edx
  shrl %edx
  addl %eax, %edx
  shrl $2, %edx
  subl %ecx, %edx
  movl %edx, %eax
.LBB2_9:
  retq

k_udr10:
  movl %edi, %eax
  testl %esi, %esi
  je .LBB3_9
  cmpl $8, %esi
  jb .LBB3_5
  pushq %rbp
  pushq %r15
  pushq %r14
  pushq %r13
  pushq %r12
  pushq %rbx
  subq $16, %rsp
  movl %esi, -116(%rsp)
  shrl $3, %esi
  movl %esi, -112(%rsp)
  movl $0, -124(%rsp)
  movl $1, -128(%rsp)
  movl $5, %ebp
  movl $18, %edx
  movl $58, %edi
  movl $179, %r12d
  movl $543, %ecx
  movq %rcx, -96(%rsp)
  movl $-24604, %r9d
  movl $-8201, %r13d
  movl $-2733, %ecx
  movl $-910, %esi
  movl $-302, %r15d
  movl $-99, %ebx
  movl $-31, %r11d
  movl $-8, %r10d
.LBB3_3:
  movq %rcx, -48(%rsp)
  movq %r13, -40(%rsp)
  movq %rsi, -32(%rsp)
  movq %r15, -24(%rsp)
  movq %rbx, -16(%rsp)
  movq %r11, -8(%rsp)
  movq %r10, (%rsp)
  movq %r9, 8(%rsp)
  movl %eax, %r8d
  movl %eax, %eax
  movq %rax, -72(%rsp)
  movl $3435973837, %esi
  imulq %rsi, %rax
  shrq $35, %rax
  imull $6561, %r8d, %ecx
  movl %ecx, -120(%rsp)
  imull $2187, %r8d, %ecx
  movq %rcx, -104(%rsp)
  imull $729, %r8d, %ecx
  movq %r12, -56(%rsp)
  movq %rcx, -80(%rsp)
  leal (%r12,%rcx), %r11d
  imull $243, %r8d, %ecx
  movq %rdi, -64(%rsp)
  movq %rcx, -88(%rsp)
  leal (%rdi,%rcx), %r10d
  leal (%r8,%r8,8), %ecx
  leal (%rcx,%rcx,8), %r15d
  movl %edx, -108(%rsp)
  addl %edx, %r15d
  leal (%rcx,%rcx,2), %ebx
  addl %ebp, %ebx
  movl -128(%rsp), %edi
  addl %edi, %ecx
  leal (%r8,%r8,2), %r9d
  movl -124(%rsp), %edx
  addl %edx, %r9d
  leal (%rax,%rax,8), %r12d
  leal (%r12,%r12,2), %r12d
  addl %eax, %r12d
  addl %eax, %r12d
  subl %r12d, %r9d
  imulq %rsi, %r9
  shrq $35, %r9
  leal (%r9,%r9,8), %r12d
  leal (%r12,%r12,2), %r12d
  addl %r9d, %r12d
  addl %r9d, %r12d
  subl %r12d, %ecx
  imull $87, %eax, %r12d
  subl %r12d, %ecx
  imulq %rsi, %rcx
  shrq $35, %rcx
  leal (%rcx,%rcx,8), %r12d
  leal (%r12,%r12,2), %r12d
  addl %ecx, %r12d
  addl %ecx, %r12d
  subl %r12d, %ebx
  imull $87, %r9d, %r12d
  subl %r12d, %ebx
  imull $261, %eax, %r12d
  subl %r12d, %ebx
  imulq %rsi, %rbx
  shrq $35, %rbx
  leal (%rbx,%rbx,8), %r12d
  leal (%r12,%r12,2), %r12d
  addl %ebx, %r12d
  addl %ebx, %r12d
  subl %r12d, %r15d
  imull $87, %ecx, %r12d
  subl %r12d, %r15d
  imull $261, %r9d, %r12d
  subl %r12d, %r15d
  imull $783, %eax, %r12d
  subl %r12d, %r15d
  imulq %rsi, %r15
  shrq $35, %r15
  leal (%r15,%r15,8), %r12d
  leal (%r12,%r12,2), %r12d
  addl %r15d, %r12d
  addl %r15d, %r12d
  subl %r12d, %r10d
  imull $87, %ebx, %r12d
  subl %r12d, %r10d
  imull $261, %ecx, %r12d
  subl %r12d, %r10d
  imull $783, %r9d, %r12d
  subl %r12d, %r10d
  imull $2349, %eax, %r12d
  subl %r12d, %r10d
  imulq %rsi, %r10
  shrq $35, %r10
  leal (%r10,%r10,8), %r12d
  leal (%r12,%r12,2), %r12d
  addl %r10d, %r12d
  addl %r10d, %r12d
  subl %r12d, %r11d
  imull $87, %r15d, %r12d
  subl %r12d, %r11d
  imull $261, %ebx, %r12d
  subl %r12d, %r11d
  imull $783, %ecx, %r12d
  subl %r12d, %r11d
  imull $2349, %r9d, %r12d
  subl %r12d, %r11d
  imull $7047, %eax, %r12d
  subl %r12d, %r11d
  imulq %rsi, %r11
  shrq $35, %r11
  leal (%r11,%r11,8), %r12d
  leal (%r12,%r12,2), %r13d
  addl %r11d, %r13d
  addl %r11d, %r13d
  movq -96(%rsp), %r14
  movq -104(%rsp), %r12
  addl %r14d, %r12d
  subl %r13d, %r12d
  imull $87, %r10d, %r13d
  subl %r13d, %r12d
  imull $261, %r15d, %r13d
  subl %r13d, %r12d
  imull $783, %ebx, %r13d
  subl %r13d, %r12d
  imull $2349, %ecx, %r13d
  subl %r13d, %r12d
  imull $7047, %r9d, %r13d
  subl %r13d, %r12d
  imull $21141, %eax, %r13d
  subl %r13d, %r12d
  imulq %rsi, %r12
  shrq $35, %r12
  leal (%r12,%r12,8), %r13d
  leal (%r13,%r13,2), %r13d
  addl %r12d, %r12d
  addl %r13d, %r12d
  movl -120(%rsp), %esi
  movl %esi, %r13d
  subl %r12d, %r13d
  movq -32(%rsp), %rsi
  imull $87, %r11d, %r11d
  subl %r11d, %r13d
  movq -8(%rsp), %r11
  imull $261, %r10d, %r10d
  subl %r10d, %r13d
  imull $783, %r15d, %r10d
  movq -24(%rsp), %r15
  subl %r10d, %r13d
  imull $2349, %ebx, %r10d
  movq -16(%rsp), %rbx
  subl %r10d, %r13d
  movq (%rsp), %r10
  imull $7047, %ecx, %ecx
  subl %ecx, %r13d
  imull $21141, %r9d, %ecx
  movq 8(%rsp), %r9
  subl %ecx, %r13d
  movq -48(%rsp), %rcx
  imull $63423, %eax, %eax
  subl %eax, %r13d
  leal (%r9,%r13), %eax
  addl $26240, %eax
  movq -40(%rsp), %r13
  addl $8, %edx
  movl %edx, -124(%rsp)
  addl $32, %edi
  movl %edi, -128(%rsp)
  addl $104, %ebp
  movl -108(%rsp), %edx
  addl $320, %edx
  movq -64(%rsp), %rdi
  addl $968, %edi
  movq -56(%rsp), %r12
  addl $2912, %r12d
  addl $8744, %r14d
  movq %r14, -96(%rsp)
  addl $26240, %r9d
  addl $8744, %r13d
  addl $2912, %ecx
  addl $968, %esi
  addl $320, %r15d
  addl $104, %ebx
  addl $32, %r11d
  addl $8, %r10d
  decl -112(%rsp)
  jne .LBB3_3
  addl -104(%rsp), %r13d
  addl -80(%rsp), %ecx
  addl -88(%rsp), %esi
  leal (%r8,%r8,8), %eax
  movq %rcx, %r14
  leal (%rax,%rax,8), %ecx
  addl %ecx, %r15d
  leal (%rax,%rax,2), %ecx
  addl %ecx, %ebx
  addl %eax, %r11d
  leal (%r8,%r8,2), %eax
  addl %eax, %r10d
  movl $3435973837, %eax
  movq -72(%rsp), %rdi
  imulq %rax, %rdi
  shrq $35, %rdi
  leal (%rdi,%rdi,8), %ecx
  leal (%rcx,%rcx,2), %ecx
  addl %edi, %ecx
  addl %edi, %ecx
  subl %ecx, %r10d
  imulq %rax, %r10
  shrq $35, %r10
  leal (%r10,%r10,8), %ecx
  leal (%rcx,%rcx,2), %ecx
  addl %r10d, %ecx
  addl %r10d, %ecx
  subl %ecx, %r11d
  imull $87, %edi, %ecx
  subl %ecx, %r11d
  imulq %rax, %r11
  shrq $35, %r11
  leal (%r11,%r11,8), %ecx
  leal (%rcx,%rcx,2), %ecx
  addl %r11d, %ecx
  addl %r11d, %ecx
  subl %ecx, %ebx
  imull $87, %r10d, %ecx
  subl %ecx, %ebx
  imull $261, %edi, %ecx
  subl %ecx, %ebx
  imulq %rax, %rbx
  shrq $35, %rbx
  leal (%rbx,%rbx,8), %ecx
  leal (%rcx,%rcx,2), %ecx
  addl %ebx, %ecx
  addl %ebx, %ecx
  subl %ecx, %r15d
  imull $87, %r11d, %ecx
  subl %ecx, %r15d
  imull $261, %r10d, %ecx
  subl %ecx, %r15d
  imull $783, %edi, %ecx
  subl %ecx, %r15d
  imulq %rax, %r15
  shrq $35, %r15
  leal (%r15,%r15,8), %ecx
  leal (%rcx,%rcx,2), %ecx
  addl %r15d, %ecx
  addl %r15d, %ecx
  subl %ecx, %esi
  imull $87, %ebx, %ecx
  subl %ecx, %esi
  imull $261, %r11d, %ecx
  subl %ecx, %esi
  imull $783, %r10d, %ecx
  subl %ecx, %esi
  imull $2349, %edi, %ecx
  subl %ecx, %esi
  imulq %rax, %rsi
  shrq $35, %rsi
  leal (%rsi,%rsi,8), %ecx
  leal (%rcx,%rcx,2), %ecx
  addl %esi, %ecx
  addl %esi, %ecx
  subl %ecx, %r14d
  imull $87, %r15d, %ecx
  subl %ecx, %r14d
  imull $261, %ebx, %ecx
  subl %ecx, %r14d
  imull $783, %r11d, %ecx
  subl %ecx, %r14d
  imull $2349, %r10d, %ecx
  subl %ecx, %r14d
  imull $7047, %edi, %ecx
  subl %ecx, %r14d
  imulq %rax, %r14
  shrq $35, %r14
  leal (%r14,%r14,8), %ecx
  leal (%rcx,%rcx,2), %ecx
  addl %r14d, %ecx
  addl %r14d, %ecx
  subl %ecx, %r13d
  imull $87, %esi, %ecx
  subl %ecx, %r13d
  imull $261, %r15d, %ecx
  subl %ecx, %r13d
  imull $783, %ebx, %ecx
  subl %ecx, %r13d
  imull $2349, %r11d, %ecx
  subl %ecx, %r13d
  imull $7047, %r10d, %ecx
  subl %ecx, %r13d
  imull $21141, %edi, %ecx
  subl %ecx, %r13d
  imulq %rax, %r13
  shrq $35, %r13
  leal (%r13,%r13,8), %eax
  leal (%rax,%rax,2), %eax
  addl %r13d, %r13d
  addl %eax, %r13d
  movl -120(%rsp), %ecx
  subl %r13d, %ecx
  imull $87, %r14d, %eax
  subl %eax, %ecx
  imull $261, %esi, %eax
  subl %eax, %ecx
  imull $783, %r15d, %eax
  subl %eax, %ecx
  imull $2349, %ebx, %eax
  subl %eax, %ecx
  imull $7047, %r11d, %eax
  subl %eax, %ecx
  imull $21141, %r10d, %eax
  subl %eax, %ecx
  imull $63423, %edi, %eax
  subl %eax, %ecx
  addl %r9d, %ecx
  movl %ecx, %eax
  movl -116(%rsp), %esi
  addq $16, %rsp
  popq %rbx
  popq %r12
  popq %r13
  popq %r14
  popq %r15
  popq %rbp
.LBB3_5:
  movl %esi, %ecx
  andl $-8, %ecx
  cmpl %esi, %ecx
  jae .LBB3_9
  movl $3435973837, %r10d
.LBB3_7:
  movl %eax, %edi
  movl %eax, %edx
  movq %rdx, %r8
  imulq %r10, %r8
  shrq $35, %r8
  leal (%rdi,%rdi,2), %eax
  leal (%r8,%r8,8), %r9d
  leal (%r9,%r9,2), %r9d
  addl %r8d, %r8d
  addl %r9d, %r8d
  subl %r8d, %eax
  addl %ecx, %eax
  incl %ecx
  cmpl %ecx, %esi
  jne .LBB3_7
  leal (%rdi,%rdi,2), %eax
  movl $3435973837, %esi
  imulq %rdx, %rsi
  shrq $35, %rsi
  leal (%rsi,%rsi,8), %edx
  leal (%rdx,%rdx,2), %edx
  addl %esi, %esi
  addl %edx, %esi
  subl %esi, %eax
  addl %ecx, %eax
  decl %eax
.LBB3_9:
  retq

k_sdr7:
  movl %edi, %eax
  testl %esi, %esi
  je .LBB4_9
  cmpl $8, %esi
  jb .LBB4_5
  movl %esi, %edx
  shrl $3, %edx
  movl $1, %ecx
.LBB4_3:
  cltq
  imulq $-1840700269, %rax, %rdi
  shrq $32, %rdi
  addl %eax, %edi
  movl %edi, %r8d
  shrl $31, %r8d
  sarl $2, %edi
  addl %r8d, %edi
  leal (,%rdi,8), %r8d
  movl %edi, %r9d
  subl %r8d, %r9d
  addl %eax, %r9d
  leal (%r9,%r9,4), %eax
  addl %edi, %eax
  leal (%rcx,%rax), %edi
  addl %ecx, %eax
  decl %eax
  cltq
  imulq $-1840700269, %rax, %rax
  shrq $32, %rax
  addl %edi, %eax
  decl %eax
  movl %eax, %r8d
  shrl $31, %r8d
  sarl $2, %eax
  addl %r8d, %eax
  leal (,%rax,8), %r8d
  movl %eax, %r9d
  subl %r8d, %r9d
  addl %r9d, %edi
  decl %edi
  leal (%rdi,%rdi,4), %edi
  addl %eax, %edi
  leal (%rcx,%rdi), %eax
  addl %ecx, %edi
  addl $-2, %edi
  movslq %edi, %rdi
  imulq $-1840700269, %rdi, %rdi
  shrq $32, %rdi
  addl %eax, %edi
  addl $-2, %edi
  movl %edi, %r8d
  shrl $31, %r8d
  sarl $2, %edi
  addl %r8d, %edi
  leal (,%rdi,8), %r8d
  movl %edi, %r9d
  subl %r8d, %r9d
  addl %r9d, %eax
  addl $-2, %eax
  leal (%rax,%rax,4), %eax
  addl %edi, %eax
  leal (%rcx,%rax), %edi
  addl %ecx, %eax
  addl $-3, %eax
  cltq
  imulq $-1840700269, %rax, %rax
  shrq $32, %rax
  addl %edi, %eax
  addl $-3, %eax
  movl %eax, %r8d
  shrl $31, %r8d
  sarl $2, %eax
  addl %r8d, %eax
  leal (,%rax,8), %r8d
  movl %eax, %r9d
  subl %r8d, %r9d
  addl %r9d, %edi
  addl $-3, %edi
  leal (%rdi,%rdi,4), %edi
  addl %eax, %edi
  leal (%rcx,%rdi), %eax
  addl %ecx, %edi
  addl $-4, %edi
  movslq %edi, %rdi
  imulq $-1840700269, %rdi, %rdi
  shrq $32, %rdi
  addl %eax, %edi
  addl $-4, %edi
  movl %edi, %r8d
  shrl $31, %r8d
  sarl $2, %edi
  addl %r8d, %edi
  leal (,%rdi,8), %r8d
  movl %edi, %r9d
  subl %r8d, %r9d
  addl %r9d, %eax
  addl $-4, %eax
  leal (%rax,%rax,4), %eax
  addl %edi, %eax
  leal (%rcx,%rax), %edi
  addl %ecx, %eax
  addl $-5, %eax
  cltq
  imulq $-1840700269, %rax, %rax
  shrq $32, %rax
  addl %edi, %eax
  addl $-5, %eax
  movl %eax, %r8d
  shrl $31, %r8d
  sarl $2, %eax
  addl %r8d, %eax
  leal (,%rax,8), %r8d
  movl %eax, %r9d
  subl %r8d, %r9d
  addl %r9d, %edi
  addl $-5, %edi
  leal (%rdi,%rdi,4), %edi
  addl %eax, %edi
  leal (%rcx,%rdi), %eax
  addl %ecx, %edi
  addl $-6, %edi
  movslq %edi, %rdi
  imulq $-1840700269, %rdi, %rdi
  shrq $32, %rdi
  addl %eax, %edi
  addl $-6, %edi
  movl %edi, %r8d
  shrl $31, %r8d
  sarl $2, %edi
  addl %r8d, %edi
  leal (,%rdi,8), %r8d
  movl %edi, %r9d
  subl %r8d, %r9d
  addl %r9d, %eax
  addl $-6, %eax
  leal (%rax,%rax,4), %eax
  addl %edi, %eax
  leal (%rcx,%rax), %r8d
  addl %ecx, %eax
  addl $-7, %eax
  cltq
  imulq $-1840700269, %rax, %rax
  shrq $32, %rax
  leal (%rax,%r8), %edi
  addl $-7, %edi
  movl %edi, %eax
  shrl $31, %eax
  sarl $2, %edi
  addl %eax, %edi
  leal (,%rdi,8), %eax
  movl %edi, %r9d
  subl %eax, %r9d
  addl %r9d, %r8d
  addl $-7, %r8d
  leal (%r8,%r8,4), %eax
  addl %edi, %eax
  addl %ecx, %eax
  addl $-8, %eax
  addl $-8, %ecx
  decl %edx
  jne .LBB4_3
  leal (%r8,%r8,4), %eax
  addl %ecx, %edi
  addl %eax, %edi
  movl %edi, %eax
.LBB4_5:
  movl %esi, %ecx
  andl $-8, %ecx
  cmpl %esi, %ecx
  jae .LBB4_9
  negl %ecx
  negl %esi
.LBB4_7:
  cltq
  imulq $-1840700269, %rax, %rdx
  shrq $32, %rdx
  addl %eax, %edx
  movl %edx, %edi
  shrl $31, %edi
  sarl $2, %edx
  addl %edi, %edx
  leal (,%rdx,8), %r8d
  movl %edx, %edi
  subl %r8d, %edi
  addl %eax, %edi
  leal (%rdi,%rdi,4), %eax
  addl %ecx, %eax
  addl %edx, %eax
  decl %ecx
  cmpl %ecx, %esi
  jne .LBB4_7
  leal (%rdi,%rdi,4), %eax
  addl %eax, %edx
  leal (%rcx,%rdx), %eax
  incl %eax
.LBB4_9:
  retq

k_srem7:
  movl %edi, %eax
  testl %esi, %esi
  je .LBB5_8
  cmpl $8, %esi
  jb .LBB5_4
  movl %esi, %ecx
  shrl $3, %ecx
  movw $7, %dx
.LBB5_3:
  cltq
  imulq $-1840700269, %rax, %rdi
  shrq $32, %rdi
  addl %eax, %edi
  movl %edi, %r8d
  shrl $31, %r8d
  sarl $2, %edi
  addl %r8d, %edi
  leal (,%rdi,8), %r8d
  subl %r8d, %edi
  addl %eax, %edi
  leal -7(%rdx), %eax
  movzwl %ax, %eax
  leal (%rax,%rdi), %r8d
  addl $-30000, %r8d
  addl %eax, %edi
  movslq %r8d, %rax
  imulq $-1840700269, %rax, %rax
  shrq $32, %rax
  addl %edi, %eax
  addl $-30000, %eax
  movl %eax, %r8d
  shrl $31, %r8d
  sarl $2, %eax
  addl %r8d, %eax
  leal (,%rax,8), %r8d
  subl %r8d, %eax
  addl %edi, %eax
  addl $-30000, %eax
  leal -6(%rdx), %edi
  movzwl %di, %edi
  leal (%rdi,%rax), %r8d
  addl $-30000, %r8d
  addl %edi, %eax
  movslq %r8d, %rdi
  imulq $-1840700269, %rdi, %rdi
  shrq $32, %rdi
  addl %eax, %edi
  addl $-30000, %edi
  movl %edi, %r8d
  shrl $31, %r8d
  sarl $2, %edi
  addl %r8d, %edi
  leal (,%rdi,8), %r8d
  subl %r8d, %edi
  addl %edi, %eax
  addl $-30000, %eax
  leal -5(%rdx), %edi
  movzwl %di, %edi
  leal (%rdi,%rax), %r8d
  addl $-30000, %r8d
  addl %edi, %eax
  movslq %r8d, %rdi
  imulq $-1840700269, %rdi, %rdi
  shrq $32, %rdi
  addl %eax, %edi
  addl $-30000, %edi
  movl %edi, %r8d
  shrl $31, %r8d
  sarl $2, %edi
  addl %r8d, %edi
  leal (,%rdi,8), %r8d
  subl %r8d, %edi
  addl %edi, %eax
  addl $-30000, %eax
  leal -4(%rdx), %edi
  movzwl %di, %edi
  leal (%rdi,%rax), %r8d
  addl $-30000, %r8d
  addl %edi, %eax
  movslq %r8d, %rdi
  imulq $-1840700269, %rdi, %rdi
  shrq $32, %rdi
  addl %eax, %edi
  addl $-30000, %edi
  movl %edi, %r8d
  shrl $31, %r8d
  sarl $2, %edi
  addl %r8d, %edi
  leal (,%rdi,8), %r8d
  subl %r8d, %edi
  addl %edi, %eax
  addl $-30000, %eax
  leal -3(%rdx), %edi
  movzwl %di, %edi
  leal (%rdi,%rax), %r8d
  addl $-30000, %r8d
  addl %edi, %eax
  movslq %r8d, %rdi
  imulq $-1840700269, %rdi, %rdi
  shrq $32, %rdi
  addl %eax, %edi
  addl $-30000, %edi
  movl %edi, %r8d
  shrl $31, %r8d
  sarl $2, %edi
  addl %r8d, %edi
  leal (,%rdi,8), %r8d
  subl %r8d, %edi
  addl %edi, %eax
  addl $-30000, %eax
  leal -2(%rdx), %edi
  movzwl %di, %edi
  leal (%rdi,%rax), %r8d
  addl $-30000, %r8d
  addl %edi, %eax
  movslq %r8d, %rdi
  imulq $-1840700269, %rdi, %rdi
  shrq $32, %rdi
  addl %eax, %edi
  addl $-30000, %edi
  movl %edi, %r8d
  shrl $31, %r8d
  sarl $2, %edi
  addl %r8d, %edi
  leal (,%rdi,8), %r8d
  subl %r8d, %edi
  addl %edi, %eax
  addl $-30000, %eax
  leal -1(%rdx), %edi
  movzwl %di, %edi
  leal (%rdi,%rax), %r8d
  addl $-30000, %r8d
  addl %edi, %eax
  movslq %r8d, %rdi
  imulq $-1840700269, %rdi, %rdi
  shrq $32, %rdi
  addl %eax, %edi
  addl $-30000, %edi
  movl %edi, %r8d
  shrl $31, %r8d
  sarl $2, %edi
  addl %r8d, %edi
  leal (,%rdi,8), %r8d
  subl %r8d, %edi
  addl %edi, %eax
  addl $-30000, %eax
  movzwl %dx, %edi
  addl %edi, %eax
  addl $-30000, %eax
  addl $8, %edx
  decl %ecx
  jne .LBB5_3
.LBB5_4:
  movl %esi, %edx
  andl $-8, %edx
  movl %esi, %ecx
  subl %edx, %ecx
  jbe .LBB5_8
  andl $65528, %esi
  movl $30001, %edx
  subl %esi, %edx
  addl $-30000, %esi
.LBB5_6:
  cltq
  imulq $-1840700269, %rax, %rdi
  shrq $32, %rdi
  addl %eax, %edi
  movl %edi, %r8d
  shrl $31, %r8d
  sarl $2, %edi
  addl %r8d, %edi
  leal (,%rdi,8), %r8d
  subl %r8d, %edi
  addl %eax, %edi
  leal (%rsi,%rdi), %eax
  decl %edx
  incl %esi
  decl %ecx
  jne .LBB5_6
  subl %edx, %edi
  movl %edi, %eax
.LBB5_8:
  retq

k_srem16:
  movl %edi, %eax
  testl %esi, %esi
  je .LBB6_8
  cmpl $8, %esi
  jb .LBB6_4
  movl %esi, %ecx
  shrl $3, %ecx
  xorl %edx, %edx
.LBB6_3:
  leal 15(%rax), %edi
  testl %eax, %eax
  cmovnsl %eax, %edi
  andl $-16, %edi
  subl %edi, %eax
  movl %edx, %edi
  andl $248, %edi
  leal (%rax,%rax,2), %eax
  leal (%rdi,%rax), %r8d
  addl $-100, %r8d
  addl %edi, %eax
  addl $-85, %eax
  testl %r8d, %r8d
  cmovnsl %r8d, %eax
  andl $-16, %eax
  subl %eax, %r8d
  leal (%r8,%r8,2), %eax
  leal (%rdi,%rax), %r8d
  addl $-99, %r8d
  addl %edi, %eax
  addl $-84, %eax
  testl %r8d, %r8d
  cmovnsl %r8d, %eax
  andl $-16, %eax
  subl %eax, %r8d
  leal (%r8,%r8,2), %eax
  leal (%rdi,%rax), %r8d
  addl $-98, %r8d
  addl %edi, %eax
  addl $-83, %eax
  testl %r8d, %r8d
  cmovnsl %r8d, %eax
  andl $-16, %eax
  subl %eax, %r8d
  leal (%r8,%r8,2), %eax
  leal (%rdi,%rax), %r8d
  addl $-97, %r8d
  addl %edi, %eax
  addl $-82, %eax
  testl %r8d, %r8d
  cmovnsl %r8d, %eax
  andl $-16, %eax
  subl %eax, %r8d
  leal (%r8,%r8,2), %eax
  leal (%rdi,%rax), %r8d
  addl $-96, %r8d
  addl %edi, %eax
  addl $-81, %eax
  testl %r8d, %r8d
  cmovnsl %r8d, %eax
  andl $-16, %eax
  subl %eax, %r8d
  leal (%r8,%r8,2), %eax
  leal (%rdi,%rax), %r8d
  addl $-95, %r8d
  addl %edi, %eax
  addl $-80, %eax
  testl %r8d, %r8d
  cmovnsl %r8d, %eax
  andl $-16, %eax
  subl %eax, %r8d
  leal (%r8,%r8,2), %eax
  leal (%rdi,%rax), %r8d
  addl $-94, %r8d
  addl %edi, %eax
  addl $-79, %eax
  testl %r8d, %r8d
  cmovnsl %r8d, %eax
  andl $-16, %eax
  subl %eax, %r8d
  leal (%r8,%r8,2), %eax
  addl %edi, %eax
  addl $-93, %eax
  addl $8, %edx
  decl %ecx
  jne .LBB6_3
.LBB6_4:
  movl %esi, %edx
  andl $-8, %edx
  movl %esi, %ecx
  subl %edx, %ecx
  jbe .LBB6_8
  andl $248, %esi
  movl $101, %edx
  subl %esi, %edx
  addl $-100, %esi
.LBB6_6:
  leal 15(%rax), %r8d
  testl %eax, %eax
  cmovnsl %eax, %r8d
  andl $-16, %r8d
  movl %eax, %edi
  subl %r8d, %edi
  leal (%rdi,%rdi,2), %eax
  addl %esi, %eax
  decl %edx
  incl %esi
  decl %ecx
  jne .LBB6_6
  leal (%rdi,%rdi,2), %eax
  subl %edx, %eax
.LBB6_8:
  retq

k_sdr16:
  testl %esi, %esi
  je .LBB7_1
  cmpl $8, %esi
  jb .LBB7_6
  movl %esi, %eax
  shrl $3, %eax
  movl $7, %ecx
.LBB7_5:
  leal 15(%rdi), %edx
  testl %edi, %edi
  cmovnsl %edi, %edx
  movl %edx, %r8d
  sarl $4, %r8d
  andl $-16, %edx
  subl %edx, %edi
  shll $3, %edi
  leal -7(%rcx), %edx
  xorl %r8d, %edx
  xorl %edi, %edx
  leal 15(%rdx), %edi
  testl %edx, %edx
  cmovnsl %edx, %edi
  movl %edi, %r8d
  sarl $4, %r8d
  andl $-16, %edi
  subl %edi, %edx
  shll $3, %edx
  leal -6(%rcx), %edi
  xorl %r8d, %edi
  xorl %edx, %edi
  leal 15(%rdi), %edx
  testl %edi, %edi
  cmovnsl %edi, %edx
  movl %edx, %r8d
  sarl $4, %r8d
  andl $-16, %edx
  subl %edx, %edi
  shll $3, %edi
  leal -5(%rcx), %edx
  xorl %r8d, %edx
  xorl %edi, %edx
  leal 15(%rdx), %edi
  testl %edx, %edx
  cmovnsl %edx, %edi
  movl %edi, %r8d
  sarl $4, %r8d
  andl $-16, %edi
  subl %edi, %edx
  shll $3, %edx
  leal -4(%rcx), %edi
  xorl %r8d, %edi
  xorl %edx, %edi
  leal 15(%rdi), %edx
  testl %edi, %edi
  cmovnsl %edi, %edx
  movl %edx, %r8d
  sarl $4, %r8d
  andl $-16, %edx
  subl %edx, %edi
  shll $3, %edi
  leal -3(%rcx), %edx
  xorl %r8d, %edx
  xorl %edi, %edx
  leal 15(%rdx), %edi
  testl %edx, %edx
  cmovnsl %edx, %edi
  movl %edi, %r8d
  sarl $4, %r8d
  andl $-16, %edi
  subl %edi, %edx
  shll $3, %edx
  leal -2(%rcx), %edi
  xorl %r8d, %edi
  xorl %edx, %edi
  leal 15(%rdi), %edx
  testl %edi, %edi
  cmovnsl %edi, %edx
  movl %edx, %r8d
  sarl $4, %r8d
  andl $-16, %edx
  subl %edx, %edi
  shll $3, %edi
  leal -1(%rcx), %edx
  xorl %r8d, %edx
  xorl %edi, %edx
  leal 15(%rdx), %r8d
  testl %edx, %edx
  cmovnsl %edx, %r8d
  movl %r8d, %edi
  sarl $4, %edi
  andl $-16, %r8d
  subl %r8d, %edx
  shll $3, %edx
  xorl %ecx, %edi
  xorl %edx, %edi
  addl $8, %ecx
  decl %eax
  jne .LBB7_5
.LBB7_6:
  movl %esi, %ecx
  andl $-8, %ecx
  cmpl %esi, %ecx
  jae .LBB7_1
.LBB7_7:
  leal 15(%rdi), %edx
  testl %edi, %edi
  cmovnsl %edi, %edx
  movl %edx, %eax
  sarl $4, %eax
  andl $-16, %edx
  subl %edx, %edi
  shll $3, %edi
  xorl %ecx, %eax
  xorl %edi, %eax
  incl %ecx
  movl %eax, %edi
  cmpl %ecx, %esi
  jne .LBB7_7
  retq
.LBB7_1:
  movl %edi, %eax
  retq

k_mul7:
  testl %esi, %esi
  je .LBB8_1
  cmpl $8, %esi
  jb .LBB8_6
  movl %esi, %eax
  shrl $3, %eax
  movl $7, %ecx
.LBB8_5:
  leal (,%rdi,8), %edx
  subl %edi, %edx
  leal -7(%rcx), %edi
  xorl %edx, %edi
  leal (,%rdi,8), %edx
  subl %edi, %edx
  leal -6(%rcx), %edi
  xorl %edx, %edi
  leal (,%rdi,8), %edx
  subl %edi, %edx
  leal -5(%rcx), %edi
  xorl %edx, %edi
  leal (,%rdi,8), %edx
  subl %edi, %edx
  leal -4(%rcx), %edi
  xorl %edx, %edi
  leal (,%rdi,8), %edx
  subl %edi, %edx
  leal -3(%rcx), %edi
  xorl %edx, %edi
  leal (,%rdi,8), %edx
  subl %edi, %edx
  leal -2(%rcx), %edi
  xorl %edx, %edi
  leal (,%rdi,8), %edx
  subl %edi, %edx
  leal -1(%rcx), %r8d
  xorl %edx, %r8d
  leal (,%r8,8), %edi
  subl %r8d, %edi
  xorl %ecx, %edi
  addl $8, %ecx
  decl %eax
  jne .LBB8_5
.LBB8_6:
  movl %esi, %ecx
  andl $-8, %ecx
  cmpl %esi, %ecx
  jae .LBB8_1
.LBB8_7:
  leal (,%rdi,8), %eax
  subl %edi, %eax
  xorl %ecx, %eax
  incl %ecx
  movl %eax, %edi
  cmpl %ecx, %esi
  jne .LBB8_7
  retq
.LBB8_1:
  movl %edi, %eax
  retq

k_mul11:
  movl %edi, %eax
  testl %esi, %esi
  je .LBB9_6
  cmpl $8, %esi
  jb .LBB9_4
  movl %esi, %ecx
  shrl $3, %ecx
  movl $7, %edx
.LBB9_3:
  leal (%rax,%rax,4), %edi
  leal (%rax,%rdi,2), %eax
  leal -7(%rdx), %edi
  xorl %eax, %edi
  leal (%rdi,%rdi,4), %eax
  leal (%rdi,%rax,2), %eax
  leal -6(%rdx), %edi
  xorl %eax, %edi
  leal (%rdi,%rdi,4), %eax
  leal (%rdi,%rax,2), %eax
  leal -5(%rdx), %edi
  xorl %eax, %edi
  leal (%rdi,%rdi,4), %eax
  leal (%rdi,%rax,2), %eax
  leal -4(%rdx), %edi
  xorl %eax, %edi
  leal (%rdi,%rdi,4), %eax
  leal (%rdi,%rax,2), %eax
  leal -3(%rdx), %edi
  xorl %eax, %edi
  leal (%rdi,%rdi,4), %eax
  leal (%rdi,%rax,2), %eax
  leal -2(%rdx), %edi
  xorl %eax, %edi
  leal (%rdi,%rdi,4), %eax
  leal (%rdi,%rax,2), %eax
  leal -1(%rdx), %edi
  xorl %eax, %edi
  leal (%rdi,%rdi,4), %eax
  leal (%rdi,%rax,2), %eax
  xorl %edx, %eax
  addl $8, %edx
  decl %ecx
  jne .LBB9_3
.LBB9_4:
  movl %esi, %ecx
  andl $-8, %ecx
  cmpl %esi, %ecx
  jae .LBB9_6
.LBB9_5:
  leal (%rax,%rax,4), %edx
  leal (%rax,%rdx,2), %eax
  xorl %ecx, %eax
  incl %ecx
  cmpl %ecx, %esi
  jne .LBB9_5
.LBB9_6:
  retq

k_mul17:
  testl %esi, %esi
  je .LBB10_1
  cmpl $8, %esi
  jb .LBB10_6
  movl %esi, %eax
  shrl $3, %eax
  movl $7, %ecx
.LBB10_5:
  movl %edi, %edx
  shll $4, %edx
  addl %edi, %edx
  leal -7(%rcx), %edi
  xorl %edx, %edi
  movl %edi, %edx
  shll $4, %edx
  addl %edi, %edx
  leal -6(%rcx), %edi
  xorl %edx, %edi
  movl %edi, %edx
  shll $4, %edx
  addl %edi, %edx
  leal -5(%rcx), %edi
  xorl %edx, %edi
  movl %edi, %edx
  shll $4, %edx
  addl %edi, %edx
  leal -4(%rcx), %edi
  xorl %edx, %edi
  movl %edi, %edx
  shll $4, %edx
  addl %edi, %edx
  leal -3(%rcx), %edi
  xorl %edx, %edi
  movl %edi, %edx
  shll $4, %edx
  addl %edi, %edx
  leal -2(%rcx), %edi
  xorl %edx, %edi
  movl %edi, %edx
  shll $4, %edx
  addl %edi, %edx
  leal -1(%rcx), %r8d
  xorl %edx, %r8d
  movl %r8d, %edi
  shll $4, %edi
  addl %r8d, %edi
  xorl %ecx, %edi
  addl $8, %ecx
  decl %eax
  jne .LBB10_5
.LBB10_6:
  movl %esi, %ecx
  andl $-8, %ecx
  cmpl %esi, %ecx
  jae .LBB10_1
  movl %edi, %eax
.LBB10_8:
  shll $4, %eax
  addl %edi, %eax
  xorl %ecx, %eax
  incl %ecx
  movl %eax, %edi
  cmpl %ecx, %esi
  jne .LBB10_8
  retq
.LBB10_1:
  movl %edi, %eax
  retq

k_mul24:
  movl %edi, %eax
  testl %esi, %esi
  je .LBB11_6
  cmpl $8, %esi
  jb .LBB11_4
  movl %esi, %ecx
  shrl $3, %ecx
  movl $7, %edx
.LBB11_3:
  shll $3, %eax
  leal (%rax,%rax,2), %eax
  leal -7(%rdx), %edi
  xorl %eax, %edi
  shll $3, %edi
  leal (%rdi,%rdi,2), %eax
  leal -6(%rdx), %edi
  xorl %eax, %edi
  shll $3, %edi
  leal (%rdi,%rdi,2), %eax
  leal -5(%rdx), %edi
  xorl %eax, %edi
  shll $3, %edi
  leal (%rdi,%rdi,2), %eax
  leal -4(%rdx), %edi
  xorl %eax, %edi
  shll $3, %edi
  leal (%rdi,%rdi,2), %eax
  leal -3(%rdx), %edi
  xorl %eax, %edi
  shll $3, %edi
  leal (%rdi,%rdi,2), %eax
  leal -2(%rdx), %edi
  xorl %eax, %edi
  shll $3, %edi
  leal (%rdi,%rdi,2), %eax
  leal -1(%rdx), %edi
  xorl %eax, %edi
  shll $3, %edi
  leal (%rdi,%rdi,2), %eax
  xorl %edx, %eax
  addl $8, %edx
  decl %ecx
  jne .LBB11_3
.LBB11_4:
  movl %esi, %ecx
  andl $-8, %ecx
  cmpl %esi, %ecx
  jae .LBB11_6
.LBB11_5:
  shll $3, %eax
  leal (%rax,%rax,2), %eax
  xorl %ecx, %eax
  incl %ecx
  cmpl %ecx, %esi
  jne .LBB11_5
.LBB11_6:
  retq

k_mul45:
  movl %edi, %eax
  testl %esi, %esi
  je .LBB12_6
  cmpl $8, %esi
  jb .LBB12_4
  movl %esi, %ecx
  shrl $3, %ecx
  movl $7, %edx
.LBB12_3:
  leal (%rax,%rax,8), %eax
  leal (%rax,%rax,4), %eax
  leal -7(%rdx), %edi
  xorl %eax, %edi
  leal (%rdi,%rdi,8), %eax
  leal (%rax,%rax,4), %eax
  leal -6(%rdx), %edi
  xorl %eax, %edi
  leal (%rdi,%rdi,8), %eax
  leal (%rax,%rax,4), %eax
  leal -5(%rdx), %edi
  xorl %eax, %edi
  leal (%rdi,%rdi,8), %eax
  leal (%rax,%rax,4), %eax
  leal -4(%rdx), %edi
  xorl %eax, %edi
  leal (%rdi,%rdi,8), %eax
  leal (%rax,%rax,4), %eax
  leal -3(%rdx), %edi
  xorl %eax, %edi
  leal (%rdi,%rdi,8), %eax
  leal (%rax,%rax,4), %eax
  leal -2(%rdx), %edi
  xorl %eax, %edi
  leal (%rdi,%rdi,8), %eax
  leal (%rax,%rax,4), %eax
  leal -1(%rdx), %edi
  xorl %eax, %edi
  leal (%rdi,%rdi,8), %eax
  leal (%rax,%rax,4), %eax
  xorl %edx, %eax
  addl $8, %edx
  decl %ecx
  jne .LBB12_3
.LBB12_4:
  movl %esi, %ecx
  andl $-8, %ecx
  cmpl %esi, %ecx
  jae .LBB12_6
.LBB12_5:
  leal (%rax,%rax,8), %eax
  leal (%rax,%rax,4), %eax
  xorl %ecx, %eax
  incl %ecx
  cmpl %ecx, %esi
  jne .LBB12_5
.LBB12_6:
  retq

k_mulm3:
  movl %edi, %eax
  testl %esi, %esi
  je .LBB13_6
  cmpl $8, %esi
  jb .LBB13_4
  movl %esi, %ecx
  shrl $3, %ecx
  movl $7, %edx
.LBB13_3:
  leal (%rax,%rax,2), %eax
  negl %eax
  leal -7(%rdx), %edi
  xorl %eax, %edi
  leal (%rdi,%rdi,2), %eax
  negl %eax
  leal -6(%rdx), %edi
  xorl %eax, %edi
  leal (%rdi,%rdi,2), %eax
  negl %eax
  leal -5(%rdx), %edi
  xorl %eax, %edi
  leal (%rdi,%rdi,2), %eax
  negl %eax
  leal -4(%rdx), %edi
  xorl %eax, %edi
  leal (%rdi,%rdi,2), %eax
  negl %eax
  leal -3(%rdx), %edi
  xorl %eax, %edi
  leal (%rdi,%rdi,2), %eax
  negl %eax
  leal -2(%rdx), %edi
  xorl %eax, %edi
  leal (%rdi,%rdi,2), %eax
  negl %eax
  leal -1(%rdx), %edi
  xorl %eax, %edi
  leal (%rdi,%rdi,2), %eax
  negl %eax
  xorl %edx, %eax
  addl $8, %edx
  decl %ecx
  jne .LBB13_3
.LBB13_4:
  movl %esi, %ecx
  andl $-8, %ecx
  cmpl %esi, %ecx
  jae .LBB13_6
.LBB13_5:
  leal (%rax,%rax,2), %eax
  negl %eax
  xorl %ecx, %eax
  incl %ecx
  cmpl %ecx, %esi
  jne .LBB13_5
.LBB13_6:
  retq

k_mul1000:
  movl %edi, %eax
  testl %esi, %esi
  je .LBB14_6
  cmpl $8, %esi
  jb .LBB14_4
  movl %esi, %ecx
  shrl $3, %ecx
  movl $7, %edx
.LBB14_3:
  imull $1000, %eax, %eax
  leal -7(%rdx), %edi
  xorl %eax, %edi
  imull $1000, %edi, %eax
  leal -6(%rdx), %edi
  xorl %eax, %edi
  imull $1000, %edi, %eax
  leal -5(%rdx), %edi
  xorl %eax, %edi
  imull $1000, %edi, %eax
  leal -4(%rdx), %edi
  xorl %eax, %edi
  imull $1000, %edi, %eax
  leal -3(%rdx), %edi
  xorl %eax, %edi
  imull $1000, %edi, %eax
  leal -2(%rdx), %edi
  xorl %eax, %edi
  imull $1000, %edi, %eax
  leal -1(%rdx), %edi
  xorl %eax, %edi
  imull $1000, %edi, %eax
  xorl %edx, %eax
  addl $8, %edx
  decl %ecx
  jne .LBB14_3
.LBB14_4:
  movl %esi, %ecx
  andl $-8, %ecx
  cmpl %esi, %ecx
  jae .LBB14_6
.LBB14_5:
  imull $1000, %eax, %eax
  xorl %ecx, %eax
  incl %ecx
  cmpl %ecx, %esi
  jne .LBB14_5
.LBB14_6:
  retq

k_fnv:
  movl %edi, %eax
  testl %esi, %esi
  je .LBB15_7
  cmpl $8, %esi
  jb .LBB15_4
  movl %esi, %ecx
  shrl $3, %ecx
  movb $7, %dl
.LBB15_3:
  leal -7(%rdx), %edi
  movzbl %dil, %edi
  xorl %eax, %edi
  imull $16777619, %edi, %eax
  leal -6(%rdx), %edi
  movzbl %dil, %edi
  xorl %eax, %edi
  imull $16777619, %edi, %eax
  leal -5(%rdx), %edi
  movzbl %dil, %edi
  xorl %eax, %edi
  imull $16777619, %edi, %eax
  leal -4(%rdx), %edi
  movzbl %dil, %edi
  xorl %eax, %edi
  imull $16777619, %edi, %eax
  leal -3(%rdx), %edi
  movzbl %dil, %edi
  xorl %eax, %edi
  imull $16777619, %edi, %eax
  leal -2(%rdx), %edi
  movzbl %dil, %edi
  xorl %eax, %edi
  imull $16777619, %edi, %eax
  leal -1(%rdx), %edi
  movzbl %dil, %edi
  xorl %eax, %edi
  imull $16777619, %edi, %eax
  movzbl %dl, %edx
  xorl %edx, %eax
  imull $16777619, %eax, %eax
  addb $8, %dl
  decl %ecx
  jne .LBB15_3
.LBB15_4:
  movl %esi, %edx
  andl $-8, %edx
  movl %esi, %ecx
  subl %edx, %ecx
  jbe .LBB15_7
  andl $248, %esi
.LBB15_6:
  xorl %esi, %eax
  imull $16777619, %eax, %eax
  incl %esi
  decl %ecx
  jne .LBB15_6
.LBB15_7:
  retq

k_digits:
  testl %esi, %esi
  je .LBB16_1
  cmpl $1, %esi
  jne .LBB16_9
  xorl %ecx, %ecx
  xorl %eax, %eax
  testb $1, %sil
  jne .LBB16_5
  jmp .LBB16_8
.LBB16_1:
  xorl %eax, %eax
  retq
.LBB16_9:
  pushq %rbx
  movl %esi, %edx
  andl $-2, %edx
  xorl %ecx, %ecx
  movl $3435973837, %r8d
  xorl %eax, %eax
  jmp .LBB16_10
.LBB16_14:
  addl $2, %ecx
  cmpl %edx, %ecx
  je .LBB16_15
.LBB16_10:
  movl %ecx, %r9d
  addl %edi, %r9d
  je .LBB16_12
.LBB16_11:
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
  ja .LBB16_11
.LBB16_12:
  leal (%rcx,%rdi), %r9d
  incl %r9d
  je .LBB16_14
.LBB16_13:
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
  ja .LBB16_13
  jmp .LBB16_14
.LBB16_15:
  popq %rbx
  testb $1, %sil
  je .LBB16_8
.LBB16_5:
  addl %edi, %ecx
  je .LBB16_8
  movl $3435973837, %edx
.LBB16_7:
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
  ja .LBB16_7
.LBB16_8:
  retq

.LCPI17_0:
.LCPI17_1:
main:
  pushq %rbp
  pushq %r15
  pushq %r14
  pushq %r13
  pushq %r12
  pushq %rbx
  subq $56, %rsp
  vstmxcsr 8(%rsp)
  orl $32832, 8(%rsp)
  vldmxcsr 8(%rsp)
  movl $20000000, %r12d
  cmpl $2, %edi
  jl .LBB17_2
  movq 8(%rsi), %rdi
  xorl %esi, %esi
  xorl %edx, %edx
  callq strtoul
  movq %rax, %r12
.LBB17_2:
  movl %r12d, %eax
  shrl $4, %eax
  movl %eax, 52(%rsp)
  leaq 8(%rsp), %r14
  movl $1, %edi
  movq %r14, %rsi
  callq clock_gettime
  vcvtsi2sdq 8(%rsp), %xmm0, %xmm0
  vmovsd %xmm0, 40(%rsp)
  vcvtsi2sdq 16(%rsp), %xmm1, %xmm0
  vmovsd %xmm0, 24(%rsp)
  movl $123456789, %edi
  movl %r12d, %esi
  callq k_urem7
  movl %eax, %ebx
  leaq 8(%rsp), %r15
  movl $1, %edi
  movq %r15, %rsi
  callq clock_gettime
  vcvtsi2sdq 8(%rsp), %xmm1, %xmm0
  vcvtsi2sdq 16(%rsp), %xmm1, %xmm1
  movq stderr(%rip), %rdi
  vsubsd 40(%rsp), %xmm0, %xmm0
  vsubsd 24(%rsp), %xmm1, %xmm1
  movl %r12d, %eax
  vcvtsi2sd %rax, %xmm2, %xmm2
  vfmadd231sd .LCPI17_0(%rip), %xmm0, %xmm1
  vmovsd .LCPI17_1(%rip), %xmm0
  vdivsd %xmm2, %xmm0, %xmm0
  vmovsd %xmm0, 40(%rsp)
  vmulsd %xmm0, %xmm1, %xmm0
  movl $.L.str.17, %esi
  movl $.L.str, %edx
  movb $1, %al
  callq fprintf
  movl $.L.str.18, %edi
  movl $.L.str, %esi
  movl %ebx, %edx
  xorl %eax, %eax
  callq printf
  movl $1, %edi
  movq %r14, %rsi
  callq clock_gettime
  vcvtsi2sdq 8(%rsp), %xmm3, %xmm0
  vmovsd %xmm0, 24(%rsp)
  vcvtsi2sdq 16(%rsp), %xmm3, %xmm0
  vmovsd %xmm0, 32(%rsp)
  movl $987654321, %edi
  movl %r12d, %esi
  callq k_urem10
  movl %eax, %r13d
  movl $1, %edi
  movq %r15, %rsi
  callq clock_gettime
  vcvtsi2sdq 8(%rsp), %xmm3, %xmm0
  vcvtsi2sdq 16(%rsp), %xmm3, %xmm1
  movl %ebx, %ebp
  shll $5, %ebp
  subl %ebx, %ebp
  addl %r13d, %ebp
  movq stderr(%rip), %rdi
  vsubsd 24(%rsp), %xmm0, %xmm0
  vsubsd 32(%rsp), %xmm1, %xmm1
  vfmadd231sd .LCPI17_0(%rip), %xmm0, %xmm1
  vmulsd 40(%rsp), %xmm1, %xmm0
  movl $.L.str.17, %esi
  movl $.L.str.1, %edx
  movb $1, %al
  callq fprintf
  movl $.L.str.18, %edi
  movl $.L.str.1, %esi
  movl %r13d, %edx
  xorl %eax, %eax
  callq printf
  movl $1, %edi
  movq %r14, %rsi
  callq clock_gettime
  vcvtsi2sdq 8(%rsp), %xmm3, %xmm0
  vmovsd %xmm0, 24(%rsp)
  vcvtsi2sdq 16(%rsp), %xmm3, %xmm0
  vmovsd %xmm0, 32(%rsp)
  movl $-559038737, %edi
  movl %r12d, %esi
  callq k_udiv7
  movl %eax, %ebx
  movl $1, %edi
  movq %r15, %rsi
  callq clock_gettime
  vcvtsi2sdq 8(%rsp), %xmm3, %xmm0
  vcvtsi2sdq 16(%rsp), %xmm3, %xmm1
  movl %ebp, %r13d
  shll $5, %r13d
  subl %ebp, %r13d
  addl %ebx, %r13d
  movq stderr(%rip), %rdi
  vsubsd 24(%rsp), %xmm0, %xmm0
  vsubsd 32(%rsp), %xmm1, %xmm1
  vfmadd231sd .LCPI17_0(%rip), %xmm0, %xmm1
  vmulsd 40(%rsp), %xmm1, %xmm0
  movl $.L.str.17, %esi
  movl $.L.str.2, %edx
  movb $1, %al
  callq fprintf
  movl $.L.str.18, %edi
  movl $.L.str.2, %esi
  movl %ebx, %edx
  xorl %eax, %eax
  callq printf
  movl $1, %edi
  movq %r14, %rsi
  callq clock_gettime
  vcvtsi2sdq 8(%rsp), %xmm3, %xmm0
  vmovsd %xmm0, 24(%rsp)
  vcvtsi2sdq 16(%rsp), %xmm3, %xmm0
  vmovsd %xmm0, 32(%rsp)
  movl $305419896, %edi
  movl %r12d, %esi
  callq k_udr10
  movl %eax, %ebp
  movl $1, %edi
  movq %r15, %rsi
  callq clock_gettime
  vcvtsi2sdq 8(%rsp), %xmm3, %xmm0
  vcvtsi2sdq 16(%rsp), %xmm3, %xmm1
  movl %r13d, %ebx
  shll $5, %ebx
  subl %r13d, %ebx
  addl %ebp, %ebx
  movq stderr(%rip), %rdi
  vsubsd 24(%rsp), %xmm0, %xmm0
  vsubsd 32(%rsp), %xmm1, %xmm1
  vfmadd231sd .LCPI17_0(%rip), %xmm0, %xmm1
  vmulsd 40(%rsp), %xmm1, %xmm0
  movl $.L.str.17, %esi
  movl $.L.str.3, %edx
  movb $1, %al
  callq fprintf
  movl $.L.str.18, %edi
  movl $.L.str.3, %esi
  movl %ebp, %edx
  xorl %eax, %eax
  callq printf
  movl $1, %edi
  movq %r14, %rsi
  callq clock_gettime
  vcvtsi2sdq 8(%rsp), %xmm3, %xmm0
  vmovsd %xmm0, 24(%rsp)
  vcvtsi2sdq 16(%rsp), %xmm3, %xmm0
  vmovsd %xmm0, 32(%rsp)
  movl $2147422772, %edi
  movl %r12d, %esi
  callq k_sdr7
  movl %eax, %ebp
  movl $1, %edi
  movq %r15, %rsi
  callq clock_gettime
  vcvtsi2sdq 8(%rsp), %xmm2, %xmm0
  vcvtsi2sdq 16(%rsp), %xmm2, %xmm1
  movl %ebx, %r13d
  shll $5, %r13d
  subl %ebx, %r13d
  addl %ebp, %r13d
  movq stderr(%rip), %rdi
  vsubsd 24(%rsp), %xmm0, %xmm0
  vsubsd 32(%rsp), %xmm1, %xmm1
  vfmadd231sd .LCPI17_0(%rip), %xmm0, %xmm1
  vmulsd 40(%rsp), %xmm1, %xmm0
  movl $.L.str.17, %esi
  movl $.L.str.4, %edx
  movb $1, %al
  callq fprintf
  movl $.L.str.18, %edi
  movl $.L.str.4, %esi
  movl %ebp, %edx
  xorl %eax, %eax
  callq printf
  movl $1, %edi
  movq %r14, %rsi
  callq clock_gettime
  vcvtsi2sdq 8(%rsp), %xmm2, %xmm0
  vmovsd %xmm0, 24(%rsp)
  vcvtsi2sdq 16(%rsp), %xmm2, %xmm0
  vmovsd %xmm0, 32(%rsp)
  movl $-2147478988, %edi
  movl %r12d, %esi
  callq k_srem7
  movl %eax, %ebp
  movl $1, %edi
  movq %r15, %rsi
  callq clock_gettime
  vcvtsi2sdq 8(%rsp), %xmm2, %xmm0
  vcvtsi2sdq 16(%rsp), %xmm2, %xmm1
  movl %r13d, %ebx
  shll $5, %ebx
  subl %r13d, %ebx
  addl %ebp, %ebx
  movq stderr(%rip), %rdi
  vsubsd 24(%rsp), %xmm0, %xmm0
  vsubsd 32(%rsp), %xmm1, %xmm1
  vfmadd231sd .LCPI17_0(%rip), %xmm0, %xmm1
  vmulsd 40(%rsp), %xmm1, %xmm0
  movl $.L.str.17, %esi
  movl $.L.str.5, %edx
  movb $1, %al
  callq fprintf
  movl $.L.str.18, %edi
  movl $.L.str.5, %esi
  movl %ebp, %edx
  xorl %eax, %eax
  callq printf
  movl $1, %edi
  movq %r14, %r13
  movq %r14, %rsi
  callq clock_gettime
  vcvtsi2sdq 8(%rsp), %xmm2, %xmm0
  vmovsd %xmm0, 24(%rsp)
  vcvtsi2sdq 16(%rsp), %xmm2, %xmm0
  vmovsd %xmm0, 32(%rsp)
  movl $-16, %edi
  movl %r12d, %esi
  callq k_srem16
  movl %eax, %ebp
  movl $1, %edi
  movq %r15, %rsi
  callq clock_gettime
  vcvtsi2sdq 8(%rsp), %xmm2, %xmm0
  vcvtsi2sdq 16(%rsp), %xmm2, %xmm1
  movl %ebx, %r14d
  shll $5, %r14d
  subl %ebx, %r14d
  addl %ebp, %r14d
  movq stderr(%rip), %rdi
  vsubsd 24(%rsp), %xmm0, %xmm0
  vsubsd 32(%rsp), %xmm1, %xmm1
  vfmadd231sd .LCPI17_0(%rip), %xmm0, %xmm1
  vmulsd 40(%rsp), %xmm1, %xmm0
  movl $.L.str.17, %esi
  movl $.L.str.6, %edx
  movb $1, %al
  callq fprintf
  movl $.L.str.18, %edi
  movl $.L.str.6, %esi
  movl %ebp, %edx
  xorl %eax, %eax
  callq printf
  movl $1, %edi
  movq %r13, %rsi
  movq %r13, %rbx
  callq clock_gettime
  vcvtsi2sdq 8(%rsp), %xmm2, %xmm0
  vmovsd %xmm0, 24(%rsp)
  vcvtsi2sdq 16(%rsp), %xmm2, %xmm0
  vmovsd %xmm0, 32(%rsp)
  movl $267242409, %edi
  movl %r12d, %esi
  callq k_sdr16
  movl %eax, %ebp
  movl $1, %edi
  movq %r15, %rsi
  callq clock_gettime
  vcvtsi2sdq 8(%rsp), %xmm2, %xmm0
  vcvtsi2sdq 16(%rsp), %xmm2, %xmm1
  movl %r14d, %r13d
  shll $5, %r13d
  subl %r14d, %r13d
  addl %ebp, %r13d
  movq stderr(%rip), %rdi
  vsubsd 24(%rsp), %xmm0, %xmm0
  vsubsd 32(%rsp), %xmm1, %xmm1
  vfmadd231sd .LCPI17_0(%rip), %xmm0, %xmm1
  vmulsd 40(%rsp), %xmm1, %xmm0
  movl $.L.str.17, %esi
  movl $.L.str.7, %edx
  movb $1, %al
  callq fprintf
  movl $.L.str.18, %edi
  movl $.L.str.7, %esi
  movl %ebp, %edx
  xorl %eax, %eax
  callq printf
  movl $1, %edi
  movq %rbx, %r14
  movq %rbx, %rsi
  callq clock_gettime
  vcvtsi2sdq 8(%rsp), %xmm2, %xmm0
  vmovsd %xmm0, 24(%rsp)
  vcvtsi2sdq 16(%rsp), %xmm2, %xmm0
  vmovsd %xmm0, 32(%rsp)
  movl $1, %edi
  movl %r12d, %esi
  callq k_mul7
  movl %eax, %ebp
  movl $1, %edi
  movq %r15, %rsi
  callq clock_gettime
  vcvtsi2sdq 8(%rsp), %xmm2, %xmm0
  vcvtsi2sdq 16(%rsp), %xmm2, %xmm1
  movl %r13d, %ebx
  shll $5, %ebx
  subl %r13d, %ebx
  addl %ebp, %ebx
  movq stderr(%rip), %rdi
  vsubsd 24(%rsp), %xmm0, %xmm0
  vsubsd 32(%rsp), %xmm1, %xmm1
  vfmadd231sd .LCPI17_0(%rip), %xmm0, %xmm1
  vmulsd 40(%rsp), %xmm1, %xmm0
  movl $.L.str.17, %esi
  movl $.L.str.8, %edx
  movb $1, %al
  callq fprintf
  movl $.L.str.18, %edi
  movl $.L.str.8, %esi
  movl %ebp, %edx
  xorl %eax, %eax
  callq printf
  movl $1, %edi
  movq %r14, %rsi
  callq clock_gettime
  vcvtsi2sdq 8(%rsp), %xmm2, %xmm0
  vmovsd %xmm0, 24(%rsp)
  vcvtsi2sdq 16(%rsp), %xmm2, %xmm0
  vmovsd %xmm0, 32(%rsp)
  movl $3, %edi
  movl %r12d, %esi
  callq k_mul11
  movl %eax, %ebp
  movl $1, %edi
  movq %r15, %rsi
  callq clock_gettime
  vcvtsi2sdq 8(%rsp), %xmm2, %xmm0
  vcvtsi2sdq 16(%rsp), %xmm2, %xmm1
  movl %ebx, %r13d
  shll $5, %r13d
  subl %ebx, %r13d
  addl %ebp, %r13d
  movq stderr(%rip), %rdi
  vsubsd 24(%rsp), %xmm0, %xmm0
  vsubsd 32(%rsp), %xmm1, %xmm1
  vfmadd231sd .LCPI17_0(%rip), %xmm0, %xmm1
  vmulsd 40(%rsp), %xmm1, %xmm0
  movl $.L.str.17, %esi
  movl $.L.str.9, %edx
  movb $1, %al
  callq fprintf
  movl $.L.str.18, %edi
  movl $.L.str.9, %esi
  movl %ebp, %edx
  xorl %eax, %eax
  callq printf
  movl $1, %edi
  movq %r14, %rsi
  callq clock_gettime
  vcvtsi2sdq 8(%rsp), %xmm2, %xmm0
  vmovsd %xmm0, 24(%rsp)
  vcvtsi2sdq 16(%rsp), %xmm2, %xmm0
  vmovsd %xmm0, 32(%rsp)
  movl $5, %edi
  movl %r12d, %esi
  callq k_mul17
  movl %eax, %ebp
  movl $1, %edi
  movq %r15, %rsi
  callq clock_gettime
  vcvtsi2sdq 8(%rsp), %xmm2, %xmm0
  vcvtsi2sdq 16(%rsp), %xmm2, %xmm1
  movl %r13d, %ebx
  shll $5, %ebx
  subl %r13d, %ebx
  addl %ebp, %ebx
  movq stderr(%rip), %rdi
  vsubsd 24(%rsp), %xmm0, %xmm0
  vsubsd 32(%rsp), %xmm1, %xmm1
  vfmadd231sd .LCPI17_0(%rip), %xmm0, %xmm1
  vmulsd 40(%rsp), %xmm1, %xmm0
  movl $.L.str.17, %esi
  movl $.L.str.10, %edx
  movb $1, %al
  callq fprintf
  movl $.L.str.18, %edi
  movl $.L.str.10, %esi
  movl %ebp, %edx
  xorl %eax, %eax
  callq printf
  movl $1, %edi
  movq %r14, %rsi
  callq clock_gettime
  vcvtsi2sdq 8(%rsp), %xmm2, %xmm0
  vmovsd %xmm0, 24(%rsp)
  vcvtsi2sdq 16(%rsp), %xmm2, %xmm0
  vmovsd %xmm0, 32(%rsp)
  movl $7, %edi
  movl %r12d, %esi
  callq k_mul24
  movl %eax, %ebp
  movl $1, %edi
  movq %r15, %rsi
  callq clock_gettime
  vcvtsi2sdq 8(%rsp), %xmm2, %xmm0
  vcvtsi2sdq 16(%rsp), %xmm2, %xmm1
  movl %ebx, %r13d
  shll $5, %r13d
  subl %ebx, %r13d
  addl %ebp, %r13d
  movq stderr(%rip), %rdi
  vsubsd 24(%rsp), %xmm0, %xmm0
  vsubsd 32(%rsp), %xmm1, %xmm1
  vfmadd231sd .LCPI17_0(%rip), %xmm0, %xmm1
  vmulsd 40(%rsp), %xmm1, %xmm0
  movl $.L.str.17, %esi
  movl $.L.str.11, %edx
  movb $1, %al
  callq fprintf
  movl $.L.str.18, %edi
  movl $.L.str.11, %esi
  movl %ebp, %edx
  xorl %eax, %eax
  callq printf
  movl $1, %edi
  movq %r14, %rsi
  callq clock_gettime
  vcvtsi2sdq 8(%rsp), %xmm2, %xmm0
  vmovsd %xmm0, 24(%rsp)
  vcvtsi2sdq 16(%rsp), %xmm2, %xmm0
  vmovsd %xmm0, 32(%rsp)
  movl $9, %edi
  movl %r12d, %esi
  callq k_mul45
  movl %eax, %ebp
  movl $1, %edi
  movq %r15, %rsi
  callq clock_gettime
  vcvtsi2sdq 8(%rsp), %xmm2, %xmm0
  vcvtsi2sdq 16(%rsp), %xmm2, %xmm1
  movl %r13d, %ebx
  shll $5, %ebx
  subl %r13d, %ebx
  addl %ebp, %ebx
  movq stderr(%rip), %rdi
  vsubsd 24(%rsp), %xmm0, %xmm0
  vsubsd 32(%rsp), %xmm1, %xmm1
  vfmadd231sd .LCPI17_0(%rip), %xmm0, %xmm1
  vmulsd 40(%rsp), %xmm1, %xmm0
  movl $.L.str.17, %esi
  movl $.L.str.12, %edx
  movb $1, %al
  callq fprintf
  movl $.L.str.18, %edi
  movl $.L.str.12, %esi
  movl %ebp, %edx
  xorl %eax, %eax
  callq printf
  movl $1, %edi
  movq %r14, %rsi
  callq clock_gettime
  vcvtsi2sdq 8(%rsp), %xmm2, %xmm0
  vmovsd %xmm0, 24(%rsp)
  vcvtsi2sdq 16(%rsp), %xmm2, %xmm0
  vmovsd %xmm0, 32(%rsp)
  movl $11, %edi
  movl %r12d, %esi
  callq k_mulm3
  movl %eax, %ebp
  movl $1, %edi
  movq %r15, %rsi
  callq clock_gettime
  vcvtsi2sdq 8(%rsp), %xmm2, %xmm0
  vcvtsi2sdq 16(%rsp), %xmm2, %xmm1
  movl %ebx, %r13d
  shll $5, %r13d
  subl %ebx, %r13d
  addl %ebp, %r13d
  movq stderr(%rip), %rdi
  vsubsd 24(%rsp), %xmm0, %xmm0
  vsubsd 32(%rsp), %xmm1, %xmm1
  vfmadd231sd .LCPI17_0(%rip), %xmm0, %xmm1
  vmulsd 40(%rsp), %xmm1, %xmm0
  movl $.L.str.17, %esi
  movl $.L.str.13, %edx
  movb $1, %al
  callq fprintf
  movl $.L.str.18, %edi
  movl $.L.str.13, %esi
  movl %ebp, %edx
  xorl %eax, %eax
  callq printf
  movl $1, %edi
  movq %r14, %rsi
  callq clock_gettime
  vcvtsi2sdq 8(%rsp), %xmm2, %xmm0
  vmovsd %xmm0, 24(%rsp)
  vcvtsi2sdq 16(%rsp), %xmm2, %xmm0
  vmovsd %xmm0, 32(%rsp)
  movl $13, %edi
  movl %r12d, %esi
  callq k_mul1000
  movl %eax, %ebp
  movl $1, %edi
  movq %r15, %rsi
  callq clock_gettime
  vcvtsi2sdq 8(%rsp), %xmm2, %xmm0
  vcvtsi2sdq 16(%rsp), %xmm2, %xmm1
  movl %r13d, %ebx
  shll $5, %ebx
  subl %r13d, %ebx
  addl %ebp, %ebx
  movq stderr(%rip), %rdi
  vsubsd 24(%rsp), %xmm0, %xmm0
  vsubsd 32(%rsp), %xmm1, %xmm1
  vfmadd231sd .LCPI17_0(%rip), %xmm0, %xmm1
  vmulsd 40(%rsp), %xmm1, %xmm0
  movl $.L.str.17, %esi
  movl $.L.str.14, %edx
  movb $1, %al
  callq fprintf
  movl $.L.str.18, %edi
  movl $.L.str.14, %esi
  movl %ebp, %edx
  xorl %eax, %eax
  callq printf
  movl $1, %edi
  movq %r14, %rsi
  callq clock_gettime
  vcvtsi2sdq 8(%rsp), %xmm2, %xmm0
  vmovsd %xmm0, 24(%rsp)
  vcvtsi2sdq 16(%rsp), %xmm2, %xmm0
  vmovsd %xmm0, 32(%rsp)
  movl $-2128831035, %edi
  movl %r12d, %esi
  callq k_fnv
  movl %eax, %ebp
  movl $1, %edi
  movq %r15, %rsi
  callq clock_gettime
  vcvtsi2sdq 8(%rsp), %xmm2, %xmm0
  vcvtsi2sdq 16(%rsp), %xmm2, %xmm1
  movl %ebx, %r12d
  shll $5, %r12d
  subl %ebx, %r12d
  addl %ebp, %r12d
  movq stderr(%rip), %rdi
  vsubsd 24(%rsp), %xmm0, %xmm0
  vsubsd 32(%rsp), %xmm1, %xmm1
  vfmadd231sd .LCPI17_0(%rip), %xmm0, %xmm1
  vmulsd 40(%rsp), %xmm1, %xmm0
  movl $.L.str.17, %esi
  movl $.L.str.15, %edx
  movb $1, %al
  callq fprintf
  movl $.L.str.18, %edi
  movl $.L.str.15, %esi
  movl %ebp, %edx
  xorl %eax, %eax
  callq printf
  movl $1, %edi
  movq %r14, %rsi
  callq clock_gettime
  vcvtsi2sdq 8(%rsp), %xmm2, %xmm0
  vmovsd %xmm0, 40(%rsp)
  vcvtsi2sdq 16(%rsp), %xmm2, %xmm0
  vmovsd %xmm0, 24(%rsp)
  movl $1234567, %edi
  movl 52(%rsp), %ebx
  movl %ebx, %esi
  callq k_digits
  movl %eax, %ebp
  movl $1, %edi
  movq %r15, %rsi
  callq clock_gettime
  vcvtsi2sdq 8(%rsp), %xmm2, %xmm0
  vcvtsi2sdq 16(%rsp), %xmm2, %xmm1
  movl %r12d, %r14d
  shll $5, %r14d
  subl %r12d, %r14d
  addl %ebp, %r14d
  movq stderr(%rip), %rdi
  vsubsd 40(%rsp), %xmm0, %xmm0
  vsubsd 24(%rsp), %xmm1, %xmm1
  vfmadd231sd .LCPI17_0(%rip), %xmm0, %xmm1
  vcvtsi2sd %ebx, %xmm2, %xmm0
  vdivsd %xmm0, %xmm1, %xmm0
  movl $.L.str.17, %esi
  movl $.L.str.16, %edx
  movb $1, %al
  callq fprintf
  movl $.L.str.18, %edi
  movl $.L.str.16, %esi
  movl %ebp, %edx
  xorl %eax, %eax
  callq printf
  movl $.L.str.19, %edi
  movl %r14d, %esi
  xorl %eax, %eax
  callq printf
  xorl %eax, %eax
  addq $56, %rsp
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

.L.str.17:

.L.str.18:

.L.str.19:

