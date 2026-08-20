main:
  movl $-1640531527, %ecx
  xorl %edx, %edx
  leaq zlib_ng_adler_data(%rip), %rax
.LBB0_1:
  imull $1103515245, %ecx, %esi
  addl $12345, %esi
  shrl $16, %esi
  movb %sil, (%rdx,%rax)
  imull $-1029531031, %ecx, %esi
  addl $-740551042, %esi
  shrl $16, %esi
  movb %sil, 1(%rdx,%rax)
  imull $-2139243339, %ecx, %esi
  addl $-1492899873, %esi
  shrl $16, %esi
  movb %sil, 2(%rdx,%rax)
  imull $-301564143, %ecx, %esi
  addl $-698016724, %esi
  shrl $16, %esi
  movb %sil, 3(%rdx,%rax)
  imull $-341751747, %ecx, %esi
  addl $229283573, %esi
  shrl $16, %esi
  movb %sil, 4(%rdx,%rax)
  imull $-740534279, %ecx, %esi
  addl $-1038148470, %esi
  shrl $16, %esi
  movb %sil, 5(%rdx,%rax)
  imull $-1691004155, %ecx, %esi
  addl $1051550459, %esi
  shrl $16, %esi
  movb %sil, 6(%rdx,%rax)
  imull $-807543007, %ecx, %ecx
  addl $-853684456, %ecx
  movl %ecx, %esi
  shrl $16, %esi
  movb %sil, 7(%rdx,%rax)
  addq $8, %rdx
  cmpq $2097152, %rdx
  jne .LBB0_1
  pushq %rbp
  pushq %rbx
  pushq %rax
  xorl %edx, %edx
  movl $2147975281, %ecx
  xorl %esi, %esi
.LBB0_3:
  leaq 1(%rdx), %rdi
  movl $2097152, %r10d
  xorl %r8d, %r8d
  movq %rax, %r11
  movl %edi, %r9d
.LBB0_4:
  xorl %ebx, %ebx
.LBB0_5:
  movzbl (%r11,%rbx,8), %ebp
  addl %r9d, %ebp
  addl %ebp, %r8d
  movzbl 1(%r11,%rbx,8), %r9d
  addl %ebp, %r9d
  addl %r9d, %r8d
  movzbl 2(%r11,%rbx,8), %ebp
  addl %r9d, %ebp
  addl %ebp, %r8d
  movzbl 3(%r11,%rbx,8), %r9d
  addl %ebp, %r9d
  addl %r9d, %r8d
  movzbl 4(%r11,%rbx,8), %ebp
  addl %r9d, %ebp
  addl %ebp, %r8d
  movzbl 5(%r11,%rbx,8), %r9d
  addl %ebp, %r9d
  addl %r9d, %r8d
  movzbl 6(%r11,%rbx,8), %ebp
  addl %r9d, %ebp
  addl %ebp, %r8d
  movzbl 7(%r11,%rbx,8), %r9d
  addl %ebp, %r9d
  addl %r9d, %r8d
  incq %rbx
  cmpl $694, %ebx
  jne .LBB0_5
  addq $5552, %r11
  movl %r9d, %ebx
  imulq %rcx, %rbx
  shrq $47, %rbx
  imull $65521, %ebx, %ebx
  subl %ebx, %r9d
  movl %r8d, %ebx
  imulq %rcx, %rbx
  shrq $47, %rbx
  imull $65521, %ebx, %ebx
  subl %ebx, %r8d
  cmpq $11103, %r10
  leaq -5552(%r10), %r10
  jg .LBB0_4
  movq $-4048, %r10
.LBB0_8:
  movzbl 2097152(%r10,%rax), %r11d
  addl %r9d, %r11d
  addl %r11d, %r8d
  movzbl 2097153(%r10,%rax), %r9d
  addl %r11d, %r9d
  addl %r9d, %r8d
  movzbl 2097154(%r10,%rax), %r11d
  addl %r9d, %r11d
  addl %r11d, %r8d
  movzbl 2097155(%r10,%rax), %r9d
  addl %r11d, %r9d
  addl %r9d, %r8d
  movzbl 2097156(%r10,%rax), %r11d
  addl %r9d, %r11d
  addl %r11d, %r8d
  movzbl 2097157(%r10,%rax), %r9d
  addl %r11d, %r9d
  addl %r9d, %r8d
  movzbl 2097158(%r10,%rax), %r11d
  addl %r9d, %r11d
  addl %r11d, %r8d
  movzbl 2097159(%r10,%rax), %r9d
  addl %r11d, %r9d
  addl %r9d, %r8d
  addq $8, %r10
  jne .LBB0_8
  movl %r9d, %r10d
  imulq %rcx, %r10
  shrq $47, %r10
  imull $65521, %r10d, %r10d
  subl %r10d, %r9d
  movl %r8d, %r10d
  imulq %rcx, %r10
  shrq $47, %r10
  imull $65521, %r10d, %r10d
  subl %r10d, %r8d
  shll $16, %r8d
  orl %r9d, %r8d
  leal (%rdx,%r8), %r9d
  xorl %r9d, %esi
  shrl $9, %r8d
  imulq $12289, %rdx, %rdx
  xorb %r8b, (%rdx,%rax)
  movq %rdi, %rdx
  cmpq $48, %rdi
  jne .LBB0_3
  leaq .L.str(%rip), %rdi
  xorl %eax, %eax
  callq printf@PLT
  xorl %eax, %eax
  addq $8, %rsp
  popq %rbx
  popq %rbp
  retq

.L.str:
  .asciz "%08x\n"

