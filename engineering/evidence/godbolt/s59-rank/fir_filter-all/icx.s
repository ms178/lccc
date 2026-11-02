.LCPI0_0:
.LCPI0_1:
.LCPI0_2:
.LCPI0_3:
.LCPI0_4:
.LCPI0_5:
.LCPI0_6:
.LCPI0_7:
.LCPI0_8:
.LCPI0_9:
.LCPI0_10:
.LCPI0_11:
.LCPI0_12:
.LCPI0_13:
.LCPI0_14:
.LCPI0_15:
.LCPI0_16:
.LCPI0_17:
main:
  pushq %rax
  vstmxcsr 4(%rsp)
  orl $32832, 4(%rsp)
  vldmxcsr 4(%rsp)
  vmovups .LCPI0_0(%rip), %ymm0
  vmovups %ymm0, main.a(%rip)
  vmovups .LCPI0_1(%rip), %ymm0
  vmovups %ymm0, main.a+32(%rip)
  vmovups .LCPI0_2(%rip), %ymm0
  vmovups %ymm0, main.a+64(%rip)
  vmovups .LCPI0_3(%rip), %ymm0
  vmovups %ymm0, main.a+96(%rip)
  vmovups .LCPI0_4(%rip), %ymm0
  vmovups %ymm0, main.a+128(%rip)
  vmovups .LCPI0_5(%rip), %ymm0
  vmovups %ymm0, main.a+160(%rip)
  vmovups .LCPI0_6(%rip), %ymm0
  vmovups %ymm0, main.a+192(%rip)
  vmovups .LCPI0_7(%rip), %ymm0
  vmovups %ymm0, main.a+224(%rip)
  vmovups .LCPI0_8(%rip), %ymm0
  vmovups %ymm0, main.a+256(%rip)
  vmovups .LCPI0_9(%rip), %ymm0
  vmovups %ymm0, main.a+288(%rip)
  vmovups .LCPI0_10(%rip), %ymm0
  vmovups %ymm0, main.a+320(%rip)
  vmovups .LCPI0_11(%rip), %ymm0
  vmovups %ymm0, main.a+352(%rip)
  vmovups .LCPI0_12(%rip), %ymm0
  vmovups %ymm0, main.a+384(%rip)
  vmovups .LCPI0_13(%rip), %ymm0
  vmovups %ymm0, main.a+416(%rip)
  vmovups .LCPI0_14(%rip), %ymm0
  vmovups %ymm0, main.a+448(%rip)
  vmovups .LCPI0_15(%rip), %ymm0
  vmovups %ymm0, main.a+480(%rip)
  vmovups .LCPI0_16(%rip), %xmm0
  vmovups %xmm0, main.a+512(%rip)
  vmovdqu .LCPI0_17(%rip), %xmm0
  vmovdqu %xmm0, main.c(%rip)
  movq $-512, %rax
.LBB0_1:
  vpmaddwd main.a+512(%rax), %xmm0, %xmm1
  vpshufd $238, %xmm1, %xmm2
  vpaddd %xmm2, %xmm1, %xmm1
  vpshufd $85, %xmm1, %xmm2
  vpaddd %xmm2, %xmm1, %xmm1
  vmovd %xmm1, main.d+1024(%rax,%rax)
  vpmaddwd main.a+514(%rax), %xmm0, %xmm1
  vpshufd $238, %xmm1, %xmm2
  vpaddd %xmm2, %xmm1, %xmm1
  vpshufd $85, %xmm1, %xmm2
  vpaddd %xmm2, %xmm1, %xmm1
  vmovd %xmm1, main.d+1028(%rax,%rax)
  vpmaddwd main.a+516(%rax), %xmm0, %xmm1
  vpshufd $238, %xmm1, %xmm2
  vpaddd %xmm2, %xmm1, %xmm1
  vpshufd $85, %xmm1, %xmm2
  vpaddd %xmm2, %xmm1, %xmm1
  vmovd %xmm1, main.d+1032(%rax,%rax)
  vpmaddwd main.a+518(%rax), %xmm0, %xmm1
  vpshufd $238, %xmm1, %xmm2
  vpaddd %xmm2, %xmm1, %xmm1
  vpshufd $85, %xmm1, %xmm2
  vpaddd %xmm2, %xmm1, %xmm1
  vmovd %xmm1, main.d+1036(%rax,%rax)
  vpmaddwd main.a+520(%rax), %xmm0, %xmm1
  vpshufd $238, %xmm1, %xmm2
  vpaddd %xmm2, %xmm1, %xmm1
  vpshufd $85, %xmm1, %xmm2
  vpaddd %xmm2, %xmm1, %xmm1
  vmovd %xmm1, main.d+1040(%rax,%rax)
  vpmaddwd main.a+522(%rax), %xmm0, %xmm1
  vpshufd $238, %xmm1, %xmm2
  vpaddd %xmm2, %xmm1, %xmm1
  vpshufd $85, %xmm1, %xmm2
  vpaddd %xmm2, %xmm1, %xmm1
  vmovd %xmm1, main.d+1044(%rax,%rax)
  vpmaddwd main.a+524(%rax), %xmm0, %xmm1
  vpshufd $238, %xmm1, %xmm2
  vpaddd %xmm2, %xmm1, %xmm1
  vpshufd $85, %xmm1, %xmm2
  vpaddd %xmm2, %xmm1, %xmm1
  vmovd %xmm1, main.d+1048(%rax,%rax)
  vpmaddwd main.a+526(%rax), %xmm0, %xmm1
  vpshufd $238, %xmm1, %xmm2
  vpaddd %xmm2, %xmm1, %xmm1
  vpshufd $85, %xmm1, %xmm2
  vpaddd %xmm2, %xmm1, %xmm1
  vmovd %xmm1, main.d+1052(%rax,%rax)
  addq $16, %rax
  jne .LBB0_1
  movl $146959, %esi
  movq $-1024, %rax
  movabsq $1099511628211, %rcx
.LBB0_3:
  movl main.d+1024(%rax), %edx
  movzwl %dx, %edi
  xorq %rsi, %rdi
  imulq %rcx, %rdi
  shrl $16, %edx
  xorq %rdi, %rdx
  imulq %rcx, %rdx
  movl main.d+1028(%rax), %esi
  movzwl %si, %edi
  xorq %rdx, %rdi
  imulq %rcx, %rdi
  shrl $16, %esi
  xorq %rdi, %rsi
  imulq %rcx, %rsi
  movl main.d+1032(%rax), %edx
  movzwl %dx, %edi
  xorq %rsi, %rdi
  imulq %rcx, %rdi
  shrl $16, %edx
  xorq %rdi, %rdx
  imulq %rcx, %rdx
  movl main.d+1036(%rax), %esi
  movzwl %si, %edi
  xorq %rdx, %rdi
  imulq %rcx, %rdi
  shrl $16, %esi
  xorq %rdi, %rsi
  imulq %rcx, %rsi
  movl main.d+1040(%rax), %edx
  movzwl %dx, %edi
  xorq %rsi, %rdi
  imulq %rcx, %rdi
  shrl $16, %edx
  xorq %rdi, %rdx
  imulq %rcx, %rdx
  movl main.d+1044(%rax), %esi
  movzwl %si, %edi
  xorq %rdx, %rdi
  imulq %rcx, %rdi
  shrl $16, %esi
  xorq %rdi, %rsi
  imulq %rcx, %rsi
  movl main.d+1048(%rax), %edx
  movzwl %dx, %edi
  xorq %rsi, %rdi
  imulq %rcx, %rdi
  shrl $16, %edx
  xorq %rdi, %rdx
  imulq %rcx, %rdx
  movl main.d+1052(%rax), %esi
  movzwl %si, %edi
  xorq %rdx, %rdi
  imulq %rcx, %rdi
  shrl $16, %esi
  xorq %rdi, %rsi
  imulq %rcx, %rsi
  addq $32, %rax
  jne .LBB0_3
  movl $.L.str, %edi
  xorl %eax, %eax
  vzeroupper
  callq printf
  xorl %eax, %eax
  popq %rcx
  retq

.L.str:

