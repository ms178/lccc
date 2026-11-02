.text
.global _start
_start:
    bl test
    mov x8, #93
    svc #0
// AAPCS64 leaves the upper half unspecified for a 32-bit argument.
// Tail-call the real 32-bit implementation without clearing those bits.
.global pc32_dirty
pc32_dirty:
    b pc32
