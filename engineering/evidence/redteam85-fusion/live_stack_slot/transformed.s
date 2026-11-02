.text
.globl transformed
.type transformed,@function
transformed:
.cfi_startproc
movq $7, -8(%rsp)
movl $9, %eax
cmpl %esi, %edi





    jae .Lzero
movq -8(%rsp), %rax
ret
.Lzero:
movq -8(%rsp), %rax
ret
.cfi_endproc
.section .note.GNU-stack,"",@progbits
