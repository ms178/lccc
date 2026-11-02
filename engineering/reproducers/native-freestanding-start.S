.text
.globl _start
_start:
call main
mov %eax,%edi
mov $60,%eax
syscall
.section .note.GNU-stack,"",%progbits
