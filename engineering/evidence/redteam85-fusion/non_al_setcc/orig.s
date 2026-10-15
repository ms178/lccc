.text
.globl orig
.type orig,@function
orig:
.cfi_startproc
movl $0, %eax
cmpl %esi, %edi
setb %cl
movzbl %al, %eax
movq %rax, -8(%rsp)
movsbq -8(%rsp), %rax
testq %rax, %rax
je .Lzero
movl $1, %eax
ret
.Lzero:
movl $2, %eax
ret
.cfi_endproc
.section .note.GNU-stack,"",@progbits
