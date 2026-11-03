.text
.globl _start
_start: call main
mov %eax,%edi
mov $60,%eax
syscall
.globl __x86_return_thunk
__x86_return_thunk: ret
int3
.section .note.GNU-stack,"",@progbits
