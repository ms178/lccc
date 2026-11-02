.text
.globl shuffle_symbolic_load
.type shuffle_symbolic_load,@function
shuffle_symbolic_load:
    pshufd $0, rip_input(%rip), %xmm0
    movdqu %xmm0, (%rdi)
    ret
.size shuffle_symbolic_load,.-shuffle_symbolic_load
.globl shift_symbolic_load
.type shift_symbolic_load,@function
shift_symbolic_load:
    vpslld $3, rip_input(%rip), %xmm0
    vmovdqu %xmm0, (%rdi)
    ret
.size shift_symbolic_load,.-shift_symbolic_load
.globl extract_symbolic_store
.type extract_symbolic_store,@function
extract_symbolic_store:
    vpbroadcastd rip_input(%rip), %zmm0
    vextracti32x4 $0, %zmm0, rip_output(%rip)
    vzeroupper
    ret
.size extract_symbolic_store,.-extract_symbolic_store
.data
.p2align 4
rip_input:
    .long 1, 0xffffffff, 0x80000000, 0x12345678
    .long 0xfeedface
.globl rip_output
rip_output:
    .fill 17, 1, 0xa5
.section .note.GNU-stack,"",@progbits
