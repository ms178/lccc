.LCPI0_0:
  .quad 0
  .quad 72623859790382856
  .quad -1
  .quad 7
main:
  pushq %rbp
  pushq %r15
  pushq %r14
  pushq %r13
  pushq %r12
  pushq %rbx
  subq $88, %rsp
  vmovaps .LCPI0_0(%rip), %ymm0
  vmovups %ymm0, 48(%rsp)
  vmovups %ymm0, 16(%rsp)
  leaq 48(%rsp), %rdi
  leaq 16(%rsp), %rsi
  movl $4, %edx
  vzeroupper
  callq glibc_memcmp_common_alignment
  testl %eax, %eax
  je .LBB0_2
  movl $2, %eax
  jmp .LBB0_8
.LBB0_2:
  movabsq $72623859790382857, %rax
  movq %rax, 24(%rsp)
  leaq 48(%rsp), %rdi
  leaq 16(%rsp), %rsi
  movl $4, %edx
  callq glibc_memcmp_common_alignment
  movl %eax, %ecx
  movl $2, %eax
  testl %ecx, %ecx
  jns .LBB0_8
  movabsq $-7046029254386353131, %rcx
  xorl %eax, %eax
  leaq glibc_left(%rip), %r14
  leaq glibc_right(%rip), %r15
.LBB0_4:
  movq %rcx, %rdx
  shlq $7, %rdx
  xorq %rcx, %rdx
  movq %rdx, %rcx
  shrq $9, %rcx
  xorq %rdx, %rcx
  movq %rcx, %rdx
  shlq $8, %rdx
  xorq %rcx, %rdx
  movq %rdx, (%r14,%rax,8)
  movq %rdx, (%r15,%rax,8)
  movq %rdx, %rcx
  shlq $7, %rcx
  xorq %rdx, %rcx
  movq %rcx, %rdx
  shrq $9, %rdx
  xorq %rcx, %rdx
  movq %rdx, %rcx
  shlq $8, %rcx
  xorq %rdx, %rcx
  movq %rcx, 8(%r14,%rax,8)
  movq %rcx, 8(%r15,%rax,8)
  addq $2, %rax
  cmpq $8192, %rax
  jne .LBB0_4
  movl $1, %r12d
  xorl %ebx, %ebx
  xorl %ebp, %ebp
  xorl %esi, %esi
.LBB0_6:
  movq %rsi, 8(%rsp)
  movl %ebx, %r13d
  andl $8191, %r13d
  shll $3, %r13d
  movq (%r13,%r14), %rax
  movq %rax, (%rsp)
  btcq %rbp, %rax
  movq %rax, (%r13,%r15)
  movl $8192, %edx
  movq %r14, %rdi
  movq %r15, %rsi
  callq glibc_memcmp_common_alignment
  movq 8(%rsp), %rsi
  addl $257, %eax
  imulq %r12, %rax
  addq %rax, %rsi
  movq (%rsp), %rax
  movq %rax, (%r13,%r15)
  addq $4051, %rbx
  addq $11, %rbp
  incq %r12
  cmpq $45056, %rbp
  jne .LBB0_6
  leaq .L.str(%rip), %rdi
  xorl %eax, %eax
  callq printf@PLT
  xorl %eax, %eax
.LBB0_8:
  addq $88, %rsp
  popq %rbx
  popq %r12
  popq %r13
  popq %r14
  popq %r15
  popq %rbp
  retq

glibc_memcmp_common_alignment:
.LBB1_1:
  movq (%rdi), %rax
  movq (%rsi), %rcx
  cmpq %rcx, %rax
  jne .LBB1_2
  movq 8(%rsi), %rax
  movq 8(%rdi), %rcx
  cmpq %rax, %rcx
  jne .LBB1_20
  movq 16(%rdi), %rax
  movq 16(%rsi), %rcx
  cmpq %rcx, %rax
  jne .LBB1_2
  movq 24(%rdi), %rax
  movq 24(%rsi), %rcx
  cmpq %rcx, %rax
  jne .LBB1_2
  addq $32, %rdi
  addq $32, %rsi
  addq $-4, %rdx
  cmpq $3, %rdx
  ja .LBB1_1
  testq %rdx, %rdx
  je .LBB1_18
  xorl %r8d, %r8d
.LBB1_16:
  movq (%rdi,%r8,8), %rax
  movq (%rsi,%r8,8), %rcx
  cmpq %rcx, %rax
  jne .LBB1_2
  incq %r8
  cmpq %r8, %rdx
  jne .LBB1_16
.LBB1_18:
  xorl %eax, %eax
  retq
.LBB1_2:
  movl %eax, %edx
  movl %ecx, %esi
  cmpb %cl, %al
  jne .LBB1_11
  movl %ecx, %esi
  shrl $8, %esi
  movl %eax, %edx
  shrl $8, %edx
  cmpb %sil, %dl
  jne .LBB1_11
  movl %eax, %edx
  shrl $16, %edx
  movl %ecx, %esi
  shrl $16, %esi
  cmpb %sil, %dl
  jne .LBB1_11
  movl %eax, %edx
  shrl $24, %edx
  movl %ecx, %esi
  shrl $24, %esi
  cmpb %sil, %dl
  jne .LBB1_11
  movq %rax, %rdx
  shrq $32, %rdx
  movq %rcx, %rsi
  shrq $32, %rsi
  cmpb %sil, %dl
  jne .LBB1_11
  movq %rax, %rdx
  shrq $40, %rdx
  movq %rcx, %rsi
  shrq $40, %rsi
  cmpb %sil, %dl
  jne .LBB1_11
  movq %rax, %rdx
  shrq $48, %rdx
  movq %rcx, %rsi
  shrq $48, %rsi
  cmpb %sil, %dl
  jne .LBB1_11
  shrq $56, %rax
  shrq $56, %rcx
  cmpl %ecx, %eax
  je .LBB1_18
  movl %eax, %edx
  movl %ecx, %esi
  jmp .LBB1_11
.LBB1_20:
  movl %ecx, %edx
  movl %eax, %esi
  cmpb %al, %cl
  jne .LBB1_11
  movl %eax, %esi
  shrl $8, %esi
  movl %ecx, %edx
  shrl $8, %edx
  cmpb %sil, %dl
  jne .LBB1_11
  movl %ecx, %edx
  shrl $16, %edx
  movl %eax, %esi
  shrl $16, %esi
  cmpb %sil, %dl
  jne .LBB1_11
  movl %ecx, %edx
  shrl $24, %edx
  movl %eax, %esi
  shrl $24, %esi
  cmpb %sil, %dl
  jne .LBB1_11
  movq %rcx, %rdx
  shrq $32, %rdx
  movq %rax, %rsi
  shrq $32, %rsi
  cmpb %sil, %dl
  jne .LBB1_11
  movq %rcx, %rdx
  shrq $40, %rdx
  movq %rax, %rsi
  shrq $40, %rsi
  cmpb %sil, %dl
  jne .LBB1_11
  movq %rcx, %rdx
  shrq $48, %rdx
  movq %rax, %rsi
  shrq $48, %rsi
  cmpb %sil, %dl
  jne .LBB1_11
  shrq $56, %rcx
  shrq $56, %rax
  cmpl %eax, %ecx
  je .LBB1_18
  movl %ecx, %edx
  movl %eax, %esi
.LBB1_11:
  movzbl %sil, %ecx
  movzbl %dl, %eax
  subl %ecx, %eax
  retq

.L.str:
  .asciz "%lu\n"

