.LC0:
  .string "switch_dispatch sum: %ld\n"
main:
  subq $8, %rsp
  xorl %ecx, %ecx
  movl $777, %eax
  xorl %edi, %edi
.L20:
  imull $1664525, %eax, %eax
  addl $1013904223, %eax
  movl %eax, %edx
  movzwl %ax, %esi
  shrl $16, %edx
  andl $15, %edx
  jmp *.L4(,%rdx,8)
.L4:
  .quad .L2
  .quad .L23
  .quad .L17
  .quad .L16
  .quad .L15
  .quad .L14
  .quad .L13
  .quad .L12
  .quad .L11
  .quad .L10
  .quad .L9
  .quad .L8
  .quad .L7
  .quad .L6
  .quad .L5
  .quad .L3
.L10:
  notl %esi
.L23:
  movl %ecx, %edx
  subl %esi, %edx
.L19:
  movslq %edx, %rdx
  addl $1, %ecx
  addq %rdx, %rdi
  cmpl $50000000, %ecx
  jne .L20
  movq %rdi, %rsi
  xorl %eax, %eax
  movl $.LC0, %edi
  call printf
  xorl %eax, %eax
  addq $8, %rsp
  ret
.L3:
  leal 1(%rsi,%rcx), %edx
  jmp .L19
.L5:
  andl %ecx, %esi
  leal 2(%rsi), %edx
  jmp .L19
.L6:
  orl %ecx, %esi
  leal -1(%rsi), %edx
  jmp .L19
.L7:
  xorl %ecx, %esi
  leal 1(%rsi), %edx
  jmp .L19
.L8:
  movl %ecx, %edx
  subl %esi, %edx
  leal (%rdx,%rdx,4), %edx
  jmp .L19
.L9:
  addl %ecx, %esi
  leal (%rsi,%rsi,2), %edx
  jmp .L19
.L11:
  movl %ecx, %edx
  notl %edx
  addl %esi, %edx
  jmp .L19
.L12:
  movl %eax, %edx
  andl $7, %edx
  sarx %edx, %ecx, %edx
  jmp .L19
.L13:
  movl %eax, %edx
  andl $7, %edx
  shlx %edx, %ecx, %edx
  jmp .L19
.L14:
  andl %ecx, %esi
  movl %esi, %edx
  jmp .L19
.L15:
  orl %ecx, %esi
  movl %esi, %edx
  jmp .L19
.L16:
  xorl %ecx, %esi
  movl %esi, %edx
  jmp .L19
.L17:
  imull %ecx, %esi
  movl %esi, %edx
  jmp .L19
.L2:
  leal (%rsi,%rcx), %edx
  jmp .L19
