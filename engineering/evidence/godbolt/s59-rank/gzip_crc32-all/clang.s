main:
  movl $305419896, %ecx
  xorl %edx, %edx
  leaq gzip_crc_data(%rip), %rax
.LBB0_1:
  imull $1664525, %ecx, %esi
  addl $1013904223, %esi
  shrl $24, %esi
  movb %sil, (%rdx,%rax)
  imull $389569705, %ecx, %esi
  addl $1196435762, %esi
  shrl $24, %esi
  movb %sil, 1(%rdx,%rax)
  imull $-1354167659, %ecx, %esi
  addl $-775096599, %esi
  shrl $24, %esi
  movb %sil, 2(%rdx,%rax)
  imull $158984081, %ecx, %esi
  addl $-1426500812, %esi
  shrl $24, %esi
  movb %sil, 3(%rdx,%rax)
  imull $-1432516515, %ecx, %esi
  addl $1649599747, %esi
  shrl $24, %esi
  movb %sil, 4(%rdx,%rax)
  imull $-1083573575, %ecx, %esi
  addl $-1624324474, %esi
  shrl $24, %esi
  movb %sil, 5(%rdx,%rax)
  imull $1851289957, %ecx, %esi
  addl $1476291629, %esi
  shrl $24, %esi
  movb %sil, 6(%rdx,%rax)
  imull $-360120287, %ecx, %ecx
  addl $-1546035288, %ecx
  movl %ecx, %esi
  shrl $24, %esi
  movb %sil, 7(%rdx,%rax)
  addq $8, %rdx
  cmpq $1048576, %rdx
  jne .LBB0_1
  movl $-1, %ecx
  xorl %edx, %edx
  leaq gzip_crc32_table(%rip), %rdi
.LBB0_3:
  xorl %esi, %esi
.LBB0_4:
  movzbl (%rsi,%rax), %r8d
  xorb %cl, %r8b
  movzbl %r8b, %r8d
  shrl $8, %ecx
  xorl (%rdi,%r8,4), %ecx
  movzbl 1(%rsi,%rax), %r8d
  xorb %cl, %r8b
  movzbl %r8b, %r8d
  shrl $8, %ecx
  xorl (%rdi,%r8,4), %ecx
  movzbl 2(%rsi,%rax), %r8d
  xorb %cl, %r8b
  movzbl %r8b, %r8d
  shrl $8, %ecx
  xorl (%rdi,%r8,4), %ecx
  movzbl 3(%rsi,%rax), %r8d
  xorb %cl, %r8b
  movzbl %r8b, %r8d
  shrl $8, %ecx
  xorl (%rdi,%r8,4), %ecx
  addq $4, %rsi
  cmpq $1048576, %rsi
  jne .LBB0_4
  movl %ecx, %esi
  notl %esi
  movq %rdx, %r8
  shlq $13, %r8
  subq %rdx, %r8
  xorb %sil, (%r8,%rax)
  incq %rdx
  cmpq $64, %rdx
  jne .LBB0_3
  pushq %rax
  leaq .L.str(%rip), %rdi
  xorl %eax, %eax
  callq printf@PLT
  xorl %eax, %eax
  popq %rcx
  retq

.L.str:

gzip_crc32_table:

