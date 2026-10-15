.text
.globl orig
.type orig,@function
orig:
.cfi_startproc
movl $9, %eax
cmpl %esi, %edi
setb %al
movzbl %al, %eax
movq %rax, (%rdx)
movsbq (%rdx), %rax
testq %rax, %rax
je .Lzero
movl $1, %eax
ret
.Lzero:
movl $2, %eax
ret
.cfi_endproc
.section .note.GNU-stack,"",@progbits
