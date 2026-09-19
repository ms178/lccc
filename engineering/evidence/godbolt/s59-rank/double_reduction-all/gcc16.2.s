.LC0:
main:
  pushq %rbp
  xorl %edx, %edx
  movl $1, %eax
  movq %rsp, %rbp
  andq $-32, %rsp
.L2:
  imull $1664525, %eax, %eax
  addq $4, %rdx
  addl $1013904223, %eax
  movl %eax, %ecx
  imull $1664525, %eax, %eax
  shrl $16, %ecx
  andl $15, %ecx
  subl $7, %ecx
  addl $1013904223, %eax
  movl %ecx, a-4(%rdx)
  movl %eax, %ecx
  imull $1664525, %eax, %eax
  shrl $16, %ecx
  andl $15, %ecx
  subl $7, %ecx
  addl $1013904223, %eax
  movl %ecx, b-4(%rdx)
  movl %eax, %ecx
  shrl $16, %ecx
  andl $15, %ecx
  subl $7, %ecx
  movl %ecx, c-4(%rdx)
  cmpq $4194304, %rdx
  jne .L2
  vpxor %xmm4, %xmm4, %xmm4
  movl $128, %edx
  xorl %esi, %esi
.L3:
  xorl %eax, %eax
  vmovdqa %ymm4, %ymm2
  vmovdqa %ymm4, %ymm1
.L4:
  vmovdqa a(%rax), %ymm0
  addq $32, %rax
  vpmulld b-32(%rax), %ymm0, %ymm3
  vpaddd %ymm2, %ymm3, %ymm2
  vpmulld c-32(%rax), %ymm0, %ymm0
  vpaddd %ymm1, %ymm0, %ymm1
  cmpq $4194304, %rax
  jne .L4
  vextracti128 $0x1, %ymm1, %xmm0
  vpaddd %xmm1, %xmm0, %xmm0
  vpsrldq $8, %xmm0, %xmm1
  vpaddd %xmm1, %xmm0, %xmm0
  vpsrldq $4, %xmm0, %xmm1
  vpaddd %xmm1, %xmm0, %xmm0
  vextracti128 $0x1, %ymm2, %xmm1
  vpaddd %xmm2, %xmm1, %xmm1
  vpsrldq $8, %xmm1, %xmm2
  vpaddd %xmm2, %xmm1, %xmm1
  vpsrldq $4, %xmm1, %xmm2
  vpaddd %xmm2, %xmm1, %xmm1
  vpaddd %xmm1, %xmm0, %xmm0
  vmovdqa %ymm4, %ymm1
  vmovd %xmm0, %eax
  vmovdqa %ymm4, %ymm0
  cltq
  addq %rax, %rsi
  xorl %eax, %eax
.L5:
  vpaddd a(%rax), %ymm1, %ymm1
  vpaddd b(%rax), %ymm0, %ymm0
  addq $32, %rax
  cmpq $4194304, %rax
  jne .L5
  vextracti128 $0x1, %ymm1, %xmm2
  vpaddd %xmm1, %xmm2, %xmm1
  vpsrldq $8, %xmm1, %xmm2
  vpaddd %xmm2, %xmm1, %xmm1
  vpsrldq $4, %xmm1, %xmm2
  vpaddd %xmm2, %xmm1, %xmm1
  vextracti128 $0x1, %ymm0, %xmm2
  vpaddd %xmm0, %xmm2, %xmm0
  vpsrldq $8, %xmm0, %xmm2
  vpaddd %xmm2, %xmm0, %xmm0
  vpsrldq $4, %xmm0, %xmm2
  vpaddd %xmm2, %xmm0, %xmm0
  vpaddd %xmm0, %xmm1, %xmm0
  vmovd %xmm0, %eax
  cltq
  addq %rax, %rsi
  subl $1, %edx
  jne .L3
  xorl %eax, %eax
  movl $.LC0, %edi
  vzeroupper
  call printf
  xorl %eax, %eax
  leave
  ret
