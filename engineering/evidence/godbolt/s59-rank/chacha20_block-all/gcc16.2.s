chacha20_core:
  pushq %rbp
  movq %rsp, %rbp
  pushq %r15
  pushq %r14
  movq %rdi, %r14
  pushq %r13
  movq %rsi, %r13
  pushq %r12
  pushq %rbx
  subq $8, %rsp
  movq (%rsi), %rax
  movq 16(%r13), %rcx
  movq 8(%rsi), %rsi
  movq 24(%r13), %rdx
  vmovdqu 32(%r13), %ymm0
  movq %rcx, -48(%rsp)
  movq %rax, -64(%rsp)
  movl -48(%rsp), %r10d
  vmovdqu %ymm0, -32(%rsp)
  movl -60(%rsp), %r9d
  movl -16(%rsp), %ecx
  movl -32(%rsp), %edi
  movl -44(%rsp), %r11d
  movq %rsi, -56(%rsp)
  movl %eax, -68(%rsp)
  movq %rdx, -40(%rsp)
  movl -12(%rsp), %edx
  movl -40(%rsp), %eax
  movl -24(%rsp), %ebx
  movl $10, -88(%rsp)
  movl -36(%rsp), %r15d
  movl -28(%rsp), %esi
  movq %r14, -96(%rsp)
  movl -56(%rsp), %r8d
  movl -20(%rsp), %r12d
  movl %ebx, -72(%rsp)
  movl -52(%rsp), %ebx
  movl -72(%rsp), %r14d
  movl %eax, -80(%rsp)
  movl %r15d, -84(%rsp)
  movl -8(%rsp), %eax
  movq %r13, -104(%rsp)
  movl -4(%rsp), %r15d
  movl -68(%rsp), %r13d
.L2:
  addl %r10d, %r13d
  addl %r11d, %r9d
  xorl %r13d, %ecx
  xorl %r9d, %edx
  rorx $16, %ecx, %ecx
  addl %ecx, %edi
  rorx $16, %edx, %edx
  addl %edx, %esi
  xorl %edi, %r10d
  xorl %esi, %r11d
  rorx $20, %r10d, %r10d
  addl %r10d, %r13d
  rorx $20, %r11d, %r11d
  addl %r11d, %r9d
  xorl %r13d, %ecx
  xorl %r9d, %edx
  rorx $24, %ecx, %ecx
  addl %ecx, %edi
  rorx $24, %edx, %edx
  addl %edx, %esi
  xorl %edi, %r10d
  movl %edi, -68(%rsp)
  xorl %esi, %r11d
  rorx $25, %r10d, %edi
  movl -80(%rsp), %r10d
  movl %esi, -76(%rsp)
  rorx $25, %r11d, %r11d
  movl %edi, -72(%rsp)
  addl %r11d, %r13d
  addl %r10d, %r8d
  xorl %r8d, %eax
  rorx $16, %eax, %eax
  leal (%rax,%r14), %esi
  xorl %esi, %r10d
  rorx $20, %r10d, %edi
  addl %edi, %r8d
  xorl %r8d, %eax
  rorx $24, %eax, %eax
  leal (%rsi,%rax), %r14d
  movl -84(%rsp), %esi
  xorl %r14d, %edi
  leal (%rsi,%rbx), %r10d
  rorx $25, %edi, %edi
  xorl %r10d, %r15d
  rorx $16, %r15d, %ebx
  addl %ebx, %r12d
  xorl %r12d, %esi
  rorx $20, %esi, %esi
  addl %esi, %r10d
  xorl %r10d, %ebx
  rorx $24, %ebx, %ebx
  addl %ebx, %r12d
  xorl %r13d, %ebx
  rorx $16, %ebx, %ebx
  addl %ebx, %r14d
  xorl %r12d, %esi
  xorl %r14d, %r11d
  addl %edi, %r9d
  rorx $25, %esi, %esi
  addl %esi, %r8d
  xorl %r9d, %ecx
  rorx $20, %r11d, %r11d
  addl %r11d, %r13d
  xorl %r8d, %edx
  rorx $16, %ecx, %ecx
  addl %ecx, %r12d
  xorl %r13d, %ebx
  rorx $16, %edx, %edx
  xorl %r12d, %edi
  rorx $24, %ebx, %r15d
  addl %r15d, %r14d
  rorx $20, %edi, %edi
  addl %edi, %r9d
  xorl %r14d, %r11d
  xorl %r9d, %ecx
  rorx $25, %r11d, %r11d
  rorx $24, %ecx, %ecx
  addl %ecx, %r12d
  xorl %r12d, %edi
  rorx $25, %edi, %ebx
  movl -68(%rsp), %edi
  movl %ebx, -80(%rsp)
  addl %edx, %edi
  xorl %edi, %esi
  rorx $20, %esi, %esi
  addl %esi, %r8d
  xorl %r8d, %edx
  rorx $24, %edx, %edx
  addl %edx, %edi
  xorl %edi, %esi
  rorx $25, %esi, %ebx
  movl %ebx, -84(%rsp)
  movl -72(%rsp), %ebx
  movl -76(%rsp), %esi
  addl %r10d, %ebx
  movl -72(%rsp), %r10d
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
  subl $1, -88(%rsp)
  rorx $25, %r10d, %r10d
  jne .L2
  movl %r13d, -68(%rsp)
  movq -104(%rsp), %r13
  movl %eax, -76(%rsp)
  movl -68(%rsp), %eax
  movl %r14d, -72(%rsp)
  movq -96(%rsp), %r14
  movl %eax, -64(%rsp)
  movl -72(%rsp), %eax
  movl %r15d, -4(%rsp)
  movl %eax, -24(%rsp)
  movl -80(%rsp), %eax
  movl %r11d, -44(%rsp)
  movl %eax, -40(%rsp)
  movl -84(%rsp), %eax
  movl %r9d, -60(%rsp)
  movl %eax, -36(%rsp)
  movl -76(%rsp), %eax
  movl %r12d, -20(%rsp)
  movl %r8d, -56(%rsp)
  movl %edx, -12(%rsp)
  movl %edi, -32(%rsp)
  movl %ebx, -52(%rsp)
  movl %esi, -28(%rsp)
  movl %r10d, -48(%rsp)
  movl %ecx, -16(%rsp)
  leaq -64(%rsp), %rcx
  movl %eax, -8(%rsp)
  xorl %eax, %eax
.L3:
  movl 0(%r13,%rax), %edx
  addl (%rcx,%rax), %edx
  movl %edx, (%r14,%rax)
  addq $4, %rax
  cmpq $64, %rax
  jne .L3
  vzeroupper
  addq $8, %rsp
  popq %rbx
  popq %r12
  popq %r13
  popq %r14
  popq %r15
  popq %rbp
  ret
.LC4:
main:
  subq $168, %rsp
  movl $test_in.0, %esi
  leaq 64(%rsp), %rdi
  call chacha20_core
  cmpl $-454561520, 64(%rsp)
  jne .L9
  cmpl $358169553, 68(%rsp)
  je .L17
.L9:
  movl $2, %eax
  addq $168, %rsp
  ret
.L17:
  cmpl $-394014517, 120(%rsp)
  jne .L9
  movq %rbx, 136(%rsp)
  movq %rbp, 144(%rsp)
  movq %r12, 152(%rsp)
  movq %r13, 160(%rsp)
  cmpl $1312575650, 124(%rsp)
  je .L18
  movq 136(%rsp), %rbx
  movq 144(%rsp), %rbp
  movq 152(%rsp), %r12
  movq 160(%rsp), %r13
  jmp .L9
.L18:
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
.L10:
  xorl %r12d, %r12d
.L12:
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
  jne .L12
  addl $1, %ebp
  cmpl $16, %ebp
  jne .L10
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
.LC0:
.LC1:
