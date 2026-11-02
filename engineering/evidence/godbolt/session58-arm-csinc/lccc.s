inc_if_sge_i32:
.cfi_startproc
    stp x29, x30, [sp, #-64]!
    .cfi_def_cfa_offset 64
    .cfi_offset x29, -64
    .cfi_offset x30, -56
    mov x29, sp
    .cfi_def_cfa_register x29
    stp x19, x20, [sp, #16]
    stp x21, x22, [sp, #32]
    stp x23, x24, [sp, #48]
    mov x20, x1
    mov x21, x2
    mov x19, x0
    cmp w20, w21
    csinc w24, w19, w19, lt
    mov x0, x24
    ldp x19, x20, [sp, #16]
    ldp x21, x22, [sp, #32]
    ldp x23, x24, [sp, #48]
    ldp x29, x30, [sp], #64
    ret
