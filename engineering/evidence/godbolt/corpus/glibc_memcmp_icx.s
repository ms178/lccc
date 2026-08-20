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
  subq $72, %rsp
  vstmxcsr (%rsp)
  orl $32832, (%rsp)
  vldmxcsr (%rsp)
  vmovups .LCPI0_0(%rip), %ymm0
  vmovups %ymm0, (%rsp)
  vmovups %ymm0, 32(%rsp)
  movq %rsp, %rdi
  leaq 32(%rsp), %rsi
  movl $4, %edx
  vzeroupper
  callq glibc_memcmp_common_alignment
  testl %eax, %eax
  je .LBB0_2
  movl $2, %ebp
  jmp .LBB0_8
.LBB0_2:
  movabsq $72623859790382857, %rax
  movq %rax, 40(%rsp)
  movq %rsp, %rdi
  leaq 32(%rsp), %rsi
  movl $4, %edx
  callq glibc_memcmp_common_alignment
  movl $2, %ebp
  testl %eax, %eax
  jns .LBB0_8
  movabsq $-7046029254386353131, %rcx
  movq $-65536, %rax
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
  movq %rdx, glibc_left+65536(%rax)
  movq %rdx, glibc_right+65536(%rax)
  movq %rdx, %rcx
  shlq $7, %rcx
  xorq %rdx, %rcx
  movq %rcx, %rdx
  shrq $9, %rdx
  xorq %rcx, %rdx
  movq %rdx, %rcx
  shlq $8, %rcx
  xorq %rdx, %rcx
  movq %rcx, glibc_left+65544(%rax)
  movq %rcx, glibc_right+65544(%rax)
  movq %rcx, %rdx
  shlq $7, %rdx
  xorq %rcx, %rdx
  movq %rdx, %rcx
  shrq $9, %rcx
  xorq %rdx, %rcx
  movq %rcx, %rdx
  shlq $8, %rdx
  xorq %rcx, %rdx
  movq %rdx, glibc_left+65552(%rax)
  movq %rdx, glibc_right+65552(%rax)
  movq %rdx, %rcx
  shlq $7, %rcx
  xorq %rdx, %rcx
  movq %rcx, %rdx
  shrq $9, %rdx
  xorq %rcx, %rdx
  movq %rdx, %rcx
  shlq $8, %rcx
  xorq %rdx, %rcx
  movq %rcx, glibc_left+65560(%rax)
  movq %rcx, glibc_right+65560(%rax)
  movq %rcx, %rdx
  shlq $7, %rdx
  xorq %rcx, %rdx
  movq %rdx, %rcx
  shrq $9, %rcx
  xorq %rdx, %rcx
  movq %rcx, %rdx
  shlq $8, %rdx
  xorq %rcx, %rdx
  movq %rdx, glibc_left+65568(%rax)
  movq %rdx, glibc_right+65568(%rax)
  movq %rdx, %rcx
  shlq $7, %rcx
  xorq %rdx, %rcx
  movq %rcx, %rdx
  shrq $9, %rdx
  xorq %rcx, %rdx
  movq %rdx, %rcx
  shlq $8, %rcx
  xorq %rdx, %rcx
  movq %rcx, glibc_left+65576(%rax)
  movq %rcx, glibc_right+65576(%rax)
  movq %rcx, %rdx
  shlq $7, %rdx
  xorq %rcx, %rdx
  movq %rdx, %rcx
  shrq $9, %rcx
  xorq %rdx, %rcx
  movq %rcx, %rdx
  shlq $8, %rdx
  xorq %rcx, %rdx
  movq %rdx, glibc_left+65584(%rax)
  movq %rdx, glibc_right+65584(%rax)
  movq %rdx, %rcx
  shlq $7, %rcx
  xorq %rdx, %rcx
  movq %rcx, %rdx
  shrq $9, %rdx
  xorq %rcx, %rdx
  movq %rdx, %rcx
  shlq $8, %rcx
  xorq %rdx, %rcx
  movq %rcx, glibc_left+65592(%rax)
  movq %rcx, glibc_right+65592(%rax)
  addq $64, %rax
  jne .LBB0_4
  movl $1, %r14d
  xorl %r15d, %r15d
  xorl %r12d, %r12d
  xorl %ebx, %ebx
.LBB0_6:
  movl %r12d, %r13d
  andl $8191, %r13d
  movq glibc_left(,%r13,8), %rbp
  movq %rbp, %rax
  btcq %r15, %rax
  movq %rax, glibc_right(,%r13,8)
  movl $glibc_left, %edi
  movl $glibc_right, %esi
  movl $8192, %edx
  callq glibc_memcmp_common_alignment
  addl $257, %eax
  imulq %r14, %rax
  addq %rax, %rbx
  movq %rbp, glibc_right(,%r13,8)
  addq $4051, %r12
  addq $11, %r15
  incq %r14
  cmpq $45056, %r15
  jne .LBB0_6
  xorl %ebp, %ebp
  movl $.L.str, %edi
  movq %rbx, %rsi
  xorl %eax, %eax
  callq printf
.LBB0_8:
  movl %ebp, %eax
  addq $72, %rsp
  popq %rbx
  popq %r12
  popq %r13
  popq %r14
  popq %r15
  popq %rbp
  retq

glibc_memcmp_common_alignment:
  cmpq $4, %rdx
  jb .LBB1_1
.LBB1_19:
  movq (%rdi), %rcx
  movq (%rsi), %r8
  cmpq %r8, %rcx
  jne .LBB1_9
  movq 8(%rsi), %rcx
  movq 8(%rdi), %r8
  cmpq %rcx, %r8
  jne .LBB1_21
  movq 16(%rdi), %rcx
  movq 16(%rsi), %r8
  cmpq %r8, %rcx
  jne .LBB1_9
  movq 24(%rdi), %rcx
  movq 24(%rsi), %r8
  cmpq %r8, %rcx
  jne .LBB1_9
  addq $32, %rdi
  addq $32, %rsi
  addq $-4, %rdx
  cmpq $3, %rdx
  ja .LBB1_19
.LBB1_1:
  testq %rdx, %rdx
  je .LBB1_31
  cmpq $1, %rdx
  je .LBB1_7
  movq %rdx, %rax
  shrq %rax
  movl $8, %r9d
.LBB1_4:
  movq -8(%rdi,%r9), %rcx
  movq -8(%rsi,%r9), %r8
  cmpq %r8, %rcx
  jne .LBB1_9
  movq (%rdi,%r9), %rcx
  movq (%rsi,%r9), %r8
  cmpq %r8, %rcx
  jne .LBB1_9
  addq $16, %r9
  decq %rax
  jne .LBB1_4
.LBB1_7:
  movl %edx, %r8d
  andl $2, %r8d
  xorl %eax, %eax
  cmpq %rdx, %r8
  jae .LBB1_32
  movq (%rdi,%r8,8), %rcx
  movq (%rsi,%r8,8), %r8
  cmpq %r8, %rcx
  je .LBB1_32
.LBB1_9:
  movl %ecx, %edx
  movl %r8d, %esi
  cmpb %r8b, %cl
  jne .LBB1_17
  movl %r8d, %esi
  shrl $8, %esi
  movl %ecx, %edx
  shrl $8, %edx
  cmpb %sil, %dl
  jne .LBB1_17
  movl %ecx, %edx
  shrl $16, %edx
  movl %r8d, %esi
  shrl $16, %esi
  cmpb %sil, %dl
  jne .LBB1_17
  movl %ecx, %edx
  shrl $24, %edx
  movl %r8d, %esi
  shrl $24, %esi
  cmpb %sil, %dl
  jne .LBB1_17
  movq %rcx, %rdx
  shrq $32, %rdx
  movq %r8, %rsi
  shrq $32, %rsi
  cmpb %sil, %dl
  jne .LBB1_17
  movq %rcx, %rdx
  shrq $40, %rdx
  movq %r8, %rsi
  shrq $40, %rsi
  cmpb %sil, %dl
  jne .LBB1_17
  movq %rcx, %rdx
  shrq $48, %rdx
  movq %r8, %rsi
  shrq $48, %rsi
  cmpb %sil, %dl
  jne .LBB1_17
  shrq $56, %rcx
  shrq $56, %r8
  xorl %eax, %eax
  movl %ecx, %edx
  movl %r8d, %esi
  cmpb %r8b, %cl
  je .LBB1_32
  jmp .LBB1_17
.LBB1_31:
  xorl %eax, %eax
.LBB1_32:
  retq
.LBB1_21:
  movl %r8d, %edx
  movl %ecx, %esi
  cmpb %cl, %r8b
  jne .LBB1_17
  movl %ecx, %esi
  shrl $8, %esi
  movl %r8d, %edx
  shrl $8, %edx
  cmpb %sil, %dl
  jne .LBB1_17
  movl %r8d, %edx
  shrl $16, %edx
  movl %ecx, %esi
  shrl $16, %esi
  cmpb %sil, %dl
  jne .LBB1_17
  movl %r8d, %edx
  shrl $24, %edx
  movl %ecx, %esi
  shrl $24, %esi
  cmpb %sil, %dl
  jne .LBB1_17
  movq %r8, %rdx
  shrq $32, %rdx
  movq %rcx, %rsi
  shrq $32, %rsi
  cmpb %sil, %dl
  jne .LBB1_17
  movq %r8, %rdx
  shrq $40, %rdx
  movq %rcx, %rsi
  shrq $40, %rsi
  cmpb %sil, %dl
  jne .LBB1_17
  movq %r8, %rdx
  shrq $48, %rdx
  movq %rcx, %rsi
  shrq $48, %rsi
  cmpb %sil, %dl
  jne .LBB1_17
  shrq $56, %r8
  shrq $56, %rcx
  xorl %eax, %eax
  movl %r8d, %edx
  movl %ecx, %esi
  cmpb %cl, %r8b
  je .LBB1_32
.LBB1_17:
  movzbl %sil, %ecx
  movzbl %dl, %eax
  subl %ecx, %eax
  retq

.L.str:
  .asciz "%lu\n"

