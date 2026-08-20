cmp:
  movl (%rdi), %eax
  subl (%rsi), %eax
  ret
.LC0:
  .string "qsort: arr[500000] = %d\n"
main:
  subq $8, %rsp
  movl $arr, %edx
  movl $42, %eax
.L4:
  imull $1664525, %eax, %eax
  addq $4, %rdx
  addl $1013904223, %eax
  movl %eax, %ecx
  andl $2147483647, %ecx
  movl %ecx, -4(%rdx)
  cmpq $arr+4000000, %rdx
  jne .L4
  movl $cmp, %ecx
  movl $4, %edx
  movl $1000000, %esi
  movl $arr, %edi
  call qsort
  movl arr+2000000(%rip), %esi
  movl $.LC0, %edi
  xorl %eax, %eax
  call printf
  xorl %eax, %eax
  addq $8, %rsp
  ret
