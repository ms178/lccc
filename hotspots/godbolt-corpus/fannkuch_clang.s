.LCPI0_0:
  .long 0
  .long 1
  .long 2
  .long 3
  .long 4
  .long 5
  .long 6
  .long 7
.LCPI0_1:
  .long 0
  .long 4294967295
  .long 4294967294
  .long 4294967293
  .long 4294967292
  .long 4294967291
  .long 4294967290
  .long 4294967289
.LCPI0_2:
  .long 7
  .long 6
  .long 5
  .long 4
  .long 3
  .long 2
  .long 1
  .long 0
.LCPI0_3:
  .long 4294967288
.LCPI0_4:
  .long 4294967280
.LCPI0_5:
  .long 4294967272
.LCPI0_6:
  .long 4294967264
.LCPI0_8:
  .long 4294967292
.LCPI0_7:
  .long 0
  .long 4294967295
  .long 4294967294
  .long 4294967293
main:
  pushq %rbp
  pushq %r15
  pushq %r14
  pushq %r13
  pushq %r12
  pushq %rbx
  subq $168, %rsp
  vmovdqa .LCPI0_0(%rip), %ymm0
  vmovdqa %ymm0, perm1(%rip)
  movabsq $38654705672, %rax
  movq %rax, perm1+32(%rip)
  movl $10, perm1+40(%rip)
  movl $11, %ebp
  xorl %r14d, %r14d
  leaq count(%rip), %r12
  vpbroadcastd .LCPI0_8(%rip), %xmm5
  vmovdqa .LCPI0_2(%rip), %ymm6
  vpbroadcastd .LCPI0_3(%rip), %ymm7
  vpbroadcastd .LCPI0_4(%rip), %ymm8
  vpbroadcastd .LCPI0_5(%rip), %ymm9
  vpbroadcastd .LCPI0_6(%rip), %ymm10
  leaq perm(%rip), %rbx
  leaq perm+12(%rip), %r13
  xorl %r15d, %r15d
  movl $0, 4(%rsp)
  vmovdqa %xmm5, 16(%rsp)
  vmovdqu %ymm7, 128(%rsp)
  vmovdqu %ymm8, 96(%rsp)
  vmovdqu %ymm9, 64(%rsp)
  vmovdqu %ymm10, 32(%rsp)
  cmpl $2, %ebp
  jl .LBB0_8
.LBB0_2:
  movl %ebp, %eax
  cmpl $5, %ebp
  jb .LBB0_18
  leaq -1(%rax), %rcx
  cmpl $33, %ebp
  jae .LBB0_13
  xorl %edx, %edx
  movq %rax, %rsi
  jmp .LBB0_5
.LBB0_13:
  movq %rcx, %rdx
  andq $-32, %rdx
  movq %rax, %rsi
  subq %rdx, %rsi
  vmovd %ebp, %xmm0
  vpbroadcastd %xmm0, %ymm0
  vpaddd .LCPI0_1(%rip), %ymm0, %ymm0
  movq %rdx, %rdi
  negq %rdi
  leaq (%r12,%rax,4), %r8
  addq $-32, %r8
  xorl %r9d, %r9d
.LBB0_14:
  vpermd %ymm0, %ymm6, %ymm1
  vpaddd %ymm7, %ymm1, %ymm2
  vpaddd %ymm1, %ymm8, %ymm3
  vpaddd %ymm1, %ymm9, %ymm4
  vmovdqu %ymm1, (%r8,%r9,4)
  vmovdqu %ymm2, -32(%r8,%r9,4)
  vmovdqu %ymm3, -64(%r8,%r9,4)
  vmovdqu %ymm4, -96(%r8,%r9,4)
  vpaddd %ymm0, %ymm10, %ymm0
  addq $-32, %r9
  cmpq %r9, %rdi
  jne .LBB0_14
  movl $1, %ebp
  cmpq %rdx, %rcx
  je .LBB0_8
  testb $28, %cl
  je .LBB0_17
.LBB0_5:
  movq %rcx, %rdi
  andq $-4, %rdi
  vmovd %esi, %xmm0
  vpbroadcastd %xmm0, %xmm0
  vpaddd .LCPI0_7(%rip), %xmm0, %xmm0
  movq %rdi, %rsi
  negq %rsi
  negq %rdx
  leaq (%r12,%rax,4), %r8
  addq $-16, %r8
  subq %rdi, %rax
.LBB0_6:
  vpshufd $27, %xmm0, %xmm1
  vmovdqu %xmm1, (%r8,%rdx,4)
  vpaddd %xmm5, %xmm0, %xmm0
  addq $-4, %rdx
  cmpq %rdx, %rsi
  jne .LBB0_6
  movl $1, %ebp
  cmpq %rdi, %rcx
  je .LBB0_8
.LBB0_18:
  movl %eax, -4(%r12,%rax,4)
  cmpq $2, %rax
  leaq -1(%rax), %rax
  ja .LBB0_18
  movl $1, %ebp
.LBB0_8:
  vmovaps perm1(%rip), %ymm0
  vmovups %ymm0, perm(%rip)
  vmovdqu perm1+12(%rip), %ymm0
  vmovdqu %ymm0, perm+12(%rip)
  movl perm(%rip), %ecx
  xorl %eax, %eax
  jmp .LBB0_9
.LBB0_24:
  movl perm(%rip), %ecx
.LBB0_25:
  incl %eax
.LBB0_9:
  testl %ecx, %ecx
  je .LBB0_26
  leal 1(%rcx), %edx
  sarl %edx
  testl %edx, %edx
  jle .LBB0_25
  movslq %ecx, %rsi
  movl %edx, %edi
  movl %edi, %ecx
  andl $3, %ecx
  cmpl $4, %edx
  jae .LBB0_32
  xorl %edx, %edx
  jmp .LBB0_22
.LBB0_32:
  leaq (%rbx,%rsi,4), %r8
  andl $2147483644, %edi
  negq %rdi
  xorl %edx, %edx
  movq %r13, %r9
.LBB0_33:
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
  jne .LBB0_33
  testq %rcx, %rcx
  je .LBB0_24
  negq %rdx
.LBB0_22:
  shlq $2, %rdx
  leaq (%rbx,%rsi,4), %rsi
  subq %rdx, %rsi
  xorl %edi, %edi
.LBB0_23:
  leaq (%rdx,%rdi,4), %r8
  movl (%rbx,%r8), %r9d
  movl (%rsi), %r10d
  movl %r10d, (%rbx,%r8)
  movl %r9d, (%rsi)
  incq %rdi
  addq $-4, %rsi
  cmpq %rdi, %rcx
  jne .LBB0_23
  jmp .LBB0_24
.LBB0_26:
  cmpl %r15d, %eax
  cmovgl %eax, %r15d
  movl %r15d, (%rsp)
  movl %eax, %ecx
  negl %ecx
  movl 4(%rsp), %edx
  testb $1, %dl
  cmovel %eax, %ecx
  addl %ecx, %r14d
  movq %r14, 8(%rsp)
  incl %edx
  movl %edx, 4(%rsp)
  movslq %ebp, %r13
  leal -1(%r13), %ebp
  leaq (,%r13,4), %r15
  leaq perm1(%rip), %rdi
  jmp .LBB0_27
.LBB0_30:
  movl %r14d, (%r15,%rdi)
  incq %r13
  incl %ebp
  decl (%r15,%r12)
  leaq 4(%r15), %r15
  jg .LBB0_31
.LBB0_27:
  cmpq $11, %r13
  je .LBB0_34
  movl perm1(%rip), %r14d
  testq %r13, %r13
  jle .LBB0_30
  movq %r15, %rdx
  movabsq $17179869180, %rax
  andq %rax, %rdx
  leaq perm1+4(%rip), %rsi
  vzeroupper
  callq memmove@PLT
  leaq perm1(%rip), %rdi
  vmovdqu 32(%rsp), %ymm10
  vmovdqu 64(%rsp), %ymm9
  vmovdqu 96(%rsp), %ymm8
  vmovdqu 128(%rsp), %ymm7
  vmovdqa .LCPI0_2(%rip), %ymm6
  vmovdqa 16(%rsp), %xmm5
  jmp .LBB0_30
.LBB0_31:
  movq 8(%rsp), %r14
  movl (%rsp), %r15d
  leaq perm+12(%rip), %r13
  cmpl $2, %ebp
  jge .LBB0_2
  jmp .LBB0_8
.LBB0_17:
  subq %rdx, %rax
  jmp .LBB0_18
.LBB0_34:
  leaq .L.str(%rip), %rdi
  movq 8(%rsp), %rsi
  movl $11, %edx
  movl (%rsp), %ecx
  xorl %eax, %eax
  vzeroupper
  callq printf@PLT
  xorl %eax, %eax
  addq $168, %rsp
  popq %rbx
  popq %r12
  popq %r13
  popq %r14
  popq %r15
  popq %rbp
  retq

.L.str:
  .asciz "%d\nPfannkuchen(%d) = %d\n"

