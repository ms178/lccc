.LC1:
  .string "sum=%ld pos=%ld max=%d scaled=%ld dot=%ld prefix=%d\n"
main:
  leaq 8(%rsp), %r10
  andq $-32, %rsp
  movl $array, %ecx
  movl $42, %eax
  pushq -8(%r10)
  leaq 40000000(%rcx), %r8
  movq %rcx, %rdx
  pushq %rbp
  movq %rsp, %rbp
  pushq %r12
  pushq %r10
  pushq %rbx
  subq $24, %rsp
.L2:
  imull $1664525, %eax, %eax
  addq $4, %rdx
  addl $1013904223, %eax
  movl %eax, %esi
  shrl %esi
  subl $1000000000, %esi
  movl %esi, -4(%rdx)
  cmpq %r8, %rdx
  jne .L2
  vpxor %xmm2, %xmm2, %xmm2
  movl $array, %eax
  vmovdqa %ymm2, %ymm1
.L3:
  vmovdqa (%rax), %ymm0
  addq $32, %rax
  vpmovsxdq %xmm0, %ymm3
  vextracti128 $0x1, %ymm0, %xmm0
  vpaddq %ymm1, %ymm3, %ymm1
  vpmovsxdq %xmm0, %ymm0
  vpaddq %ymm1, %ymm0, %ymm1
  cmpq %r8, %rax
  jne .L3
  vextracti128 $0x1, %ymm1, %xmm0
  movl $array, %eax
  vmovdqa %ymm2, %ymm4
  vpaddq %xmm1, %xmm0, %xmm0
  vmovdqa %ymm2, %ymm6
  vpsrldq $8, %xmm0, %xmm1
  vpaddq %xmm1, %xmm0, %xmm0
  vmovq %xmm0, %r10
.L4:
  vmovdqa (%rax), %ymm1
  addq $32, %rax
  vpcmpgtd %ymm6, %ymm1, %ymm0
  vpmovsxdq %xmm1, %ymm5
  vextracti128 $0x1, %ymm1, %xmm1
  vpmovsxdq %xmm1, %ymm1
  vpmovsxdq %xmm0, %ymm3
  vextracti128 $0x1, %ymm0, %xmm0
  vpand %ymm5, %ymm3, %ymm3
  vpmovsxdq %xmm0, %ymm0
  vpaddq %ymm4, %ymm3, %ymm4
  vpand %ymm1, %ymm0, %ymm0
  vpaddq %ymm4, %ymm0, %ymm4
  cmpq %rax, %r8
  jne .L4
  vextracti128 $0x1, %ymm4, %xmm0
  movl $array+4, %esi
  vpaddq %xmm4, %xmm0, %xmm0
  leaq 39999968(%rsi), %rdx
  movq %rsi, %rax
  vpsrldq $8, %xmm0, %xmm1
  vpaddq %xmm1, %xmm0, %xmm0
  vpbroadcastd array(%rip), %ymm1
  vmovq %xmm0, %rdi
.L5:
  vpmaxsd (%rax), %ymm1, %ymm1
  addq $32, %rax
  cmpq %rdx, %rax
  jne .L5
  vextracti128 $0x1, %ymm1, %xmm0
  movq %r8, %rax
  vpmaxsd %xmm1, %xmm0, %xmm0
  subq %rdx, %rax
  vpsrldq $8, %xmm0, %xmm1
  vpmaxsd %xmm1, %xmm0, %xmm0
  vpsrldq $4, %xmm0, %xmm1
  vpmaxsd %xmm1, %xmm0, %xmm0
  testb $4, %al
  je .L6
  vmovd (%rdx), %xmm1
  addq $4, %rdx
  vpmaxsd %xmm1, %xmm0, %xmm0
  vmovd %xmm0, %r11d
  cmpq %r8, %rdx
  je .L27
.L6:
  vmovd (%rdx), %xmm1
  vpmaxsd %xmm1, %xmm0, %xmm0
  addq $8, %rdx
  vmovd -4(%rdx), %xmm1
  vpmaxsd %xmm1, %xmm0, %xmm0
  vmovd %xmm0, %r11d
  cmpq %r8, %rdx
  jne .L6
.L27:
  vpcmpeqd %ymm3, %ymm3, %ymm3
  xorl %eax, %eax
  vpsrld $29, %ymm3, %ymm3
.L7:
  vmovdqa array(%rax), %ymm1
  addq $32, %rax
  vpslld $1, %ymm1, %ymm0
  vpaddd %ymm1, %ymm0, %ymm0
  vpaddd %ymm3, %ymm0, %ymm0
  vmovdqa %ymm0, buf.0-32(%rax)
  cmpq $40000000, %rax
  jne .L7
  movl $buf.0, %eax
  vmovdqa %ymm2, %ymm1
  leaq 40000000(%rax), %rdx
.L8:
  vmovdqa (%rax), %ymm0
  addq $32, %rax
  vpmovsxdq %xmm0, %ymm3
  vextracti128 $0x1, %ymm0, %xmm0
  vpaddq %ymm1, %ymm3, %ymm1
  vpmovsxdq %xmm0, %ymm0
  vpaddq %ymm1, %ymm0, %ymm1
  cmpq %rax, %rdx
  jne .L8
  vextracti128 $0x1, %ymm1, %xmm0
  xorl %eax, %eax
  vpaddq %xmm1, %xmm0, %xmm0
  vpsrldq $8, %xmm0, %xmm1
  vpaddq %xmm1, %xmm0, %xmm0
  vmovq %xmm0, %r8
.L9:
  vmovdqa array(%rax), %ymm1
  vmovdqa buf.0(%rax), %ymm0
  addq $32, %rax
  vpmuldq %ymm1, %ymm0, %ymm3
  vpsrlq $32, %ymm0, %ymm0
  vpsrlq $32, %ymm1, %ymm1
  vpmuldq %ymm1, %ymm0, %ymm0
  vpaddq %ymm2, %ymm3, %ymm2
  vpaddq %ymm2, %ymm0, %ymm2
  cmpq $4000000, %rax
  jne .L9
  vextracti128 $0x1, %ymm2, %xmm0
  movl $array+40000, %ebx
  movl $42, %edx
  vpaddq %xmm2, %xmm0, %xmm0
  vpsrldq $8, %xmm0, %xmm1
  vpaddq %xmm1, %xmm0, %xmm0
  vmovq %xmm0, %r9
.L10:
  imull $1664525, %edx, %edx
  addq $4, %rcx
  leal 1013904223(%rdx), %eax
  movq %rax, %rdx
  imulq $1374389535, %rax, %rax
  movl %edx, %r12d
  shrq $37, %rax
  imull $100, %eax, %eax
  subl %eax, %r12d
  movl %r12d, -4(%rcx)
  cmpq $array+40000, %rcx
  jne .L10
  movl array(%rip), %eax
.L11:
  addl (%rsi), %eax
  addq $4, %rsi
  movl %eax, -4(%rsi)
  cmpq %rsi, %rbx
  jne .L11
  movl array+39996(%rip), %eax
  subq $8, %rsp
  movq %rdi, %rdx
  movq %r10, %rsi
  movl %r11d, %ecx
  movl $.LC1, %edi
  pushq %rax
  xorl %eax, %eax
  vzeroupper
  call printf
  popq %rax
  popq %rdx
  leaq -24(%rbp), %rsp
  xorl %eax, %eax
  popq %rbx
  popq %r10
  popq %r12
  popq %rbp
  leaq -8(%r10), %rsp
  ret
