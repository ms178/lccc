.text
.globl caller_upper_call
.type caller_upper_call,@function
caller_upper_call:
.cfi_startproc
    subq $8, %rsp
.cfi_def_cfa_offset 16
    vmovupd (%rdi), %ymm0
    movsd 32(%rdi), %xmm2
    movq %rsi, %rdi
    addsd %xmm2, %xmm0
    call wide_witness
    # LCCC_CALL_ARGS 1
    # LCCC_CALL_FP 1
    addq $8, %rsp
.cfi_def_cfa_offset 8
    ret
.cfi_endproc
.size caller_upper_call,.-caller_upper_call

.globl pipeline_positive
.type pipeline_positive,@function
pipeline_positive:
.cfi_startproc
    movq %rax, %rax
    ret
.cfi_endproc
.size pipeline_positive,.-pipeline_positive
.section .note.GNU-stack,"",@progbits
