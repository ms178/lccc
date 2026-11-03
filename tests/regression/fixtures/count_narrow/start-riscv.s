.text
.global _start
_start:
    call test
    li a7, 93
    ecall
