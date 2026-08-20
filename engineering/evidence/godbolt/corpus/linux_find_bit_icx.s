.LCPI0_0:
  .quad 0
  .quad 1
  .quad 2
  .quad 3
.LCPI0_1:
  .quad 63
.LCPI0_2:
  .quad 5
.LCPI0_4:
  .quad 1
.LCPI0_3:
  .long 7
  .long 20
  .long 33
  .long 46
main:
  pushq %rax
  vstmxcsr 4(%rsp)
  orl $32832, 4(%rsp)
  vldmxcsr 4(%rsp)
  movl $4, %eax
  vpcmpeqd %ymm0, %ymm0, %ymm0
  vxorps %xmm1, %xmm1, %xmm1
  vmovdqu .LCPI0_0(%rip), %ymm2
  vpbroadcastq .LCPI0_1(%rip), %ymm3
  vpbroadcastq .LCPI0_2(%rip), %ymm4
  vmovdqu .LCPI0_3(%rip), %xmm5
  vpbroadcastq .LCPI0_4(%rip), %ymm6
.LBB0_1:
  vmovdqu %ymm0, linux_bitmap_b-32(,%rax,8)
  vmovups %ymm1, linux_bitmap_a-32(,%rax,8)
  vmovq %rax, %xmm7
  vpbroadcastq %xmm7, %ymm7
  vpor %ymm2, %ymm7, %ymm7
  vpand %ymm3, %ymm7, %ymm7
  vpcmpeqq %ymm4, %ymm7, %ymm7
  leal (%rax,%rax,2), %ecx
  leal (%rax,%rcx,4), %ecx
  vmovd %ecx, %xmm8
  vpbroadcastb %xmm8, %xmm8
  vpaddd %xmm5, %xmm8, %xmm8
  vpmovzxdq %xmm8, %ymm8
  vpand %ymm3, %ymm8, %ymm8
  vpsllvq %ymm8, %ymm6, %ymm8
  vpand %ymm7, %ymm8, %ymm8
  vpxor %ymm0, %ymm7, %ymm7
  vmovdqu %ymm7, linux_bitmap_b(,%rax,8)
  vmovdqu %ymm8, linux_bitmap_a(,%rax,8)
  leaq 8(%rax), %rcx
  cmpq $16380, %rax
  movq %rcx, %rax
  jb .LBB0_1
  xorl %eax, %eax
  movl $1, %ecx
  xorl %esi, %esi
  jmp .LBB0_3
.LBB0_10:
  imull $832, %eax, %edx
  andl $16320, %edx
  leal (,%rax,8), %edi
  subl %eax, %edi
  shlxq %rdi, %rcx, %rdi
  xorq %rdi, linux_bitmap_b+40(,%rdx,8)
  incq %rax
  cmpq $1024, %rax
  je .LBB0_11
.LBB0_3:
  movl %eax, %r8d
  andl $63, %r8d
  movq %rax, %rdx
  shlq $19, %rdx
.LBB0_4:
  movq %r8, %rdi
  shrq $6, %rdi
  movq linux_bitmap_b(,%rdi,8), %r9
  shrxq %r8, linux_bitmap_a(,%rdi,8), %r10
  shlxq %r8, %r10, %r10
  andnq %r10, %r9, %r9
  je .LBB0_5
  andl $-64, %r8d
  movq %r8, %rdi
  jmp .LBB0_8
.LBB0_5:
  leaq (,%rdi,8), %r8
  shlq $6, %rdi
.LBB0_6:
  addq $64, %rdi
  cmpq $1048575, %rdi
  ja .LBB0_10
  movq linux_bitmap_b+8(%r8), %r9
  andnq linux_bitmap_a+8(%r8), %r9, %r9
  leaq 8(%r8), %r8
  je .LBB0_6
.LBB0_8:
  movq %r9, %r8
  shrq $32, %r8
  xorl %r10d, %r10d
  testl %r9d, %r9d
  sete %r10b
  cmovneq %r9, %r8
  shll $5, %r10d
  leal 16(%r10), %r9d
  movq %r8, %r11
  shrq $16, %r11
  testw %r8w, %r8w
  cmovneq %r8, %r11
  cmovnel %r10d, %r9d
  leal 8(%r9), %r8d
  movq %r11, %r10
  shrq $8, %r10
  testb %r11b, %r11b
  cmovneq %r11, %r10
  cmovnel %r9d, %r8d
  movq %r10, %r11
  shrq $4, %r11
  testb $15, %r10b
  cmovneq %r10, %r11
  leal 4(%r8), %r10d
  cmovnel %r8d, %r10d
  leal 2(%r10), %r9d
  movl %r11d, %r8d
  shrl $2, %r8d
  testb $3, %r11b
  cmovneq %r11, %r8
  cmovnel %r10d, %r9d
  andl $1, %r8d
  cmpq $1, %r8
  adcl $0, %r9d
  addq %rdi, %r9
  cmpq $1048575, %r9
  ja .LBB0_10
  leaq (%r9,%rdx), %rdi
  xorq %rdi, %rsi
  leaq 1(%r9), %r8
  cmpq $1048575, %r9
  jne .LBB0_4
  jmp .LBB0_10
.LBB0_11:
  movl $.L.str, %edi
  xorl %eax, %eax
  vzeroupper
  callq printf
  xorl %eax, %eax
  popq %rcx
  retq

.L.str:
  .asciz "%lu\n"

