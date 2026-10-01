.LCPI0_0:
.LCPI0_1:
.LCPI0_2:
.LCPI0_3:
.LCPI0_5:
.LCPI0_6:
.LCPI0_7:
.LCPI0_8:
.LCPI0_9:
main:
  pushq %rax
  vstmxcsr 4(%rsp)
  orl $32832, 4(%rsp)
  vldmxcsr 4(%rsp)
  movl $11472, %eax
  movq $-16, %rcx
  vmovdqu .LCPI0_0(%rip), %ymm0
  vmovdqu .LCPI0_1(%rip), %ymm1
  vpbroadcastd .LCPI0_2(%rip), %ymm2
  vpbroadcastd .LCPI0_3(%rip), %ymm3
  vpbroadcastw .LCPI0_7(%rip), %ymm4
.LBB0_1:
  leal -11472(%rax), %edx
  vmovd %edx, %xmm5
  vpbroadcastd %xmm5, %ymm5
  vpaddd %ymm1, %ymm5, %ymm6
  vpshufd $245, %ymm6, %ymm7
  vpmuludq %ymm2, %ymm7, %ymm7
  vpmuludq %ymm2, %ymm6, %ymm8
  vpshufd $245, %ymm8, %ymm8
  vpblendd $170, %ymm7, %ymm8, %ymm7
  vpsubd %ymm7, %ymm6, %ymm8
  vpsrld $1, %ymm8, %ymm8
  vpaddd %ymm7, %ymm8, %ymm7
  vpsrld $10, %ymm7, %ymm7
  vpmulld %ymm3, %ymm7, %ymm7
  vpaddd %ymm0, %ymm5, %ymm5
  vpshufd $245, %ymm5, %ymm8
  vpmuludq %ymm2, %ymm8, %ymm8
  vpmuludq %ymm2, %ymm5, %ymm9
  vpshufd $245, %ymm9, %ymm9
  vpblendd $170, %ymm8, %ymm9, %ymm8
  vpsubd %ymm8, %ymm5, %ymm9
  vpsrld $1, %ymm9, %ymm9
  vpaddd %ymm8, %ymm9, %ymm8
  vpsrld $10, %ymm8, %ymm8
  vpmulld %ymm3, %ymm8, %ymm8
  vpsubd %ymm7, %ymm6, %ymm6
  vpsubd %ymm8, %ymm5, %ymm5
  vpackusdw %ymm6, %ymm5, %ymm5
  vpermq $216, %ymm5, %ymm5
  vpaddw %ymm4, %ymm5, %ymm5
  vmovdqu %ymm5, main.a+32(%rcx,%rcx)
  vmovd %eax, %xmm5
  vpbroadcastd %xmm5, %ymm5
  vpaddd %ymm0, %ymm5, %ymm6
  vpaddd %ymm1, %ymm5, %ymm5
  vpshufd $245, %ymm5, %ymm7
  vpmuludq %ymm2, %ymm7, %ymm7
  vpmuludq %ymm2, %ymm5, %ymm8
  vpshufd $245, %ymm8, %ymm8
  vpblendd $170, %ymm7, %ymm8, %ymm7
  vpsubd %ymm7, %ymm5, %ymm8
  vpsrld $1, %ymm8, %ymm8
  vpaddd %ymm7, %ymm8, %ymm7
  vpsrld $10, %ymm7, %ymm7
  vpmulld %ymm3, %ymm7, %ymm7
  vpsubd %ymm7, %ymm5, %ymm5
  vpshufd $245, %ymm6, %ymm7
  vpmuludq %ymm2, %ymm7, %ymm7
  vpmuludq %ymm2, %ymm6, %ymm8
  vpshufd $245, %ymm8, %ymm8
  vpblendd $170, %ymm7, %ymm8, %ymm7
  vpsubd %ymm7, %ymm6, %ymm8
  vpsrld $1, %ymm8, %ymm8
  vpaddd %ymm7, %ymm8, %ymm7
  vpsrld $10, %ymm7, %ymm7
  vpmulld %ymm3, %ymm7, %ymm7
  vpsubd %ymm7, %ymm6, %ymm6
  vpackusdw %ymm5, %ymm6, %ymm5
  vpermq $216, %ymm5, %ymm5
  vpaddw %ymm4, %ymm5, %ymm5
  vmovdqu %ymm5, main.a+64(%rcx,%rcx)
  addl $22944, %eax
  addq $32, %rcx
  cmpq $1008, %rcx
  jb .LBB0_1
  xorl %eax, %eax
  vpbroadcastw .LCPI0_8(%rip), %xmm0
  vpbroadcastw .LCPI0_9(%rip), %xmm1
.LBB0_3:
  vpbroadcastw main.a(%rax), %xmm2
  vmovdqu main.a(%rax), %xmm3
  vpmovsxwd %xmm3, %ymm4
  vmovdqu main.a+16(%rax), %xmm5
  vpmovsxwd %xmm5, %ymm6
  vpaddd %ymm6, %ymm4, %ymm4
  vpminsw %xmm3, %xmm5, %xmm6
  vpminsw %xmm2, %xmm6, %xmm6
  vpmaxsw %xmm3, %xmm5, %xmm3
  vpmaxsw %xmm2, %xmm3, %xmm2
  vextracti128 $1, %ymm4, %xmm3
  vpaddd %xmm3, %xmm4, %xmm3
  vpshufd $238, %xmm3, %xmm4
  vpaddd %xmm4, %xmm3, %xmm3
  vpshufd $85, %xmm3, %xmm4
  vpaddd %xmm4, %xmm3, %xmm3
  vpxor %xmm0, %xmm6, %xmm4
  vphminposuw %xmm4, %xmm4
  vmovd %xmm4, %ecx
  xorl $32768, %ecx
  vpxor %xmm1, %xmm2, %xmm2
  vpbroadcastw main.a+2(%rax), %xmm4
  vphminposuw %xmm2, %xmm2
  vmovd %xmm2, %edx
  vmovd %xmm3, main.sums(%rax,%rax)
  xorl $32767, %edx
  movw %cx, main.mns(%rax)
  movw %dx, main.mxs(%rax)
  vmovdqu main.a+2(%rax), %xmm2
  vpmovsxwd %xmm2, %ymm3
  vmovdqu main.a+18(%rax), %xmm5
  vpmovsxwd %xmm5, %ymm6
  vpaddd %ymm6, %ymm3, %ymm3
  vpminsw %xmm2, %xmm5, %xmm6
  vpminsw %xmm4, %xmm6, %xmm6
  vpmaxsw %xmm2, %xmm5, %xmm2
  vpmaxsw %xmm4, %xmm2, %xmm2
  vextracti128 $1, %ymm3, %xmm4
  vpaddd %xmm4, %xmm3, %xmm3
  vpshufd $238, %xmm3, %xmm4
  vpaddd %xmm4, %xmm3, %xmm3
  vpshufd $85, %xmm3, %xmm4
  vpaddd %xmm4, %xmm3, %xmm3
  vpxor %xmm0, %xmm6, %xmm4
  vphminposuw %xmm4, %xmm4
  vmovd %xmm4, %ecx
  xorl $32768, %ecx
  vpxor %xmm1, %xmm2, %xmm2
  vphminposuw %xmm2, %xmm2
  vmovd %xmm2, %edx
  xorl $32767, %edx
  vmovd %xmm3, main.sums+4(%rax,%rax)
  movw %cx, main.mns+2(%rax)
  movw %dx, main.mxs+2(%rax)
  addq $4, %rax
  cmpq $2016, %rax
  jne .LBB0_3
  vpbroadcastw main.a+2016(%rip), %xmm0
  vmovdqu main.a+2016(%rip), %xmm1
  vmovdqu main.a+2032(%rip), %xmm2
  vpmovsxwd %xmm1, %ymm3
  vpmovsxwd %xmm2, %ymm4
  vpaddd %ymm4, %ymm3, %ymm3
  vpminsw %xmm1, %xmm2, %xmm4
  vpminsw %xmm0, %xmm4, %xmm4
  vpmaxsw %xmm1, %xmm2, %xmm1
  vpmaxsw %xmm0, %xmm1, %xmm0
  vextracti128 $1, %ymm3, %xmm1
  vpaddd %xmm1, %xmm3, %xmm1
  vpshufd $238, %xmm1, %xmm2
  vpaddd %xmm2, %xmm1, %xmm1
  vpshufd $85, %xmm1, %xmm2
  vpaddd %xmm2, %xmm1, %xmm1
  vmovd %xmm1, %edx
  vpxor .LCPI0_5(%rip), %xmm4, %xmm2
  vphminposuw %xmm2, %xmm2
  vmovd %xmm2, %ecx
  xorl $32768, %ecx
  vpxor .LCPI0_6(%rip), %xmm0, %xmm0
  vphminposuw %xmm0, %xmm0
  vmovd %xmm0, %eax
  xorl $32767, %eax
  vmovd %xmm1, main.sums+4032(%rip)
  movw %cx, main.mns+2016(%rip)
  movw %ax, main.mxs+2016(%rip)
  movq $-2016, %rsi
  xorl %edi, %edi
.LBB0_5:
  movl main.sums+4032(%rsi,%rsi), %r8d
  movswl main.mns+2016(%rsi), %r9d
  movswl main.mxs+2016(%rsi), %r10d
  imulq $961, %rdi, %rdi
  movq %r8, %r11
  shlq $5, %r11
  subq %r8, %r11
  addl $1000, %r9d
  addq %rdi, %r9
  addq %r11, %r9
  movq %r9, %rdi
  shlq $5, %rdi
  subq %r9, %rdi
  addl $1000, %r10d
  addq %rdi, %r10
  movl main.sums+4036(%rsi,%rsi), %edi
  movswl main.mns+2018(%rsi), %r8d
  movswl main.mxs+2018(%rsi), %r9d
  imulq $961, %r10, %r10
  movq %rdi, %r11
  shlq $5, %r11
  subq %rdi, %r11
  addl $1000, %r8d
  addq %r11, %r8
  addq %r10, %r8
  movq %r8, %rdi
  shlq $5, %rdi
  subq %r8, %rdi
  addl $1000, %r9d
  addq %rdi, %r9
  movl main.sums+4040(%rsi,%rsi), %edi
  movswl main.mns+2020(%rsi), %r8d
  movswl main.mxs+2020(%rsi), %r10d
  imulq $961, %r9, %r9
  movq %rdi, %r11
  shlq $5, %r11
  subq %rdi, %r11
  addl $1000, %r8d
  addq %r11, %r8
  addq %r9, %r8
  movq %r8, %rdi
  shlq $5, %rdi
  subq %r8, %rdi
  addl $1000, %r10d
  addq %rdi, %r10
  movl main.sums+4044(%rsi,%rsi), %r8d
  movswl main.mns+2022(%rsi), %r9d
  movswl main.mxs+2022(%rsi), %edi
  imulq $961, %r10, %r10
  movq %r8, %r11
  shlq $5, %r11
  subq %r8, %r11
  addl $1000, %r9d
  addq %r11, %r9
  addq %r10, %r9
  movq %r9, %r8
  shlq $5, %r8
  subq %r9, %r8
  addl $1000, %edi
  addq %r8, %rdi
  addq $8, %rsi
  jne .LBB0_5
  imulq $961, %rdi, %rsi
  movl %edx, %edx
  movq %rdx, %rdi
  shlq $5, %rdi
  subq %rdx, %rdi
  movswl %cx, %ecx
  addl $1000, %ecx
  addq %rdi, %rcx
  addq %rsi, %rcx
  movq %rcx, %rdx
  shlq $5, %rdx
  subq %rcx, %rdx
  movswl %ax, %esi
  addl $1000, %esi
  addq %rdx, %rsi
  movl $.L.str, %edi
  xorl %eax, %eax
  vzeroupper
  callq printf
  xorl %eax, %eax
  popq %rcx
  retq

.L.str:

