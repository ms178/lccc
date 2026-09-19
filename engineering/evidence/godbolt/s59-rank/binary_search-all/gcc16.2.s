main:
  movl $8, %edx
  vpcmpeqd %ymm3, %ymm3, %ymm3
  movl $table, %eax
  vmovdqa .LC0(%rip), %ymm1
  vmovd %edx, %xmm2
  vpsrld $29, %ymm3, %ymm3
  vpbroadcastd %xmm2, %ymm2
.L2:
  vpslld $1, %ymm1, %ymm0
  addq $32, %rax
  vpaddd %ymm1, %ymm0, %ymm0
  vpaddd %ymm2, %ymm1, %ymm1
  vpaddd %ymm3, %ymm0, %ymm0
  vmovdqa %ymm0, -32(%rax)
  cmpq $table+16384, %rax
  jne .L2
  movl $-1000, %edi
  xorl %r8d, %r8d
.L3:
  xorl %edx, %edx
  movl $4095, %ecx
  jmp .L7
.L16:
  leal 1(%rax), %edx
  cmpl %edx, %ecx
  jl .L9
.L7:
  movl %ecx, %eax
  subl %edx, %eax
  sarl %eax
  addl %edx, %eax
  movslq %eax, %rsi
  cmpl %edi, table(,%rsi,4)
  je .L4
  jl .L16
  leal -1(%rax), %ecx
  cmpl %edx, %ecx
  jge .L7
.L9:
  addl $7, %edi
  cmpl $13294, %edi
  jne .L3
.L8:
  xorl %eax, %eax
  cmpl $1198665, %r8d
  setne %al
  addl %eax, %eax
  vzeroupper
  ret
.L4:
  xorl %edx, %edx
  testl %eax, %eax
  cmovs %edx, %eax
  addl $7, %edi
  addl %eax, %r8d
  cmpl $13294, %edi
  jne .L3
  jmp .L8
.LC0:
