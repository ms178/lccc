.text
.globl pc32
.type pc32, %function
pc32:
.cfi_startproc
    stp x29, x30, [sp, #-32]!
    .cfi_def_cfa_offset 32
    .cfi_offset x29, -32
    .cfi_offset x30, -24
    mov x29, sp
    .cfi_def_cfa_register x29
    str x19, [sp, #16]
    mov x19, x0
    mov x0, x19
    lsr w1, w0, #1
    and w1, w1, #0x55555555
    sub w0, w0, w1
    lsr w1, w0, #2
    and w0, w0, #0x33333333
    and w1, w1, #0x33333333
    add w0, w0, w1
    add w0, w0, w0, lsr #4
    and w0, w0, #0x0f0f0f0f
    mov w1, #0x01010101
    mul w0, w0, w1
    lsr w0, w0, #24
    ldr x19, [sp, #16]
    ldp x29, x30, [sp], #32
    ret
.cfi_endproc
.size pc32, .-pc32
.globl pc64
.type pc64, %function
pc64:
.cfi_startproc
    stp x29, x30, [sp, #-32]!
    .cfi_def_cfa_offset 32
    .cfi_offset x29, -32
    .cfi_offset x30, -24
    mov x29, sp
    .cfi_def_cfa_register x29
    str x19, [sp, #16]
    mov x19, x0
    mov x0, x19
    lsr x1, x19, #1
    and x1, x1, #0x5555555555555555
    sub x0, x0, x1
    lsr x1, x0, #2
    and x0, x0, #0x3333333333333333
    and x1, x1, #0x3333333333333333
    add x0, x0, x1
    add x0, x0, x0, lsr #4
    and x0, x0, #0x0f0f0f0f0f0f0f0f
    mov x1, #0x0101010101010101
    mul x0, x0, x1
    lsr x0, x0, #56
    mov w0, w0
    ldr x19, [sp, #16]
    ldp x29, x30, [sp], #32
    ret
.cfi_endproc
.size pc64, .-pc64
.globl pcs32
.type pcs32, %function
pcs32:
.cfi_startproc
    stp x29, x30, [sp, #-32]!
    .cfi_def_cfa_offset 32
    .cfi_offset x29, -32
    .cfi_offset x30, -24
    mov x29, sp
    .cfi_def_cfa_register x29
    str x19, [sp, #16]
    mov x19, x0
    mov x0, x19
    mov w0, w0
    lsr w1, w0, #1
    and w1, w1, #0x55555555
    sub w0, w0, w1
    lsr w1, w0, #2
    and w0, w0, #0x33333333
    and w1, w1, #0x33333333
    add w0, w0, w1
    add w0, w0, w0, lsr #4
    and w0, w0, #0x0f0f0f0f
    mov w1, #0x01010101
    mul w0, w0, w1
    lsr w0, w0, #24
    ldr x19, [sp, #16]
    ldp x29, x30, [sp], #32
    ret
.cfi_endproc
.size pcs32, .-pcs32
.globl pcs64
.type pcs64, %function
pcs64:
.cfi_startproc
    stp x29, x30, [sp, #-32]!
    .cfi_def_cfa_offset 32
    .cfi_offset x29, -32
    .cfi_offset x30, -24
    mov x29, sp
    .cfi_def_cfa_register x29
    stp x19, x20, [sp, #16]
    mov x19, x0
    mov x0, x19
    lsr x1, x19, #1
    and x1, x1, #0x5555555555555555
    sub x0, x0, x1
    lsr x1, x0, #2
    and x0, x0, #0x3333333333333333
    and x1, x1, #0x3333333333333333
    add x0, x0, x1
    add x0, x0, x0, lsr #4
    and x0, x0, #0x0f0f0f0f0f0f0f0f
    mov x1, #0x0101010101010101
    mul x0, x0, x1
    lsr x0, x0, #56
    mov w0, w0
    ldp x19, x20, [sp, #16]
    ldp x29, x30, [sp], #32
    ret
.cfi_endproc
.size pcs64, .-pcs64
.globl pressure
.type pressure, %function
pressure:
.cfi_startproc
    stp x29, x30, [sp, #-96]!
    .cfi_def_cfa_offset 96
    .cfi_offset x29, -96
    .cfi_offset x30, -88
    mov x29, sp
    .cfi_def_cfa_register x29
    stp x19, x20, [sp, #16]
    stp x21, x22, [sp, #32]
    stp x23, x24, [sp, #48]
    stp x25, x26, [sp, #64]
    stp x27, x28, [sp, #80]
    mov x19, x0
    mov x20, x1
    mov x21, x2
    mov x23, x4
    ldr x0, [sp, #96]
    mov x27, x0
    ldr x0, [sp, #104]
    mov x28, x0
    mov x0, x19
    lsr x1, x19, #1
    and x1, x1, #0x5555555555555555
    sub x0, x0, x1
    lsr x1, x0, #2
    and x0, x0, #0x3333333333333333
    and x1, x1, #0x3333333333333333
    add x0, x0, x1
    add x0, x0, x0, lsr #4
    and x0, x0, #0x0f0f0f0f0f0f0f0f
    mov x1, #0x0101010101010101
    mul x0, x0, x1
    lsr x0, x0, #56
    add x19, x0, x20
    mov x2, #3
    madd x19, x21, x2, x19
    mov x2, #5
    madd x19, x3, x2, x19
    mov x2, #7
    madd x19, x23, x2, x19
    mov x2, #11
    madd x19, x5, x2, x19
    mov x2, #13
    madd x19, x6, x2, x19
    mov x2, #17
    madd x19, x7, x2, x19
    mov x2, #19
    madd x19, x27, x2, x19
    mov x2, #23
    madd x4, x28, x2, x19
    mov x0, x4
    ldp x19, x20, [sp, #16]
    ldp x21, x3, [sp, #32]
    ldp x23, x5, [sp, #48]
    ldp x6, x7, [sp, #64]
    ldp x27, x28, [sp, #80]
    ldp x29, x30, [sp], #96
    ret
.cfi_endproc
.size pressure, .-pressure
.globl pcu8
.type pcu8, %function
pcu8:
.cfi_startproc
    stp x29, x30, [sp, #-32]!
    .cfi_def_cfa_offset 32
    .cfi_offset x29, -32
    .cfi_offset x30, -24
    mov x29, sp
    .cfi_def_cfa_register x29
    str x19, [sp, #16]
    mov x19, x0
    and w5, w19, #0xff
    mov x0, x5
    lsr w1, w0, #1
    and w1, w1, #0x55555555
    sub w0, w0, w1
    lsr w1, w0, #2
    and w0, w0, #0x33333333
    and w1, w1, #0x33333333
    add w0, w0, w1
    add w0, w0, w0, lsr #4
    and w0, w0, #0x0f0f0f0f
    mov w1, #0x01010101
    mul w0, w0, w1
    lsr w0, w0, #24
    ldr x19, [sp, #16]
    ldp x29, x30, [sp], #32
    ret
.cfi_endproc
.size pcu8, .-pcu8
.globl pcs8
.type pcs8, %function
pcs8:
.cfi_startproc
    stp x29, x30, [sp, #-32]!
    .cfi_def_cfa_offset 32
    .cfi_offset x29, -32
    .cfi_offset x30, -24
    mov x29, sp
    .cfi_def_cfa_register x29
    str x19, [sp, #16]
    mov x19, x0
    sxtb x5, w19
    mov x0, x5
    lsr w1, w0, #1
    and w1, w1, #0x55555555
    sub w0, w0, w1
    lsr w1, w0, #2
    and w0, w0, #0x33333333
    and w1, w1, #0x33333333
    add w0, w0, w1
    add w0, w0, w0, lsr #4
    and w0, w0, #0x0f0f0f0f
    mov w1, #0x01010101
    mul w0, w0, w1
    lsr w0, w0, #24
    ldr x19, [sp, #16]
    ldp x29, x30, [sp], #32
    ret
.cfi_endproc
.size pcs8, .-pcs8
.globl pcu16
.type pcu16, %function
pcu16:
.cfi_startproc
    stp x29, x30, [sp, #-32]!
    .cfi_def_cfa_offset 32
    .cfi_offset x29, -32
    .cfi_offset x30, -24
    mov x29, sp
    .cfi_def_cfa_register x29
    str x19, [sp, #16]
    mov x19, x0
    and w5, w19, #0xffff
    mov x0, x5
    lsr w1, w0, #1
    and w1, w1, #0x55555555
    sub w0, w0, w1
    lsr w1, w0, #2
    and w0, w0, #0x33333333
    and w1, w1, #0x33333333
    add w0, w0, w1
    add w0, w0, w0, lsr #4
    and w0, w0, #0x0f0f0f0f
    mov w1, #0x01010101
    mul w0, w0, w1
    lsr w0, w0, #24
    ldr x19, [sp, #16]
    ldp x29, x30, [sp], #32
    ret
.cfi_endproc
.size pcu16, .-pcu16
.globl pcs16
.type pcs16, %function
pcs16:
.cfi_startproc
    stp x29, x30, [sp, #-32]!
    .cfi_def_cfa_offset 32
    .cfi_offset x29, -32
    .cfi_offset x30, -24
    mov x29, sp
    .cfi_def_cfa_register x29
    str x19, [sp, #16]
    mov x19, x0
    sxth x5, w19
    mov x0, x5
    lsr w1, w0, #1
    and w1, w1, #0x55555555
    sub w0, w0, w1
    lsr w1, w0, #2
    and w0, w0, #0x33333333
    and w1, w1, #0x33333333
    add w0, w0, w1
    add w0, w0, w0, lsr #4
    and w0, w0, #0x0f0f0f0f
    mov w1, #0x01010101
    mul w0, w0, w1
    lsr w0, w0, #24
    ldr x19, [sp, #16]
    ldp x29, x30, [sp], #32
    ret
.cfi_endproc
.size pcs16, .-pcs16
.globl pcl
.type pcl, %function
pcl:
.cfi_startproc
    stp x29, x30, [sp, #-32]!
    .cfi_def_cfa_offset 32
    .cfi_offset x29, -32
    .cfi_offset x30, -24
    mov x29, sp
    .cfi_def_cfa_register x29
    str x19, [sp, #16]
    mov x19, x0
    mov x0, x19
    lsr x1, x19, #1
    and x1, x1, #0x5555555555555555
    sub x0, x0, x1
    lsr x1, x0, #2
    and x0, x0, #0x3333333333333333
    and x1, x1, #0x3333333333333333
    add x0, x0, x1
    add x0, x0, x0, lsr #4
    and x0, x0, #0x0f0f0f0f0f0f0f0f
    mov x1, #0x0101010101010101
    mul x0, x0, x1
    lsr x0, x0, #56
    mov w0, w0
    ldr x19, [sp, #16]
    ldp x29, x30, [sp], #32
    ret
.cfi_endproc
.size pcl, .-pcl
.section .note.GNU-stack,"",@progbits
