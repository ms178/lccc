# Actual failing copy-commute shape: the removed move feeds implicit RAX.
.text
.globl original_gp_product
.type original_gp_product,@function
original_gp_product:
    xorl %eax,%eax
    movq %rdi,%rax
    addq %rsi,%rax
    mulq %r8
    movq %rdx,%rax
    ret
.size original_gp_product,.-original_gp_product
.globl rewritten_gp_product
.type rewritten_gp_product,@function
rewritten_gp_product:
    xorl %eax,%eax
    addq %rdi,%rsi
    mulq %r8
    movq %rdx,%rax
    ret
.size rewritten_gp_product,.-rewritten_gp_product
.section .note.GNU-stack,"",@progbits
