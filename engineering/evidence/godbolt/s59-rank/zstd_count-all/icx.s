main:
  pushq %rbp
  pushq %r15
  pushq %r14
  pushq %rbx
  pushq %rax
  vstmxcsr 4(%rsp)
  orl $32832, 4(%rsp)
  vldmxcsr 4(%rsp)
  movl $-2058980567, %ecx
  movl $3, %eax
  jmp .LBB0_1
.LBB0_32:
  movl %ecx, %edx
  shrl $24, %edx
.LBB0_33:
  movb %dl, buffer(%rax)
  addq $4, %rax
  cmpq $1048579, %rax
  je .LBB0_34
.LBB0_1:
  imull $1664525, %ecx, %ecx
  addl $1013904223, %ecx
  testb $7, %cl
  jne .LBB0_20
  leaq -3(%rax), %rdx
  cmpq $64, %rdx
  jb .LBB0_20
  movl %ecx, %edx
  shrl $28, %edx
  addl %eax, %edx
  addl $-67, %edx
  movzbl buffer(%rdx), %edx
  jmp .LBB0_21
.LBB0_20:
  movl %ecx, %edx
  shrl $24, %edx
.LBB0_21:
  movb %dl, buffer-3(%rax)
  imull $1664525, %ecx, %ecx
  addl $1013904223, %ecx
  testb $7, %cl
  jne .LBB0_24
  leaq -2(%rax), %rdx
  cmpq $64, %rdx
  jb .LBB0_24
  movl %ecx, %edx
  shrl $28, %edx
  addl %eax, %edx
  addl $-66, %edx
  movzbl buffer(%rdx), %edx
  jmp .LBB0_25
.LBB0_24:
  movl %ecx, %edx
  shrl $24, %edx
.LBB0_25:
  movb %dl, buffer-2(%rax)
  imull $1664525, %ecx, %ecx
  addl $1013904223, %ecx
  testb $7, %cl
  jne .LBB0_28
  leaq -1(%rax), %rdx
  cmpq $64, %rdx
  jb .LBB0_28
  movl %ecx, %edx
  shrl $28, %edx
  addl %eax, %edx
  addl $-65, %edx
  movzbl buffer(%rdx), %edx
  jmp .LBB0_29
.LBB0_28:
  movl %ecx, %edx
  shrl $24, %edx
.LBB0_29:
  movb %dl, buffer-1(%rax)
  imull $1664525, %ecx, %ecx
  addl $1013904223, %ecx
  testb $7, %cl
  jne .LBB0_32
  cmpq $64, %rax
  jb .LBB0_32
  movl %ecx, %edx
  shrl $28, %edx
  addl %eax, %edx
  addl $-64, %edx
  movzbl buffer(%rdx), %edx
  jmp .LBB0_33
.LBB0_34:
  movl $324508639, %eax
  xorl %ecx, %ecx
  xorl %esi, %esi
  jmp .LBB0_4
.LBB0_18:
  incl %ecx
  cmpl $16, %ecx
  je .LBB0_19
.LBB0_4:
  xorl %edx, %edx
  jmp .LBB0_5
.LBB0_14:
  xorl %r9d, %r9d
  tzcntq %r15, %r9
  shrl $3, %r9d
  addq %rbx, %r9
.LBB0_17:
  subl %r8d, %r9d
  leal (,%rdx,8), %r8d
  shlxq %r8, %r9, %r8
  xorq %rdi, %r8
  addq %r8, %rsi
  incl %edx
  cmpl $131072, %edx
  je .LBB0_18
.LBB0_5:
  imull $1664525, %eax, %eax
  addl $1013904223, %eax
  movl %eax, %edi
  andl $1048319, %edi
  movl %eax, %r9d
  shrl $12, %r9d
  andl $1048319, %r9d
  movl %eax, %r11d
  andl $127, %r11d
  leaq buffer(%rdi), %r8
  leaq buffer(%r9), %r10
  leaq (%r11,%rdi), %r14
  addq $buffer+9, %r14
  cmpq %r8, %r14
  jbe .LBB0_6
  leaq buffer(%rdi), %rbx
  movq %r8, %r9
.LBB0_13:
  movq (%r9), %r15
  xorq (%r10), %r15
  jne .LBB0_14
  addq $8, %r9
  addq $8, %r10
  addq $8, %rbx
  cmpq %r14, %r9
  jb .LBB0_13
  jmp .LBB0_7
.LBB0_6:
  movq %r8, %r9
.LBB0_7:
  addq %rdi, %r11
  addq $buffer+16, %r11
  cmpq %r11, %r9
  jae .LBB0_17
  movq %r11, %rbx
  subq %r9, %rbx
  xorl %r14d, %r14d
.LBB0_9:
  movzbl (%r9,%r14), %ebp
  cmpb (%r10,%r14), %bpl
  jne .LBB0_16
  incq %r14
  cmpq %r14, %rbx
  jne .LBB0_9
  movq %r11, %r9
  jmp .LBB0_17
.LBB0_16:
  addq %r14, %r9
  jmp .LBB0_17
.LBB0_19:
  movl $.L.str, %edi
  xorl %eax, %eax
  callq printf
  xorl %eax, %eax
  addq $8, %rsp
  popq %rbx
  popq %r14
  popq %r15
  popq %rbp
  retq

.L.str:

