# Data-directive expression parity with GNU as.
#
# Two historical bugs locked in here:
# 1. `symbol + (expr) - const + const` chains (kernel level1_fixmap_pgt
#    pattern): the " - " splitter broke at the wrong boundary and swallowed
#    arithmetic into the symbol name.
# 2. `.quad a - b` with a < b: the internal-reloc patcher wrote only 4 bytes
#    for 8-byte diffs, leaving the sign-extension upper half zero.
    .data
    .globl base_sym
base_sym:
    .quad 0
lo_mark:
    .quad 0
hi_mark:
    .quad base_sym + (2 << 12) - 8 + 16
    .quad base_sym + (1 << 4)
    .quad hi_mark - lo_mark
    .quad lo_mark - hi_mark
    .long hi_mark - lo_mark
    .quad hi_mark - lo_mark - 1
    .quad hi_mark - lo_mark + 7
