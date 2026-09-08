chacha20_core:
  pushq %rbp
  movq %rsp, %rbp
  pushq %r15
  pushq %r14
  movq %rsi, %r14
  pushq %r13
  pushq %r12
  pushq %rbx
  andq $-32, %rsp
  subq $8, %rsp
  movq %rdi, -88(%rsp)
  movq (%rsi), %rax
  movq 16(%r14), %rcx
  movq 24(%r14), %rdx
  movq 8(%rsi), %rsi
  vmovdqu 32(%r14), %ymm0
  movq %rax, -56(%rsp)
  movl %eax, %r13d
  movq %rcx, -40(%rsp)
  movl -52(%rsp), %r9d
  vmovdqa %ymm0, -24(%rsp)
  movl -40(%rsp), %r10d
  movl -8(%rsp), %ecx
  movq %rsi, -48(%rsp)
  movl -24(%rsp), %edi
  movq %rdx, -32(%rsp)
  movl -36(%rsp), %r11d
  movl -32(%rsp), %eax
  movl -16(%rsp), %ebx
  movl $10, -80(%rsp)
  movl -28(%rsp), %r15d
  movl -4(%rsp), %edx
  movq %r14, -96(%rsp)
  movl -20(%rsp), %esi
  movl -48(%rsp), %r8d
  movl %ebx, -60(%rsp)
  movl -12(%rsp), %r12d
  movl -44(%rsp), %ebx
  movl %eax, -72(%rsp)
  movl -60(%rsp), %r14d
  movl (%rsp), %eax
  movl %r15d, -76(%rsp)
  movl 4(%rsp), %r15d
.L2:
  addl %r11d, %r9d
  addl %r10d, %r13d
  xorl %r9d, %edx
  xorl %r13d, %ecx
  rorx $16, %edx, %edx
  addl %edx, %esi
  rorx $16, %ecx, %ecx
  addl %ecx, %edi
  xorl %esi, %r11d
  xorl %edi, %r10d
  rorx $20, %r11d, %r11d
  addl %r11d, %r9d
  rorx $20, %r10d, %r10d
  addl %r10d, %r13d
  xorl %r9d, %edx
  xorl %r13d, %ecx
  rorx $24, %edx, %edx
  addl %edx, %esi
  rorx $24, %ecx, %ecx
  addl %ecx, %edi
  xorl %esi, %r11d
  movl %esi, -68(%rsp)
  movl -72(%rsp), %esi
  xorl %edi, %r10d
  movl %edi, -60(%rsp)
  rorx $25, %r10d, %edi
  rorx $25, %r11d, %r11d
  addl %r11d, %r13d
  addl %esi, %r8d
  movl %edi, -64(%rsp)
  xorl %r8d, %eax
  rorx $16, %eax, %eax
  addl %eax, %r14d
  xorl %r14d, %esi
  rorx $20, %esi, %edi
  movl -76(%rsp), %esi
  addl %edi, %r8d
  xorl %r8d, %eax
  leal (%rsi,%rbx), %r10d
  rorx $24, %eax, %eax
  addl %eax, %r14d
  xorl %r10d, %r15d
  xorl %r14d, %edi
  rorx $16, %r15d, %ebx
  addl %ebx, %r12d
  rorx $25, %edi, %edi
  xorl %r12d, %esi
  rorx $20, %esi, %esi
  addl %esi, %r10d
  xorl %r10d, %ebx
  rorx $24, %ebx, %ebx
  addl %ebx, %r12d
  xorl %r12d, %esi
  xorl %r13d, %ebx
  addl %edi, %r9d
  xorl %r9d, %ecx
  rorx $16, %ebx, %ebx
  addl %ebx, %r14d
  rorx $25, %esi, %esi
  rorx $16, %ecx, %ecx
  addl %ecx, %r12d
  xorl %r14d, %r11d
  addl %esi, %r8d
  xorl %r12d, %edi
  rorx $20, %r11d, %r11d
  addl %r11d, %r13d
  xorl %r8d, %edx
  rorx $20, %edi, %edi
  addl %edi, %r9d
  xorl %r13d, %ebx
  rorx $16, %edx, %edx
  xorl %r9d, %ecx
  rorx $24, %ebx, %r15d
  addl %r15d, %r14d
  rorx $24, %ecx, %ecx
  addl %ecx, %r12d
  xorl %r14d, %r11d
  xorl %r12d, %edi
  rorx $25, %r11d, %r11d
  rorx $25, %edi, %ebx
  movl -60(%rsp), %edi
  movl %ebx, -72(%rsp)
  addl %edx, %edi
  xorl %edi, %esi
  rorx $20, %esi, %esi
  addl %esi, %r8d
  xorl %r8d, %edx
  rorx $24, %edx, %edx
  addl %edx, %edi
  xorl %edi, %esi
  rorx $25, %esi, %ebx
  movl %ebx, -76(%rsp)
  movl -64(%rsp), %ebx
  movl -68(%rsp), %esi
  addl %r10d, %ebx
  movl -64(%rsp), %r10d
  xorl %ebx, %eax
  rorx $16, %eax, %eax
  addl %eax, %esi
  xorl %esi, %r10d
  rorx $20, %r10d, %r10d
  addl %r10d, %ebx
  xorl %ebx, %eax
  rorx $24, %eax, %eax
  addl %eax, %esi
  xorl %esi, %r10d
  subl $1, -80(%rsp)
  rorx $25, %r10d, %r10d
  jne .L2
  movl %r14d, -60(%rsp)
  movq -96(%rsp), %r14
  movl %r13d, -64(%rsp)
  movl %r13d, -56(%rsp)
  movl -60(%rsp), %r13d
  movl %eax, -68(%rsp)
  movl %r13d, -16(%rsp)
  movl -72(%rsp), %r13d
  movl %eax, (%rsp)
  movq -88(%rsp), %rax
  movl %r13d, -32(%rsp)
  movl -76(%rsp), %r13d
  movl %r15d, 4(%rsp)
  movl %r13d, -28(%rsp)
  leaq -4(%rax), %r13
  subq %r14, %r13
  movl %r11d, -36(%rsp)
  movl %r9d, -52(%rsp)
  movl %ecx, -8(%rsp)
  movl %r12d, -12(%rsp)
  movl %r8d, -48(%rsp)
  movl %edx, -4(%rsp)
  movl %edi, -24(%rsp)
  movl %ebx, -44(%rsp)
  movl %esi, -20(%rsp)
  movl %r10d, -40(%rsp)
  cmpq $24, %r13
  jbe .L3
  vmovdqu (%r14), %ymm0
  vpaddd -56(%rsp), %ymm0, %ymm0
  vmovdqu %ymm0, (%rax)
  vmovdqu 32(%r14), %ymm0
  vpaddd -24(%rsp), %ymm0, %ymm0
  vmovdqu %ymm0, 32(%rax)
.L6:
  vzeroupper
  leaq -40(%rbp), %rsp
  popq %rbx
  popq %r12
  popq %r13
  popq %r14
  popq %r15
  popq %rbp
  ret
.L3:
  movl -64(%rsp), %r13d
  addl (%r14), %r13d
  movl %r13d, %eax
  movq -88(%rsp), %r13
  movl %eax, 0(%r13)
  addl 4(%r14), %r9d
  movl %r9d, 4(%r13)
  addl 8(%r14), %r8d
  movl %r8d, 8(%r13)
  addl 12(%r14), %ebx
  movl %ebx, 12(%r13)
  addl 16(%r14), %r10d
  movl %r10d, 16(%r13)
  addl 20(%r14), %r11d
  movl %r11d, 20(%r13)
  movl -72(%rsp), %r8d
  addl 24(%r14), %r8d
  movl %r8d, 24(%r13)
  movl -76(%rsp), %r8d
  addl 28(%r14), %r8d
  movl %r8d, 28(%r13)
  addl 32(%r14), %edi
  movl %edi, 32(%r13)
  addl 36(%r14), %esi
  movl %esi, 36(%r13)
  movl -60(%rsp), %esi
  addl 40(%r14), %esi
  movl %esi, 40(%r13)
  movl 44(%r14), %esi
  addl %r12d, %esi
  movl %esi, 44(%r13)
  addl 48(%r14), %ecx
  movl %ecx, 48(%r13)
  addl 52(%r14), %edx
  movl %edx, 52(%r13)
  movl -68(%rsp), %eax
  addl 56(%r14), %eax
  movl %eax, 56(%r13)
  movl 60(%r14), %eax
  addl %r15d, %eax
  movl %eax, 60(%r13)
  jmp .L6
.LC4:
  .string "%016llx\n"
main:
  subq $168, %rsp
  movl $test_in.0, %esi
  leaq 64(%rsp), %rdi
  call chacha20_core
  cmpl $-454561520, 64(%rsp)
  jne .L10
  cmpl $358169553, 68(%rsp)
  je .L18
.L10:
  movl $2, %eax
  addq $168, %rsp
  ret
.L18:
  cmpl $-394014517, 120(%rsp)
  jne .L10
  movq %rbx, 136(%rsp)
  movq %rbp, 144(%rsp)
  movq %r12, 152(%rsp)
  movq %r13, 160(%rsp)
  cmpl $1312575650, 124(%rsp)
  je .L19
  movq 136(%rsp), %rbx
  movq 144(%rsp), %rbp
  movq 152(%rsp), %r12
  movq 160(%rsp), %r13
  jmp .L10
.L19:
  vmovdqa .LC0(%rip), %xmm0
  movl $67372036, %r13d
  xorl %ebp, %ebp
  xorl %ebx, %ebx
  movabsq $723401728363923721, %rax
  movl $185273099, 44(%rsp)
  movq %rax, 36(%rsp)
  movabsq $81985530642677487, %rax
  vmovdqa %xmm0, (%rsp)
  vmovdqa .LC1(%rip), %xmm0
  movq %rax, 52(%rsp)
  movl $-1985229329, 60(%rsp)
  vmovdqu %xmm0, 20(%rsp)
.L11:
  xorl %r12d, %r12d
.L13:
  movl %r12d, %eax
  movq %rsp, %rsi
  leaq 64(%rsp), %rdi
  movl %r13d, 16(%rsp)
  xorl %ebp, %eax
  addl $1, %r12d
  movl %eax, 48(%rsp)
  call chacha20_core
  movl 64(%rsp), %edx
  movl 92(%rsp), %eax
  movq %rdx, %rcx
  salq $16, %rax
  movzbl %dl, %edx
  salq $32, %rcx
  xorl %edx, %r13d
  xorq %rcx, %rax
  movl 124(%rsp), %ecx
  xorq %rcx, %rax
  addq %rax, %rbx
  cmpl $131072, %r12d
  jne .L13
  addl $1, %ebp
  cmpl $16, %ebp
  jne .L11
  movq %rbx, %rsi
  movl $.LC4, %edi
  xorl %eax, %eax
  call printf
  movq 136(%rsp), %rbx
  xorl %eax, %eax
  movq 144(%rsp), %rbp
  movq 152(%rsp), %r12
  movq 160(%rsp), %r13
  addq $168, %rsp
  ret
test_in.0:
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
.LC0:
  .long 1634760805
  .long 857760878
  .long 2036477234
  .long 1797285236
.LC1:
  .long 84215045
  .long 101058054
  .long 117901063
  .long 134744072
