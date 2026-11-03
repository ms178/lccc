.text
.global _start
_start:
    # The psABI entry contract: gp holds __global_pointer$ before any code
    # that the linker may have relaxed to gp-relative small-data accesses.
    .option push
    .option norelax
    lla gp, __global_pointer$
    .option pop
    call test
    li a7, 93
    ecall
