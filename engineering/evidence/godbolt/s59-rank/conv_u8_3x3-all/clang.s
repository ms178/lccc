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
.LCPI0_12:
.LCPI0_11:
main:
  pushq %rbp
  pushq %r15
  pushq %r14
  pushq %r13
  pushq %r12
  pushq %rbx
  pushq %rax
  leaq img(%rip), %rax
  xorl %ecx, %ecx
  vpbroadcastq .LCPI0_8(%rip), %ymm8
  vpbroadcastq .LCPI0_9(%rip), %ymm9
.LBB0_1:
  leaq 3(%rcx), %rdx
  leaq (%rcx,%rcx,4), %rsi
  vmovq %rsi, %xmm0
  vpbroadcastq %xmm0, %ymm10
  vmovq %rdx, %xmm0
  vpbroadcastq %xmm0, %ymm11
  xorl %edx, %edx
  vmovdqa .LCPI0_7(%rip), %ymm12
  vmovdqa .LCPI0_6(%rip), %ymm13
  vmovdqa .LCPI0_5(%rip), %ymm14
  vmovdqa .LCPI0_4(%rip), %ymm15
  vmovdqa .LCPI0_3(%rip), %ymm3
  vmovdqa .LCPI0_2(%rip), %ymm2
  vmovdqa .LCPI0_1(%rip), %ymm1
  vmovdqa .LCPI0_0(%rip), %ymm0
.LBB0_2:
  vpmuludq %ymm11, %ymm12, %ymm4
  vpmuludq %ymm11, %ymm13, %ymm5
  vpmuludq %ymm11, %ymm14, %ymm6
  vpmuludq %ymm11, %ymm15, %ymm7
  vpaddq %ymm5, %ymm10, %ymm5
  vpaddq %ymm4, %ymm10, %ymm4
  vpand %ymm4, %ymm8, %ymm4
  vpand %ymm5, %ymm8, %ymm5
  vpackusdw %ymm5, %ymm4, %ymm4
  vpmuludq %ymm3, %ymm11, %ymm5
  vpaddq %ymm7, %ymm10, %ymm7
  vpaddq %ymm6, %ymm10, %ymm6
  vpand %ymm6, %ymm8, %ymm6
  vpand %ymm7, %ymm8, %ymm7
  vpackusdw %ymm7, %ymm6, %ymm6
  vpmuludq %ymm2, %ymm11, %ymm7
  vpermq $216, %ymm4, %ymm4
  vpermq $216, %ymm6, %ymm6
  vpackusdw %ymm6, %ymm4, %ymm4
  vpmuludq %ymm1, %ymm11, %ymm6
  vpaddq %ymm7, %ymm10, %ymm7
  vpaddq %ymm5, %ymm10, %ymm5
  vpand %ymm5, %ymm8, %ymm5
  vpand %ymm7, %ymm8, %ymm7
  vpackusdw %ymm7, %ymm5, %ymm5
  vpmuludq %ymm0, %ymm11, %ymm7
  vpaddq %ymm7, %ymm10, %ymm7
  vpaddq %ymm6, %ymm10, %ymm6
  vpand %ymm6, %ymm8, %ymm6
  vpand %ymm7, %ymm8, %ymm7
  vpackusdw %ymm7, %ymm6, %ymm6
  vpermq $216, %ymm5, %ymm5
  vpermq $216, %ymm6, %ymm6
  vpackusdw %ymm6, %ymm5, %ymm5
  vpermq $216, %ymm4, %ymm4
  vpermq $216, %ymm5, %ymm5
  vpackuswb %ymm5, %ymm4, %ymm4
  vpermq $216, %ymm4, %ymm4
  vmovdqu %ymm4, (%rax,%rdx)
  addq $32, %rdx
  vpaddq %ymm9, %ymm12, %ymm12
  vpaddq %ymm9, %ymm13, %ymm13
  vpaddq %ymm9, %ymm14, %ymm14
  vpaddq %ymm9, %ymm15, %ymm15
  vpaddq %ymm3, %ymm9, %ymm3
  vpaddq %ymm2, %ymm9, %ymm2
  vpaddq %ymm1, %ymm9, %ymm1
  vpaddq %ymm0, %ymm9, %ymm0
  cmpq $64, %rdx
  jne .LBB0_2
  incq %rcx
  addq $64, %rax
  cmpq $64, %rcx
  jne .LBB0_1
  movl $1, %eax
  leaq img+138(%rip), %rcx
  leaq out+73(%rip), %rdx
  vmovq .LCPI0_12(%rip), %xmm0
  vpbroadcastd .LCPI0_11(%rip), %ymm1
  movl $255, %r8d
  jmp .LBB0_5
.LBB0_8:
  vpextrb $7, %xmm7, %r11d
  vpextrb $7, %xmm6, %r15d
  vpextrb $6, %xmm7, %esi
  vpextrb $6, %xmm6, %edi
  movzbl -6(%r10), %r12d
  movzbl 122(%r10), %r14d
  movl %r11d, %ebx
  subl %r15d, %ebx
  addl %r12d, %edi
  subl %edi, %esi
  addl %r14d, %esi
  leal (%rsi,%rbx,2), %edi
  movl %edi, %esi
  negl %esi
  cmovsl %edi, %esi
  cmpl $255, %esi
  cmovael %r8d, %esi
  movzbl -5(%r10), %ebp
  movzbl 123(%r10), %ebx
  movl %r14d, %edi
  subl %r12d, %edi
  addl %ebp, %r15d
  subl %r15d, %r11d
  addl %ebx, %r11d
  leal (%r11,%rdi,2), %r11d
  movl %r11d, %edi
  negl %edi
  cmovsl %r11d, %edi
  cmpl $255, %edi
  cmovael %r8d, %edi
  movzbl -4(%r10), %r15d
  movzbl 124(%r10), %r11d
  addl %r15d, %r12d
  subl %r12d, %r14d
  movl %ebx, %r12d
  subl %ebp, %r12d
  addl %r11d, %r14d
  leal (%r14,%r12,2), %r14d
  movl %r14d, %r13d
  negl %r13d
  cmovsl %r14d, %r13d
  movb %sil, 57(%r9)
  cmpl $255, %r13d
  cmovael %r8d, %r13d
  movb %dil, 58(%r9)
  movzbl -3(%r10), %r12d
  movzbl 125(%r10), %r14d
  movl %r11d, %esi
  subl %r15d, %esi
  addl %r12d, %ebp
  subl %ebp, %ebx
  addl %r14d, %ebx
  leal (%rbx,%rsi,2), %esi
  movl %esi, %edi
  negl %edi
  cmovsl %esi, %edi
  cmpl $255, %edi
  cmovael %r8d, %edi
  movzbl -2(%r10), %ebx
  movzbl 126(%r10), %esi
  addl %ebx, %r15d
  subl %r15d, %r11d
  movl %r14d, %r15d
  subl %r12d, %r15d
  addl %esi, %r11d
  leal (%r11,%r15,2), %r11d
  movl %r11d, %ebp
  negl %ebp
  cmovsl %r11d, %ebp
  movb %r13b, 59(%r9)
  cmpl $255, %ebp
  cmovael %r8d, %ebp
  movb %dil, 60(%r9)
  movb %bpl, 61(%r9)
  movzbl -1(%r10), %edi
  movzbl 127(%r10), %r10d
  subl %ebx, %esi
  addl %r12d, %edi
  subl %edi, %r14d
  addl %r10d, %r14d
  leal (%r14,%rsi,2), %esi
  movl %esi, %edi
  negl %edi
  cmovsl %esi, %edi
  cmpl $255, %edi
  cmovael %r8d, %edi
  movb %dil, 62(%r9)
  incq %rax
  addq $64, %rcx
  addq $64, %rdx
  cmpq $63, %rax
  je .LBB0_9
.LBB0_5:
  movq %rax, %rsi
  shlq $6, %rsi
  leaq img(%rip), %r11
  vpbroadcastb 65(%rsi,%r11), %xmm2
  leaq out(%rip), %rdi
  leaq (%rdi,%rsi), %r9
  vpbroadcastb -63(%rsi,%r11), %xmm3
  leaq (%r11,%rsi), %r10
  vpbroadcastb 64(%rsi,%r11), %xmm4
  vpbroadcastb -64(%rsi,%r11), %xmm5
  xorl %r11d, %r11d
.LBB0_6:
  vmovq -136(%rcx,%r11), %xmm6
  vpunpcklbw %xmm3, %xmm6, %xmm3
  vpshufb %xmm0, %xmm3, %xmm8
  vpunpcklbw %xmm5, %xmm8, %xmm3
  vpshufb %xmm0, %xmm3, %xmm3
  vpmovzxbd %xmm3, %ymm3
  vpmovzxbd %xmm8, %ymm5
  vpmovzxbd %xmm6, %ymm7
  vpaddd %ymm7, %ymm3, %ymm3
  vmovq -8(%rcx,%r11), %xmm7
  vpunpcklbw %xmm2, %xmm7, %xmm2
  vpshufb %xmm0, %xmm2, %xmm9
  vpunpcklbw %xmm4, %xmm9, %xmm2
  vpshufb %xmm0, %xmm2, %xmm2
  vpmovzxbd %xmm2, %ymm2
  vpsubd %ymm3, %ymm2, %ymm2
  vpmovzxbd %xmm9, %ymm3
  vpsubd %ymm5, %ymm3, %ymm3
  vpmovzxbd %xmm7, %ymm4
  vpaddd %ymm3, %ymm3, %ymm3
  vpaddd %ymm3, %ymm4, %ymm3
  vpaddd %ymm3, %ymm2, %ymm2
  vpabsd %ymm2, %ymm2
  vpminud %ymm1, %ymm2, %ymm2
  vextracti128 $1, %ymm2, %xmm3
  vpackusdw %xmm3, %xmm2, %xmm2
  vpackuswb %xmm2, %xmm2, %xmm2
  vmovq %xmm2, -8(%rdx,%r11)
  cmpq $48, %r11
  je .LBB0_8
  vmovq -128(%rcx,%r11), %xmm3
  vpunpcklbw %xmm6, %xmm3, %xmm2
  vpshufb %xmm0, %xmm2, %xmm5
  vpunpcklbw %xmm8, %xmm5, %xmm2
  vpshufb %xmm0, %xmm2, %xmm2
  vpmovzxbd %xmm2, %ymm2
  vpmovzxbd %xmm5, %ymm6
  vpmovzxbd %xmm3, %ymm4
  vpaddd %ymm4, %ymm2, %ymm8
  vmovq (%rcx,%r11), %xmm2
  vpunpcklbw %xmm7, %xmm2, %xmm4
  vpshufb %xmm0, %xmm4, %xmm4
  vpunpcklbw %xmm9, %xmm4, %xmm7
  vpshufb %xmm0, %xmm7, %xmm7
  vpmovzxbd %xmm7, %ymm7
  vpsubd %ymm8, %ymm7, %ymm7
  vpmovzxbd %xmm4, %ymm8
  vpsubd %ymm6, %ymm8, %ymm6
  vpmovzxbd %xmm2, %ymm8
  vpaddd %ymm6, %ymm6, %ymm6
  vpaddd %ymm6, %ymm8, %ymm6
  vpaddd %ymm6, %ymm7, %ymm6
  vpabsd %ymm6, %ymm6
  vpminud %ymm1, %ymm6, %ymm6
  vextracti128 $1, %ymm6, %xmm7
  vpackusdw %xmm7, %xmm6, %xmm6
  vpackuswb %xmm6, %xmm6, %xmm6
  vmovq %xmm6, (%rdx,%r11)
  addq $16, %r11
  jmp .LBB0_6
.LBB0_9:
  leaq out+7(%rip), %rax
  xorl %esi, %esi
  xorl %ecx, %ecx
.LBB0_10:
  xorl %edx, %edx
.LBB0_11:
  movq %rsi, %rdi
  shlq $5, %rdi
  subq %rsi, %rdi
  movzbl -7(%rax,%rdx), %esi
  addq %rdi, %rsi
  movq %rsi, %rdi
  shlq $5, %rdi
  subq %rsi, %rdi
  movzbl -6(%rax,%rdx), %esi
  addq %rdi, %rsi
  movq %rsi, %rdi
  shlq $5, %rdi
  subq %rsi, %rdi
  movzbl -5(%rax,%rdx), %esi
  addq %rdi, %rsi
  movq %rsi, %rdi
  shlq $5, %rdi
  subq %rsi, %rdi
  movzbl -4(%rax,%rdx), %esi
  addq %rdi, %rsi
  movq %rsi, %rdi
  shlq $5, %rdi
  subq %rsi, %rdi
  movzbl -3(%rax,%rdx), %esi
  addq %rdi, %rsi
  movq %rsi, %rdi
  shlq $5, %rdi
  subq %rsi, %rdi
  movzbl -2(%rax,%rdx), %esi
  addq %rdi, %rsi
  movq %rsi, %rdi
  shlq $5, %rdi
  subq %rsi, %rdi
  movzbl -1(%rax,%rdx), %esi
  addq %rdi, %rsi
  movq %rsi, %rdi
  shlq $5, %rdi
  subq %rsi, %rdi
  movzbl (%rax,%rdx), %esi
  addq %rdi, %rsi
  addq $8, %rdx
  cmpq $64, %rdx
  jne .LBB0_11
  incq %rcx
  addq $64, %rax
  cmpq $64, %rcx
  jne .LBB0_10
  leaq .L.str(%rip), %rdi
  xorl %eax, %eax
  vzeroupper
  callq printf@PLT
  xorl %eax, %eax
  addq $8, %rsp
  popq %rbx
  popq %r12
  popq %r13
  popq %r14
  popq %r15
  popq %rbp
  retq

.L.str:

