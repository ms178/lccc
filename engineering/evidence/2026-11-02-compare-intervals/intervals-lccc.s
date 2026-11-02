.text
.globl uchar_le
.p2align 4
.type uchar_le, @function
uchar_le:
.cfi_startproc
    # LCCC_RET_XMM 0
    # LCCC_RET_XMM_BITS 512
    # LCCC_RET_RDX 0
    # LCCC_RET_RAX 1
    movl $1, %eax
    ret
.cfi_endproc
.size uchar_le, .-uchar_le

.globl uchar_lt
.p2align 4
.type uchar_lt, @function
uchar_lt:
.cfi_startproc
    # LCCC_RET_XMM 0
    # LCCC_RET_XMM_BITS 512
    # LCCC_RET_RDX 0
    # LCCC_RET_RAX 1
    movl $1, %eax
    ret
.cfi_endproc
.size uchar_lt, .-uchar_lt

.globl uchar_gt
.p2align 4
.type uchar_gt, @function
uchar_gt:
.cfi_startproc
    # LCCC_RET_XMM 0
    # LCCC_RET_XMM_BITS 512
    # LCCC_RET_RDX 0
    # LCCC_RET_RAX 1
    xorl %eax, %eax
    ret
.cfi_endproc
.size uchar_gt, .-uchar_gt

.globl uchar_eq
.p2align 4
.type uchar_eq, @function
uchar_eq:
.cfi_startproc
    # LCCC_RET_XMM 0
    # LCCC_RET_XMM_BITS 512
    # LCCC_RET_RDX 0
    # LCCC_RET_RAX 1
    xorl %eax, %eax
    ret
.cfi_endproc
.size uchar_eq, .-uchar_eq

.globl ushort_le
.p2align 4
.type ushort_le, @function
ushort_le:
.cfi_startproc
    # LCCC_RET_XMM 0
    # LCCC_RET_XMM_BITS 512
    # LCCC_RET_RDX 0
    # LCCC_RET_RAX 1
    movl $1, %eax
    ret
.cfi_endproc
.size ushort_le, .-ushort_le

.globl uint_le
.p2align 4
.type uint_le, @function
uint_le:
.cfi_startproc
    # LCCC_RET_XMM 0
    # LCCC_RET_XMM_BITS 512
    # LCCC_RET_RDX 0
    # LCCC_RET_RAX 1
    movl $1, %eax
    ret
.cfi_endproc
.size uint_le, .-uint_le

.globl schar_lt
.p2align 4
.type schar_lt, @function
schar_lt:
.cfi_startproc
    # LCCC_RET_XMM 0
    # LCCC_RET_XMM_BITS 512
    # LCCC_RET_RDX 0
    # LCCC_RET_RAX 1
    movl $1, %eax
    ret
.cfi_endproc
.size schar_lt, .-schar_lt

.globl schar_gt
.p2align 4
.type schar_gt, @function
schar_gt:
.cfi_startproc
    # LCCC_RET_XMM 0
    # LCCC_RET_XMM_BITS 512
    # LCCC_RET_RDX 0
    # LCCC_RET_RAX 1
    xorl %eax, %eax
    ret
.cfi_endproc
.size schar_gt, .-schar_gt

.globl schar_ge
.p2align 4
.type schar_ge, @function
schar_ge:
.cfi_startproc
    # LCCC_RET_XMM 0
    # LCCC_RET_XMM_BITS 512
    # LCCC_RET_RDX 0
    # LCCC_RET_RAX 1
    movl $1, %eax
    ret
.cfi_endproc
.size schar_ge, .-schar_ge

.globl schar_eq
.p2align 4
.type schar_eq, @function
schar_eq:
.cfi_startproc
    # LCCC_RET_XMM 0
    # LCCC_RET_XMM_BITS 512
    # LCCC_RET_RDX 0
    # LCCC_RET_RAX 1
    xorl %eax, %eax
    ret
.cfi_endproc
.size schar_eq, .-schar_eq

.globl sshort_le
.p2align 4
.type sshort_le, @function
sshort_le:
.cfi_startproc
    # LCCC_RET_XMM 0
    # LCCC_RET_XMM_BITS 512
    # LCCC_RET_RDX 0
    # LCCC_RET_RAX 1
    movl $1, %eax
    ret
.cfi_endproc
.size sshort_le, .-sshort_le

.globl int_gt_max
.p2align 4
.type int_gt_max, @function
int_gt_max:
.cfi_startproc
    # LCCC_RET_XMM 0
    # LCCC_RET_XMM_BITS 512
    # LCCC_RET_RDX 0
    # LCCC_RET_RAX 1
    xorl %eax, %eax
    ret
.cfi_endproc
.size int_gt_max, .-int_gt_max

.globl int_lt_min
.p2align 4
.type int_lt_min, @function
int_lt_min:
.cfi_startproc
    # LCCC_RET_XMM 0
    # LCCC_RET_XMM_BITS 512
    # LCCC_RET_RDX 0
    # LCCC_RET_RAX 1
    xorl %eax, %eax
    ret
.cfi_endproc
.size int_lt_min, .-int_lt_min

.globl uint_le_max
.p2align 4
.type uint_le_max, @function
uint_le_max:
.cfi_startproc
    # LCCC_RET_XMM 0
    # LCCC_RET_XMM_BITS 512
    # LCCC_RET_RDX 0
    # LCCC_RET_RAX 1
    movl $1, %eax
    ret
.cfi_endproc
.size uint_le_max, .-uint_le_max

.globl uint_gt_max
.p2align 4
.type uint_gt_max, @function
uint_gt_max:
.cfi_startproc
    # LCCC_RET_XMM 0
    # LCCC_RET_XMM_BITS 512
    # LCCC_RET_RDX 0
    # LCCC_RET_RAX 1
    xorl %eax, %eax
    ret
.cfi_endproc
.size uint_gt_max, .-uint_gt_max

.globl int_ule_neg1
.p2align 4
.type int_ule_neg1, @function
int_ule_neg1:
.cfi_startproc
    # LCCC_RET_XMM 0
    # LCCC_RET_XMM_BITS 512
    # LCCC_RET_RDX 0
    # LCCC_RET_RAX 1
    movl $1, %eax
    ret
.cfi_endproc
.size int_ule_neg1, .-int_ule_neg1

.globl keep_uchar_lt
.p2align 4
.type keep_uchar_lt, @function
keep_uchar_lt:
.cfi_startproc
    # LCCC_RET_XMM 0
    # LCCC_RET_XMM_BITS 512
    # LCCC_RET_RDX 0
    # LCCC_RET_RAX 1
    xorl %eax, %eax
    cmpb $100, %dil
    setb %al
    ret
.cfi_endproc
.size keep_uchar_lt, .-keep_uchar_lt

.globl keep_schar_gt
.p2align 4
.type keep_schar_gt, @function
keep_schar_gt:
.cfi_startproc
    # LCCC_RET_XMM 0
    # LCCC_RET_XMM_BITS 512
    # LCCC_RET_RDX 0
    # LCCC_RET_RAX 1
    xorl %eax, %eax
    cmpb $-100, %dil
    setg %al
    ret
.cfi_endproc
.size keep_schar_gt, .-keep_schar_gt

.globl keep_schar_ult
.p2align 4
.type keep_schar_ult, @function
keep_schar_ult:
.cfi_startproc
    # LCCC_RET_XMM 0
    # LCCC_RET_XMM_BITS 512
    # LCCC_RET_RDX 0
    # LCCC_RET_RAX 1
    movsbl %dil, %edx
    xorl %eax, %eax
    cmpl $300, %edx
    setb %al
    ret
.cfi_endproc
.size keep_schar_ult, .-keep_schar_ult

.globl keep_uint_ge_max
.p2align 4
.type keep_uint_ge_max, @function
keep_uint_ge_max:
.cfi_startproc
    # LCCC_RET_XMM 0
    # LCCC_RET_XMM_BITS 512
    # LCCC_RET_RDX 0
    # LCCC_RET_RAX 1
    movl $4294967295, %ecx
    xorl %eax, %eax
    cmpl %ecx, %edi
    setae %al
    ret
.cfi_endproc
.size keep_uint_ge_max, .-keep_uint_ge_max

.globl keep_int_eq
.p2align 4
.type keep_int_eq, @function
keep_int_eq:
.cfi_startproc
    # LCCC_RET_XMM 0
    # LCCC_RET_XMM_BITS 512
    # LCCC_RET_RDX 0
    # LCCC_RET_RAX 1
    xorl %eax, %eax
    cmpl $5, %edi
    sete %al
    ret
.cfi_endproc
.size keep_int_eq, .-keep_int_eq


.section .note.GNU-stack,"",@progbits
