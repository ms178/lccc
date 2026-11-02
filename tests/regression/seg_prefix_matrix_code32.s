.code32
.text
.globl _start
_start:
    movl %gs:(%eax), %ecx
    movl %fs:0x30, %eax
    movl %ecx, %gs:(%eax)
    incl %gs:(%eax)
    movl %ds:8(%eax), %ebx
    movl %ss:8(%ebp), %eax
    movl %es:8(%eax), %eax
    movw %es:8(%eax), %bx
    shll $1, %gs:(%eax)
    lock incl %gs:(%eax)
    movl $7, %gs:(%eax)
    flds %gs:(%eax)
    movaps %gs:(%eax), %xmm0
