# Third input of the strmerge family: named symbols inside merged sections
# (their values must follow the pool), a reference past the end of a merged
# input, an 8-aligned string group that must stay apart from the 1-aligned
# one, and a string section with a relocation of its own (bfd leaves it
# unmerged).
        .section .rodata.str1.1,"aMS",@progbits,1
        .globl gstr
gstr:   .string "ending"
lstr:   .string "only in c"
lend:
        .section .rodata.str1.8,"aMS",@progbits,1
        .p2align 3
w8:     .string "shared format %d\n"
        .section .rodata.str1.1,"aMS",@progbits,1,unique,1
r1:     .string "x"
        .dword gstr
        .section .srodata.cst8,"aM",@progbits,8
        .p2align 3
c8:     .dword 0x4005bf0a8b145769
        .data
        .globl c_refs
        .p2align 3
c_refs: .dword lstr, lend, w8, r1, c8, gstr + 3
