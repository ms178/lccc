.text
.globl transformed
.type transformed,@function
transformed:
.cfi_startproc
movl $9, %eax
cmpl %esi, %edi





    jae .Lzero
movl $0, %eax
setbe %al
ret
.Lzero:
movl $0, %eax
setbe %al
ret
.cfi_endproc
.section .note.GNU-stack,"",@progbits
