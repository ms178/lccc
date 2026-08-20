.LCPI0_0:
  .long 3294967296
.LCPI0_1:
  .long 7
.LCPI0_2:
  .long 1374389535
.LCPI0_3:
  .long 100
main:
  subq $24, %rsp
  vstmxcsr 20(%rsp)
  orl $32832, 20(%rsp)
  vldmxcsr 20(%rsp)
  movl $42, %ecx
  xorl %eax, %eax
  vpbroadcastd .LCPI0_0(%rip), %ymm0
.LBB0_1:
  imull $1664525, %ecx, %edx
  addl $1013904223, %edx
  imull $1664525, %edx, %esi
  addl $1013904223, %esi
  imull $1664525, %esi, %edi
  addl $1013904223, %edi
  imull $1664525, %edi, %r8d
  addl $1013904223, %r8d
  imull $1664525, %r8d, %r9d
  addl $1013904223, %r9d
  imull $1664525, %r9d, %r10d
  addl $1013904223, %r10d
  imull $1664525, %r10d, %r11d
  addl $1013904223, %r11d
  imull $1664525, %r11d, %ecx
  vmovd %edx, %xmm1
  vpinsrd $1, %esi, %xmm1, %xmm1
  vpinsrd $2, %edi, %xmm1, %xmm1
  vpinsrd $3, %r8d, %xmm1, %xmm1
  addl $1013904223, %ecx
  vmovd %r9d, %xmm2
  vpinsrd $1, %r10d, %xmm2, %xmm2
  vpinsrd $2, %r11d, %xmm2, %xmm2
  vpinsrd $3, %ecx, %xmm2, %xmm2
  vinserti128 $1, %xmm2, %ymm1, %ymm1
  vpsrld $1, %ymm1, %ymm1
  vpaddd %ymm0, %ymm1, %ymm1
  vmovdqu %ymm1, array(%rax)
  addq $32, %rax
  cmpq $40000000, %rax
  jne .LBB0_1
  vpxor %xmm6, %xmm6, %xmm6
  movq $-8, %rax
  vpbroadcastd .LCPI0_1(%rip), %ymm7
  vpxor %xmm0, %xmm0, %xmm0
  vpxor %xmm1, %xmm1, %xmm1
  vpxor %xmm2, %xmm2, %xmm2
  vpxor %xmm4, %xmm4, %xmm4
  vpxor %xmm3, %xmm3, %xmm3
  vpxor %xmm5, %xmm5, %xmm5
.LBB0_3:
  vmovdqu array+32(,%rax,4), %ymm8
  vpmovsxdq array+48(,%rax,4), %ymm9
  vpaddq %ymm1, %ymm9, %ymm1
  vpmovsxdq array+32(,%rax,4), %ymm9
  vpaddq %ymm0, %ymm9, %ymm0
  vpmaxsd %ymm6, %ymm8, %ymm9
  vextracti128 $1, %ymm9, %xmm10
  vpmovzxdq %xmm10, %ymm10
  vpaddq %ymm4, %ymm10, %ymm4
  vpmovzxdq %xmm9, %ymm9
  vpaddq %ymm2, %ymm9, %ymm2
  vpaddd %ymm8, %ymm8, %ymm9
  vpaddd %ymm7, %ymm8, %ymm8
  vpaddd %ymm9, %ymm8, %ymm8
  vmovdqu %ymm8, main.buf+32(,%rax,4)
  vextracti128 $1, %ymm8, %xmm9
  vpmovsxdq %xmm9, %ymm9
  vpaddq %ymm5, %ymm9, %ymm5
  vpmovsxdq %xmm8, %ymm8
  vpaddq %ymm3, %ymm8, %ymm3
  vmovdqu array+64(,%rax,4), %ymm8
  vpmovsxdq array+64(,%rax,4), %ymm9
  vpmovsxdq array+80(,%rax,4), %ymm10
  vpaddq %ymm0, %ymm9, %ymm0
  vpaddq %ymm1, %ymm10, %ymm1
  vpmaxsd %ymm6, %ymm8, %ymm9
  vpmovzxdq %xmm9, %ymm10
  vpaddq %ymm2, %ymm10, %ymm2
  vextracti128 $1, %ymm9, %xmm9
  vpmovzxdq %xmm9, %ymm9
  vpaddd %ymm8, %ymm8, %ymm10
  vpaddd %ymm7, %ymm8, %ymm8
  vpaddd %ymm10, %ymm8, %ymm8
  vmovdqu %ymm8, main.buf+64(,%rax,4)
  vpaddq %ymm4, %ymm9, %ymm4
  vpmovsxdq %xmm8, %ymm9
  vpaddq %ymm3, %ymm9, %ymm3
  vextracti128 $1, %ymm8, %xmm8
  vpmovsxdq %xmm8, %ymm8
  vpaddq %ymm5, %ymm8, %ymm5
  addq $16, %rax
  cmpq $9999992, %rax
  jb .LBB0_3
  movl $48, %eax
  vpbroadcastd array(%rip), %ymm6
.LBB0_5:
  vpmaxsd array-188(,%rax,4), %ymm6, %ymm6
  vpmaxsd array-156(,%rax,4), %ymm6, %ymm6
  vpmaxsd array-124(,%rax,4), %ymm6, %ymm6
  vpmaxsd array-92(,%rax,4), %ymm6, %ymm6
  vpmaxsd array-60(,%rax,4), %ymm6, %ymm6
  vpmaxsd array-28(,%rax,4), %ymm6, %ymm6
  vpmaxsd array+4(,%rax,4), %ymm6, %ymm6
  cmpq $9999983, %rax
  ja .LBB0_7
  vpmaxsd array+36(,%rax,4), %ymm6, %ymm6
  addq $64, %rax
  jmp .LBB0_5
.LBB0_7:
  vextracti128 $1, %ymm6, %xmm7
  vpmaxsd %xmm7, %xmm6, %xmm6
  vpshufd $238, %xmm6, %xmm7
  vpmaxsd %xmm7, %xmm6, %xmm6
  vpshufd $85, %xmm6, %xmm7
  vpbroadcastd %xmm6, %xmm6
  vpmaxsd %xmm7, %xmm6, %xmm6
  movl $39999972, %eax
  vpmaxsd array(%rax), %xmm6, %xmm6
  vpshufd $238, %xmm6, %xmm7
  vpmaxsd %xmm7, %xmm6, %xmm6
  vpshufd $85, %xmm6, %xmm7
  vpmaxsd %xmm7, %xmm6, %xmm6
  vmovd %xmm6, %ecx
  xorl %eax, %eax
  movl $39999988, %edx
.LBB0_8:
  movl array(%rdx,%rax,4), %esi
  cmpl %ecx, %esi
  cmovgl %esi, %ecx
  incq %rax
  cmpq $3, %rax
  jne .LBB0_8
  vpxor %xmm6, %xmm6, %xmm6
  movq $-4, %rax
.LBB0_10:
  vpmovzxdq array+16(,%rax,4), %ymm7
  vpmovzxdq main.buf+16(,%rax,4), %ymm8
  vpmuldq %ymm7, %ymm8, %ymm7
  vpaddq %ymm6, %ymm7, %ymm6
  vpmovzxdq array+32(,%rax,4), %ymm7
  vpmovzxdq main.buf+32(,%rax,4), %ymm8
  vpmuldq %ymm7, %ymm8, %ymm7
  vpaddq %ymm6, %ymm7, %ymm6
  addq $8, %rax
  cmpq $999996, %rax
  jb .LBB0_10
  movl $42, %edx
  movq $-40000, %rax
  vpbroadcastd .LCPI0_2(%rip), %ymm7
  vpbroadcastd .LCPI0_3(%rip), %ymm8
.LBB0_12:
  imull $1664525, %edx, %edx
  addl $1013904223, %edx
  imull $1664525, %edx, %esi
  addl $1013904223, %esi
  imull $1664525, %esi, %edi
  addl $1013904223, %edi
  imull $1664525, %edi, %r8d
  addl $1013904223, %r8d
  imull $1664525, %r8d, %r9d
  addl $1013904223, %r9d
  imull $1664525, %r9d, %r10d
  addl $1013904223, %r10d
  imull $1664525, %r10d, %r11d
  addl $1013904223, %r11d
  vmovd %edx, %xmm9
  vpinsrd $1, %esi, %xmm9, %xmm9
  vpinsrd $2, %edi, %xmm9, %xmm9
  imull $1664525, %r11d, %edx
  vpinsrd $3, %r8d, %xmm9, %xmm9
  vmovd %r9d, %xmm10
  vpinsrd $1, %r10d, %xmm10, %xmm10
  vpinsrd $2, %r11d, %xmm10, %xmm10
  addl $1013904223, %edx
  vpinsrd $3, %edx, %xmm10, %xmm10
  vinserti128 $1, %xmm10, %ymm9, %ymm9
  vpshufd $245, %ymm9, %ymm10
  vpmuludq %ymm7, %ymm10, %ymm10
  vpmuludq %ymm7, %ymm9, %ymm11
  vpshufd $245, %ymm11, %ymm11
  vpblendd $170, %ymm10, %ymm11, %ymm10
  vpsrld $5, %ymm10, %ymm10
  vpmulld %ymm8, %ymm10, %ymm10
  vpsubd %ymm10, %ymm9, %ymm9
  vmovdqu %ymm9, array+40000(%rax)
  addq $32, %rax
  jne .LBB0_12
  movq $-39968, %rax
  movl array(%rip), %edx
.LBB0_14:
  addl array+39972(%rax), %edx
  movl %edx, array+39972(%rax)
  addl array+39976(%rax), %edx
  movl %edx, array+39976(%rax)
  addl array+39980(%rax), %edx
  movl %edx, array+39980(%rax)
  addl array+39984(%rax), %edx
  movl %edx, array+39984(%rax)
  addl array+39988(%rax), %edx
  movl %edx, array+39988(%rax)
  addl array+39992(%rax), %edx
  movl %edx, array+39992(%rax)
  addl array+39996(%rax), %edx
  movl %edx, array+39996(%rax)
  addl array+40000(%rax), %edx
  movl %edx, array+40000(%rax)
  addq $32, %rax
  jne .LBB0_14
  movl array+39968(%rip), %eax
  addl array+39972(%rip), %eax
  movl %eax, array+39972(%rip)
  addl array+39976(%rip), %eax
  movl %eax, array+39976(%rip)
  addl array+39980(%rip), %eax
  movl %eax, array+39980(%rip)
  addl array+39984(%rip), %eax
  movl %eax, array+39984(%rip)
  addl array+39988(%rip), %eax
  movl %eax, array+39988(%rip)
  addl array+39992(%rip), %eax
  movl %eax, array+39992(%rip)
  addl array+39996(%rip), %eax
  movl %eax, array+39996(%rip)
  vextracti128 $1, %ymm6, %xmm7
  vpaddq %xmm7, %xmm6, %xmm6
  vpshufd $238, %xmm6, %xmm7
  vpaddq %xmm7, %xmm6, %xmm6
  vmovq %xmm6, %r9
  vpaddq %ymm5, %ymm3, %ymm3
  vextracti128 $1, %ymm3, %xmm5
  vpaddq %xmm5, %xmm3, %xmm3
  vpshufd $238, %xmm3, %xmm5
  vpaddq %xmm5, %xmm3, %xmm3
  vmovq %xmm3, %r8
  vpaddq %ymm4, %ymm2, %ymm2
  vextracti128 $1, %ymm2, %xmm3
  vpaddq %xmm3, %xmm2, %xmm2
  vpshufd $238, %xmm2, %xmm3
  vpaddq %xmm3, %xmm2, %xmm2
  vmovq %xmm2, %rdx
  vpaddq %ymm1, %ymm0, %ymm0
  vextracti128 $1, %ymm0, %xmm1
  vpaddq %xmm1, %xmm0, %xmm0
  vpshufd $238, %xmm0, %xmm1
  vpaddq %xmm1, %xmm0, %xmm0
  vmovq %xmm0, %rsi
  movl %eax, (%rsp)
  movl $.L.str, %edi
  xorl %eax, %eax
  vzeroupper
  callq printf
  xorl %eax, %eax
  addq $24, %rsp
  retq

.L.str:
  .asciz "sum=%ld pos=%ld max=%d scaled=%ld dot=%ld prefix=%d\n"

