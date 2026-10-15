.text
.globl transformed
.type transformed,@function
transformed:
.cfi_startproc
movl $9, %eax
cmpl %esi, %edi





    jae .Lzero
ret
.Lzero:
ret
.cfi_endproc
.section .note.GNU-stack,"",@progbits
