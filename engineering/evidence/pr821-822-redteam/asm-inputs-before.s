.text
.type double_bits, %function
double_bits:
.cfi_startproc
    stp x29, x30, [sp, #-64]!
    .cfi_def_cfa_offset 64
    .cfi_offset x29, -64
    .cfi_offset x30, -56
    mov x29, sp
    .cfi_def_cfa_register x29
    str x0, [sp, #16]
    ldr x0, [sp, #16]
    str x0, [sp, #16]
    ldr x0, [sp, #16]
    str x0, [sp, #16]
    ldr x0, [sp, #16]
    str x0, [sp, #24]
    ldr x9, [sp, #24]
    ldr x0, [x9]
    str x0, [sp, #32]
    ldr x9, [sp, #32]
    fmov d16, x9
//APP
    fmov x9, d16
//NO_APP
    str x9, [sp, #40]
    ldr x0, [sp, #40]
    str x0, [sp, #48]
    ldp x29, x30, [sp], #64
    ret
.cfi_endproc
.size double_bits, .-double_bits
.type float_bits, %function
float_bits:
.cfi_startproc
    stp x29, x30, [sp, #-64]!
    .cfi_def_cfa_offset 64
    .cfi_offset x29, -64
    .cfi_offset x30, -56
    mov x29, sp
    .cfi_def_cfa_register x29
    str x0, [sp, #16]
    ldr x0, [sp, #16]
    str x0, [sp, #16]
    ldr x0, [sp, #16]
    str x0, [sp, #16]
    ldr x0, [sp, #16]
    str x0, [sp, #24]
    ldr x9, [sp, #24]
    ldr w0, [x9]
    str x0, [sp, #32]
    ldr w9, [sp, #32]
    fmov s16, w9
//APP
    fmov w9, s16
//NO_APP
    str x9, [sp, #40]
    ldr x0, [sp, #40]
    str x0, [sp, #48]
    ldp x29, x30, [sp], #64
    ret
.cfi_endproc
.size float_bits, .-float_bits
.type integer_bits, %function
integer_bits:
.cfi_startproc
    stp x29, x30, [sp, #-48]!
    .cfi_def_cfa_offset 48
    .cfi_offset x29, -48
    .cfi_offset x30, -40
    mov x29, sp
    .cfi_def_cfa_register x29
    str x0, [sp, #16]
    ldr x0, [sp, #16]
    str x0, [sp, #16]
    ldr x0, [sp, #16]
    str x0, [sp, #16]
    ldr x0, [sp, #16]
    str x0, [sp, #24]
    ldr x9, [sp, #24]
    fmov d16, x9
//APP
    fmov x9, d16
//NO_APP
    str x9, [sp, #32]
    ldr x0, [sp, #32]
    str x0, [sp, #40]
    ldp x29, x30, [sp], #48
    ret
.cfi_endproc
.size integer_bits, .-integer_bits
.type high_lane, %function
high_lane:
.cfi_startproc
    stp x29, x30, [sp, #-48]!
    .cfi_def_cfa_offset 48
    .cfi_offset x29, -48
    .cfi_offset x30, -40
    mov x29, sp
    .cfi_def_cfa_register x29
    str x0, [sp, #16]
    ldr x0, [sp, #16]
    str x0, [sp, #16]
    ldr x0, [sp, #16]
    str x0, [sp, #16]
    ldr x0, [sp, #16]
    str x0, [sp, #24]
    movz x17, #24
    add x17, sp, x17
    ldr q16, [x17]
//APP
    umov x9, v16.d[1]
//NO_APP
    str x9, [sp, #32]
    ldr x0, [sp, #32]
    str x0, [sp, #40]
    ldp x29, x30, [sp], #48
    ret
.cfi_endproc
.size high_lane, .-high_lane
.globl test
.type test, %function
test:
.cfi_startproc
    stp x29, x30, [sp, #-160]!
    .cfi_def_cfa_offset 160
    .cfi_offset x29, -160
    .cfi_offset x30, -152
    mov x29, sp
    .cfi_def_cfa_register x29
    movz x0, #16372, lsl #48
    str x0, [sp, #16]
    movz x0, #16388, lsl #48
    fmov d0, x0
    fneg d0, d0
    fmov x0, d0
    str x0, [sp, #128]
    str x0, [sp, #24]
    mov x0, #0
    fmov d0, x0
    fneg d0, d0
    fmov x0, d0
    str x0, [sp, #136]
    str x0, [sp, #32]
    movz x0, #16372, lsl #48
    str x0, [sp, #40]
    movz x0, #49156, lsl #48
    str x0, [sp, #48]
    movz x0, #32768, lsl #48
    str x0, [sp, #56]
    movz x0, #16288, lsl #16
    str w0, [sp, #64]
    movz x0, #16416, lsl #16
    fmov s0, w0
    fneg s0, s0
    fmov w0, s0
    mov w0, w0
    str x0, [sp, #128]
    str w0, [sp, #68]
    mov x0, #0
    fmov s0, w0
    fneg s0, s0
    fmov w0, s0
    mov w0, w0
    str x0, [sp, #136]
    str w0, [sp, #72]
    movz x0, #16288, lsl #16
    str w0, [sp, #80]
    movz x0, #49184, lsl #16
    str w0, [sp, #84]
    movz x0, #32768, lsl #16
    str w0, [sp, #88]
    mov x0, #0
    sxtw x0, w0
    str x0, [sp, #112]
.LBB5:
    mov x0, #3
    sxtw x0, w0
    str x0, [sp, #136]
    ldr x0, [sp, #112]
    mov x1, x0
    ldr x0, [sp, #136]
    cmp w1, w0
    b.lt .LBB6
.Lskip_6:
    b .LBB14
.LBB6:
    ldr x0, [sp, #112]
    sxtw x0, w0
    mov x1, x0
    mov x2, #8
    add x0, sp, #16
    mov x3, x0
    madd x0, x1, x2, x0
    str x0, [sp, #136]
    ldr x0, [sp, #136]
    mov x9, x0
    mov x0, x9
    bl double_bits
    str x0, [sp, #128]
    ldr x0, [sp, #112]
    sxtw x0, w0
    mov x1, x0
    mov x2, #8
    mul x0, x1, x2
    str x0, [sp, #136]
    add x1, sp, #40
    add x0, x1, x0
    str x0, [sp, #144]
    ldr x9, [sp, #144]
    ldr x0, [x9]
    str x0, [sp, #136]
    ldr x0, [sp, #128]
    mov x1, x0
    ldr x0, [sp, #136]
    cmp x1, x0
    b.ne .LBB20
.Lskip_7:
.LBB7:
    ldr x0, [sp, #112]
    sxtw x0, w0
    mov x1, x0
    mov x2, #4
    add x0, sp, #64
    mov x3, x0
    madd x0, x1, x2, x0
    str x0, [sp, #136]
    ldr x0, [sp, #136]
    mov x9, x0
    mov x0, x9
    bl float_bits
    str x0, [sp, #128]
    ldr x0, [sp, #112]
    sxtw x0, w0
    mov x1, x0
    mov x2, #4
    mul x0, x1, x2
    str x0, [sp, #136]
    add x1, sp, #80
    add x0, x1, x0
    str x0, [sp, #144]
    ldr x9, [sp, #144]
    ldr w0, [x9]
    str x0, [sp, #136]
    ldr x0, [sp, #128]
    str x0, [sp, #144]
    ldr x0, [sp, #136]
    str x0, [sp, #128]
    ldr x0, [sp, #144]
    mov x1, x0
    ldr x0, [sp, #128]
    cmp w1, w0
    b.ne .LBB19
.Lskip_8:
.LBB8:
    ldr x0, [sp, #112]
    sxtw x0, w0
    mov x1, x0
    mov x2, #8
    mul x0, x1, x2
    str x0, [sp, #128]
    add x1, sp, #40
    add x0, x1, x0
    str x0, [sp, #136]
    ldr x9, [sp, #136]
    ldr x0, [x9]
    str x0, [sp, #128]
    ldr x0, [sp, #128]
    mov x9, x0
    mov x0, x9
    bl integer_bits
    str x0, [sp, #136]
    ldr x0, [sp, #112]
    sxtw x0, w0
    mov x1, x0
    mov x2, #8
    mul x0, x1, x2
    str x0, [sp, #128]
    add x1, sp, #40
    add x0, x1, x0
    str x0, [sp, #144]
    ldr x9, [sp, #144]
    ldr x0, [x9]
    str x0, [sp, #128]
    ldr x0, [sp, #136]
    mov x1, x0
    ldr x0, [sp, #128]
    cmp x1, x0
    b.ne .LBB18
.Lskip_9:
.LBB9:
.LBB10:
    ldr x0, [sp, #112]
    add w0, w0, #1
    str x0, [sp, #112]
    mov x0, #3
    sxtw x0, w0
    str x0, [sp, #136]
    ldr x0, [sp, #112]
    mov x1, x0
    ldr x0, [sp, #136]
    cmp w1, w0
    b.lt .LBB6
.Lskip_10:
.LBB14:
    movz x0, #57072
    movk x0, #39612, lsl #16
    movk x0, #22136, lsl #32
    movk x0, #4660, lsl #48
    str x0, [sp, #96]
    movz x0, #12816
    movk x0, #30292, lsl #16
    movk x0, #47768, lsl #32
    movk x0, #65244, lsl #48
    str x0, [sp, #104]
    add x0, sp, #96
    mov x9, x0
    mov x0, x9
    bl high_lane
    str x0, [sp, #128]
    ldr x0, [sp, #128]
    mov x1, x0
    movz x0, #12816
    movk x0, #30292, lsl #16
    movk x0, #47768, lsl #32
    movk x0, #65244, lsl #48
    cmp x1, x0
    b.ne .LBB15
.Lskip_11:
    b .LBB17
.LBB15:
    mov x0, #4
    str x0, [sp, #120]
.LBB16:
    ldr x0, [sp, #120]
    ldp x29, x30, [sp], #160
    ret
.LBB17:
    mov x0, #0
    str xzr, [sp, #120]
    b .LBB16
.LBB18:
    mov x0, #3
    sxtw x0, w0
    str x0, [sp, #128]
    ldp x29, x30, [sp], #160
    ret
.LBB19:
    mov x0, #2
    sxtw x0, w0
    str x0, [sp, #128]
    ldp x29, x30, [sp], #160
    ret
.LBB20:
    mov x0, #1
    sxtw x0, w0
    str x0, [sp, #128]
    ldp x29, x30, [sp], #160
    ret
.cfi_endproc
.size test, .-test
.section .note.GNU-stack,"",@progbits
