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

