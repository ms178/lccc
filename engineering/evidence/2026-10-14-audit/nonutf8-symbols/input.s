.text
.globl _start
_start:
 call symA
 mov %eax, %ebx
 call symB
 add %ebx, %eax
 sub $3, %eax
 mov %eax, %edi
 mov $60, %eax
 syscall
.globl symA
symA:
 mov $1, %eax
 ret
.globl symB
symB:
 mov $2, %eax
 ret
.section .note.GNU-stack,"",@progbits
