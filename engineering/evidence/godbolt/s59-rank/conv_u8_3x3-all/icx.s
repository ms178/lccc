.LCPI0_0:
.LCPI0_1:
.LCPI0_2:
.LCPI0_3:
.LCPI0_4:
.LCPI0_5:
.LCPI0_7:
main:
  pushq %rbp
  pushq %rbx
  pushq %rax
  vstmxcsr 4(%rsp)
  orl $32832, 4(%rsp)
  vldmxcsr 4(%rsp)
  movl $img, %eax
  xorl %ecx, %ecx
  vmovdqu .LCPI0_0(%rip), %ymm0
  vmovdqu .LCPI0_1(%rip), %ymm1
  vmovdqu .LCPI0_2(%rip), %ymm2
  vmovdqu .LCPI0_3(%rip), %ymm3
  vpbroadcastq .LCPI0_4(%rip), %ymm4
.LBB0_1:
  leaq 3(%rcx), %rdx
  vmovq %rdx, %xmm5
  vpbroadcastq %xmm5, %ymm5
  leaq (%rcx,%rcx,4), %rdx
  vmovq %rdx, %xmm6
  vpbroadcastq %xmm6, %ymm6
  xorl %edx, %edx
.LBB0_2:
  vmovq %rdx, %xmm7
  vpbroadcastq %xmm7, %ymm7
  vpor %ymm0, %ymm7, %ymm8
  vpor %ymm1, %ymm7, %ymm9
  vpor %ymm2, %ymm7, %ymm10
  vpor %ymm3, %ymm7, %ymm7
  vpmuludq %ymm7, %ymm5, %ymm7
  vpmuludq %ymm5, %ymm10, %ymm10
  vpmuludq %ymm5, %ymm9, %ymm9
  vpmuludq %ymm5, %ymm8, %ymm8
  vpaddq %ymm6, %ymm8, %ymm8
  vpaddq %ymm6, %ymm9, %ymm9
  vpaddq %ymm6, %ymm10, %ymm10
  vpaddq %ymm6, %ymm7, %ymm7
  vpand %ymm4, %ymm7, %ymm7
  vpand %ymm4, %ymm10, %ymm10
  vpackusdw %ymm7, %ymm10, %ymm7
  vpermq $216, %ymm7, %ymm7
  vpand %ymm4, %ymm9, %ymm9
  vpand %ymm4, %ymm8, %ymm8
  vpackusdw %ymm9, %ymm8, %ymm8
  vpermq $216, %ymm8, %ymm8
  vpackusdw %ymm7, %ymm8, %ymm7
  vextracti128 $1, %ymm7, %xmm8
  vpackuswb %xmm8, %xmm7, %xmm7
  vpshufd $216, %xmm7, %xmm7
  vmovdqu %xmm7, (%rax,%rdx)
  leaq 16(%rdx), %rsi
  cmpq $48, %rdx
  movq %rsi, %rdx
  jb .LBB0_2
  leaq 1(%rcx), %rdx
  addq $64, %rax
  cmpq $63, %rcx
  movq %rdx, %rcx
  jne .LBB0_1
  xorl %eax, %eax
  vpbroadcastd .LCPI0_5(%rip), %ymm0
  vpbroadcastd .LCPI0_5(%rip), %xmm1
  vpbroadcastd .LCPI0_7(%rip), %xmm2
  movl $255, %ecx
  xorl %edx, %edx
.LBB0_5:
  movq %rdx, %rdi
  shlq $6, %rdi
  vpmovzxbd img(%rdi), %ymm3
  vpmovzxbd img+8(%rdi), %ymm4
  vpmovzxbd img+1(%rdi), %ymm5
  vpmovzxbd img+9(%rdi), %ymm6
  vpaddd %ymm6, %ymm6, %ymm6
  vpaddd %ymm5, %ymm5, %ymm5
  vpmovzxbd img+2(%rdi), %ymm7
  vpaddd %ymm7, %ymm3, %ymm3
  vpaddd %ymm3, %ymm5, %ymm3
  vpmovzxbd img+10(%rdi), %ymm5
  vpaddd %ymm5, %ymm4, %ymm4
  vpaddd %ymm4, %ymm6, %ymm4
  leaq 2(%rdx), %r8
  shlq $6, %r8
  vpmovzxbd img(%r8), %ymm5
  vpsubd %ymm3, %ymm5, %ymm3
  vpmovzxbd img+8(%r8), %ymm5
  vpsubd %ymm4, %ymm5, %ymm4
  vpmovzxbd img+1(%r8), %ymm5
  vpmovzxbd img+9(%r8), %ymm6
  vpaddd %ymm6, %ymm6, %ymm6
  vpaddd %ymm5, %ymm5, %ymm5
  vpmovzxbd img+2(%r8), %ymm7
  vpaddd %ymm7, %ymm5, %ymm5
  vpaddd %ymm5, %ymm3, %ymm3
  vpmovzxbd img+10(%r8), %ymm5
  vpaddd %ymm5, %ymm6, %ymm5
  vpaddd %ymm5, %ymm4, %ymm4
  vpabsd %ymm3, %ymm3
  vpabsd %ymm4, %ymm4
  vpminud %ymm0, %ymm4, %ymm4
  vpminud %ymm0, %ymm3, %ymm3
  vpackusdw %ymm4, %ymm3, %ymm3
  leaq 1(%rdx), %rsi
  movq %rsi, %r9
  shlq $6, %r9
  vextracti128 $1, %ymm3, %xmm4
  vpackuswb %xmm4, %xmm3, %xmm3
  vpshufd $216, %xmm3, %xmm3
  vmovdqu %xmm3, out+1(%r9)
  vpmovzxbd img+16(%rdi), %ymm3
  vpmovzxbd img+24(%rdi), %ymm4
  vpmovzxbd img+17(%rdi), %ymm5
  vpmovzxbd img+25(%rdi), %ymm6
  vpaddd %ymm6, %ymm6, %ymm6
  vpaddd %ymm5, %ymm5, %ymm5
  vpmovzxbd img+18(%rdi), %ymm7
  vpaddd %ymm7, %ymm3, %ymm3
  vpaddd %ymm3, %ymm5, %ymm3
  vpmovzxbd img+26(%rdi), %ymm5
  vpaddd %ymm5, %ymm4, %ymm4
  vpaddd %ymm4, %ymm6, %ymm4
  vpmovzxbd img+16(%r8), %ymm5
  vpmovzxbd img+24(%r8), %ymm6
  vpsubd %ymm3, %ymm5, %ymm3
  vpsubd %ymm4, %ymm6, %ymm4
  vpmovzxbd img+17(%r8), %ymm5
  vpmovzxbd img+25(%r8), %ymm6
  vpaddd %ymm6, %ymm6, %ymm6
  vpaddd %ymm5, %ymm5, %ymm5
  vpmovzxbd img+18(%r8), %ymm7
  vpaddd %ymm7, %ymm5, %ymm5
  vpaddd %ymm5, %ymm3, %ymm3
  vpmovzxbd img+26(%r8), %ymm5
  vpaddd %ymm5, %ymm6, %ymm5
  vpaddd %ymm5, %ymm4, %ymm4
  vpabsd %ymm3, %ymm3
  vpabsd %ymm4, %ymm4
  vpminud %ymm0, %ymm4, %ymm4
  vpminud %ymm0, %ymm3, %ymm3
  vpackusdw %ymm4, %ymm3, %ymm3
  vextracti128 $1, %ymm3, %xmm4
  vpackuswb %xmm4, %xmm3, %xmm3
  vpshufd $216, %xmm3, %xmm3
  vmovdqu %xmm3, out+17(%r9)
  vpmovzxbd img+32(%rdi), %ymm3
  vpmovzxbd img+40(%rdi), %ymm4
  vpmovzxbd img+33(%rdi), %ymm5
  vpmovzxbd img+41(%rdi), %ymm6
  vpaddd %ymm6, %ymm6, %ymm6
  vpaddd %ymm5, %ymm5, %ymm5
  vpmovzxbd img+34(%rdi), %ymm7
  vpaddd %ymm7, %ymm3, %ymm3
  vpaddd %ymm3, %ymm5, %ymm3
  vpmovzxbd img+42(%rdi), %ymm5
  vpaddd %ymm5, %ymm4, %ymm4
  vpaddd %ymm4, %ymm6, %ymm4
  vpmovzxbd img+32(%r8), %ymm5
  vpsubd %ymm3, %ymm5, %ymm3
  vpmovzxbd img+40(%r8), %ymm5
  vpmovzxbd img+33(%r8), %ymm6
  vpsubd %ymm4, %ymm5, %ymm4
  vpmovzxbd img+41(%r8), %ymm5
  vpaddd %ymm5, %ymm5, %ymm5
  vpaddd %ymm6, %ymm6, %ymm6
  vpmovzxbd img+34(%r8), %ymm7
  vpaddd %ymm7, %ymm6, %ymm6
  vpaddd %ymm6, %ymm3, %ymm3
  vpmovzxbd img+42(%r8), %ymm6
  vpaddd %ymm6, %ymm5, %ymm5
  vpaddd %ymm5, %ymm4, %ymm4
  vpabsd %ymm3, %ymm3
  vpabsd %ymm4, %ymm4
  vpminud %ymm0, %ymm4, %ymm4
  vpminud %ymm0, %ymm3, %ymm3
  vpackusdw %ymm4, %ymm3, %ymm3
  vextracti128 $1, %ymm3, %xmm4
  vpackuswb %xmm4, %xmm3, %xmm3
  vpshufd $216, %xmm3, %xmm3
  vmovdqu %xmm3, out+33(%r9)
  xorl %r9d, %r9d
.LBB0_6:
  vpmovzxbd img+48(%rax,%r9), %xmm3
  vpmovzxbd img+49(%rax,%r9), %xmm4
  vpaddd %xmm4, %xmm4, %xmm4
  vpmovzxbd img+50(%rax,%r9), %xmm5
  vpaddd %xmm5, %xmm3, %xmm3
  vpaddd %xmm3, %xmm4, %xmm3
  vpmovzxbd img+176(%rax,%r9), %xmm4
  vpsubd %xmm3, %xmm4, %xmm3
  vpmovzxbd img+177(%rax,%r9), %xmm4
  vpmovzxbd img+178(%rax,%r9), %xmm5
  vpaddd %xmm4, %xmm4, %xmm4
  vpaddd %xmm5, %xmm4, %xmm4
  vpaddd %xmm4, %xmm3, %xmm3
  vpabsd %xmm3, %xmm3
  vpminud %xmm1, %xmm3, %xmm3
  vpshufb %xmm2, %xmm3, %xmm3
  vmovd %xmm3, out+113(%rax,%r9)
  leaq 4(%r9), %r10
  addq $48, %r9
  cmpq $56, %r9
  movq %r10, %r9
  jb .LBB0_6
  movzbl img+60(%rdi), %ebp
  movzbl img+61(%rdi), %r10d
  movzbl img+60(%r8), %r9d
  movzbl img+61(%r8), %r8d
  movq $-2, %rdi
.LBB0_8:
  movzbl %bpl, %r11d
  movzbl %r10b, %ebx
  movl %r10d, %ebp
  movzbl img+64(%rax,%rdi), %r10d
  addl %r10d, %r11d
  leal (%r11,%rbx,2), %r11d
  movzbl %r9b, %r9d
  subl %r11d, %r9d
  movzbl %r8b, %r11d
  leal (%r9,%r11,2), %r11d
  movl %r8d, %r9d
  movzbl img+192(%rax,%rdi), %r8d
  addl %r8d, %r11d
  movl %r11d, %ebx
  negl %ebx
  cmovsl %r11d, %ebx
  cmpl $255, %ebx
  cmovgel %ecx, %ebx
  movb %bl, out+127(%rax,%rdi)
  incq %rdi
  jne .LBB0_8
  addq $64, %rax
  cmpq $61, %rdx
  movq %rsi, %rdx
  jne .LBB0_5
  movq $-4096, %rax
  xorl %esi, %esi
.LBB0_11:
  movzbl out+4096(%rax), %ecx
  movq %rsi, %rdx
  shlq $5, %rdx
  subq %rsi, %rdx
  addq %rcx, %rdx
  movzbl out+4097(%rax), %ecx
  movq %rdx, %rsi
  shlq $5, %rsi
  subq %rdx, %rsi
  addq %rcx, %rsi
  movzbl out+4098(%rax), %ecx
  movq %rsi, %rdx
  shlq $5, %rdx
  subq %rsi, %rdx
  addq %rcx, %rdx
  movzbl out+4099(%rax), %ecx
  movq %rdx, %rsi
  shlq $5, %rsi
  subq %rdx, %rsi
  addq %rcx, %rsi
  movzbl out+4100(%rax), %ecx
  movq %rsi, %rdx
  shlq $5, %rdx
  subq %rsi, %rdx
  addq %rcx, %rdx
  movzbl out+4101(%rax), %ecx
  movq %rdx, %rsi
  shlq $5, %rsi
  subq %rdx, %rsi
  addq %rcx, %rsi
  movzbl out+4102(%rax), %ecx
  movq %rsi, %rdx
  shlq $5, %rdx
  subq %rsi, %rdx
  addq %rcx, %rdx
  movzbl out+4103(%rax), %ecx
  movq %rdx, %rsi
  shlq $5, %rsi
  subq %rdx, %rsi
  addq %rcx, %rsi
  addq $8, %rax
  jne .LBB0_11
  movl $.L.str, %edi
  xorl %eax, %eax
  vzeroupper
  callq printf
  xorl %eax, %eax
  addq $8, %rsp
  popq %rbx
  popq %rbp
  retq

.L.str:

