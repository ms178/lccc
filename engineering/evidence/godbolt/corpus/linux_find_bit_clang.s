.LCPI0_0:
  .quad 0
  .quad 1
  .quad 2
  .quad 3
.LCPI0_1:
  .quad 63
.LCPI0_2:
  .quad 5
.LCPI0_3:
  .quad 256
.LCPI0_4:
  .quad 4
main:
  pushq %r14
  pushq %rbx
  pushq %rax
  vmovdqa .LCPI0_0(%rip), %ymm0
  xorl %edx, %edx
  leaq linux_bitmap_a(%rip), %rax
  vpxor %xmm1, %xmm1, %xmm1
  leaq linux_bitmap_b(%rip), %rcx
  vpcmpeqd %ymm2, %ymm2, %ymm2
  vpbroadcastq .LCPI0_1(%rip), %ymm3
  vpbroadcastq .LCPI0_2(%rip), %ymm4
  vpbroadcastq .LCPI0_3(%rip), %ymm5
  vpbroadcastq .LCPI0_4(%rip), %ymm6
.LBB0_1:
  vmovdqu %ymm1, (%rdx,%rax)
  vmovdqu %ymm2, (%rdx,%rcx)
  vpand %ymm3, %ymm0, %ymm7
  vpcmpeqq %ymm4, %ymm7, %ymm7
  vpmaskmovq %ymm5, %ymm7, (%rdx,%rax)
  vpmaskmovq %ymm1, %ymm7, (%rdx,%rcx)
  vpaddq %ymm6, %ymm0, %ymm0
  addq $32, %rdx
  cmpq $131072, %rdx
  jne .LBB0_1
  xorl %edx, %edx
  movl $1, %edi
  xorl %esi, %esi
  jmp .LBB0_3
.LBB0_10:
  imull $832, %edx, %r8d
  andl $16320, %r8d
  leal (,%rdx,8), %r9d
  subl %edx, %r9d
  shlxq %r9, %rdi, %r9
  xorq %r9, 40(%rcx,%r8,8)
  incq %rdx
  cmpq $1024, %rdx
  je .LBB0_11
.LBB0_3:
  movl %edx, %r10d
  andl $63, %r10d
  movq %rdx, %r8
  shlq $19, %r8
  movq %rsi, %r9
.LBB0_4:
  movq %r9, %rsi
  cmpq $1048575, %r10
  ja .LBB0_10
  movq %r10, %r9
  shrq $6, %r9
  movq (%rcx,%r9,8), %r11
  shrxq %r10, (%rax,%r9,8), %rbx
  shlxq %r10, %rbx, %rbx
  andnq %rbx, %r11, %r11
  je .LBB0_6
  andl $1048512, %r10d
  movq %r10, %r9
  jmp .LBB0_9
.LBB0_6:
  leaq 1(%r9), %r10
  shlq $6, %r9
.LBB0_7:
  addq $64, %r9
  cmpq $1048575, %r9
  ja .LBB0_10
  movq (%rcx,%r10,8), %r11
  andnq (%rax,%r10,8), %r11, %r11
  leaq 1(%r10), %r10
  je .LBB0_7
.LBB0_9:
  movq %r11, %r10
  shrq $32, %r10
  xorl %ebx, %ebx
  testl %r11d, %r11d
  sete %bl
  cmovneq %r11, %r10
  shll $5, %ebx
  leal 16(%rbx), %r11d
  movq %r10, %r14
  shrq $16, %r14
  testw %r10w, %r10w
  cmovneq %r10, %r14
  cmovnel %ebx, %r11d
  leal 8(%r11), %r10d
  movq %r14, %rbx
  shrq $8, %rbx
  testb %r14b, %r14b
  cmovneq %r14, %rbx
  cmovnel %r11d, %r10d
  leal 4(%r10), %r11d
  movq %rbx, %r14
  shrq $4, %r14
  testb $15, %bl
  cmovneq %rbx, %r14
  cmovnel %r10d, %r11d
  leal 2(%r11), %r10d
  movl %r14d, %ebx
  shrl $2, %ebx
  testb $3, %r14b
  cmovnel %r11d, %r10d
  cmovnel %r14d, %ebx
  notl %ebx
  andl $1, %ebx
  addl %r10d, %ebx
  addq %r9, %rbx
  cmpq $1048576, %rbx
  movl $1048576, %r10d
  cmovbq %rbx, %r10
  leaq (%r10,%r8), %r9
  xorq %rsi, %r9
  incq %r10
  cmpq $1048576, %rbx
  jb .LBB0_4
  jmp .LBB0_10
.LBB0_11:
  leaq .L.str(%rip), %rdi
  xorl %eax, %eax
  vzeroupper
  callq printf@PLT
  xorl %eax, %eax
  addq $8, %rsp
  popq %rbx
  popq %r14
  retq

.L.str:
  .asciz "%lu\n"

