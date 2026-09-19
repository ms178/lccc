.LCPI0_0:
main:
  pushq %rax
  vstmxcsr 4(%rsp)
  orl $32832, 4(%rsp)
  vldmxcsr 4(%rsp)
  movq $-16, %rax
  movw $1488, %cx
  vmovdqu .LCPI0_0(%rip), %ymm0
.LBB0_1:
  leal -1488(%rcx), %edx
  vmovd %edx, %xmm1
  vpbroadcastw %xmm1, %ymm1
  vpaddw %ymm0, %ymm1, %ymm1
  vmovdqu %ymm1, main.a+32(%rax,%rax)
  leal -992(%rcx), %edx
  vmovd %edx, %xmm1
  vpbroadcastw %xmm1, %ymm1
  vpaddw %ymm0, %ymm1, %ymm1
  vmovdqu %ymm1, main.a+64(%rax,%rax)
  leal -496(%rcx), %edx
  vmovd %edx, %xmm1
  vpbroadcastw %xmm1, %ymm1
  vpaddw %ymm0, %ymm1, %ymm1
  vmovdqu %ymm1, main.a+96(%rax,%rax)
  vmovd %ecx, %xmm1
  vpbroadcastw %xmm1, %ymm1
  vpaddw %ymm0, %ymm1, %ymm1
  vmovdqu %ymm1, main.a+128(%rax,%rax)
  addq $64, %rax
  addl $1984, %ecx
  cmpq $1008, %rax
  jb .LBB0_1
  xorl %ecx, %ecx
  movq $-2048, %rax
.LBB0_3:
  movzwl main.a+2048(%rax), %edx
  addl %ecx, %edx
  movl %edx, main.d+4096(%rax,%rax)
  movzwl main.a+2050(%rax), %ecx
  addl %edx, %ecx
  movl %ecx, main.d+4100(%rax,%rax)
  movzwl main.a+2052(%rax), %edx
  addl %ecx, %edx
  movl %edx, main.d+4104(%rax,%rax)
  movzwl main.a+2054(%rax), %ecx
  addl %edx, %ecx
  movl %ecx, main.d+4108(%rax,%rax)
  movzwl main.a+2056(%rax), %edx
  addl %ecx, %edx
  movl %edx, main.d+4112(%rax,%rax)
  movzwl main.a+2058(%rax), %ecx
  addl %edx, %ecx
  movl %ecx, main.d+4116(%rax,%rax)
  movzwl main.a+2060(%rax), %edx
  addl %ecx, %edx
  movl %edx, main.d+4120(%rax,%rax)
  movzwl main.a+2062(%rax), %ecx
  addl %edx, %ecx
  movl %ecx, main.d+4124(%rax,%rax)
  addq $16, %rax
  jne .LBB0_3
  movl main.d+2044(%rip), %ecx
  addl main.d(%rip), %ecx
  addl main.d+4092(%rip), %ecx
  movq $-4032, %rax
.LBB0_5:
  movl main.d+4032(%rax), %edx
  movq %rcx, %rsi
  shlq $5, %rsi
  addq %rcx, %rsi
  addq %rdx, %rsi
  movl main.d+4060(%rax), %ecx
  addq %rsi, %rcx
  shlq $5, %rsi
  addq %rsi, %rcx
  movl main.d+4088(%rax), %edx
  addq %rcx, %rdx
  shlq $5, %rcx
  addq %rcx, %rdx
  movl main.d+4116(%rax), %ecx
  addq %rdx, %rcx
  shlq $5, %rdx
  addq %rdx, %rcx
  movl main.d+4144(%rax), %edx
  addq %rcx, %rdx
  shlq $5, %rcx
  addq %rcx, %rdx
  movl main.d+4172(%rax), %ecx
  addq %rdx, %rcx
  shlq $5, %rdx
  addq %rdx, %rcx
  movl main.d+4200(%rax), %edx
  addq %rcx, %rdx
  shlq $5, %rcx
  addq %rcx, %rdx
  movl main.d+4228(%rax), %ecx
  addq %rdx, %rcx
  shlq $5, %rdx
  addq %rdx, %rcx
  addq $224, %rax
  jne .LBB0_5
  movl main.d+4032(%rip), %eax
  movq %rcx, %rdx
  shlq $5, %rdx
  addq %rcx, %rax
  addq %rdx, %rax
  movl main.d+4060(%rip), %ecx
  addq %rax, %rcx
  shlq $5, %rax
  addq %rax, %rcx
  movl main.d+4088(%rip), %esi
  addq %rcx, %rsi
  shlq $5, %rcx
  addq %rcx, %rsi
  movl $.L.str, %edi
  xorl %eax, %eax
  vzeroupper
  callq printf
  xorl %eax, %eax
  popq %rcx
  retq

.L.str:

