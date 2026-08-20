main:
  movl $42, %eax
  movl $4, %ecx
  leaq arr(%rip), %rdx
.LBB0_1:
  imull $1664525, %eax, %esi
  addl $1013904223, %esi
  andl $2147483647, %esi
  movl %esi, -16(%rdx,%rcx,4)
  imull $389569705, %eax, %esi
  addl $1196435762, %esi
  andl $2147483647, %esi
  movl %esi, -12(%rdx,%rcx,4)
  imull $-1354167659, %eax, %esi
  addl $-775096599, %esi
  andl $2147483647, %esi
  movl %esi, -8(%rdx,%rcx,4)
  imull $158984081, %eax, %esi
  addl $-1426500812, %esi
  andl $2147483647, %esi
  movl %esi, -4(%rdx,%rcx,4)
  imull $-1432516515, %eax, %eax
  addl $1649599747, %eax
  movl %eax, %esi
  andl $2147483647, %esi
  movl %esi, (%rdx,%rcx,4)
  addq $5, %rcx
  cmpq $1000004, %rcx
  jne .LBB0_1
  pushq %rax
  leaq arr(%rip), %rdi
  leaq cmp(%rip), %rcx
  movl $1000000, %esi
  movl $4, %edx
  callq qsort@PLT
  movl arr+2000000(%rip), %esi
  leaq .L.str(%rip), %rdi
  xorl %eax, %eax
  callq printf@PLT
  xorl %eax, %eax
  popq %rcx
  retq

cmp:
  movl (%rdi), %eax
  subl (%rsi), %eax
  retq

.L.str:
  .asciz "qsort: arr[500000] = %d\n"

