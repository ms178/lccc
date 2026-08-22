count_primes:
.cfi_startproc
    stp x29, x30, [sp, #-128]!
    .cfi_def_cfa_offset 128
    .cfi_offset x29, -128
    .cfi_offset x30, -120
    mov x29, sp
    .cfi_def_cfa_register x29
    stp x19, x20, [sp, #40]
    stp x21, x22, [sp, #56]
    stp x23, x24, [sp, #72]
    stp x25, x26, [sp, #88]
    stp x27, x28, [sp, #104]
    adrp x0, sieve
    add x0, x0, :lo12:sieve
    str x0, [sp, #32]
    mov x9, x0
    mov x10, #1
    movz x0, #38529
    movk x0, #152, lsl #16
    mov x11, x0
    mov x0, x9
    mov x1, x10
    mov x2, x11
    bl memset
    adrp x0, sieve
    add x0, x0, :lo12:sieve
    mov x22, x0
    add x0, x22, #1
    str x0, [sp, #32]
    strb wzr, [x0]
    mov x0, #0
    strb wzr, [x22]
    adrp x0, sieve
    add x0, x0, :lo12:sieve
    mov x23, x0
    adrp x0, sieve
    add x0, x0, :lo12:sieve
    str x0, [sp, #16]
    movz x0, #38528
    movk x0, #152, lsl #16
    mov x24, x0
    mov x0, #2
    mov x25, #2
.LBB1:
    mul w5, w25, w25
    cmp w5, w24
    b.le .LBB2
.Lskip_0:
    b .LBB7
.LBB2:
    sxtw x20, w25
    ldr x1, [sp, #16]
    add x0, x1, x20
    mov x5, x0
    ldrsb x0, [x0]
    mov x6, x0
    cbz x0, .Lskip_1
    b .LBB3
.Lskip_1:
    b .LBB6
.LBB3:
    mul w26, w25, w25
.LBB4:
    cmp w26, w24
    b.le .LBB5
.Lskip_2:
    b .LBB6
.LBB5:
    sxtw x19, w26
    strb wzr, [x23, x26]
    add w26, w26, w25
    cmp w26, w24
    b.le .LBB5
    b .Lskip_2
.LBB6:
    add w25, w25, #1
    b .LBB1
.LBB7:
    adrp x0, sieve
    add x0, x0, :lo12:sieve
    str x0, [sp, #16]
    movz x0, #38528
    movk x0, #152, lsl #16
    str x0, [sp, #24]
    mov x27, #0
    mov x0, #2
    mov x28, #2
.LBB8:
    ldr x0, [sp, #24]
    cmp w28, w0
    b.le .LBB9
.Lskip_3:
    b .LBB10
.LBB9:
    sxtw x21, w28
    ldr x1, [sp, #16]
    add x0, x1, x21
    mov x5, x0
    ldrsb x0, [x0]
    mov x6, x0
    str x0, [sp, #32]
    cmp x0, #0
    csinc w27, w27, w27, eq
    add w28, w28, #1
    ldr x0, [sp, #24]
    cmp w28, w0
    b.le .LBB9
    b .Lskip_3
.LBB10:
    mov x0, x27
    ldp x19, x20, [sp, #40]
    ldp x21, x22, [sp, #56]
    ldp x23, x24, [sp, #72]
    ldp x25, x26, [sp, #88]
    ldp x27, x28, [sp, #104]
    ldp x29, x30, [sp], #128
    ret
.cfi_endproc
.size count_primes, .-count_primes
