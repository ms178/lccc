linux_find_next_andnot_bit.part.0:
  movq %rsi, %r8
  movq %rcx, %rsi
  movq %rdx, %rax
  movl %ecx, %edx
  shrq $6, %rsi
  movq $-1, %rcx
  movq (%r8,%rsi,8), %r9
  shlx %rdx, %rcx, %rcx
  andq (%rdi,%rsi,8), %rcx
  andn %rcx, %r9, %rdx
  andn %rcx, %r9, %r9
  jne .L2
  leaq 1(%rsi), %rcx
  salq $6, %rcx
  jmp .L4
.L16:
  addq $1, %rsi
  movq (%r8,%rsi,8), %rdx
  andn (%rdi,%rsi,8), %rdx, %rdx
  testq %rdx, %rdx
  jne .L5
  addq $64, %rcx
.L4:
  cmpq %rax, %rcx
  jb .L16
  ret
.L2:
  movq %rsi, %rcx
  salq $6, %rcx
.L5:
  movq %rdx, %rdi
  xorl %esi, %esi
  shrq $32, %rdi
  testl $4294967295, %edx
  cmove %rdi, %rdx
  movl $32, %edi
  cmove %edi, %esi
  movq %rdx, %r8
  andl $65535, %r8d
  leal 16(%rsi), %edi
  cmove %edi, %esi
  movq %rdx, %rdi
  shrq $16, %rdi
  testq %r8, %r8
  cmove %rdi, %rdx
  leal 8(%rsi), %edi
  movq %rdx, %r8
  andl $255, %r8d
  cmove %edi, %esi
  movq %rdx, %rdi
  shrq $8, %rdi
  testq %r8, %r8
  cmove %rdi, %rdx
  leal 4(%rsi), %edi
  movq %rdx, %r8
  andl $15, %r8d
  cmove %edi, %esi
  movq %rdx, %rdi
  shrq $4, %rdi
  testq %r8, %r8
  cmove %rdi, %rdx
  leal 2(%rsi), %edi
  movq %rdx, %r8
  andl $3, %r8d
  cmove %edi, %esi
  movq %rdx, %rdi
  shrq $2, %rdi
  testq %r8, %r8
  cmove %rdi, %rdx
  andl $1, %edx
  cmpq $1, %rdx
  adcl $0, %esi
  addq %rcx, %rsi
  cmpq %rax, %rsi
  cmovbe %rsi, %rax
  ret
.LC2:
  .string "%lu\n"
main:
  subq $88, %rsp
  vmovdqa .LC0(%rip), %xmm0
  xorl %ecx, %ecx
  movl $192, %edx
  leaq 32(%rsp), %rsi
  movq %rsp, %rdi
  movq $0, 16(%rsp)
  vmovdqa %xmm0, (%rsp)
  vmovdqa .LC1(%rip), %xmm0
  movq $-1, 48(%rsp)
  vmovdqa %xmm0, 32(%rsp)
  call linux_find_next_andnot_bit.part.0
  cmpq $5, %rax
  jne .L20
  movl $6, %ecx
  movl $192, %edx
  leaq 32(%rsp), %rsi
  movq %rsp, %rdi
  movq %rbx, 64(%rsp)
  movq %rbp, 72(%rsp)
  movq %r12, 80(%rsp)
  call linux_find_next_andnot_bit.part.0
  cmpq $192, %rax
  jne .L34
  xorl %eax, %eax
.L24:
  movq $0, linux_bitmap_a(,%rax,8)
  movq %rax, %rdx
  movq $-1, linux_bitmap_b(,%rax,8)
  andl $63, %edx
  cmpq $5, %rdx
  je .L35
  addq $1, %rax
  cmpq $16384, %rax
  jne .L24
  xorl %r11d, %r11d
  xorl %ebp, %ebp
  xorl %r10d, %r10d
  xorl %ebx, %ebx
  movl $1, %r12d
  jmp .L23
.L25:
  movzbl %bpl, %edx
  leal 0(,%rbx,8), %eax
  addq $13, %rbp
  addq $524288, %r11
  subl %ebx, %eax
  salq $6, %rdx
  addl $1, %ebx
  shlx %rax, %r12, %rax
  xorq %rax, linux_bitmap_b+40(,%rdx,8)
  cmpl $1024, %ebx
  je .L36
.L23:
  movl %ebx, %ecx
  andl $63, %ecx
.L26:
  movl $1048576, %edx
  movl $linux_bitmap_b, %esi
  movl $linux_bitmap_a, %edi
  call linux_find_next_andnot_bit.part.0
  cmpq $1048576, %rax
  je .L25
  leaq (%r11,%rax), %rdx
  leaq 1(%rax), %rcx
  xorq %rdx, %r10
  cmpq $1048575, %rax
  jne .L26
  jmp .L25
.L35:
  movq $256, linux_bitmap_a(,%rax,8)
  movq $0, linux_bitmap_b(,%rax,8)
  addq $1, %rax
  jmp .L24
.L34:
  movq 64(%rsp), %rbx
  movq 72(%rsp), %rbp
  movq 80(%rsp), %r12
.L20:
  movl $2, %eax
.L17:
  addq $88, %rsp
  ret
.L36:
  movq %r10, %rsi
  movl $.LC2, %edi
  xorl %eax, %eax
  call printf
  movq 64(%rsp), %rbx
  movq 72(%rsp), %rbp
  xorl %eax, %eax
  movq 80(%rsp), %r12
  jmp .L17
.LC0:
  .quad 32
  .quad 4
.LC1:
  .quad 0
  .quad -1
