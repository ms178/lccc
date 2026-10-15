.text
.globl orig
.type orig,@function
orig:
.cfi_startproc
movl $9, %eax
cmpl %esi, %edi
setb %al
movzbl %al, %eax
movq %rax, -8(%rsp)
movsbq -8(%rsp), %rax
testq %rax, %rax
je .Lzero
movl $0, %eax
setbe %al
ret
.Lzero:
movl $0, %eax
setbe %al
ret
.cfi_endproc
.section .note.GNU-stack,"",@progbits
