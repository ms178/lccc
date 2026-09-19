.LCPI0_1:
main:
  pushq %rax
  vstmxcsr 4(%rsp)
  orl $32832, 4(%rsp)
  vldmxcsr 4(%rsp)
  movl $305419896, %ecx
  movq $-1048576, %rax
  vpbroadcastd .LCPI0_1(%rip), %xmm0
.LBB0_1:
  imull $1664525, %ecx, %ecx
  addl $1013904223, %ecx
  imull $1664525, %ecx, %edx
  addl $1013904223, %edx
  imull $1664525, %edx, %esi
  addl $1013904223, %esi
  imull $1664525, %esi, %edi
  addl $1013904223, %edi
  vmovd %ecx, %xmm1
  vpinsrd $1, %edx, %xmm1, %xmm1
  vpinsrd $2, %esi, %xmm1, %xmm1
  vpinsrd $3, %edi, %xmm1, %xmm1
  vpshufb %xmm0, %xmm1, %xmm1
  vmovd %xmm1, gzip_crc_data+1048576(%rax)
  imull $1664525, %edi, %edx
  addl $1013904223, %edx
  imull $1664525, %edx, %esi
  addl $1013904223, %esi
  imull $1664525, %esi, %edi
  addl $1013904223, %edi
  imull $1664525, %edi, %ecx
  addl $1013904223, %ecx
  vmovd %edx, %xmm1
  vpinsrd $1, %esi, %xmm1, %xmm1
  vpinsrd $2, %edi, %xmm1, %xmm1
  vpinsrd $3, %ecx, %xmm1, %xmm1
  vpshufb %xmm0, %xmm1, %xmm1
  vmovd %xmm1, gzip_crc_data+1048580(%rax)
  addq $8, %rax
  jne .LBB0_1
  movl $-1, %esi
  xorl %eax, %eax
.LBB0_3:
  movq $-1048576, %rcx
.LBB0_4:
  movzbl gzip_crc_data+1048576(%rcx), %edx
  xorb %sil, %dl
  movzbl %dl, %edx
  shrl $8, %esi
  xorl gzip_crc32_table(,%rdx,4), %esi
  movzbl gzip_crc_data+1048577(%rcx), %edx
  xorb %sil, %dl
  movzbl %dl, %edx
  shrl $8, %esi
  xorl gzip_crc32_table(,%rdx,4), %esi
  movzbl gzip_crc_data+1048578(%rcx), %edx
  xorb %sil, %dl
  movzbl %dl, %edx
  shrl $8, %esi
  xorl gzip_crc32_table(,%rdx,4), %esi
  movzbl gzip_crc_data+1048579(%rcx), %edx
  xorb %sil, %dl
  movzbl %dl, %edx
  shrl $8, %esi
  xorl gzip_crc32_table(,%rdx,4), %esi
  movzbl gzip_crc_data+1048580(%rcx), %edx
  xorb %sil, %dl
  movzbl %dl, %edx
  shrl $8, %esi
  xorl gzip_crc32_table(,%rdx,4), %esi
  movzbl gzip_crc_data+1048581(%rcx), %edx
  xorb %sil, %dl
  movzbl %dl, %edx
  shrl $8, %esi
  xorl gzip_crc32_table(,%rdx,4), %esi
  movzbl gzip_crc_data+1048582(%rcx), %edx
  xorb %sil, %dl
  movzbl %dl, %edx
  shrl $8, %esi
  xorl gzip_crc32_table(,%rdx,4), %esi
  movzbl gzip_crc_data+1048583(%rcx), %edx
  xorb %sil, %dl
  movzbl %dl, %edx
  shrl $8, %esi
  xorl gzip_crc32_table(,%rdx,4), %esi
  addq $8, %rcx
  jne .LBB0_4
  movq %rax, %rcx
  shlq $13, %rcx
  subq %rax, %rcx
  movzbl gzip_crc_data(%rcx), %edx
  xorb %sil, %dl
  notb %dl
  movb %dl, gzip_crc_data(%rcx)
  leaq 1(%rax), %rcx
  cmpq $63, %rax
  movq %rcx, %rax
  jne .LBB0_3
  notl %esi
  movl $.L.str, %edi
  xorl %eax, %eax
  callq printf
  xorl %eax, %eax
  popq %rcx
  retq

.L.str:

gzip_crc32_table:

