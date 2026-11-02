.text
.globl orig
.type orig,@function
orig:
.cfi_startproc
movq $7, -8(%rsp)
movl $9, %eax
cmpl %esi, %edi
setb %al
movzbl %al, %eax
movq %rax, -8(%rsp)
movsbq -8(%rsp), %rax
testq %rax, %rax
je .Lzero
movq -8(%rsp), %rax
ret
.Lzero:
movq -8(%rsp), %rax
ret
.cfi_endproc
.section .note.GNU-stack,"",@progbits
