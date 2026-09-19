mix.constprop.0:
  movq %rsi, %r8
  xorl %eax, %eax
  movq %rdx, %rsi
  xorl %ecx, %ecx
.L2:
  movl (%r8,%rax), %edx
  addl (%rdi,%rax), %edx
  addl (%rsi,%rax), %edx
  addl %edx, %ecx
  movl %ecx, (%rdi,%rax)
  addq $4, %rax
  cmpq $512, %rax
  jne .L2
  movl %ecx, %eax
  ret
kernel:
  testl %edi, %edi
  jle .L8
  movl %edi, %r11d
  xorl %r10d, %r10d
  xorl %r9d, %r9d
.L7:
  movl $perm, %edi
  movl $count, %edx
  movl $perm1, %esi
  addl $1, %r10d
  call mix.constprop.0
  movq %rdi, %rdx
  movl $count, %esi
  movl $perm1, %edi
  addl %eax, %r9d
  call mix.constprop.0
  movq %rdi, %rdx
  movl $perm, %esi
  movl $count, %edi
  addl %eax, %r9d
  call mix.constprop.0
  addl %eax, %r9d
  cmpl %r10d, %r11d
  jne .L7
  movl %r9d, %eax
  ret
.L8:
  xorl %r9d, %r9d
  movl %r9d, %eax
  ret
.LC3:
main:
  pushq %rbp
  movq %rsp, %rbp
  andq $-32, %rsp
  cmpl $1, %edi
  jg .L17
  movl $2000, %edi
.L11:
  movl $128, %ecx
  vmovdqa .LC0(%rip), %ymm0
  xorl %edx, %edx
  vmovd %ecx, %xmm3
  movl $8, %ecx
  vmovd %ecx, %xmm2
  vpbroadcastd %xmm3, %ymm3
  vpbroadcastd %xmm2, %ymm2
.L12:
  vpsubd %ymm0, %ymm3, %ymm1
  vmovdqa %ymm0, perm(%rdx)
  addq $32, %rdx
  vmovdqa %ymm1, perm1-32(%rdx)
  vpmulld %ymm0, %ymm0, %ymm1
  vpaddd %ymm2, %ymm0, %ymm0
  vmovdqa %ymm1, count-32(%rdx)
  cmpq $512, %rdx
  jne .L12
  call kernel
  movl $.LC3, %edi
  movl %eax, %esi
  xorl %eax, %eax
  vzeroupper
  call printf
  xorl %eax, %eax
  leave
  ret
.L17:
  movq 8(%rsi), %rdi
  movl $10, %edx
  xorl %esi, %esi
  call __isoc23_strtol
  movl %eax, %edi
  jmp .L11
.LC0:
