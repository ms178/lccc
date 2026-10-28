"""Recognize how the vectorizer divides a 64-bit byte IV back into an element
index, so shape gates stop pinning one BMI2-free mnemonic.

WHY THIS IS SHARED
==================
Two independent shape gates read emitted assembly and ask the same question
("is there a 64-bit divide of the byte IV that the scalar remainder starts
from?"):

  * tests/regression/check_matmul_negative_bound.sh -- the matmul/FMA arm, and
  * tests/regression/check_vector_remainder_codegen.sh -- the matmul arm plus
    the four reduction kernels of tests/benchmark/patterns/simd_fp_oracle.c.

Each used to spell the answer as "a `shr[bwlq]` immediate". That is one of two
legal lowerings. With BMI2 available (`-march=x86-64-v3`), codegen rewrites
`x >> 3` into `rorxq $3, %r, %r` whenever the shift's single consumer discards
the top three bits (src/backend/x86/codegen/prologue.rs, `rorx_masked_shifts`:
a rotate only differs from the shift in the bits it rotates in at the top).
`rorx` breaks the shift's false dependency on the flags and needs no third
register, so the emitted rotate is the better sequence -- but a gate that only
knows the shift mnemonic reports a regression for an improvement.

The equivalence is accepted only for the shapes this module can prove:

  * a 64-bit form (`shrq` or `rorxq`): a 32-bit divide (`rorxl`/`shrl`) of a
    byte IV can wrap the trip count of an I32 bound, which is exactly the
    negative-bound hazard the matmul gate exists to catch. The remainder gate
    predates that hazard and still accepts the narrow mnemonic spellings it was
    written against (`require_64bit=False`), so tightening there is left to a
    change that can measure the reduction kernels;
  * an immediate whose decimal spelling ENDS at the amount (`$3`, never `$31`
    matching a `$3` prefix -- the old regexes accepted that);
  * for the narrowing form, the `movslq` must re-extend the 32-bit alias of the
    very register the divide wrote, so a divide of one value and a narrowing of
    a different value can never satisfy the gate.

Both gates import this module instead of carrying their own copy of the
pattern, because two copies drift: the negative-bound gate grew the BMI2 form
first and the remainder gate then failed on identical, correct output.
"""

import re

_GPR64_TO_GPR32 = {
    "rax": "eax",
    "rbx": "ebx",
    "rcx": "ecx",
    "rdx": "edx",
    "rsi": "esi",
    "rdi": "edi",
    "rbp": "ebp",
    "rsp": "esp",
    **{f"r{n}": f"r{n}d" for n in range(8, 16)},
}

# Either a two-operand shift (`shrq $3, %rsi`) or the three-operand BMI2
# rotate (`rorxq $3, %rsi, %rsi`).
_DIVIDE = (
    r"(?:(?P<shr>shr[bwlq]?)[ \t]+\$(?P<shr_amt>\d+)(?:,[ \t]*(?P<shr_dst>%[a-z0-9]+))?"
    r"|(?P<rorx>rorxq)[ \t]+\$(?P<rorx_amt>\d+),[ \t]*(?P<rorx_src>%[a-z0-9]+),"
    r"[ \t]*(?P<rorx_dst>%[a-z0-9]+))"
)
_DIVIDE_RE = re.compile(rf"(?m)^[ \t]*{_DIVIDE}[ \t]*$")
_NARROW_RE = re.compile(rf"(?m)^[ \t]*{_DIVIDE}\n[ \t]*movslq[ \t]+%(?P<narrow>[a-z0-9]+),")


def _is_64bit(match, require_64bit):
    if not require_64bit:
        return True
    return match.group("rorx") is not None or match.group("shr") == "shrq"


def _amount(match):
    return int(match.group("shr_amt") or match.group("rorx_amt"))


def divides_byte_iv(body, amount, *, require_64bit=False):
    """True when `body` divides a byte IV by 2**amount.

    With `require_64bit` the divide must be a 64-bit instruction, which is the
    form that cannot wrap an I32/I64 byte bound.
    """
    return any(
        _amount(match) == amount and _is_64bit(match, require_64bit)
        for match in _DIVIDE_RE.finditer(body)
    )


def narrows_after_divide(body, amount, *, require_64bit=True):
    """True when the element index is narrowed AFTER a 64-bit divide.

    The `movslq` must consume the 32-bit alias of the register the divide
    wrote: dividing the 64-bit byte IV and then re-extending it is the correct
    order, while truncating the byte IV to 32 bits first can wrap the bound.
    """
    for match in _NARROW_RE.finditer(body):
        if _amount(match) != amount or not _is_64bit(match, require_64bit):
            continue
        dst = match.group("shr_dst") or match.group("rorx_dst")
        if dst is None:
            continue
        alias = _GPR64_TO_GPR32.get(dst.lstrip("%"))
        if alias is not None and alias == match.group("narrow"):
            return True
    return False
