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
    fmov s0, w0
    cnt v0.8b, v0.8b
    uaddlv h0, v0.8b
    fmov w0, s0
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
    fmov d0, x19
    cnt v0.8b, v0.8b
    uaddlv h0, v0.8b
    fmov w0, s0
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
    fmov s0, w0
    cnt v0.8b, v0.8b
    uaddlv h0, v0.8b
    fmov w0, s0
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
    fmov d0, x19
    cnt v0.8b, v0.8b
    uaddlv h0, v0.8b
    fmov w0, s0
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
    mov x22, x3
    mov x23, x4
    mov x24, x5
    mov x25, x6
    mov x26, x7
    ldr x0, [sp, #96]
    mov x27, x0
    ldr x0, [sp, #104]
    mov x28, x0
    mov x0, x19
    fmov d0, x19
    cnt v0.8b, v0.8b
    uaddlv h0, v0.8b
    fmov w0, s0
    add x19, x0, x20
    mov x2, #3
    madd x19, x21, x2, x19
    mov x2, #5
    madd x19, x22, x2, x19
    mov x2, #7
    madd x19, x23, x2, x19
    mov x2, #11
    madd x19, x24, x2, x19
    mov x2, #13
    madd x19, x25, x2, x19
    mov x2, #17
    madd x19, x26, x2, x19
    mov x2, #19
    madd x19, x27, x2, x19
    mov x2, #23
    madd x4, x28, x2, x19
    mov x0, x4
    ldp x19, x20, [sp, #16]
    ldp x21, x22, [sp, #32]
    ldp x23, x24, [sp, #48]
    ldp x25, x26, [sp, #64]
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
    fmov s0, w0
    cnt v0.8b, v0.8b
    uaddlv h0, v0.8b
    fmov w0, s0
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
    fmov s0, w0
    cnt v0.8b, v0.8b
    uaddlv h0, v0.8b
    fmov w0, s0
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
    fmov s0, w0
    cnt v0.8b, v0.8b
    uaddlv h0, v0.8b
    fmov w0, s0
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
    fmov s0, w0
    cnt v0.8b, v0.8b
    uaddlv h0, v0.8b
    fmov w0, s0
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
    fmov d0, x19
    cnt v0.8b, v0.8b
    uaddlv h0, v0.8b
    fmov w0, s0
    mov w0, w0
    ldr x19, [sp, #16]
    ldp x29, x30, [sp], #32
    ret
.cfi_endproc
.size pcl, .-pcl
.section .note.GNU-stack,"",@progbits
