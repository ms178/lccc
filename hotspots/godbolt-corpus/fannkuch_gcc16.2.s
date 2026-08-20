.LC2:
  .string "%d\nPfannkuchen(%d) = %d\n"
main:
  pushq %rbp
  xorl %r10d, %r10d
  xorl %r11d, %r11d
  xorl %r9d, %r9d
  movabsq $38654705672, %rax
  movl $11, %r8d
  movq %rsp, %rbp
  pushq %r13
  pushq %r12
  movl $1, %r12d
  pushq %rbx
  xorl %ebx, %ebx
  subq $8, %rsp
  movq %rax, perm1+32(%rip)
  vmovdqa .LC0(%rip), %ymm0
  movl $0, maxflips(%rip)
  movl $0, checksum(%rip)
  movl $10, perm1+40(%rip)
  vmovdqa %ymm0, perm1(%rip)
.L2:
  cmpl $1, %r8d
  jle .L31
.L3:
  leal -1(%r8), %eax
  movl %r8d, count(,%rax,4)
  subl $1, %r8d
  cmpl $1, %r8d
  jg .L3
.L31:
  movq perm1+8(%rip), %rax
  movq perm1(%rip), %rdx
  xorl %r13d, %r13d
  vmovdqu perm1+28(%rip), %xmm0
  movq %rax, perm+8(%rip)
  movq perm1+16(%rip), %rax
  movq %rdx, perm(%rip)
  movq %rax, perm+16(%rip)
  movq perm1+24(%rip), %rax
  movq %rax, perm+24(%rip)
  vmovdqu %xmm0, perm+28(%rip)
  testl %edx, %edx
  je .L4
.L7:
  leal 1(%rdx), %ecx
  sarl %ecx
  testl %ecx, %ecx
  jle .L5
  movl %edx, %edx
  movl $perm, %eax
  leaq perm(,%rcx,4), %rdi
  leaq perm(,%rdx,4), %rdx
.L6:
  movl (%rdx), %esi
  movl (%rax), %ecx
  addq $4, %rax
  subq $4, %rdx
  movl %esi, -4(%rax)
  movl %ecx, 4(%rdx)
  cmpq %rax, %rdi
  jne .L6
  movl perm(%rip), %edx
.L5:
  addl $1, %r13d
  testl %edx, %edx
  jne .L7
.L4:
  cmpl %r9d, %r13d
  movl %r13d, %eax
  cmovg %r13d, %r9d
  cmovg %r12d, %r11d
  negl %eax
  testb $1, %bl
  cmovne %eax, %r13d
  movslq %r8d, %rax
  addl %r13d, %r10d
  jmp .L18
.L34:
  movl %edx, %edx
  vmovdqu perm1+4(%rip), %ymm1
  vmovdqu perm1-28(%rdx), %ymm0
  vmovdqu %ymm1, perm1(%rip)
  vmovdqu %ymm0, perm1-32(%rdx)
.L10:
  movl count(,%rax,4), %edi
  movl %ecx, perm1(,%rax,4)
  leal -1(%rdi), %edx
  movl %edx, count(,%rax,4)
  testl %edx, %edx
  jg .L32
  addq $1, %rax
  cmpl $11, %eax
  je .L33
.L18:
  movl perm1(%rip), %ecx
  testl %eax, %eax
  jle .L10
  movl %eax, %edx
  salq $2, %rdx
  cmpl $32, %edx
  jnb .L34
  cmpl $16, %edx
  jnb .L13
  cmpl $8, %edx
  jb .L35
  movl %edx, %edx
  movq perm1+4(%rip), %rdi
  movq perm1-4(%rdx), %rsi
  movq %rdi, perm1(%rip)
  movq %rsi, perm1-8(%rdx)
  jmp .L10
.L13:
  movl %edx, %edx
  vmovdqu perm1+4(%rip), %xmm1
  vmovdqu perm1-12(%rdx), %xmm0
  vmovdqu %xmm1, perm1(%rip)
  vmovdqu %xmm0, perm1-16(%rdx)
  jmp .L10
.L35:
  movl %edx, %edx
  movl perm1+4(%rip), %edi
  movl perm1(%rdx), %esi
  movl %edi, perm1(%rip)
  movl %esi, perm1-4(%rdx)
  jmp .L10
.L33:
  movl %r10d, checksum(%rip)
  testb %r11b, %r11b
  je .L22
  movl %r9d, maxflips(%rip)
.L19:
  xorl %eax, %eax
  movl %r9d, %ecx
  movl $11, %edx
  movl %r10d, %esi
  movl $.LC2, %edi
  vzeroupper
  call printf
  addq $8, %rsp
  xorl %eax, %eax
  popq %rbx
  popq %r12
  popq %r13
  popq %rbp
  ret
.L32:
  movl %eax, %r8d
  addl $1, %ebx
  jmp .L2
.L22:
  xorl %r9d, %r9d
  jmp .L19
.LC0:
  .long 0
  .long 1
  .long 2
  .long 3
  .long 4
  .long 5
  .long 6
  .long 7
