.LCPI0_1:
  .long 2147975281
.LCPI0_2:
  .long 65521
.LCPI0_3:
  .byte 2
  .byte 6
  .byte 10
  .byte 14
main:
  pushq %rbx
  subq $16, %rsp
  vstmxcsr 12(%rsp)
  orl $32832, 12(%rsp)
  vldmxcsr 12(%rsp)
  movl $-1640531527, %ecx
  movq $-2097152, %rax
  vpbroadcastd .LCPI0_3(%rip), %xmm0
.LBB0_1:
  imull $1103515245, %ecx, %ecx
  addl $12345, %ecx
  imull $1103515245, %ecx, %edx
  addl $12345, %edx
  imull $1103515245, %edx, %esi
  addl $12345, %esi
  imull $1103515245, %esi, %edi
  addl $12345, %edi
  vmovd %ecx, %xmm1
  vpinsrd $1, %edx, %xmm1, %xmm1
  vpinsrd $2, %esi, %xmm1, %xmm1
  vpinsrd $3, %edi, %xmm1, %xmm1
  vpshufb %xmm0, %xmm1, %xmm1
  vmovd %xmm1, zlib_ng_adler_data+2097152(%rax)
  imull $1103515245, %edi, %edx
  addl $12345, %edx
  imull $1103515245, %edx, %esi
  addl $12345, %esi
  imull $1103515245, %esi, %edi
  addl $12345, %edi
  imull $1103515245, %edi, %ecx
  addl $12345, %ecx
  vmovd %edx, %xmm1
  vpinsrd $1, %esi, %xmm1, %xmm1
  vpinsrd $2, %edi, %xmm1, %xmm1
  vpinsrd $3, %ecx, %xmm1, %xmm1
  vpshufb %xmm0, %xmm1, %xmm1
  vmovd %xmm1, zlib_ng_adler_data+2097156(%rax)
  addq $8, %rax
  jne .LBB0_1
  xorl %ecx, %ecx
  movl $2147975281, %eax
  vpbroadcastd .LCPI0_1(%rip), %xmm0
  vpbroadcastd .LCPI0_2(%rip), %xmm1
  xorl %esi, %esi
.LBB0_3:
  leaq 1(%rcx), %rdx
  movl $zlib_ng_adler_data, %r8d
  movl $2097152, %r9d
  xorl %edi, %edi
  movl %edx, %r10d
.LBB0_4:
  xorl %r11d, %r11d
.LBB0_5:
  movzbl (%r8,%r11,8), %ebx
  addl %r10d, %ebx
  addl %ebx, %edi
  movzbl 1(%r8,%r11,8), %r10d
  addl %ebx, %r10d
  addl %r10d, %edi
  movzbl 2(%r8,%r11,8), %ebx
  addl %r10d, %ebx
  addl %ebx, %edi
  movzbl 3(%r8,%r11,8), %r10d
  addl %ebx, %r10d
  addl %r10d, %edi
  movzbl 4(%r8,%r11,8), %ebx
  addl %r10d, %ebx
  addl %ebx, %edi
  movzbl 5(%r8,%r11,8), %r10d
  addl %ebx, %r10d
  addl %r10d, %edi
  movzbl 6(%r8,%r11,8), %ebx
  addl %r10d, %ebx
  addl %ebx, %edi
  movzbl 7(%r8,%r11,8), %r10d
  addl %ebx, %r10d
  addl %r10d, %edi
  incq %r11
  cmpl $694, %r11d
  jne .LBB0_5
  addq $-5552, %r9
  addq $5552, %r8
  movl %r10d, %r11d
  imulq %rax, %r11
  shrq $47, %r11
  imull $65521, %r11d, %r11d
  subl %r11d, %r10d
  movl %edi, %r11d
  imulq %rax, %r11
  shrq $47, %r11
  imull $65521, %r11d, %r11d
  subl %r11d, %edi
  cmpq $5551, %r9
  ja .LBB0_4
  movq $-4048, %r8
.LBB0_8:
  movzbl zlib_ng_adler_data+2097152(%r8), %r9d
  addl %r10d, %r9d
  addl %r9d, %edi
  movzbl zlib_ng_adler_data+2097153(%r8), %r10d
  addl %r9d, %r10d
  addl %r10d, %edi
  movzbl zlib_ng_adler_data+2097154(%r8), %r9d
  addl %r10d, %r9d
  addl %r9d, %edi
  movzbl zlib_ng_adler_data+2097155(%r8), %r10d
  addl %r9d, %r10d
  addl %r10d, %edi
  movzbl zlib_ng_adler_data+2097156(%r8), %r9d
  addl %r10d, %r9d
  addl %r9d, %edi
  movzbl zlib_ng_adler_data+2097157(%r8), %r10d
  addl %r9d, %r10d
  addl %r10d, %edi
  movzbl zlib_ng_adler_data+2097158(%r8), %r9d
  addl %r10d, %r9d
  addl %r9d, %edi
  movzbl zlib_ng_adler_data+2097159(%r8), %r10d
  addl %r9d, %r10d
  addl %r10d, %edi
  addq $8, %r8
  jne .LBB0_8
  vmovd %edi, %xmm2
  vpinsrd $1, %r10d, %xmm2, %xmm3
  vmovd %r10d, %xmm4
  vpbroadcastd %xmm4, %xmm4
  vpmuludq %xmm0, %xmm4, %xmm4
  vpmuludq %xmm0, %xmm2, %xmm2
  vpshufd $245, %xmm2, %xmm2
  vpblendd $10, %xmm4, %xmm2, %xmm2
  vpsrld $15, %xmm2, %xmm2
  vpmulld %xmm1, %xmm2, %xmm2
  vpsubd %xmm2, %xmm3, %xmm2
  vmovd %xmm2, %edi
  shll $16, %edi
  vpextrd $1, %xmm2, %r8d
  orl %edi, %r8d
  leal (%rcx,%r8), %edi
  xorl %edi, %esi
  shrl $9, %r8d
  imulq $12289, %rcx, %rcx
  xorb %r8b, zlib_ng_adler_data(%rcx)
  movq %rdx, %rcx
  cmpq $48, %rdx
  jne .LBB0_3
  movl $.L.str, %edi
  xorl %eax, %eax
  callq printf
  xorl %eax, %eax
  addq $16, %rsp
  popq %rbx
  retq

.L.str:
  .asciz "%08x\n"

