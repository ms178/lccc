#!/usr/bin/env python3
"""Exact C integer-semantics emulator for oracle-free compiler stress tests.

The emulator models the x86-64 SysV / LP64 integer model (char 8, short 16,
int 32, long 64, long long 64, __int128 128).  Every operation follows the
C17 rules to the letter:

* integer promotions (C17 6.3.1.1p2) — rank below `int` promotes to `int`;
* usual arithmetic conversions (6.3.1.8) for binary operators;
* value-preserving conversions to unsigned types wrap modulo 2^N (6.3.1.3p2);
* conversions to narrower *signed* types wrap two's-complement — this is
  implementation-defined in C17 but every mainstream compiler (GCC, Clang,
  ICX, LCCC) defines it as modulo wrap, so a differential expectation of the
  same result is justified;
* signed overflow in `+ - *`, division by zero, `INT_MIN / -1`, shift counts
  outside `[0, width)`, left shifts of negative values, and left shifts whose
  result does not fit are reported as **UB** by raising `Undefined` — the
  generators discard such cases so that every emitted program has a single,
  defined answer that GCC, Clang and LCCC must all agree on.

The point of an oracle-free emulator is independence: a differential fuzzer
that compares LCCC only against GCC cannot see a bug shared by both, and it
cannot say *which* value is right.  The emulator computes the answer from the
standard, so a mismatch is attributable and a reducer is unnecessary — each
case's expected value is compiled into the test program itself.
"""
from __future__ import annotations

from dataclasses import dataclass


class Undefined(Exception):
    """Raised when an operation would have undefined behaviour in C."""


@dataclass(frozen=True)
class IntTy:
    """A C integer type in the LP64 model."""

    name: str      # C spelling (from <stdint.h> where possible)
    bits: int
    signed: bool
    rank: int      # conversion rank (char=1, short=2, int=3, long=4, int128=5)

    @property
    def minv(self) -> int:
        return -(1 << (self.bits - 1)) if self.signed else 0

    @property
    def maxv(self) -> int:
        return (1 << (self.bits - 1)) - 1 if self.signed else (1 << self.bits) - 1

    @property
    def mask(self) -> int:
        return (1 << self.bits) - 1

    def wrap(self, v: int) -> int:
        """Convert an arbitrary Python int to this type (modulo 2^bits)."""
        v &= self.mask
        if self.signed and v > self.maxv:
            v -= 1 << self.bits
        return v

    def fits(self, v: int) -> bool:
        return self.minv <= v <= self.maxv

    def literal(self, v: int) -> str:
        """Spell `v` (already a value of this type) as a C expression."""
        if not self.fits(v):
            raise ValueError(f"{v} does not fit {self.name}")
        if self.bits == 128:
            u = v & ((1 << 128) - 1)
            hi = (u >> 64) & ((1 << 64) - 1)
            lo = u & ((1 << 64) - 1)
            base = f"(((unsigned __int128)0x{hi:x}ull << 64) | (unsigned __int128)0x{lo:x}ull)"
            if self.signed:
                return f"((__int128)({base}))"
            return f"({base})"
        if self.signed:
            if v == self.minv and self.bits >= 32:
                # INT_MIN cannot be spelled as a decimal literal of its own type.
                suf = "ll" if self.bits == 64 else ""
                return f"(({self.name})(-{-(v + 1)}{suf} - 1))"
            suf = "ll" if self.bits == 64 else ""
            return f"(({self.name}){v}{suf})"
        suf = "ull" if self.bits == 64 else "u"
        return f"(({self.name}){v}{suf})"


I8 = IntTy("int8_t", 8, True, 1)
U8 = IntTy("uint8_t", 8, False, 1)
I16 = IntTy("int16_t", 16, True, 2)
U16 = IntTy("uint16_t", 16, False, 2)
I32 = IntTy("int32_t", 32, True, 3)
U32 = IntTy("uint32_t", 32, False, 3)
I64 = IntTy("int64_t", 64, True, 4)
U64 = IntTy("uint64_t", 64, False, 4)
I128 = IntTy("__int128", 128, True, 5)
U128 = IntTy("unsigned __int128", 128, False, 5)
BOOL = IntTy("_Bool", 1, False, 0)

STD_TYPES = (I8, U8, I16, U16, I32, U32, I64, U64)
ALL_TYPES = STD_TYPES + (I128, U128)


def promote(t: IntTy) -> IntTy:
    """Integer promotion: anything of rank below int becomes int."""
    if t.rank < I32.rank:
        return I32
    return t


def unsigned_of(t: IntTy) -> IntTy:
    for u in (U8, U16, U32, U64, U128):
        if u.bits == t.bits:
            return u
    raise ValueError(t)


def signed_of(t: IntTy) -> IntTy:
    for s in (I8, I16, I32, I64, I128):
        if s.bits == t.bits:
            return s
    raise ValueError(t)


def usual_arith(a: IntTy, b: IntTy) -> IntTy:
    """Usual arithmetic conversions on two integer types (promotes first)."""
    a, b = promote(a), promote(b)
    if a == b:
        return a
    if a.signed == b.signed:
        return a if a.rank >= b.rank else b
    u, s = (a, b) if not a.signed else (b, a)
    if u.rank >= s.rank:
        return u
    # signed has higher rank; can it represent all values of the unsigned type?
    if s.bits > u.bits:
        return s
    return unsigned_of(s)


def convert(v: int, frm: IntTy, to: IntTy) -> int:
    """C conversion of value `v` of type `frm` to type `to`."""
    del frm  # the source type does not affect the numeric result
    if to is BOOL:
        return 1 if v != 0 else 0
    return to.wrap(v)


def binop(op: str, a: int, ta: IntTy, b: int, tb: IntTy) -> tuple[int, IntTy]:
    """Evaluate `a op b` with exact C semantics.  Returns (value, type)."""
    if op in ("<<", ">>"):
        rt = promote(ta)
        av = convert(a, ta, rt)
        pb = promote(tb)
        bv = convert(b, tb, pb)
        if bv < 0 or bv >= rt.bits:
            raise Undefined(f"shift count {bv} out of range for {rt.name}")
        if op == "<<":
            if rt.signed and av < 0:
                raise Undefined("left shift of negative value")
            r = av << bv
            if rt.signed and r > rt.maxv:
                raise Undefined("left shift overflow")
            return rt.wrap(r), rt
        # Right shift of a negative signed value is implementation-defined;
        # it is arithmetic on every compiler under test (GCC documents it).
        return rt.wrap(av >> bv), rt

    if op in ("<", "<=", ">", ">=", "==", "!="):
        rt = usual_arith(ta, tb)
        av, bv = convert(a, ta, rt), convert(b, tb, rt)
        r = {"<": av < bv, "<=": av <= bv, ">": av > bv, ">=": av >= bv,
             "==": av == bv, "!=": av != bv}[op]
        return (1 if r else 0), I32

    if op in ("&&", "||"):
        r = (a != 0 and b != 0) if op == "&&" else (a != 0 or b != 0)
        return (1 if r else 0), I32

    rt = usual_arith(ta, tb)
    av, bv = convert(a, ta, rt), convert(b, tb, rt)
    if op == "+":
        r = av + bv
    elif op == "-":
        r = av - bv
    elif op == "*":
        r = av * bv
    elif op in ("/", "%"):
        if bv == 0:
            raise Undefined("division by zero")
        if rt.signed and av == rt.minv and bv == -1:
            raise Undefined("INT_MIN / -1")
        q = abs(av) // abs(bv)
        if (av < 0) != (bv < 0):
            q = -q
        r = q if op == "/" else av - q * bv
        return r, rt  # always representable
    elif op == "&":
        return rt.wrap(av & bv), rt
    elif op == "|":
        return rt.wrap(av | bv), rt
    elif op == "^":
        return rt.wrap(av ^ bv), rt
    else:
        raise ValueError(op)
    if rt.signed and not rt.fits(r):
        raise Undefined(f"signed overflow in {op}")
    return rt.wrap(r), rt


def unop(op: str, a: int, ta: IntTy) -> tuple[int, IntTy]:
    rt = promote(ta)
    av = convert(a, ta, rt)
    if op == "-":
        if rt.signed and av == rt.minv:
            raise Undefined("negation overflow")
        return rt.wrap(-av), rt
    if op == "~":
        return rt.wrap(~av), rt
    if op == "!":
        return (1 if av == 0 else 0), I32
    if op == "+":
        return av, rt
    raise ValueError(op)


# ---------------------------------------------------------------------------
# Builtins (GCC family).  All operate on the value already converted to the
# builtin's parameter type; the caller supplies that type.
# ---------------------------------------------------------------------------

def popcount(v: int, t: IntTy) -> int:
    return bin(v & t.mask).count("1")


def clz(v: int, t: IntTy) -> int:
    v &= t.mask
    if v == 0:
        raise Undefined("__builtin_clz(0)")
    return t.bits - v.bit_length()


def ctz(v: int, t: IntTy) -> int:
    v &= t.mask
    if v == 0:
        raise Undefined("__builtin_ctz(0)")
    return (v & -v).bit_length() - 1


def ffs(v: int, t: IntTy) -> int:
    v &= t.mask
    return 0 if v == 0 else ctz(v, t) + 1


def parity(v: int, t: IntTy) -> int:
    return popcount(v, t) & 1


def bswap(v: int, t: IntTy) -> int:
    n = t.bits // 8
    return int.from_bytes((v & t.mask).to_bytes(n, "little"), "big")


def rotl(v: int, n: int, t: IntTy) -> int:
    n %= t.bits
    v &= t.mask
    return ((v << n) | (v >> (t.bits - n))) & t.mask


def overflow_builtin(op: str, a: int, b: int, rt: IntTy) -> tuple[int, int]:
    """__builtin_{add,sub,mul}_overflow on mathematically exact operands.

    Returns (stored result, overflow flag).  The stored result is the exact
    result wrapped to the result type; the flag is set iff the exact result
    is not representable.
    """
    r = {"add": a + b, "sub": a - b, "mul": a * b}[op]
    return rt.wrap(r), (0 if rt.fits(r) else 1)


def bitfield_store(v: int, width: int, signed: bool) -> int:
    """Value observed after storing `v` into a bit-field of `width` bits."""
    v &= (1 << width) - 1
    if signed and width > 0 and v >> (width - 1):
        v -= 1 << width
    return v


def self_test() -> None:
    assert usual_arith(I32, U32) is U32
    assert usual_arith(I64, U32) is I64
    assert usual_arith(I64, U64) is U64
    assert usual_arith(I8, U8) is I32
    assert usual_arith(U16, I16) is I32
    assert binop("+", 65535, U16, 1, U16) == (65536, I32)
    assert binop("*", 255, U8, 255, U8) == (65025, I32)
    assert binop("-", 0, U32, 1, U32) == (0xFFFFFFFF, U32)
    assert binop("/", -7, I32, 2, I32) == (-3, I32)
    assert binop("%", -7, I32, 2, I32) == (-1, I32)
    assert binop("%", 7, I32, -2, I32) == (1, I32)
    assert binop("<", -1, I32, 0, U32) == (0, I32)          # -1 converts to UINT_MAX
    assert binop("<", -1, I64, 0, U32) == (1, I32)          # long can hold uint
    assert binop(">>", -128, I8, 7, I32) == (-1, I32)
    assert convert(-1, I32, U8) == 255
    assert convert(200, U8, I8) == -56
    assert I8.literal(-128) == "((int8_t)-128)"
    assert I32.literal(-2147483648) == "((int32_t)(-2147483647 - 1))"
    assert bswap(0x01020304, U32) == 0x04030201
    assert rotl(0x80000001, 1, U32) == 0x00000003
    assert overflow_builtin("add", 2147483647, 1, I32) == (-2147483648, 1)
    assert bitfield_store(5, 3, True) == -3
    for bad in (lambda: binop("+", 2147483647, I32, 1, I32),
                lambda: binop("<<", 1, I32, 32, I32),
                lambda: binop("/", 1, I32, 0, I32),
                lambda: binop("/", -2147483648, I32, -1, I32),
                lambda: clz(0, U32)):
        try:
            bad()
        except Undefined:
            pass
        else:
            raise AssertionError("expected Undefined")


if __name__ == "__main__":
    self_test()
    print("cemu self-test OK")
