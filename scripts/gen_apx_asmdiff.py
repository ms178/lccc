#!/usr/bin/env python3
"""Generate an APX asmdiff corpus covering REX2, EGPR, NDD, {nf}/{evex}/{rex2}.

The cases that beat GAS on size (zero-extending `movl` of an unsigned 32-bit
immediate into %r16..%r31 instead of 11-byte REX2-movabs) are tagged
`betterok`. Everything else is expected to match GNU as 2.47 byte-for-byte.

Usage:
    python3 scripts/gen_apx_asmdiff.py > tests/asm-diff/apx.casefile
"""
from __future__ import annotations

ALU = ("add", "or", "adc", "sbb", "and", "sub", "xor")
EGPRS = [f"r{n}" for n in range(16, 32)]


def main() -> None:
    lines: list[str] = []

    def case(name: str, body: list[str], betterok: bool = False) -> None:
        tag = f";;; {name} betterok" if betterok else f";;; {name}"
        lines.append(tag)
        lines.append(".text")
        lines.extend(body)
        lines.append("")

    rex2 = [
        "addq %rax, %r16",
        "addq %r16, %rax",
        "addq %r16, %r17",
        "addq %r24, %rax",
        "addq (%r16), %rax",
        "addq %rax, (%r16)",
        "leaq (%rax), %r16",
        "leaq (%r16,%r17,4), %r18",
        "pushq %r16",
        "popq %r16",
        "incq %r16",
        "addq $1, %r16",
        "imulq %r16, %rax",
        "cmovzq %rax, %r16",
        "setzb %r16b",
        "bswapq %r16",
        "xchgq %r16, %rax",
        "lock addq %rax, (%r16)",
        "{rex2} addq %rax, %rbx",
        "jmp *%r16",
        "call *%r16",
    ]
    case("apx_rex2", rex2)

    ndd = [
        "addq %rcx, %rdx, %r16",
        "addq %rcx, %rdx, %r17",
        "addq %rcx, %rdx, %r8",
        "addq %rcx, %rdx, %r24",
        *[f"{op}q %rcx, %rdx, %r16" for op in ALU],
        "addl %ecx, %edx, %r16d",
        "addq %rcx, (%rax), %r16",
        "addq (%rax), %rcx, %r16",
        "addq $1, %rax, %r16",
        "{nf} addq %rcx, %rdx, %r16",
        "{evex} addq %rax, %rbx",
        "{nf} addq %rax, %rbx",
        "{nf}{evex} addq %rax, %rbx",
        "{evex} addq %r16, %rax",
        "{evex} addq %rax, %r16",
        "{evex} addq %r24, %rax",
        "{evex} addq %rax, %r24",
        "{evex} addl %eax, %ebx",
        "{evex} addw %ax, %bx",
        "{nf} addq $1, %rax",
        "{nf} xorq %rax, %rax",
        "incq %rax, %r16",
        "{nf} incq %rax",
        "notq %rax, %r16",
        "shlq $1, %rax, %r16",
        "shlq %cl, %rax, %r16",
        "cmovzq %rax, %rbx, %r16",
        "imulq %rcx, %rdx, %r16",
    ]
    case("apx_evex_ndd", ndd)

    # Beats GAS: unsigned 32-bit immediates into an EGPR are a 7-byte
    # zero-extending movl, not 11-byte REX2-movabs.
    case(
        "apx_mov_imm",
        [
            "movq $0xffffffff, %r16",
            "movl $0xffffffff, %r16d",
            "movq $0x80000000, %r16",
        ],
        betterok=True,
    )

    # 0F38 promotions: EGPR / `{evex}` → APX EVEX map-4 with remapped opcodes
    # (crc32 keeps F0/F1; adcx/adox F6→66; movbe F0/F1→60/61). Not REX2.
    case(
        "apx_map4_0f38",
        [
            "crc32q %rax, %r16",
            "crc32q %r16, %rax",
            "crc32l %eax, %r16d",
            "crc32b %al, %r16d",
            "crc32w %ax, %r16d",
            "crc32q (%r16), %rax",
            "{evex} crc32q %rax, %rbx",
            "adcxq %r16, %rax",
            "adcxq %rax, %r16",
            "adoxq %r16, %rax",
            "adoxq %rax, %r16",
            "adcxl %eax, %r16d",
            "movbeq (%r16), %rax",
            "movbeq %rax, (%r16)",
            "movbel (%r16), %eax",
            "movbew (%r16), %ax",
            "{evex} adcxq %rax, %rbx",
            "{evex} movbeq (%rax), %rbx",
        ],
    )

    # BMI/BMI2: no-EGPR stays VEX; EGPR/{nf}/{evex} promote to EVEX mmm=2
    # (rorx mmm=3). NF is legal for andn/bextr/bzhi/bls* only.
    case(
        "apx_bmi2",
        [
            "andnq %rcx, %rdx, %rax",
            "shlxq %rcx, %rdx, %rax",
            "shlxq %rcx, %rdx, %r16",
            "{evex} shlxq %rcx, %rdx, %rax",
            "{nf} andnq %rcx, %rdx, %rax",
            "{nf} andnq %rcx, %rdx, %r16",
            "{nf} bextrq %rcx, %rdx, %rax",
            "{nf} bzhiq %rcx, %rdx, %rax",
            "{nf} blsmskq %rax, %rcx",
            "{nf} blsiq %rax, %r16",
            "rorxq $1, %rax, %r16",
            "{evex} rorxq $1, %rax, %rcx",
        ],
    )

    case(
        "apx_push2_jmpabs",
        [
            "pushw %r16w",
            "pushp %rax",
            "pushp %r16",
            "pushp %rsp",
            "popp %rax",
            "push2 %rax, %rcx",
            "push2 %r16, %rax",
            "push2 %rax, %r16",
            "push2 %r16, %r16",
            "pop2 %rax, %rcx",
            "push2p %rax, %rcx",
            "pop2p %rax, %rcx",
            "jmpabs $0x123456789abcdef0",
        ],
    )

    case(
        "apx_ccmp_ctest",
        [
            "ccmpneq %rax, %rbx",
            "ccmpne %rax, %rbx",
            "ccmpneq {dfv=cf} %rax, %rbx",
            "ccmpneq {dfv=of,sf,zf,cf} %rax, %rbx",
            "ctestneq %rax, %rbx",
            "ctesteb $0x80, %al",
            "ctesteq $0, %rax",
            "ccmpneq (%r16), %rax",
            "ccmpneq %rax, (%r16)",
            "ccmpneq $1, (%r16)",
            "ctestneb %al, %bl",
            "ccmpnew %ax, %bx",
            "ccmpnew %ax, %r16w",
            "ccmpnel %eax, %r16d",
            "ccmpneb %al, %r16b",
            "ccmpneq %rax, (%rax,%r16,4)",
        ],
    )

    case(
        "apx_nf_map4_extra",
        [
            "{evex} setzb %al",
            "{evex} setzb %r16b",
            "{nf} lzcntq %rax, %rcx",
            "{nf} tzcntq %rax, %rcx",
            "{nf} popcntq %rax, %rcx",
            "{nf} shldq $1, %rax, %rcx",
            "{nf} shrdq $1, %rax, %rcx",
            "{nf} shldq %cl, %rax, %rcx",
            "{nf} imulq %rcx, %rax",
            "{evex} imulq %rcx, %rax",
            "{nf} mulq %rax",
            "{nf} divq %rax",
            "{nf} idivq %rax",
            "negq %rax, %r16",
            "{nf} negq %rax, %r16",
            "addq $1, %rax, %rcx",
            "imulq $2, %rax, %r16",
        ],
    )

    case(
        "apx_imulzu_cfcmov_ndd_shld",
        [
            "imulzuw $2, %ax, %cx",
            "imulzu $2, %ax, %cx",
            "imulzuw $2, %ax",
            "imulzuw $2, (%rax), %cx",
            "imulzuw $2, %ax, %r16w",
            "{nf} imulzuw $2, %ax, %cx",
            "setzuz %al",
            "setzub %al",
            "setzune %al",
            "setzuz %r16b",
            "cfcmovzq %rax, %rbx",
            "cfcmovzw %ax, %bx",
            "cfcmovzl %eax, %ebx",
            "cfcmovzq (%rax), %rbx",
            "cfcmovzq %rax, (%rbx)",
            "cfcmovzq %rax, %rbx, %r16",
            "cfcmovneq %rcx, %rdx, %r8",
            "shldq $1, %rax, %rcx, %r16",
            "shldq %cl, %rax, %rcx, %r16",
            "{nf} shldq $1, %rax, %rcx, %r16",
            "shrdq $1, %rax, %rcx, %r16",
            "shldw $1, %ax, %cx, %r16w",
            "shldl $1, %eax, %ecx, %r16d",
        ],
    )

    case(
        "apx_rex2_leftovers",
        [
            "rdpid %r16",
            "wrfsbase %r16",
            "rdfsbase %r16",
            "rdgsbase %r31",
            "sldt %r16",
            "smsw %r16",
            "lmsw %r16w",
            "fxsaveq (%r16)",
            "fxrstorq (%r16)",
            "movq %r16, %xmm0",
            "movq %xmm0, %r16",
            "cvtsi2sdq %r16, %xmm0",
            "movq %fs, %r16",
            "movq %r16, %fs",
            "movq %mm0, %r16",
            "mov %cr0, %r16",
            "xaddq %rax, %r16",
            "cmpxchgq %rax, %r16",
            "lock xaddq %rax, (%r16)",
        ],
    )

    print("\n".join(lines).rstrip() + "\n")


if __name__ == "__main__":
    main()
