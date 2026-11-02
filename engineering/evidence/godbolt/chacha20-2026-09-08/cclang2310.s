.LCPI0_0:
  .long 3840405776
  .long 358169553
  .long 3900952779
  .long 1312575650
.LCPI0_2:
  .long 134744072
  .long 151587081
  .long 168430090
  .long 185273099
.LCPI0_1:
  .long 1634760805
  .long 857760878
  .long 2036477234
  .long 1797285236
  .long 67372036
  .long 84215045
  .long 101058054
  .long 117901063
main:
  pushq %rbp
  pushq %r15
  pushq %r14
  pushq %r13
  pushq %r12
  pushq %rbx
  subq $136, %rsp
  leaq check_known_vector.test_in(%rip), %rsi
  movq %rsp, %rdi
  callq chacha20_core
  vmovdqu (%rsp), %ymm0
  vmovdqu 32(%rsp), %ymm1
  vextracti128 $1, %ymm1, %xmm1
  vpblendd $12, %xmm1, %xmm0, %xmm0
  vpxor .LCPI0_0(%rip), %xmm0, %xmm0
  vptest %xmm0, %xmm0
  movl $2, %ebp
  jne .LBB0_6
  vmovaps .LCPI0_1(%rip), %ymm0
  vmovups %ymm0, (%rsp)
  vmovdqa .LCPI0_2(%rip), %xmm0
  vmovdqa %xmm0, 32(%rsp)
  movabsq $81985530642677487, %rax
  movq %rax, 52(%rsp)
  movl $-1985229329, 60(%rsp)
  movl $67372036, %ebp
  xorl %ebx, %ebx
  leaq 64(%rsp), %r14
  movq %rsp, %r15
  xorl %r12d, %r12d
.LBB0_2:
  xorl %r13d, %r13d
.LBB0_3:
  movl %r13d, %eax
  xorl %r12d, %eax
  movl %eax, 48(%rsp)
  movq %r14, %rdi
  movq %r15, %rsi
  vzeroupper
  callq chacha20_core
  movl 64(%rsp), %eax
  movl 92(%rsp), %ecx
  movzbl %al, %edx
  shlq $32, %rax
  movl 124(%rsp), %esi
  orq %rax, %rsi
  shlq $16, %rcx
  xorq %rsi, %rcx
  addq %rcx, %rbx
  xorl %edx, %ebp
  movl %ebp, 16(%rsp)
  incl %r13d
  cmpl $131072, %r13d
  jne .LBB0_3
  incl %r12d
  cmpl $16, %r12d
  jne .LBB0_2
  leaq .L.str(%rip), %rdi
  xorl %ebp, %ebp
  movq %rbx, %rsi
  xorl %eax, %eax
  callq printf@PLT
.LBB0_6:
  movl %ebp, %eax
  addq $136, %rsp
  popq %rbx
  popq %r12
  popq %r13
  popq %r14
  popq %r15
  popq %rbp
  vzeroupper
  retq

chacha20_core:
  pushq %rbp
  pushq %r15
  pushq %r14
  pushq %r13
  pushq %r12
  pushq %rbx
  movq %rdi, -8(%rsp)
  movl (%rsi), %ebx
  movl 4(%rsi), %r11d
  movl 8(%rsi), %r10d
  movl 12(%rsi), %r9d
  movl 16(%rsi), %ebp
  movl 20(%rsi), %r13d
  movl 24(%rsi), %edx
  movl 28(%rsi), %eax
  movl %eax, -36(%rsp)
  movl 32(%rsi), %r8d
  movl 36(%rsi), %ecx
  movl 40(%rsi), %eax
  movl %eax, -44(%rsp)
  movl 44(%rsi), %eax
  movl %eax, -40(%rsp)
  movl %edx, %eax
  movl 48(%rsi), %r12d
  movl 52(%rsi), %edi
  movl 56(%rsi), %r14d
  movq %rsi, -16(%rsp)
  movl 60(%rsi), %esi
  movl %esi, -32(%rsp)
  movl $10, %esi
.LBB1_1:
  movl %esi, -20(%rsp)
  addl %ebp, %ebx
  xorl %ebx, %r12d
  rorxl $16, %r12d, %esi
  addl %esi, %r8d
  xorl %r8d, %ebp
  rorxl $20, %ebp, %ebp
  addl %ebp, %ebx
  xorl %ebx, %esi
  rorxl $24, %esi, %esi
  addl %esi, %r8d
  xorl %r8d, %ebp
  rorxl $25, %ebp, %edx
  movl %edx, -28(%rsp)
  addl %r13d, %r11d
  xorl %r11d, %edi
  rorxl $16, %edi, %edi
  addl %edi, %ecx
  xorl %ecx, %r13d
  rorxl $20, %r13d, %ebp
  addl %ebp, %r11d
  xorl %r11d, %edi
  rorxl $24, %edi, %edi
  addl %edi, %ecx
  movl %ecx, -24(%rsp)
  xorl %ecx, %ebp
  rorxl $25, %ebp, %r13d
  addl %eax, %r10d
  xorl %r10d, %r14d
  rorxl $16, %r14d, %ebp
  movl -44(%rsp), %edx
  addl %ebp, %edx
  xorl %edx, %eax
  rorxl $20, %eax, %r14d
  addl %r14d, %r10d
  xorl %r10d, %ebp
  rorxl $24, %ebp, %ebp
  addl %ebp, %edx
  xorl %edx, %r14d
  rorxl $25, %r14d, %r14d
  movl -36(%rsp), %eax
  addl %eax, %r9d
  movl -32(%rsp), %r15d
  xorl %r9d, %r15d
  rorxl $16, %r15d, %r12d
  movl -40(%rsp), %ecx
  addl %r12d, %ecx
  xorl %ecx, %eax
  rorxl $20, %eax, %r15d
  addl %r15d, %r9d
  xorl %r9d, %r12d
  rorxl $24, %r12d, %r12d
  addl %r12d, %ecx
  xorl %ecx, %r15d
  rorxl $25, %r15d, %r15d
  addl %r13d, %ebx
  xorl %ebx, %r12d
  rorxl $16, %r12d, %r12d
  addl %r12d, %edx
  xorl %edx, %r13d
  rorxl $20, %r13d, %r13d
  addl %r13d, %ebx
  xorl %ebx, %r12d
  rorxl $24, %r12d, %r12d
  movl %r12d, -32(%rsp)
  addl %r12d, %edx
  movl %edx, -44(%rsp)
  xorl %edx, %r13d
  rorxl $25, %r13d, %r13d
  addl %r14d, %r11d
  xorl %r11d, %esi
  rorxl $16, %esi, %esi
  addl %esi, %ecx
  xorl %ecx, %r14d
  rorxl $20, %r14d, %r14d
  addl %r14d, %r11d
  xorl %r11d, %esi
  rorxl $24, %esi, %r12d
  movl -20(%rsp), %esi
  addl %r12d, %ecx
  movl %ecx, -40(%rsp)
  xorl %ecx, %r14d
  movl -24(%rsp), %ecx
  rorxl $25, %r14d, %eax
  addl %r15d, %r10d
  xorl %r10d, %edi
  rorxl $16, %edi, %edi
  addl %edi, %r8d
  xorl %r8d, %r15d
  rorxl $20, %r15d, %r14d
  addl %r14d, %r10d
  xorl %r10d, %edi
  rorxl $24, %edi, %edi
  addl %edi, %r8d
  xorl %r8d, %r14d
  rorxl $25, %r14d, %r14d
  movl %r14d, -36(%rsp)
  movl -28(%rsp), %edx
  addl %edx, %r9d
  xorl %r9d, %ebp
  rorxl $16, %ebp, %ebp
  addl %ebp, %ecx
  xorl %ecx, %edx
  rorxl $20, %edx, %r15d
  addl %r15d, %r9d
  xorl %r9d, %ebp
  rorxl $24, %ebp, %r14d
  addl %r14d, %ecx
  xorl %ecx, %r15d
  rorxl $25, %r15d, %ebp
  decl %esi
  jne .LBB1_1
  movq -16(%rsp), %rsi
  addl (%rsi), %ebx
  movq -8(%rsp), %r15
  movl %ebx, (%r15)
  addl 4(%rsi), %r11d
  movl %r11d, 4(%r15)
  addl 8(%rsi), %r10d
  movl %r10d, 8(%r15)
  addl 12(%rsi), %r9d
  movl %r9d, 12(%r15)
  addl 16(%rsi), %ebp
  movl %ebp, 16(%r15)
  addl 20(%rsi), %r13d
  movl %r13d, 20(%r15)
  addl 24(%rsi), %eax
  movl %eax, 24(%r15)
  movl -36(%rsp), %r9d
  addl 28(%rsi), %r9d
  movl %r9d, 28(%r15)
  addl 32(%rsi), %r8d
  movl %r8d, 32(%r15)
  addl 36(%rsi), %ecx
  movl %ecx, 36(%r15)
  movl -44(%rsp), %eax
  addl 40(%rsi), %eax
  movl %eax, 40(%r15)
  movl -40(%rsp), %eax
  addl 44(%rsi), %eax
  movl %eax, 44(%r15)
  addl 48(%rsi), %r12d
  movl %r12d, 48(%r15)
  addl 52(%rsi), %edi
  movl %edi, 52(%r15)
  addl 56(%rsi), %r14d
  movl %r14d, 56(%r15)
  movl -32(%rsp), %eax
  addl 60(%rsi), %eax
  movl %eax, 60(%r15)
  popq %rbx
  popq %r12
  popq %r13
  popq %r14
  popq %r15
  popq %rbp
  retq

.L.str:
  .asciz "%016llx\n"

check_known_vector.test_in:
  .long 1634760805
  .long 857760878
  .long 2036477234
  .long 1797285236
  .long 50462976
  .long 117835012
  .long 185207048
  .long 252579084
  .long 319951120
  .long 387323156
  .long 454695192
  .long 522067228
  .long 1
  .long 150994944
  .long 1241513984
  .long 0

