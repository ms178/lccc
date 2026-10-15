.text
.globl transformed
.type transformed,@function
transformed:
.cfi_startproc
movl $9, %eax
cmpl %esi, %edi





    jae .Lzero
movl $1, %eax
ret
.Lzero:
movl $2, %eax
ret
.cfi_endproc
.section .note.GNU-stack,"",@progbits
