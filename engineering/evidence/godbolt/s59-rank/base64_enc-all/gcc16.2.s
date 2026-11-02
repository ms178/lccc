.LC8:
main:
  movl $8, %ecx
  pushq %rbp
  vpcmpeqd %ymm3, %ymm3, %ymm3
  movl $a.1, %eax
  vmovd %ecx, %xmm9
  movl $16, %ecx
  vmovdqa .LC0(%rip), %ymm1
  vpxor %xmm2, %xmm2, %xmm2
  vmovd %ecx, %xmm8
  movl $24, %ecx
  movl $a.1+288, %edx
  vmovd %ecx, %xmm7
  movl $-117901064, %ecx
  movq %rsp, %rbp
  pushq %rbx
  vmovd %ecx, %xmm6
  movl $185273099, %ecx
  vpbroadcastd %xmm9, %ymm9
  vmovd %ecx, %xmm5
  movl $32, %ecx
  vpbroadcastd %xmm8, %ymm8
  vmovd %ecx, %xmm4
  andq $-32, %rsp
  vpbroadcastd %xmm7, %ymm7
  vpsrlw $8, %ymm3, %ymm3
  vpbroadcastd %xmm6, %ymm6
  vpbroadcastd %xmm5, %ymm5
  vpbroadcastd %xmm4, %ymm4
.L2:
  vpaddd %ymm9, %ymm1, %ymm0
  vpblendw $170, %ymm2, %ymm1, %ymm11
  vpaddd %ymm7, %ymm1, %ymm12
  addq $32, %rax
  vpblendw $170, %ymm2, %ymm0, %ymm0
  vpblendw $170, %ymm2, %ymm12, %ymm12
  vpackusdw %ymm0, %ymm11, %ymm11
  vpaddd %ymm8, %ymm1, %ymm0
  vpaddd %ymm4, %ymm1, %ymm1
  vpblendw $170, %ymm2, %ymm0, %ymm0
  vpermq $216, %ymm11, %ymm11
  vpackusdw %ymm12, %ymm0, %ymm0
  vpand %ymm11, %ymm3, %ymm11
  vpermq $216, %ymm0, %ymm0
  vpand %ymm0, %ymm3, %ymm0
  vpackuswb %ymm0, %ymm11, %ymm11
  vpermq $216, %ymm11, %ymm11
  vpsllw $3, %ymm11, %ymm0
  vpand %ymm0, %ymm6, %ymm0
  vpaddb %ymm11, %ymm0, %ymm0
  vpaddb %ymm0, %ymm0, %ymm0
  vpaddb %ymm0, %ymm0, %ymm0
  vpaddb %ymm11, %ymm0, %ymm0
  vpaddb %ymm5, %ymm0, %ymm0
  vmovdqa %ymm0, -32(%rax)
  cmpq $a.1+288, %rax
  jne .L2
  movl $-85, %eax
.L3:
  movb %al, (%rdx)
  addl $37, %eax
  addq $1, %rdx
  cmpb $103, %al
  jne .L3
  movl $d.0, %edi
  xorl %ecx, %ecx
  movq %rdi, %rsi
.L4:
  movzbl a.1(%rcx), %edx
  xorl %ebx, %ebx
  addq $3, %rcx
  addq $4, %rsi
  movzbl a.1-2(%rcx), %eax
  sall $16, %edx
  sall $8, %eax
  orl %edx, %eax
  movzbl a.1-1(%rcx), %edx
  movl %eax, %r8d
  orl %eax, %edx
  shrl $12, %eax
  andl $63, %eax
  shrl $18, %r8d
  movb tab(%r8), %bl
  movb tab(%rax), %bh
  movl %edx, %eax
  andl $63, %edx
  shrl $6, %eax
  andl $63, %eax
  movw %bx, -4(%rsi)
  movzbl tab(%rax), %eax
  movb %al, -2(%rsi)
  movzbl tab(%rdx), %eax
  movb %al, -1(%rsi)
  cmpq $300, %rcx
  jne .L4
  movl $400, %esi
.L5:
  movq %rsi, %rax
  addq $1, %rdi
  salq $5, %rax
  addq %rax, %rsi
  movzbl -1(%rdi), %eax
  addq %rax, %rsi
  cmpq $d.0+400, %rdi
  jne .L5
  xorl %eax, %eax
  movl $.LC8, %edi
  vzeroupper
  call printf
  movq -8(%rbp), %rbx
  xorl %eax, %eax
  leave
  ret
tab:
.LC0:
