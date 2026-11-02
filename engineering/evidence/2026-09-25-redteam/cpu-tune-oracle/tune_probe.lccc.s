.text
.globl popcnt_sum
.p2align 4
.type popcnt_sum, @function
popcnt_sum:
.cfi_startproc
    .cfi_def_cfa_offset 8
    # LCCC_RET_XMM 0
    # LCCC_RET_RDX 0
    # LCCC_RET_RAX 1
    xorl %edx, %edx
    xorl %r8d, %r8d
.LBB1:
    cmpq %rsi, %r8
jae .LBB3
.p2align 4,,10
.p2align 3
.LBB2:
    movq (%rdi, %r8, 8), %r10
    popcntq %r10, %r10
    addq %r10, %rdx
    addq $1, %r8
    cmpq %rsi, %r8
jb .LBB2
.LBB3:
    movq %rdx, %rax
    ret
.cfi_endproc
.size popcnt_sum, .-popcnt_sum

.globl ctz_of
.p2align 4
.type ctz_of, @function
ctz_of:
.cfi_startproc
    .cfi_def_cfa_offset 8
    # LCCC_RET_XMM 0
    # LCCC_RET_RDX 0
    # LCCC_RET_RAX 1
    tzcntq %rdi, %rsi
    movq %rsi, %rax
    ret
.cfi_endproc
.size ctz_of, .-ctz_of

.globl clz_of
.p2align 4
.type clz_of, @function
clz_of:
.cfi_startproc
    .cfi_def_cfa_offset 8
    # LCCC_RET_XMM 0
    # LCCC_RET_RDX 0
    # LCCC_RET_RAX 1
    lzcntq %rdi, %rsi
    movq %rsi, %rax
    ret
.cfi_endproc
.size clz_of, .-clz_of

.globl shl_var
.p2align 4
.type shl_var, @function
shl_var:
.cfi_startproc
    .cfi_def_cfa_offset 8
    # LCCC_RET_XMM 0
    # LCCC_RET_RDX 0
    # LCCC_RET_RAX 1
    shlxq %rsi, %rdi, %rdi
    movq %rdi, %rax
    ret
.cfi_endproc
.size shl_var, .-shl_var

.globl shr_var
.p2align 4
.type shr_var, @function
shr_var:
.cfi_startproc
    .cfi_def_cfa_offset 8
    # LCCC_RET_XMM 0
    # LCCC_RET_RDX 0
    # LCCC_RET_RAX 1
    shrxq %rsi, %rdi, %rdi
    movq %rdi, %rax
    ret
.cfi_endproc
.size shr_var, .-shr_var

.globl sar_var
.p2align 4
.type sar_var, @function
sar_var:
.cfi_startproc
    .cfi_def_cfa_offset 8
    # LCCC_RET_XMM 0
    # LCCC_RET_RDX 0
    # LCCC_RET_RAX 1
    sarxq %rsi, %rdi, %rdi
    movq %rdi, %rax
    ret
.cfi_endproc
.size sar_var, .-sar_var

.globl rot_hash
.p2align 4
.type rot_hash, @function
rot_hash:
.cfi_startproc
    .cfi_def_cfa_offset 8
    # LCCC_RET_XMM 0
    # LCCC_RET_RDX 0
    # LCCC_RET_RAX 1
    xorl %r8d, %r8d
    xorl %r9d, %r9d
.LBB10:
    cmpq %rsi, %r9
jae .LBB12
.p2align 4,,10
.p2align 3
.LBB11:
    shlxl %edx, %r8d, %r11d
    movl %r11d, %r8d
    xorl (%rdi, %r9, 4), %r8d
    addq $1, %r9
    cmpq %rsi, %r9
jb .LBB11
.LBB12:
    movl %r8d, %eax
    ret
.cfi_endproc
.size rot_hash, .-rot_hash

.globl copy_200
.p2align 4
.type copy_200, @function
copy_200:
.cfi_startproc
    .cfi_def_cfa_offset 8
    # LCCC_RET_XMM 0
    # LCCC_RET_RDX 0
    # LCCC_RET_RAX 0
    vmovdqu 0(%rsi), %ymm0
    vmovdqu %ymm0, 0(%rdi)
    vmovdqu 32(%rsi), %ymm1
    vmovdqu %ymm1, 32(%rdi)
    vmovdqu 64(%rsi), %ymm0
    vmovdqu %ymm0, 64(%rdi)
    vmovdqu 96(%rsi), %ymm1
    vmovdqu %ymm1, 96(%rdi)
    vmovdqu 128(%rsi), %ymm0
    vmovdqu %ymm0, 128(%rdi)
    vmovdqu 160(%rsi), %ymm1
    vmovdqu %ymm1, 160(%rdi)
    movq 192(%rsi), %rax
    movq %rax, 192(%rdi)
    vzeroupper
    ret
.cfi_endproc
.size copy_200, .-copy_200

.globl copy_2112
.p2align 4
.type copy_2112, @function
copy_2112:
.cfi_startproc
    .cfi_def_cfa_offset 8
    # LCCC_RET_XMM 0
    # LCCC_RET_RDX 0
    # LCCC_RET_RAX 0
    movl $2112, %ecx
    rep movsb
    ret
.cfi_endproc
.size copy_2112, .-copy_2112

.globl copy_4096
.p2align 4
.type copy_4096, @function
copy_4096:
.cfi_startproc
    .cfi_def_cfa_offset 8
    # LCCC_RET_XMM 0
    # LCCC_RET_RDX 0
    # LCCC_RET_RAX 0
    movl $4096, %ecx
    rep movsb
    ret
.cfi_endproc
.size copy_4096, .-copy_4096

.globl copy_8192
.p2align 4
.type copy_8192, @function
copy_8192:
.cfi_startproc
    .cfi_def_cfa_offset 8
    # LCCC_RET_XMM 0
    # LCCC_RET_RDX 0
    # LCCC_RET_RAX 0
    movl $8192, %ecx
    rep movsb
    ret
.cfi_endproc
.size copy_8192, .-copy_8192


.section .note.GNU-stack,"",@progbits
