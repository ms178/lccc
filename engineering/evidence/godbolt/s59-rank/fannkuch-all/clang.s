.LCPI0_0:
.LCPI0_1:
.LCPI0_2:
.LCPI0_3:
.LCPI0_4:
.LCPI0_5:
.LCPI0_6:
main:
  pushq %rbp
  pushq %r15
  pushq %r14
  pushq %r13
  pushq %r12
  pushq %rbx
  subq $152, %rsp
  vmovdqa .LCPI0_0(%rip), %ymm0
  vmovdqa %ymm0, perm1(%rip)
  movabsq $38654705672, %rax
  movq %rax, perm1+32(%rip)
  movl $10, perm1+40(%rip)
  movl $11, %r15d
  xorl %r14d, %r14d
  vmovdqa .LCPI0_2(%rip), %ymm5
  vpbroadcastd .LCPI0_3(%rip), %ymm6
  vpbroadcastd .LCPI0_4(%rip), %ymm7
  vpbroadcastd .LCPI0_5(%rip), %ymm8
  vpbroadcastd .LCPI0_6(%rip), %ymm9
  leaq perm(%rip), %r13
  leaq perm+12(%rip), %rbx
  xorl %ebp, %ebp
  xorl %r12d, %r12d
  vmovdqu %ymm6, 112(%rsp)
  vmovdqu %ymm7, 80(%rsp)
  vmovdqu %ymm8, 48(%rsp)
  vmovdqu %ymm9, 16(%rsp)
  cmpl $2, %r15d
  jl .LBB0_16
.LBB0_2:
  movl %r15d, %eax
  cmpl $9, %r15d
  jae .LBB0_4
  movq %rax, %rsi
  jmp .LBB0_13
.LBB0_4:
  leaq -1(%rax), %rcx
  cmpl $33, %r15d
  jae .LBB0_6
  xorl %edx, %edx
  movq %rax, %rsi
  jmp .LBB0_10
.LBB0_6:
  movq %rcx, %rdx
  andq $-32, %rdx
  movq %rax, %rsi
  subq %rdx, %rsi
  vmovd %r15d, %xmm0
  vpbroadcastd %xmm0, %ymm0
  vpaddd .LCPI0_1(%rip), %ymm0, %ymm0
  movq %rdx, %rdi
  negq %rdi
  leaq count(%rip), %r8
  leaq (%r8,%rax,4), %r8
  addq $-32, %r8
  xorl %r9d, %r9d
.LBB0_7:
  vpermd %ymm0, %ymm5, %ymm1
  vpaddd %ymm6, %ymm1, %ymm2
  vpaddd %ymm7, %ymm1, %ymm3
  vpaddd %ymm1, %ymm8, %ymm4
  vmovdqu %ymm1, (%r8,%r9,4)
  vmovdqu %ymm2, -32(%r8,%r9,4)
  vmovdqu %ymm3, -64(%r8,%r9,4)
  vmovdqu %ymm4, -96(%r8,%r9,4)
  vpaddd %ymm0, %ymm9, %ymm0
  addq $-32, %r9
  cmpq %r9, %rdi
  jne .LBB0_7
  movl $1, %r15d
  cmpq %rdx, %rcx
  je .LBB0_16
  testb $24, %cl
  je .LBB0_13
.LBB0_10:
  movq %rcx, %rdi
  andq $-8, %rdi
  vmovd %esi, %xmm0
  vpbroadcastd %xmm0, %ymm0
  vpaddd .LCPI0_1(%rip), %ymm0, %ymm0
  movq %rdi, %rsi
  negq %rsi
  negq %rdx
  leaq count(%rip), %r8
  leaq (%r8,%rax,4), %r8
  addq $-32, %r8
  subq %rdi, %rax
.LBB0_11:
  vpermd %ymm0, %ymm5, %ymm1
  vmovdqu %ymm1, (%r8,%rdx,4)
  vpaddd %ymm6, %ymm0, %ymm0
  addq $-8, %rdx
  cmpq %rdx, %rsi
  jne .LBB0_11
  movl $1, %r15d
  movq %rax, %rsi
  cmpq %rdi, %rcx
  je .LBB0_16
.LBB0_13:
  leaq count(%rip), %rax
.LBB0_14:
  movl %esi, -4(%rax,%rsi,4)
  cmpq $2, %rsi
  leaq -1(%rsi), %rsi
  ja .LBB0_14
  movl $1, %r15d
.LBB0_16:
  vmovaps perm1(%rip), %ymm0
  vmovups %ymm0, perm(%rip)
  vmovdqu perm1+12(%rip), %ymm0
  vmovdqu %ymm0, perm+12(%rip)
  movl perm(%rip), %ecx
  xorl %eax, %eax
  jmp .LBB0_17
.LBB0_25:
  movl perm(%rip), %ecx
.LBB0_26:
  incl %eax
.LBB0_17:
  testl %ecx, %ecx
  je .LBB0_27
  leal 1(%rcx), %edx
  sarl %edx
  testl %edx, %edx
  jle .LBB0_26
  movslq %ecx, %rsi
  movl %edx, %edi
  movl %edi, %ecx
  andl $3, %ecx
  cmpl $4, %edx
  jae .LBB0_29
  xorl %edx, %edx
  jmp .LBB0_23
.LBB0_29:
  leaq (,%rsi,4), %r8
  addq %r13, %r8
  andl $2147483644, %edi
  negq %rdi
  xorl %edx, %edx
  movq %rbx, %r9
.LBB0_30:
  movl -12(%r9), %r10d
  movl (%r8,%rdx,4), %r11d
  movl %r11d, -12(%r9)
  movl %r10d, (%r8,%rdx,4)
  movl -8(%r9), %r10d
  movl -4(%r8,%rdx,4), %r11d
  movl %r11d, -8(%r9)
  movl %r10d, -4(%r8,%rdx,4)
  movl -4(%r9), %r10d
  movl -8(%r8,%rdx,4), %r11d
  movl %r11d, -4(%r9)
  movl %r10d, -8(%r8,%rdx,4)
  movl (%r9), %r10d
  movl -12(%r8,%rdx,4), %r11d
  movl %r11d, (%r9)
  movl %r10d, -12(%r8,%rdx,4)
  addq $16, %r9
  addq $-4, %rdx
  cmpq %rdx, %rdi
  jne .LBB0_30
  testq %rcx, %rcx
  je .LBB0_25
  negq %rdx
.LBB0_23:
  shlq $2, %rdx
  leaq (,%rsi,4), %rsi
  addq %r13, %rsi
  subq %rdx, %rsi
  xorl %edi, %edi
.LBB0_24:
  leaq (%rdx,%rdi,4), %r8
  movl (%r13,%r8), %r9d
  movl (%rsi), %r10d
  movl %r10d, (%r13,%r8)
  movl %r9d, (%rsi)
  incq %rdi
  addq $-4, %rsi
  cmpq %rdi, %rcx
  jne .LBB0_24
  jmp .LBB0_25
.LBB0_27:
  cmpl %ebp, %eax
  cmovgl %eax, %ebp
  movl %ebp, 12(%rsp)
  movl %eax, %ecx
  negl %ecx
  movl %r12d, %edx
  testb $1, %dl
  cmovel %eax, %ecx
  addl %ecx, %r14d
  cmpl $11, %r15d
  leaq count(%rip), %rax
  leaq perm1(%rip), %rdi
  je .LBB0_36
  incl %r12d
  movslq %r15d, %r15
  leaq (,%r15,4), %rbx
.LBB0_32:
  movl perm1(%rip), %ebp
  testq %r15, %r15
  jle .LBB0_34
  movq %rbx, %rdx
  movabsq $17179869180, %rax
  andq %rax, %rdx
  leaq perm1+4(%rip), %rsi
  vzeroupper
  callq memmove@PLT
  leaq perm1(%rip), %rdi
  vmovdqu 16(%rsp), %ymm9
  vmovdqu 48(%rsp), %ymm8
  leaq count(%rip), %rax
  vmovdqu 80(%rsp), %ymm7
  vmovdqu 112(%rsp), %ymm6
  vmovdqa .LCPI0_2(%rip), %ymm5
.LBB0_34:
  movl %ebp, (%rbx,%rdi)
  decl (%rbx,%rax)
  jg .LBB0_35
  incq %r15
  addq $4, %rbx
  cmpq $11, %r15
  jne .LBB0_32
  jmp .LBB0_36
.LBB0_35:
  movl 12(%rsp), %ebp
  leaq perm+12(%rip), %rbx
  cmpl $2, %r15d
  jge .LBB0_2
  jmp .LBB0_16
.LBB0_36:
  leaq .L.str(%rip), %rdi
  movl %r14d, %esi
  movl $11, %edx
  movl 12(%rsp), %ecx
  xorl %eax, %eax
  vzeroupper
  callq printf@PLT
  xorl %eax, %eax
  addq $152, %rsp
  popq %rbx
  popq %r12
  popq %r13
  popq %r14
  popq %r15
  popq %rbp
  retq

.L.str:

