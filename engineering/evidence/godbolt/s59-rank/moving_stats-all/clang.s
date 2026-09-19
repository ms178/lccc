.LCPI0_0:
.LCPI0_1:
.LCPI0_2:
.LCPI0_3:
.LCPI0_4:
.LCPI0_5:
.LCPI0_7:
.LCPI0_8:
.LCPI0_11:
.LCPI0_9:
.LCPI0_10:
main:
  vmovdqa .LCPI0_0(%rip), %ymm0
  vmovdqa .LCPI0_1(%rip), %ymm1
  xorl %eax, %eax
  vpbroadcastd .LCPI0_2(%rip), %ymm2
  vpbroadcastd .LCPI0_3(%rip), %ymm3
  vpbroadcastd .LCPI0_4(%rip), %ymm4
  vpbroadcastd .LCPI0_5(%rip), %ymm5
  vpbroadcastd .LCPI0_11(%rip), %ymm6
  leaq main.a(%rip), %rdx
  vpbroadcastd .LCPI0_7(%rip), %ymm7
  vpbroadcastd .LCPI0_8(%rip), %ymm8
.LBB0_1:
  vpmulld %ymm2, %ymm1, %ymm9
  vpmulld %ymm2, %ymm0, %ymm10
  vpaddd %ymm3, %ymm10, %ymm11
  vpaddd %ymm3, %ymm9, %ymm12
  vpshufd $245, %ymm12, %ymm13
  vpmuludq %ymm4, %ymm13, %ymm13
  vpmuludq %ymm4, %ymm12, %ymm14
  vpshufd $245, %ymm14, %ymm14
  vpblendd $170, %ymm13, %ymm14, %ymm13
  vpsubd %ymm13, %ymm12, %ymm14
  vpsrld $1, %ymm14, %ymm14
  vpaddd %ymm13, %ymm14, %ymm13
  vpsrld $10, %ymm13, %ymm13
  vpmulld %ymm5, %ymm13, %ymm13
  vpsubd %ymm13, %ymm12, %ymm12
  vpshufd $245, %ymm11, %ymm13
  vpmuludq %ymm4, %ymm13, %ymm13
  vpmuludq %ymm4, %ymm11, %ymm14
  vpshufd $245, %ymm14, %ymm14
  vpblendd $170, %ymm13, %ymm14, %ymm13
  vpsubd %ymm13, %ymm11, %ymm14
  vpsrld $1, %ymm14, %ymm14
  vpaddd %ymm13, %ymm14, %ymm13
  vpsrld $10, %ymm13, %ymm13
  vpmulld %ymm5, %ymm13, %ymm13
  vpsubd %ymm13, %ymm11, %ymm11
  vpackusdw %ymm11, %ymm12, %ymm11
  vpermq $216, %ymm11, %ymm11
  vpaddw %ymm6, %ymm11, %ymm11
  vmovdqu %ymm11, (%rdx,%rax,2)
  vpaddd %ymm7, %ymm10, %ymm10
  vpaddd %ymm7, %ymm9, %ymm9
  vpshufd $245, %ymm9, %ymm11
  vpmuludq %ymm4, %ymm11, %ymm11
  vpmuludq %ymm4, %ymm9, %ymm12
  vpshufd $245, %ymm12, %ymm12
  vpblendd $170, %ymm11, %ymm12, %ymm11
  vpsubd %ymm11, %ymm9, %ymm12
  vpsrld $1, %ymm12, %ymm12
  vpaddd %ymm11, %ymm12, %ymm11
  vpsrld $10, %ymm11, %ymm11
  vpmulld %ymm5, %ymm11, %ymm11
  vpsubd %ymm11, %ymm9, %ymm9
  vpshufd $245, %ymm10, %ymm11
  vpmuludq %ymm4, %ymm11, %ymm11
  vpmuludq %ymm4, %ymm10, %ymm12
  vpshufd $245, %ymm12, %ymm12
  vpblendd $170, %ymm11, %ymm12, %ymm11
  vpsubd %ymm11, %ymm10, %ymm12
  vpsrld $1, %ymm12, %ymm12
  vpaddd %ymm11, %ymm12, %ymm11
  vpsrld $10, %ymm11, %ymm11
  vpmulld %ymm5, %ymm11, %ymm11
  vpsubd %ymm11, %ymm10, %ymm10
  vpackusdw %ymm10, %ymm9, %ymm9
  vpermq $216, %ymm9, %ymm9
  vpaddw %ymm6, %ymm9, %ymm9
  vmovdqu %ymm9, 32(%rdx,%rax,2)
  addq $32, %rax
  vpaddd %ymm1, %ymm8, %ymm1
  vpaddd %ymm0, %ymm8, %ymm0
  cmpq $1024, %rax
  jne .LBB0_1
  pushq %rbx
  vpbroadcastw main.a(%rip), %xmm0
  leaq main.sums(%rip), %rsi
  xorl %edi, %edi
  leaq main.mns(%rip), %rax
  leaq main.mxs(%rip), %rcx
.LBB0_3:
  vmovdqa %xmm0, %xmm1
  vmovdqu 2(%rdi,%rdx), %xmm0
  vmovdqu 4(%rdi,%rdx), %xmm2
  vmovdqu 6(%rdi,%rdx), %xmm3
  vmovdqu 8(%rdi,%rdx), %xmm4
  vpalignr $14, %xmm1, %xmm0, %xmm1
  vpmovsxwd %xmm1, %ymm5
  vpmovsxwd %xmm0, %ymm6
  vpmovsxwd %xmm2, %ymm7
  vpaddd %ymm7, %ymm6, %ymm6
  vpaddd %ymm5, %ymm6, %ymm5
  vpminsw %xmm0, %xmm2, %xmm6
  vpminsw %xmm1, %xmm6, %xmm6
  vpmaxsw %xmm0, %xmm2, %xmm2
  vpmaxsw %xmm1, %xmm2, %xmm1
  vpmovsxwd %xmm3, %ymm2
  vpmovsxwd %xmm4, %ymm7
  vpaddd %ymm7, %ymm2, %ymm2
  vpminsw %xmm3, %xmm4, %xmm7
  vpmaxsw %xmm3, %xmm4, %xmm3
  vmovdqu 10(%rdi,%rdx), %xmm4
  vpmovsxwd %xmm4, %ymm8
  vpaddd %ymm2, %ymm8, %ymm2
  vpaddd %ymm2, %ymm5, %ymm2
  vpminsw %xmm7, %xmm4, %xmm5
  vpminsw %xmm6, %xmm5, %xmm5
  vpmaxsw %xmm3, %xmm4, %xmm3
  vpmaxsw %xmm1, %xmm3, %xmm1
  vmovdqu 12(%rdi,%rdx), %xmm3
  vpmovsxwd %xmm3, %ymm4
  vmovdqu 14(%rdi,%rdx), %xmm6
  vpmovsxwd %xmm6, %ymm7
  vpaddd %ymm7, %ymm4, %ymm4
  vpminsw %xmm3, %xmm6, %xmm7
  vpmaxsw %xmm3, %xmm6, %xmm3
  vmovdqa 16(%rdi,%rdx), %xmm6
  vpmovsxwd %xmm6, %ymm8
  vpaddd %ymm4, %ymm8, %ymm4
  vpminsw %xmm7, %xmm6, %xmm7
  vpmaxsw %xmm3, %xmm6, %xmm3
  vmovdqu 18(%rdi,%rdx), %xmm6
  vpmovsxwd %xmm6, %ymm8
  vpaddd %ymm4, %ymm8, %ymm4
  vpaddd %ymm4, %ymm2, %ymm2
  vpminsw %xmm7, %xmm6, %xmm4
  vpminsw %xmm5, %xmm4, %xmm4
  vpmaxsw %xmm3, %xmm6, %xmm3
  vpmaxsw %xmm1, %xmm3, %xmm1
  vmovdqu 20(%rdi,%rdx), %xmm3
  vpmovsxwd %xmm3, %ymm5
  vmovdqu 22(%rdi,%rdx), %xmm6
  vpmovsxwd %xmm6, %ymm7
  vpaddd %ymm7, %ymm5, %ymm5
  vpminsw %xmm3, %xmm6, %xmm7
  vpmaxsw %xmm3, %xmm6, %xmm3
  vmovdqu 24(%rdi,%rdx), %xmm6
  vpmovsxwd %xmm6, %ymm8
  vpaddd %ymm5, %ymm8, %ymm5
  vpminsw %xmm7, %xmm6, %xmm7
  vpmaxsw %xmm3, %xmm6, %xmm3
  vmovdqu 26(%rdi,%rdx), %xmm6
  vpmovsxwd %xmm6, %ymm8
  vpaddd %ymm5, %ymm8, %ymm5
  vpminsw %xmm7, %xmm6, %xmm7
  vpmaxsw %xmm3, %xmm6, %xmm3
  vmovdqu 28(%rdi,%rdx), %xmm6
  vpmovsxwd %xmm6, %ymm8
  vpaddd %ymm5, %ymm8, %ymm5
  vpaddd %ymm5, %ymm2, %ymm2
  vpminsw %xmm7, %xmm6, %xmm5
  vpminsw %xmm4, %xmm5, %xmm4
  vpmaxsw %xmm3, %xmm6, %xmm3
  vpmaxsw %xmm1, %xmm3, %xmm1
  vmovdqu 30(%rdi,%rdx), %xmm3
  vpmovsxwd %xmm3, %ymm5
  vpaddd %ymm5, %ymm2, %ymm2
  vpminsw %xmm4, %xmm3, %xmm4
  vpmaxsw %xmm1, %xmm3, %xmm1
  vmovdqu %ymm2, (%rsi)
  vmovdqa %xmm4, (%rdi,%rax)
  vmovdqa %xmm1, (%rdi,%rcx)
  addq $16, %rdi
  addq $32, %rsi
  cmpq $2016, %rdi
  jne .LBB0_3
  vpextrw $7, %xmm0, %edx
  movswl %dx, %esi
  vmovdqu main.a+2018(%rip), %xmm0
  vmovd %xmm0, %edi
  movswl %di, %edi
  vpextrw $1, %xmm0, %r8d
  movswl %r8w, %r8d
  addl %edi, %r8d
  vpextrw $2, %xmm0, %edi
  addl %esi, %r8d
  movswl %di, %edi
  vpextrw $3, %xmm0, %r9d
  movswl %r9w, %r9d
  addl %edi, %r9d
  vpextrw $4, %xmm0, %edi
  movswl %di, %edi
  addl %r9d, %edi
  vpextrw $5, %xmm0, %r9d
  addl %r8d, %edi
  movswl %r9w, %r8d
  vpextrw $6, %xmm0, %r9d
  movswl %r9w, %r9d
  addl %r8d, %r9d
  vpextrw $7, %xmm0, %r8d
  movswl %r8w, %r8d
  addl %r9d, %r8d
  movq main.a+2034(%rip), %r9
  vmovq %r9, %xmm1
  movswl %r9w, %r10d
  addl %r8d, %r10d
  addl %edi, %r10d
  movl %r9d, %edi
  sarl $16, %edi
  movq %r9, %r8
  shrq $32, %r8
  movswl %r8w, %r8d
  addl %edi, %r8d
  shrq $48, %r9
  movswl %r9w, %edi
  addl %r8d, %edi
  movswl main.a+2042(%rip), %r8d
  addl %r8d, %edi
  movswl main.a+2044(%rip), %r11d
  addl %r11d, %edi
  movswl main.a+2046(%rip), %r9d
  addl %r9d, %edi
  vpminsw %xmm1, %xmm0, %xmm2
  vpblendd $12, %xmm0, %xmm2, %xmm2
  vpxor .LCPI0_9(%rip), %xmm2, %xmm2
  addl %r10d, %edi
  vphminposuw %xmm2, %xmm2
  vmovd %xmm2, %ebx
  xorl $32768, %ebx
  cmpw %r8w, %bx
  cmovgel %r8d, %ebx
  cmpw %r9w, %r11w
  movl %r9d, %r10d
  cmovll %r11d, %r10d
  cmovgl %r11d, %r9d
  cmpw %r10w, %bx
  cmovll %ebx, %r10d
  cmpw %si, %r10w
  cmovgel %edx, %r10d
  vpmaxsw %xmm1, %xmm0, %xmm1
  vpblendd $12, %xmm0, %xmm1, %xmm0
  vpxor .LCPI0_10(%rip), %xmm0, %xmm0
  vphminposuw %xmm0, %xmm0
  vmovd %xmm0, %r11d
  xorl $32767, %r11d
  cmpw %r8w, %r11w
  cmovlel %r8d, %r11d
  cmpw %r9w, %r11w
  cmovlel %r9d, %r11d
  cmpw %si, %r11w
  cmovlel %edx, %r11d
  movl %edi, main.sums+4032(%rip)
  movw %r10w, main.mns+2016(%rip)
  movw %r11w, main.mxs+2016(%rip)
  movl $1, %edx
  xorl %esi, %esi
  leaq main.sums(%rip), %rdi
.LBB0_5:
  movq %rsi, %r8
  shlq $5, %r8
  subq %rsi, %r8
  movl -4(%rdi,%rdx,4), %esi
  addq %r8, %rsi
  movq %rsi, %r8
  shlq $5, %r8
  subq %rsi, %r8
  movswl -2(%rax,%rdx,2), %esi
  addl $1000, %esi
  addq %r8, %rsi
  movq %rsi, %r8
  shlq $5, %r8
  subq %rsi, %r8
  movswl -2(%rcx,%rdx,2), %esi
  addl $1000, %esi
  addq %r8, %rsi
  cmpq $1009, %rdx
  je .LBB0_7
  movq %rsi, %r8
  shlq $5, %r8
  subq %rsi, %r8
  movl (%rdi,%rdx,4), %esi
  addq %r8, %rsi
  movq %rsi, %r8
  shlq $5, %r8
  subq %rsi, %r8
  movswl (%rax,%rdx,2), %esi
  addl $1000, %esi
  addq %r8, %rsi
  movq %rsi, %r8
  shlq $5, %r8
  subq %rsi, %r8
  movswl (%rcx,%rdx,2), %esi
  addl $1000, %esi
  addq %r8, %rsi
  addq $2, %rdx
  jmp .LBB0_5
.LBB0_7:
  leaq .L.str(%rip), %rdi
  xorl %eax, %eax
  vzeroupper
  callq printf@PLT
  xorl %eax, %eax
  popq %rbx
  retq

.L.str:

