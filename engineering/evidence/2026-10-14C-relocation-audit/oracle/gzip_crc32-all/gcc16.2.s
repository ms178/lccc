.LC0:
main:
  movl $1, %edx
  movl $2082672712, %eax
.L2:
  movzbl check.0(%rdx), %ecx
  addq $1, %rdx
  xorl %eax, %ecx
  shrl $8, %eax
  movzbl %cl, %ecx
  xorl gzip_crc32_table(,%rcx,4), %eax
  cmpq $9, %rdx
  jne .L2
  movl $2, %edx
  cmpl $873187033, %eax
  je .L17
  movl %edx, %eax
  ret
.L17:
  pushq %rcx
  xorl %edx, %edx
  movl $305419896, %eax
.L4:
  imull $1664525, %eax, %eax
  addq $1, %rdx
  addl $1013904223, %eax
  movl %eax, %ecx
  shrl $24, %ecx
  movb %cl, gzip_crc_data-1(%rdx)
  cmpq $1048576, %rdx
  jne .L4
  movl $gzip_crc_data, %ecx
  movl $gzip_crc_data+524224, %edi
  xorl %esi, %esi
.L6:
  notl %esi
  xorl %eax, %eax
.L5:
  movzbl gzip_crc_data(%rax), %edx
  addq $1, %rax
  xorl %esi, %edx
  shrl $8, %esi
  movzbl %dl, %edx
  xorl gzip_crc32_table(,%rdx,4), %esi
  cmpq $1048576, %rax
  jne .L5
  notl %esi
  xorb %sil, (%rcx)
  addq $8191, %rcx
  cmpq %rcx, %rdi
  jne .L6
  movl $.LC0, %edi
  xorl %eax, %eax
  call printf
  xorl %eax, %eax
  popq %rdx
  ret
check.0:
gzip_crc32_table:
