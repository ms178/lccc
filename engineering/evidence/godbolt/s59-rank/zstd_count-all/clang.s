main:
  movl $-2058980567, %edx
  xorl %ecx, %ecx
  leaq buffer(%rip), %rax
  jmp .LBB0_1
.LBB0_7:
  movl %edx, %esi
  shrl $24, %esi
.LBB0_9:
  movb %sil, 1(%rcx,%rax)
  addq $2, %rcx
  cmpq $1048576, %rcx
  je .LBB0_10
.LBB0_1:
  imull $1664525, %edx, %edx
  addl $1013904223, %edx
  cmpq $64, %rcx
  jb .LBB0_4
  movl %edx, %esi
  andl $7, %esi
  jne .LBB0_4
  movl %edx, %esi
  shrl $28, %esi
  addl %ecx, %esi
  addl $-64, %esi
  movzbl (%rsi,%rax), %esi
  jmp .LBB0_5
.LBB0_4:
  movl %edx, %esi
  shrl $24, %esi
.LBB0_5:
  movb %sil, (%rcx,%rax)
  imull $1664525, %edx, %edx
  addl $1013904223, %edx
  cmpq $64, %rcx
  jb .LBB0_7
  movl %edx, %esi
  andl $7, %esi
  jne .LBB0_7
  movl %edx, %esi
  shrl $28, %esi
  addl %ecx, %esi
  addl $-63, %esi
  movzbl (%rsi,%rax), %esi
  jmp .LBB0_9
.LBB0_10:
  pushq %rbp
  pushq %r15
  pushq %r14
  pushq %r13
  pushq %r12
  pushq %rbx
  pushq %rax
  movl $324508639, %ecx
  xorl %esi, %esi
  xorl %edx, %edx
  jmp .LBB0_11
.LBB0_22:
  movl 4(%rsp), %edx
  incl %edx
  cmpl $16, %edx
  je .LBB0_23
.LBB0_11:
  movl %edx, 4(%rsp)
  xorl %edi, %edi
  jmp .LBB0_12
.LBB0_20:
  xorq %rbp, %rdx
  tzcntq %rdx, %rdx
  shrl $3, %edx
  addq %rdx, %r10
.LBB0_21:
  subl %r9d, %r10d
  leal (,%rdi,8), %edx
  shlxq %rdx, %r10, %rdx
  xorq %r8, %rdx
  addq %rdx, %rsi
  incl %edi
  cmpl $131072, %edi
  je .LBB0_22
.LBB0_12:
  imull $1664525, %ecx, %ecx
  addl $1013904223, %ecx
  movl %ecx, %r8d
  andl $1048319, %r8d
  movl %ecx, %r11d
  andl $127, %r11d
  leaq (%rax,%r8), %r9
  leaq (%r11,%r9), %rbx
  addq $16, %rbx
  leaq (%r11,%r9), %r12
  addq $9, %r12
  movl %ecx, %r14d
  shrl $12, %r14d
  andl $1048319, %r14d
  leaq (%rax,%r14), %r13
  movl $8, %ebp
  movq %r9, %r10
.LBB0_13:
  movq %rbp, %r15
  movq (%r13), %rbp
  movq (%r10), %rdx
  cmpq %rdx, %rbp
  jne .LBB0_20
  addq $8, %r10
  addq $8, %r13
  leaq 8(%r15), %rbp
  cmpq %r12, %r10
  jb .LBB0_13
  cmpq %rbx, %r10
  jae .LBB0_21
  addq $16, %r11
.LBB0_17:
  leaq (%r14,%r15), %rdx
  movzbl (%r10), %ebp
  cmpb (%rax,%rdx), %bpl
  jne .LBB0_21
  incq %r10
  incq %r15
  cmpq %r15, %r11
  jne .LBB0_17
  movq %rbx, %r10
  jmp .LBB0_21
.LBB0_23:
  leaq .L.str(%rip), %rdi
  xorl %eax, %eax
  callq printf@PLT
  xorl %eax, %eax
  addq $8, %rsp
  popq %rbx
  popq %r12
  popq %r13
  popq %r14
  popq %r15
  popq %rbp
  retq

.L.str:

