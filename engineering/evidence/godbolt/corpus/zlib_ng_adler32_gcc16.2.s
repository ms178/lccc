.LC0:
  .string "%08x\n"
main:
  xorl %eax, %eax
  movl $1, %edx
  movl $check.0, %ecx
.L2:
  movzbl (%rcx), %esi
  addq $1, %rcx
  addl %esi, %edx
  addl %edx, %eax
  cmpq $check.0+9, %rcx
  jne .L2
  movl %eax, %ecx
  movl $2147975281, %esi
  imulq %rsi, %rcx
  shrq $47, %rcx
  imull $65521, %ecx, %ecx
  subl %ecx, %eax
  movl %edx, %ecx
  imulq %rsi, %rcx
  sall $16, %eax
  shrq $47, %rcx
  imull $65521, %ecx, %ecx
  subl %ecx, %edx
  orl %edx, %eax
  movl $2, %edx
  cmpl $152961502, %eax
  je .L21
  movl %edx, %eax
  ret
.L21:
  pushq %r15
  xorl %edx, %edx
  movl $-1640531527, %eax
  pushq %r14
  pushq %r13
  pushq %r12
  pushq %rbp
  pushq %rbx
  subq $24, %rsp
.L4:
  imull $1103515245, %eax, %eax
  addq $1, %rdx
  addl $12345, %eax
  movl %eax, %ecx
  shrl $16, %ecx
  movb %cl, zlib_ng_adler_data-1(%rdx)
  cmpq $2097152, %rdx
  jne .L4
  movl $zlib_ng_adler_data, %r14d
  xorl %edx, %edx
  xorl %r13d, %r13d
  movl $2147975281, %r12d
.L8:
  movl %r13d, 12(%rsp)
  addl $1, %r13d
  movl $zlib_ng_adler_data+5552, %r15d
  xorl %ebp, %ebp
  movl %r13d, %esi
.L6:
  leaq -5552(%r15), %rcx
.L5:
  movzbl (%rcx), %eax
  movzbl 1(%rcx), %ebx
  addq $8, %rcx
  movzbl -6(%rcx), %r11d
  movzbl -5(%rcx), %r10d
  addl %esi, %eax
  movzbl -4(%rcx), %r9d
  movzbl -3(%rcx), %r8d
  addl %eax, %ebx
  movzbl -2(%rcx), %edi
  movzbl -1(%rcx), %esi
  addl %ebx, %r11d
  addl %ebx, %eax
  addl %r11d, %r10d
  addl %r11d, %eax
  addl %r10d, %r9d
  addl %r10d, %eax
  addl %r9d, %r8d
  addl %r9d, %eax
  addl %r8d, %edi
  addl %r8d, %eax
  addl %edi, %esi
  addl %edi, %eax
  addl %esi, %eax
  addl %eax, %ebp
  cmpq %r15, %rcx
  jne .L5
  movl %esi, %eax
  leaq 5552(%rcx), %r15
  imulq %r12, %rax
  shrq $47, %rax
  imull $65521, %eax, %eax
  subl %eax, %esi
  movl %ebp, %eax
  imulq %r12, %rax
  shrq $47, %rax
  imull $65521, %eax, %eax
  subl %eax, %ebp
  cmpq $zlib_ng_adler_data+2093104, %rcx
  jne .L6
  movl $zlib_ng_adler_data+2093104, %ecx
.L7:
  movzbl (%rcx), %eax
  movzbl 1(%rcx), %ebx
  addq $8, %rcx
  movzbl -6(%rcx), %r11d
  movzbl -5(%rcx), %r10d
  addl %esi, %eax
  movzbl -4(%rcx), %r9d
  movzbl -3(%rcx), %r8d
  addl %eax, %ebx
  movzbl -2(%rcx), %edi
  movzbl -1(%rcx), %esi
  addl %ebx, %r11d
  addl %ebx, %eax
  addl %r11d, %r10d
  addl %r11d, %eax
  addl %r10d, %r9d
  addl %r10d, %eax
  addl %r9d, %r8d
  addl %r9d, %eax
  addl %r8d, %edi
  addl %r8d, %eax
  addl %edi, %esi
  addl %edi, %eax
  addl %esi, %eax
  addl %eax, %ebp
  cmpq $zlib_ng_adler_data+2097152, %rcx
  jne .L7
  movl %ebp, %ecx
  movl %ebp, %eax
  imulq %r12, %rcx
  shrq $47, %rcx
  imull $65521, %ecx, %ecx
  subl %ecx, %eax
  movl %esi, %ecx
  imulq %r12, %rcx
  sall $16, %eax
  shrq $47, %rcx
  imull $65521, %ecx, %ecx
  subl %ecx, %esi
  movl 12(%rsp), %ecx
  orl %esi, %eax
  addl %eax, %ecx
  shrl $9, %eax
  xorb %al, (%r14)
  addq $12289, %r14
  xorl %ecx, %edx
  cmpl $48, %r13d
  jne .L8
  movl %edx, %esi
  movl $.LC0, %edi
  xorl %eax, %eax
  call printf
  addq $24, %rsp
  xorl %eax, %eax
  popq %rbx
  popq %rbp
  popq %r12
  popq %r13
  popq %r14
  popq %r15
  ret
check.0:
  .string "123456789"
