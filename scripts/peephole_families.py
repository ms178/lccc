#!/usr/bin/env python3
"""Peephole-targeted stress families for LCCC (oracle-free, self-checking).

The x86 backend runs a ~32 kLOC text-level peephole optimizer over the
assembly produced by the accumulator code generator
(``src/backend/x86/codegen/peephole``).  Its 70+ sub-passes reason about
flags, partial registers, stack-slot contents, call clobbers, frames and
tail positions purely from assembly text.  Every one of those reasoning
domains has a classic soundness hazard, and the generic families in
``families.py`` reach them only by luck.  Each family below is built around
one hazard class and biases its inputs so the hazard is *observable* (a wrong
flag, a stale upper half, a forwarded-but-overwritten slot ...) rather than
merely present.

Families (all register into ``families.FAMILIES`` on import):

  flags      arithmetic whose x86 lowering leaves flags in a non-obvious
             state (lea, inc/dec, shift-by-variable incl. count 0, rotates,
             imul, bswap/popcnt/tzcnt) immediately consumed by compares,
             selects and add-overflow idioms          -> flag_peepholes,
             compare_branch, setcc_cmov, xor_move_fold, self_test, load_test_cmp
  narrow     cast chains through 8/16/32/64-bit types of alternating
             signedness with narrow arithmetic in between; stale upper bits
             become the return value                   -> redundant_ext,
             self_zext, narrow_copy_fold, narrow_imm, fuse_movq_ext, ext_relay
  memalias   byte/half/word/quad stores and loads at RUNTIME offsets into one
             buffer (partial overlaps, mixed widths, volatile, memmove) so the
             text peephole cannot prove slots disjoint  -> store_forwarding,
             memory_fold, dead_stores, dead_writes, spill_deref, load_reuse
  tailcall   ``return g(...)`` shapes where converting call+epilogue+ret into
             jmp is illegal or needs care: escaping locals, VLAs, sret, more
             stack args than the caller owns, implicit narrowing of the
             return value, variadic and indirect callees -> tail_call
  framecall  many values live across opaque calls (callee-saved pressure),
             VLAs (frame pointer required), inline asm clobbering %rbx/%r12..
             -> callee_saves, frame_compact, push_pop, copy_propagation
  speculate  ``c ? *p : d`` / ``c ? a / b : d`` / ``c ? arr[i] : d`` where the
             untaken arm traps (NULL, zero divisor, INT_MIN/-1, wild index);
             any if-conversion to cmov/unconditional load is a crash
             -> setcc_cmov, if_convert, identical_blocks
  shiftchain cascaded shifts whose total count reaches or exceeds the width,
             shift-by-zero rotates, inc/dec chains, count masking (& and %)
             -> cascaded_shifts, inc_chain, rotate_idiom, narrow_imm

All expectations are computed here from the language definition (``cemu``)
so a shared LCCC/GCC misunderstanding cannot hide a bug.  No case contains
undefined behaviour on its executed path: signed arithmetic is done in
unsigned types and reinterpreted, shift counts are masked, and every trap
arm is guarded by the condition that selects it.

Run them through the generic lab (``run_stress.py`` imports this module) or
through the peephole-isolating lab (``peephole_lab.py``), which additionally
bisects any failure down to the responsible peephole sub-pass.
"""
from __future__ import annotations

import random

import cemu
from cemu import (I8, I16, I32, I64, U8, U16, U32, U64, IntTy, STD_TYPES,
                  Undefined, binop, convert, promote)
import families
from families import Case, Param, rand_value

MASK64 = (1 << 64) - 1


def _u(t: IntTy) -> IntTy:
    return cemu.unsigned_of(t)


def _s(t: IntTy) -> IntTy:
    return cemu.signed_of(t)


def _sv(v: int, t: IntTy) -> int:
    """Two's-complement signed view of an unsigned value of width t.bits."""
    v &= t.mask
    return v - (1 << t.bits) if v >> (t.bits - 1) else v


# ---------------------------------------------------------------------------
# flags — flag producers with surprising x86 semantics feeding flag consumers
# ---------------------------------------------------------------------------

def _flag_step(rng: random.Random, ut: IntTy, a: int, b: int) -> tuple[str, int, str]:
    """Return (C expression over a,b evaluated in the promoted type, value, kind).

    Every expression is written so that its value, truncated to ``ut``, is the
    recorded one; the caller wraps it in ``(ut)``.
    """
    W = ut.bits
    kind = rng.choice(["lea", "lea3", "inc", "dec", "shl_var", "shr_var", "sar_var",
                       "neg", "not", "and", "xor", "or", "imul", "sub", "rotl", "rotr",
                       "bswap", "popcnt", "ctz", "adc_idiom", "sbb_idiom", "andn", "blsr",
                       "mul_const", "shl_const", "shr_const"])
    m = ut.mask
    if kind == "lea":
        s = rng.choice([1, 2, 4, 8])
        k = rng.choice([0, 1, 7, 8, 127, 128, 255, 256, 4096])
        return f"(a + b * {s}u + {k}u)", (a + b * s + k) & m, kind
    if kind == "lea3":
        return "(a + a * 2u)", (a * 3) & m, kind
    if kind == "inc":
        return "(a + 1u)", (a + 1) & m, kind
    if kind == "dec":
        return "(a - 1u)", (a - 1) & m, kind
    if kind == "shl_var":
        # count may be 0: shift-by-zero leaves flags UNCHANGED on x86.
        return f"(a << (b & {W - 1}))", (a << (b & (W - 1))) & m, kind
    if kind == "shr_var":
        return f"(a >> (b & {W - 1}))", (a >> (b & (W - 1))) & m, kind
    if kind == "sar_var":
        st = _s(ut)
        return (f"(({ut.name})((({st.name})a) >> (b & {W - 1})))",
                (_sv(a, ut) >> (b & (W - 1))) & m, kind)
    if kind == "neg":
        return "(0u - a)", (-a) & m, kind
    if kind == "not":
        return "(~a)", (~a) & m, kind
    if kind == "and":
        return "(a & b)", a & b, kind
    if kind == "xor":
        return "(a ^ b)", a ^ b, kind
    if kind == "or":
        return "(a | b)", a | b, kind
    if kind == "imul":
        return "(a * b)", (a * b) & m, kind
    if kind == "sub":
        return "(a - b)", (a - b) & m, kind
    if kind == "rotl":
        c = b & (W - 1)
        v = ((a << c) | (a >> ((-c) & (W - 1)))) & m if c else a
        return f"((a << (b & {W - 1})) | (a >> ((0u - b) & {W - 1})))", v, kind
    if kind == "rotr":
        c = b & (W - 1)
        v = ((a >> c) | (a << ((-c) & (W - 1)))) & m if c else a
        return f"((a >> (b & {W - 1})) | (a << ((0u - b) & {W - 1})))", v, kind
    if kind == "bswap":
        if W < 16:
            return "(a)", a, "id"
        fn = {16: "__builtin_bswap16", 32: "__builtin_bswap32", 64: "__builtin_bswap64"}[W]
        return f"{fn}(a)", cemu.bswap(a, ut), kind
    if kind == "popcnt":
        fn = "__builtin_popcountll" if W == 64 else "__builtin_popcount"
        return f"(({ut.name}){fn}(a))", bin(a).count("1"), kind
    if kind == "ctz":
        fn = "__builtin_ctzll" if W == 64 else "__builtin_ctz"
        av = a | 1
        return f"(({ut.name}){fn}(a | 1u))", cemu.ctz(av, ut), kind
    if kind == "adc_idiom":
        s = a + b
        carry = 1 if s > m else 0
        return f"((a + b) + (({ut.name})(a + b) < a))", (s + carry) & m, kind
    if kind == "sbb_idiom":
        borrow = 1 if a < b else 0
        return "((a - b) - (a < b))", (a - b - borrow) & m, kind
    if kind == "andn":
        return "(~a & b)", (~a) & b & m, kind
    if kind == "blsr":
        return "(a & (a - 1u))", a & ((a - 1) & m), kind
    if kind == "mul_const":
        k = rng.choice([3, 5, 6, 7, 9, 10, 12, 15, 17, 24, 25, 36, 100, 255, 257])
        return f"(a * {k}u)", (a * k) & m, kind
    if kind == "shl_const":
        k = rng.randrange(0, W)
        return f"(a << {k})", (a << k) & m, kind
    k = rng.randrange(0, W)
    return f"(a >> {k})", (a >> k) & m, "shr_const"


def _flag_cond(rng: random.Random, ut: IntTy, r: int, a: int, b: int) -> tuple[str, bool]:
    st = _s(ut)
    W = ut.bits
    kind = rng.choice(["eq0", "ne0", "lt0", "ge0", "ltb", "slta", "eqb", "gtk", "lek",
                       "carry", "borrow", "bit", "eq_narrow", "signbit_xor", "le0s", "gt0s"])
    rs = _sv(r, ut)
    if kind == "eq0":
        return "(r == 0)", r == 0
    if kind == "ne0":
        return "(r != 0)", r != 0
    if kind == "lt0":
        return f"((({st.name})r) < 0)", rs < 0
    if kind == "ge0":
        return f"((({st.name})r) >= 0)", rs >= 0
    if kind == "ltb":
        return "(r < b)", r < b
    if kind == "slta":
        return f"((({st.name})r) < (({st.name})a))", rs < _sv(a, ut)
    if kind == "eqb":
        return "(r == b)", r == b
    if kind == "gtk":
        k = rng.choice([0, 1, 2, 7, 127, 128, 255, 256, 32767, 65535]) & ut.mask
        return f"(r > {ut.literal(k)})", r > k
    if kind == "lek":
        k = rng.choice([0, 1, 3, 15, 16, 100, 254, 255]) & ut.mask
        return f"(r <= {ut.literal(k)})", r <= k
    if kind == "carry":
        return f"((({ut.name})(r + a)) < r)", (r + a) > ut.mask
    if kind == "borrow":
        return "(r < a)", r < a
    if kind == "bit":
        n = rng.randrange(0, W)
        return f"((r >> {n}) & 1u)", bool((r >> n) & 1)
    if kind == "eq_narrow":
        nt = rng.choice([U8, U16]) if W > 16 else U8
        return f"((({nt.name})r) == (({nt.name})a))", (r & nt.mask) == (a & nt.mask)
    if kind == "signbit_xor":
        return f"(((({st.name})({ut.name})(r ^ a)) < 0))", _sv(r ^ a, ut) < 0
    if kind == "le0s":
        return f"((({st.name})r) <= 0)", rs <= 0
    return f"((({st.name})r) > 0)", rs > 0


def gen_flags(rng: random.Random, count: int) -> list[Case]:
    out: list[Case] = []
    while len(out) < count:
        ut = _u(rng.choice(STD_TYPES))
        a = rand_value(rng, ut)
        b = rand_value(rng, ut)
        if rng.random() < 0.3:
            b = rng.choice([0, 1, ut.bits - 1, ut.bits, ut.bits + 1, 2 * ut.bits, ut.mask, ut.mask - 1]) & ut.mask
        sexpr, r, skind = _flag_step(rng, ut, a, b)
        lines = [f"{ut.name} a = p0, b = p1;", f"{ut.name} r = ({ut.name}){sexpr};"]
        acc = rng.choice([0, 1, 5, 0x55, 0x7f, ut.mask]) & ut.mask
        lines.append(f"{ut.name} acc = {ut.literal(acc)};")
        shape_desc = [skind]
        for _ in range(rng.randint(1, 4)):
            cexpr, cval = _flag_cond(rng, ut, r, a, b)
            use = rng.choice(["if_add", "if_xor", "sel_ab", "sel_k", "mask", "sum", "neg_mask", "sel_r0"])
            shape_desc.append(use)
            if use == "if_add":
                k = rng.choice([1, 3, 7, 13]) & ut.mask
                lines.append(f"if {cexpr} acc += {ut.literal(k)};")
                if cval:
                    acc = (acc + k) & ut.mask
            elif use == "if_xor":
                lines.append(f"if {cexpr} acc ^= r;")
                if cval:
                    acc ^= r
            elif use == "sel_ab":
                lines.append(f"acc += {cexpr} ? a : b;")
                acc = (acc + (a if cval else b)) & ut.mask
            elif use == "sel_k":
                k1 = rng.choice([0, 1, 2, 100]) & ut.mask
                k2 = rng.choice([0, 1, 5, 255]) & ut.mask
                lines.append(f"acc ^= {cexpr} ? {ut.literal(k1)} : {ut.literal(k2)};")
                acc ^= (k1 if cval else k2)
            elif use == "mask":
                lines.append(f"acc += ({ut.name})({cexpr} ? r : 0u);")
                acc = (acc + (r if cval else 0)) & ut.mask
            elif use == "sum":
                lines.append(f"acc += ({ut.name}){cexpr};")
                acc = (acc + (1 if cval else 0)) & ut.mask
            elif use == "neg_mask":
                lines.append(f"acc ^= ({ut.name})((0u - ({ut.name}){cexpr}) & a);")
                acc ^= (a if cval else 0)
            else:
                lines.append(f"acc += {cexpr} ? r : ({ut.name})0;")
                acc = (acc + (r if cval else 0)) & ut.mask
        lines.append(f"return ({ut.name})(acc + r);")
        expected = (acc + r) & ut.mask
        out.append(Case(f"flags_{len(out)}", "flags", ut,
                        [Param(ut.name, ut.literal(a), raw=a), Param(ut.name, ut.literal(b), raw=b)],
                        "\n    ".join(lines), expected, desc=" ".join(shape_desc)))
    return out


# ---------------------------------------------------------------------------
# narrow — cast chains with narrow arithmetic; stale upper bits are the result
# ---------------------------------------------------------------------------

NARROW_OPS = ["+", "-", "*", "^", "|", "&", "<<", ">>"]


def _narrow_one(rng: random.Random) -> Case:
    """One narrow-chain case; raises Undefined when a step is UB (caller retries)."""
    t0 = rng.choice(STD_TYPES)
    t1 = rng.choice(STD_TYPES)
    v0 = rand_value(rng, t0)
    v1 = rand_value(rng, t1)
    cur_ty, cur_val = t0, v0
    lines = [f"{t0.name} v0 = p0;", f"{t1.name} w = p1;"]
    chain: list[tuple[str, IntTy, int]] = [("v0", t0, v0)]
    steps = rng.randint(2, 6)
    desc = [t0.name]
    for i in range(steps):
        nt = rng.choice(STD_TYPES)
        op = rng.choice(NARROW_OPS)
        if op in ("<<", ">>"):
            pt = promote(cur_ty)
            k = rng.randrange(0, pt.bits)
            rhs_txt, rhs_val, rhs_ty = str(k), k, I32
            if op == "<<" and pt.signed:
                pv = convert(cur_val, cur_ty, pt)
                if pv < 0 or (pv << k) > pt.maxv:
                    op = ">>"
        elif rng.random() < 0.4:
            rhs_txt, rhs_val, rhs_ty = "w", v1, t1
        else:
            ct = rng.choice([I32, U32, I64, U64])
            cv = rng.choice([1, 2, 3, 7, 0x7f, 0x80, 0xff, 0x100, 0x7fff, 0x8000,
                             0xffff, 0x10000, 0x7fffffff, 0x80000000, 0xffffffff])
            cv = ct.wrap(cv) if rng.random() < 0.8 else ct.wrap(-cv)
            rhs_txt, rhs_val, rhs_ty = ct.literal(cv), cv, ct
        if rng.random() < 0.15:
            uop = rng.choice(["-", "~"])
            uval, uty = cemu.unop(uop, cur_val, cur_ty)
            val, vt = binop(op, uval, uty, rhs_val, rhs_ty)
            expr_txt = f"(({uop}v{i}) {op} {rhs_txt})"
        else:
            val, vt = binop(op, cur_val, cur_ty, rhs_val, rhs_ty)
            expr_txt = f"(v{i} {op} {rhs_txt})"
        nv = convert(val, vt, nt)
        lines.append(f"{nt.name} v{i + 1} = ({nt.name}){expr_txt};")
        cur_ty, cur_val = nt, nv
        chain.append((f"v{i + 1}", nt, nv))
        desc.append(f"{op}->{nt.name}")
    rt = rng.choice(STD_TYPES)
    mix = rng.choice(["plain", "add_w", "xor_v0", "sum_all", "cmp_chain"])
    if mix == "plain":
        fv = convert(cur_val, cur_ty, rt)
        lines.append(f"return ({rt.name})v{steps};")
    elif mix == "add_w":
        val, vt = binop("+", cur_val, cur_ty, v1, t1)
        fv = convert(val, vt, rt)
        lines.append(f"return ({rt.name})(v{steps} + w);")
    elif mix == "xor_v0":
        val, vt = binop("^", cur_val, cur_ty, v0, t0)
        fv = convert(val, vt, rt)
        lines.append(f"return ({rt.name})(v{steps} ^ v0);")
    elif mix == "sum_all":
        total = 0
        for _, ty_n, val_n in chain:
            total = (total + convert(val_n, ty_n, U64)) & MASK64
        fv = convert(total, U64, rt)
        lines.append(f"return ({rt.name})({' + '.join('(uint64_t)' + n for n, _, _ in chain)});")
    else:
        # mixed-signedness compares between intermediates: promotion decides
        acc = 0
        terms = []
        for j in range(1, len(chain)):
            n1, ty1, x1 = chain[j - 1]
            n2, ty2, x2 = chain[j]
            op = rng.choice(["<", "<=", "==", "!=", ">"])
            val, _ = binop(op, x1, ty1, x2, ty2)
            acc += val
            terms.append(f"({n1} {op} {n2})")
        fv = convert(acc, I32, rt)
        lines.append(f"return ({rt.name})({' + '.join(terms)});")
    return Case("narrow", "narrow", rt,
                [Param(t0.name, t0.literal(v0), raw=v0), Param(t1.name, t1.literal(v1), raw=v1)],
                "\n    ".join(lines), fv, desc=" ".join(desc) + f" {mix}")


def gen_narrow(rng: random.Random, count: int) -> list[Case]:
    out: list[Case] = []
    attempts = 0
    while len(out) < count and attempts < count * 60:
        attempts += 1
        try:
            c = _narrow_one(rng)
        except Undefined:
            continue
        c.name = f"narrow_{len(out)}"
        out.append(c)
    return out


# ---------------------------------------------------------------------------
# memalias — runtime-offset mixed-width stores/loads in one buffer
# ---------------------------------------------------------------------------

MEMALIAS_DECLS = [
    "static inline uint8_t  ma_ld8 (const uint8_t *p) { return *p; }",
    "static inline uint16_t ma_ld16(const uint8_t *p) { uint16_t v; __builtin_memcpy(&v, p, 2); return v; }",
    "static inline uint32_t ma_ld32(const uint8_t *p) { uint32_t v; __builtin_memcpy(&v, p, 4); return v; }",
    "static inline uint64_t ma_ld64(const uint8_t *p) { uint64_t v; __builtin_memcpy(&v, p, 8); return v; }",
    "static inline void ma_st8 (uint8_t *p, uint8_t  v) { *p = v; }",
    "static inline void ma_st16(uint8_t *p, uint16_t v) { __builtin_memcpy(p, &v, 2); }",
    "static inline void ma_st32(uint8_t *p, uint32_t v) { __builtin_memcpy(p, &v, 4); }",
    "static inline void ma_st64(uint8_t *p, uint64_t v) { __builtin_memcpy(p, &v, 8); }",
    "__attribute__((noinline)) static void ma_touch(uint8_t *p, unsigned n) { for (unsigned i = 0; i < n; i++) p[i] ^= (uint8_t)(i * 3 + 1); }",
]

_WTY = {1: U8, 2: U16, 4: U32, 8: U64}


def gen_memalias(rng: random.Random, count: int) -> list[Case]:
    out: list[Case] = []
    N = 40
    for idx in range(count):
        buf = bytearray((i * 37 + 11) & 0xff for i in range(N))
        offs = [rng.randint(0, 24) for _ in range(3)]
        if rng.random() < 0.5:                       # bias toward near-aliasing pointers
            offs[1] = max(0, min(24, offs[0] + rng.choice([-3, -2, -1, 0, 1, 2, 3, 4])))
        params = [Param("uint32_t", str(o), raw=o) for o in offs]
        lines = [f"_Alignas(16) uint8_t buf[{N}];",
                 f"for (int i = 0; i < {N}; i++) buf[i] = (uint8_t)(i * 37 + 11);",
                 "uint64_t s = 0;"]
        s = 0
        regs: dict[str, int] = {}
        nload = 0
        nops = 0

        def emit_load(w: int, k: int, c: int, extra: str = "") -> None:
            nonlocal s, nload
            addr = offs[k] + c + int(extra or 0)
            val = int.from_bytes(buf[addr:addr + w], "little")
            rn = f"l{nload}"
            nload += 1
            regs[rn] = val
            lines.append(f"uint64_t {rn} = ma_ld{8 * w}(buf + p{k} + {c}{' + (' + extra + ')' if extra else ''});")
            lines.append(f"s = s * 131u + {rn};")
            s = (s * 131 + val) & MASK64

        for step in range(rng.randint(3, 9)):
            kind = rng.choice(["st", "st", "ld", "ld", "rmw", "vol", "move", "touch", "stld"])
            w = rng.choice([1, 2, 4, 8])
            k = rng.randrange(3)
            c = rng.randint(0, 8)
            addr = offs[k] + c
            if addr + w > N:
                continue
            ty = _WTY[w]
            nops += 1
            if kind == "st":
                if regs and rng.random() < 0.5:
                    rn = rng.choice(list(regs))
                    val = (regs[rn] * 3 + 1) & ty.mask
                    vtxt = f"({ty.name})({rn} * 3u + 1u)"
                else:
                    val = rng.getrandbits(8 * w)
                    vtxt = ty.literal(val)
                buf[addr:addr + w] = val.to_bytes(w, "little")
                lines.append(f"ma_st{8 * w}(buf + p{k} + {c}, {vtxt});")
            elif kind == "ld":
                emit_load(w, k, c)
            elif kind == "rmw":
                val = int.from_bytes(buf[addr:addr + w], "little")
                nval = (val + 0x51 + step) & ty.mask
                buf[addr:addr + w] = nval.to_bytes(w, "little")
                lines.append(f"ma_st{8 * w}(buf + p{k} + {c}, ({ty.name})(ma_ld{8 * w}(buf + p{k} + {c}) + {0x51 + step}u));")
            elif kind == "vol":
                nval = buf[addr] ^ 0xA5
                buf[addr] = nval
                lines.append(f"*(volatile uint8_t *)(buf + p{k} + {c}) ^= 0xA5;")
                lines.append(f"s += *(volatile uint8_t *)(buf + p{k} + {c});")
                s = (s + nval) & MASK64
            elif kind == "move":
                n = rng.choice([1, 2, 3, 4, 5, 7, 8, 9, 12, 16])
                k2 = rng.randrange(3)
                c2 = rng.randint(0, 8)
                src = offs[k2] + c2
                if addr + n > N or src + n > N:
                    continue
                buf[addr:addr + n] = bytes(buf[src:src + n])
                lines.append(f"__builtin_memmove(buf + p{k} + {c}, buf + p{k2} + {c2}, {n});")
            elif kind == "touch":
                n = rng.choice([1, 2, 4, 8])
                if addr + n > N:
                    continue
                for i in range(n):
                    buf[addr + i] ^= (i * 3 + 1) & 0xff
                lines.append(f"ma_touch(buf + p{k} + {c}, {n});")
            else:  # store, then reload a different width at an overlapping address
                val = rng.getrandbits(8 * w)
                buf[addr:addr + w] = val.to_bytes(w, "little")
                lines.append(f"ma_st{8 * w}(buf + p{k} + {c}, {ty.literal(val)});")
                w2 = rng.choice([1, 2, 4, 8])
                d = rng.randint(-3, 3)
                if addr + d < offs[k] or addr + d + w2 > N:
                    d = 0
                emit_load(w2, k, c, str(d))
        lines.append(f"for (int i = 0; i < {N}; i++) s = s * 131u + buf[i];")
        for i in range(N):
            s = (s * 131 + buf[i]) & MASK64
        lines.append("return s;")
        out.append(Case(f"memalias_{idx}", "memalias", U64, params, "\n    ".join(lines), s,
                        desc=f"offs={offs} {nops} ops", decls=list(MEMALIAS_DECLS)))
    return out


# ---------------------------------------------------------------------------
# tailcall — return g(...) shapes where tail-jump conversion is illegal/subtle
# ---------------------------------------------------------------------------

def gen_tailcall(rng: random.Random, count: int) -> list[Case]:
    out: list[Case] = []
    for idx in range(count):
        kind = rng.choice(["narrow_ret", "escape_local", "escape_array", "stack_args_more",
                           "stack_args_shuffle", "sret", "sret_field", "fnptr", "varargs",
                           "fp_to_int", "int_to_fp", "live_across", "vla_value", "vla_ptr",
                           "narrow_param", "bool_ret"])
        x = rng.randint(-1000, 1000)
        y = rng.randint(-1000, 1000)
        pre = f"tc{idx}_"
        decls: list[str] = []
        params = [Param("int32_t", str(x), raw=x), Param("int32_t", str(y), raw=y)]
        rt: IntTy = I32
        if kind == "narrow_ret":
            rt = rng.choice([I8, U8, I16, U16])
            decls.append(f"__attribute__((noinline)) int {pre}g(int a, int b) {{ return (a * 7 + b) ^ 0x12340000; }}")
            body = f"return {pre}g(p0, p1);"
            v = convert(I32.wrap((x * 7 + y) ^ 0x12340000), I32, rt)
        elif kind == "escape_local":
            decls.append(f"__attribute__((noinline)) int {pre}g(int *p, int k) {{ int t = *p; *p = 0; return t * 3 + k; }}")
            body = f"int loc = p0 * 5 + 1; return {pre}g(&loc, p1);"
            v = I32.wrap((x * 5 + 1) * 3 + y)
        elif kind == "escape_array":
            decls.append(f"__attribute__((noinline)) int {pre}g(const int *p, int n) {{ int s = 0; for (int i = 0; i < n; i++) s += p[i] * (i + 1); return s; }}")
            body = f"int arr[6]; for (int i = 0; i < 6; i++) arr[i] = p0 + i * p1; return {pre}g(arr, 6);"
            v = I32.wrap(sum((x + i * y) * (i + 1) for i in range(6)))
        elif kind == "stack_args_more":
            decls.append(f"__attribute__((noinline)) long {pre}g(long a, long b, long c, long d, long e, long f, long g, long h, long i, long j) {{ return a - b + c * 2 - d + e * 3 - f + g - h * 5 + i + j * 7; }}")
            body = f"return (int32_t){pre}g(p0, p1, p0 + 1, p1 + 2, p0 + 3, p1 + 4, p0 + 5, p1 + 6, p0 + 7, p1 + 8);"
            a = [x, y, x + 1, y + 2, x + 3, y + 4, x + 5, y + 6, x + 7, y + 8]
            v = I32.wrap(a[0] - a[1] + a[2] * 2 - a[3] + a[4] * 3 - a[5] + a[6] - a[7] * 5 + a[8] + a[9] * 7)
        elif kind == "stack_args_shuffle":
            decls.append(f"__attribute__((noinline)) long {pre}g(long a, long b, long c, long d, long e, long f, long g, long h) {{ return a * 1 + b * 2 + c * 3 + d * 4 + e * 5 + f * 6 + g * 7 + h * 8; }}")
            decls.append(f"__attribute__((noinline)) long {pre}f(long a, long b, long c, long d, long e, long f, long g, long h) {{ return {pre}g(h, g, f, e, d, c, b, a); }}")
            body = f"return (int32_t){pre}f(p0, p1, p0 - 1, p1 - 2, p0 - 3, p1 - 4, p0 - 5, p1 - 6);"
            r = list(reversed([x, y, x - 1, y - 2, x - 3, y - 4, x - 5, y - 6]))
            v = I32.wrap(sum(r[i] * (i + 1) for i in range(8)))
        elif kind in ("sret", "sret_field"):
            decls.append(f"struct {pre}S {{ long a, b, c; }};")
            decls.append(f"__attribute__((noinline)) struct {pre}S {pre}g(int x, int y) {{ struct {pre}S s = {{ x + 1, y * 2, x - y }}; return s; }}")
            if kind == "sret":
                decls.append(f"__attribute__((noinline)) struct {pre}S {pre}f(int x, int y) {{ return {pre}g(y, x); }}")
                body = f"struct {pre}S s = {pre}f(p0, p1); return (int32_t)(s.a + s.b * 3 + s.c * 5);"
                v = I32.wrap((y + 1) + (x * 2) * 3 + (y - x) * 5)
            else:
                body = f"return (int32_t){pre}g(p0, p1).c;"
                v = I32.wrap(x - y)
        elif kind == "fnptr":
            decls.append(f"__attribute__((noinline)) int {pre}g1(int a, int b) {{ return a * 3 - b; }}")
            decls.append(f"__attribute__((noinline)) int {pre}g2(int a, int b) {{ return a + b * 5; }}")
            decls.append(f"typedef int (*{pre}fp)(int, int);")
            decls.append(f"static volatile {pre}fp {pre}sel = {pre}g1;")
            body = f"{pre}fp f = {pre}sel; if (p1 & 1) f = {pre}g2; return f(p0, p1);"
            v = I32.wrap(x + y * 5) if (y & 1) else I32.wrap(x * 3 - y)
        elif kind == "varargs":
            decls.append("#include <stdarg.h>")
            decls.append(f"__attribute__((noinline)) int {pre}vs(int n, ...) {{ va_list ap; va_start(ap, n); int s = 0; for (int i = 0; i < n; i++) s = s * 3 + va_arg(ap, int); va_end(ap); return s; }}")
            n = rng.randint(1, 6)
            body = f"return {pre}vs({n}, {', '.join(f'p0 + {i} * p1' for i in range(n))});"
            v = 0
            for i in range(n):
                v = I32.wrap(v * 3 + x + i * y)
        elif kind == "fp_to_int":
            decls.append(f"__attribute__((noinline)) double {pre}g(int a, int b) {{ return a * 0.5 + b; }}")
            body = f"return (int32_t){pre}g(p0, p1);"
            v = I32.wrap(int(x * 0.5 + y))
        elif kind == "int_to_fp":
            decls.append(f"__attribute__((noinline)) int {pre}g(int a, int b) {{ return a * 9 - b; }}")
            decls.append(f"__attribute__((noinline)) double {pre}f(int a, int b) {{ return {pre}g(a, b); }}")
            body = f"return (int32_t)({pre}f(p0, p1) * 2.0);"
            v = I32.wrap((x * 9 - y) * 2)
        elif kind == "live_across":
            decls.append(f"__attribute__((noinline)) int {pre}h(int a) {{ return a * 3 + 1; }}")
            decls.append(f"__attribute__((noinline)) int {pre}g(int a, int b, int c) {{ return a - b * 2 + c; }}")
            body = f"int t = {pre}h(p0); int u = {pre}h(p1 ^ t); return {pre}g(t + p0, u, p1);"
            t = I32.wrap(x * 3 + 1)
            u = I32.wrap((y ^ t) * 3 + 1)
            v = I32.wrap((t + x) - u * 2 + y)
        elif kind in ("vla_value", "vla_ptr"):
            n = rng.randint(1, 9)
            params = [Param("int32_t", str(n), raw=n), Param("int32_t", str(y), raw=y)]
            if kind == "vla_value":
                decls.append(f"__attribute__((noinline)) int {pre}g(int a, int b) {{ return a * 5 + b; }}")
                body = f"int vla[p0]; for (int i = 0; i < p0; i++) vla[i] = p1 + i * 7; return {pre}g(vla[p0 - 1], p0);"
                v = I32.wrap((y + (n - 1) * 7) * 5 + n)
            else:
                decls.append(f"__attribute__((noinline)) int {pre}g(const int *p, int n) {{ int s = 0; for (int i = 0; i < n; i++) s += p[i]; return s; }}")
                body = f"int vla[p0]; for (int i = 0; i < p0; i++) vla[i] = p1 + i * 7; return {pre}g(vla, p0);"
                v = I32.wrap(sum(y + i * 7 for i in range(n)))
        elif kind == "narrow_param":
            decls.append(f"__attribute__((noinline)) int {pre}g(unsigned char a, short b) {{ return a * 3 + b; }}")
            body = f"return {pre}g(p0, p1);"
            v = I32.wrap((x & 0xff) * 3 + I16.wrap(y))
        else:  # bool_ret: int -> _Bool conversion must not be skipped by a tail jump
            decls.append(f"__attribute__((noinline)) int {pre}g(int a, int b) {{ return (a ^ b) << 8; }}")
            decls.append(f"__attribute__((noinline)) _Bool {pre}f(int a, int b) {{ return {pre}g(a, b); }}")
            body = f"return {pre}f(p0, p1) ? 17 : 4;"
            v = 17 if I32.wrap((x ^ y) << 8) != 0 else 4
        out.append(Case(f"tailcall_{idx}", "tailcall", rt, params, body, v, desc=kind, decls=decls))
    return out


# ---------------------------------------------------------------------------
# framecall — callee-saved pressure, VLAs and inline-asm clobbers
# ---------------------------------------------------------------------------

def _mix(x: int) -> int:
    x &= MASK64
    return ((x * 0x9E3779B97F4A7C15) & MASK64) ^ (x >> 29)


FRAMECALL_DECLS = [
    "__attribute__((noinline)) static uint64_t fc_mix(uint64_t x) { return (x * 0x9E3779B97F4A7C15ull) ^ (x >> 29); }",
]


def gen_framecall(rng: random.Random, count: int) -> list[Case]:
    out: list[Case] = []
    for idx in range(count):
        nargs = rng.randint(3, 6)
        vals = [rng.getrandbits(64) for _ in range(nargs)]
        params = [Param("uint64_t", U64.literal(v), raw=v) for v in vals]
        names = [f"p{i}" for i in range(nargs)]
        live = dict(zip(names, vals))
        lines: list[str] = []
        feats: set[str] = set()
        use_vla = rng.random() < 0.35
        use_asm = rng.random() < 0.35
        vla: list[int] = []
        if use_vla:
            feats.add("vla")
            n = (vals[0] & 7) + 1
            lines.append("const uint64_t nvla = (p0 & 7) + 1;   /* frozen: asm below may rewrite p0 */")
            lines.append("uint64_t vla[nvla];")
            lines.append("for (uint64_t i = 0; i < nvla; i++) vla[i] = p1 + i * 0x101;")
            vla = [(vals[1] + i * 0x101) & MASK64 for i in range(n)]
        ncalls = rng.randint(2, 6)
        for c in range(ncalls):
            src = rng.choice(list(live))
            extra = rng.choice(names)
            lines.append(f"uint64_t r{c} = fc_mix({src} ^ {extra});")
            live[f"r{c}"] = _mix(live[src] ^ live[extra])
            if use_asm and rng.random() < 0.5:
                feats.add("asm")
                tgt = rng.choice(list(live))
                clob = rng.choice(["rbx", "r12", "r13", "r14", "r15"])
                lines.append("{ uint64_t t; __asm__ volatile(\"movq %1, %0\\n\\txorq %%" + clob +
                             ", %%" + clob + "\\n\\taddq $3, %0\" : \"=&r\"(t) : \"r\"(" + tgt +
                             ") : \"" + clob + "\", \"cc\"); " + tgt + " = t; }")
                live[tgt] = (live[tgt] + 3) & MASK64
        terms = list(live.items())
        rng.shuffle(terms)
        expr = " ^ ".join(f"({k} * {i + 1}u)" for i, (k, _) in enumerate(terms))
        acc = 0
        for i, (_, v) in enumerate(terms):
            acc ^= (v * (i + 1)) & MASK64
        lines.append(f"uint64_t acc = {expr};")
        if use_vla:
            lines.append("for (uint64_t i = 0; i < nvla; i++) acc += vla[i] * (i + 3);")
            for i, v in enumerate(vla):
                acc = (acc + v * (i + 3)) & MASK64
        lines.append("return acc;")
        out.append(Case(f"framecall_{idx}", "framecall", U64, params, "\n    ".join(lines), acc,
                        desc=f"{nargs} args {ncalls} calls " + " ".join(sorted(feats)),
                        decls=list(FRAMECALL_DECLS)))
    return out


# ---------------------------------------------------------------------------
# speculate — the untaken arm traps; if-conversion must not execute it
# ---------------------------------------------------------------------------

def gen_speculate(rng: random.Random, count: int) -> list[Case]:
    out: list[Case] = []
    for idx in range(count):
        kind = rng.choice(["null_load", "div_zero", "wild_index", "null_store", "mod_zero",
                           "null_load_chain", "div_intmin", "deref_deref"])
        c = rng.choice([0, 0, 1, 2, 255, 0x80000000])
        taken = c != 0
        d = rng.getrandbits(31)
        pre = f"sp{idx}_"
        decls = [f"static uint32_t {pre}g[8] = {{ 11, 22, 33, 44, 55, 66, 77, 88 }};",
                 f"typedef uint32_t *{pre}ptr;"]
        if kind in ("null_load", "null_load_chain", "null_store", "deref_deref"):
            ptr = f"&{pre}g[3]" if taken else f"({pre}ptr)0"
            params = [Param("uint32_t", str(c), raw=c), Param(f"{pre}ptr", ptr, raw=None),
                      Param("uint32_t", str(d), raw=d)]
            if kind == "null_load":
                body = "return p0 ? *p1 : p2;"
                v = 44 if taken else d
            elif kind == "null_load_chain":
                body = "uint32_t r = p2; if (p0) r = p1[1] + p1[-1]; return r + (p0 ? p1[0] : 1u);"
                v = U32.wrap(55 + 33 + 44) if taken else U32.wrap(d + 1)
            elif kind == "null_store":
                body = "if (p0) *p1 = p2; return p0 ? *p1 * 2u : p2 ^ 7u;"
                v = U32.wrap(d * 2) if taken else U32.wrap(d ^ 7)
            else:
                decls.append(f"static {pre}ptr {pre}pp = &{pre}g[5];")
                decls.append(f"typedef {pre}ptr *{pre}pptr;")
                params[1] = Param(f"{pre}pptr", f"&{pre}pp" if taken else f"({pre}pptr)0", raw=None)
                body = "return p0 ? **p1 + 1u : p2;"
                v = 67 if taken else d
        elif kind in ("div_zero", "mod_zero"):
            b = rng.choice([3, 7, 1000]) if taken else 0
            params = [Param("uint32_t", str(c), raw=c), Param("uint32_t", str(b), raw=b),
                      Param("uint32_t", str(d), raw=d)]
            if kind == "div_zero":
                body = "return p0 ? p2 / p1 : p2 + 1u;"
                v = d // b if taken else U32.wrap(d + 1)
            else:
                body = "return p0 ? (p2 % p1) * 3u : p2 - 1u;"
                v = U32.wrap((d % b) * 3) if taken else U32.wrap(d - 1)
        elif kind == "div_intmin":
            a = rng.randint(-1000, 1000) if taken else -2147483648
            b = rng.choice([3, -7, 11]) if taken else -1
            params = [Param("uint32_t", str(c), raw=c), Param("int32_t", I32.literal(a), raw=a),
                      Param("int32_t", I32.literal(b), raw=b)]
            body = "return p0 ? (uint32_t)(p1 / p2) : (uint32_t)(p1 ^ p2);"
            if taken:
                q = abs(a) // abs(b)
                if (a < 0) != (b < 0):
                    q = -q
                v = U32.wrap(q)
            else:
                v = U32.wrap(a ^ b)
        else:  # wild_index
            i = rng.randint(0, 7) if taken else rng.choice([0x7fffffff, 0x40000000, 0xfffffff0])
            params = [Param("uint32_t", str(c), raw=c), Param("uint32_t", str(i), raw=i),
                      Param("uint32_t", str(d), raw=d)]
            body = f"return p0 ? {pre}g[p1] : p2;"
            v = [11, 22, 33, 44, 55, 66, 77, 88][i] if taken else d
        out.append(Case(f"speculate_{idx}", "speculate", U32, params, body, v,
                        desc=f"{kind} {'taken' if taken else 'guarded'}", decls=decls))
    return out


# ---------------------------------------------------------------------------
# shiftchain — cascaded / masked / zero-count shift idioms
# ---------------------------------------------------------------------------

def gen_shiftchain(rng: random.Random, count: int) -> list[Case]:
    out: list[Case] = []
    while len(out) < count:
        ut = _u(rng.choice(STD_TYPES))
        pt = promote(ut)          # arithmetic happens here (int for 8/16-bit)
        W = pt.bits
        a = rand_value(rng, ut)
        n = rng.randint(-70, 70)
        kind = rng.choice(["shl_shl", "shr_shr", "shl_shr", "shr_shl", "rot0", "inc_chain",
                           "mask_and", "mask_mod", "shl_var_var", "sar_chain", "shl_then_narrow",
                           "byte_extract"])
        av = convert(a, ut, pt)      # promoted value (signed int for narrow ut)
        ua = av & pt.mask            # its bit pattern
        m = pt.mask
        # Narrow operands promote to *signed* int; keep every left shift on a
        # non-negative promoted value inside the representable range or fall
        # back to an unsigned promoted type so the C is UB-free.
        if pt.signed:
            pt = cemu.unsigned_of(pt)
            src = f"(({pt.name})p0)"
        else:
            src = "p0"
        if kind == "shl_shl":
            k1, k2 = rng.randrange(0, W), rng.randrange(0, W)
            expr, v = f"(({src} << {k1}) << {k2})", (((ua << k1) & m) << k2) & m
        elif kind == "shr_shr":
            k1, k2 = rng.randrange(0, W), rng.randrange(0, W)
            expr, v = f"(({src} >> {k1}) >> {k2})", (ua >> k1) >> k2
        elif kind == "shl_shr":
            k1, k2 = rng.randrange(0, W), rng.randrange(0, W)
            expr, v = f"(({src} << {k1}) >> {k2})", ((ua << k1) & m) >> k2
        elif kind == "shr_shl":
            k1, k2 = rng.randrange(0, W), rng.randrange(0, W)
            expr, v = f"(({src} >> {k1}) << {k2})", ((ua >> k1) << k2) & m
        elif kind == "rot0":
            c = n & (W - 1)
            expr = f"(({src} << (p1 & {W - 1})) | ({src} >> ((0u - (uint32_t)p1) & {W - 1})))"
            v = ((ua << c) | (ua >> ((-c) & (W - 1)))) & m if c else ua
        elif kind == "inc_chain":
            k = rng.randint(2, 9)
            expr, v = "(" + src + " + 1u" * k + ")", (ua + k) & m
        elif kind == "mask_and":
            expr, v = f"({src} << (p1 & {W - 1}))", (ua << (n & (W - 1))) & m
        elif kind == "mask_mod":
            nn = abs(n) % W
            expr, v = f"({src} >> ((p1 < 0 ? 0u - (uint32_t)p1 : (uint32_t)p1) % {W}u))", ua >> nn
        elif kind == "shl_var_var":
            k1, k2 = abs(n) % W, (abs(n) * 3) % W
            expr = (f"(({src} << ((uint32_t)(p1 < 0 ? -p1 : p1) % {W}u)) "
                    f"<< ((uint32_t)(p1 < 0 ? -p1 : p1) * 3u % {W}u))")
            v = (((ua << k1) & m) << k2) & m
        elif kind == "sar_chain":
            st = cemu.signed_of(pt)
            k1, k2 = rng.randrange(0, W), rng.randrange(0, W)
            sv = _sv(ua, pt)
            expr, v = f"(({pt.name})(((({st.name}){src}) >> {k1}) >> {k2}))", ((sv >> k1) >> k2) & m
        elif kind == "shl_then_narrow":
            nt = rng.choice([U8, U16, U32])
            k = rng.randrange(0, W)
            expr, v = f"((uint64_t)({nt.name})({src} << {k}))", ((ua << k) & m) & nt.mask
        else:  # byte_extract
            k = rng.randrange(0, W // 8) * 8
            expr, v = f"(({src} >> {k}) & 0xffu)", (ua >> k) & 0xff
        rt = rng.choice([pt, U64, ut])
        params = [Param(ut.name, ut.literal(a), raw=a), Param("int32_t", str(n), raw=n)]
        body = f"return ({rt.name}){expr};"
        expected = convert(v & m, U64 if kind == "shl_then_narrow" else pt, rt)
        out.append(Case(f"shiftchain_{len(out)}", "shiftchain", rt, params, body, expected,
                        desc=f"{kind} {ut.name}"))
    return out


PEEPHOLE_FAMILIES = {
    "flags": gen_flags,
    "narrow": gen_narrow,
    "memalias": gen_memalias,
    "tailcall": gen_tailcall,
    "framecall": gen_framecall,
    "speculate": gen_speculate,
    "shiftchain": gen_shiftchain,
}

families.FAMILIES.update(PEEPHOLE_FAMILIES)
