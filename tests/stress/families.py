#!/usr/bin/env python3
"""Stress-test families for LCCC: oracle-free, self-checking C programs.

Every family is a generator ``gen_<name>(rng, count) -> list[Case]``.  A Case
is one ``__attribute__((noinline))`` C function plus the *exact* value it must
return, computed here from the C standard (see ``cemu.py``) rather than from
another compiler.  ``run_stress.py`` assembles the cases into a program whose
``main`` feeds the arguments through ``volatile`` storage (so the callee really
executes the operation) and reports every mismatch by case name.

Design rules for every family:

* **No undefined behaviour.**  Cases whose evaluation raises ``Undefined`` are
  discarded and regenerated; the emitted program has one defined answer.
* **No unspecified evaluation order.**  Each case is a single call whose
  arguments are simple loads.
* **Deterministic.**  Same seed → byte-identical program.
* **Attributable.**  A failing case names its family, its parameters and the
  expected value; the failing function is < 20 lines of C.
* **Two modes.**  ``mode="rt"`` passes arguments through ``volatile`` memory
  (exercises the backend on runtime values); ``mode="cf"`` inlines them as
  constants (exercises SCCP/constant folding/complete unrolling on the same
  source shape).  Both must agree with the emulator.

Families and the compiler subsystems they target:

  intexpr   integer promotions, usual arithmetic conversions, narrow types,
            mixed signedness compares, casts, ternaries  → sema, IR lowering,
            narrow.rs, simplify.rs, sccp, instruction selection
  divmod    division/modulo by constants and variables at every width/sign,
            edge dividends                                → div_by_const.rs
  shifts    variable/constant shifts on promoted narrow types, rotate idioms
            at 8/16/32/64 bits                            → bit_idioms.rs, isel
  builtins  clz/ctz/popcount/bswap/parity/ffs and the add/sub/mul_overflow
            family across all width combinations         → intrinsics, isel
  loops     counted loops over every IV type/direction/stride/comparator with
            near-wrap bounds, tiny and moderate trip counts, secondary IVs,
            conditional and multi-reductions             → loop_unroll,
            iv_widen, iv_strength_reduce, vectorize, univsr, loop_rotate
  bitfield  random bit-field layouts incl. straddling 64-bit storage units,
            signed narrow fields, read-modify-write chains → frontend layout,
            load/store lowering, DSE/store-load forwarding
  fpcmp     IEEE compares/selects with NaN, ±0, ±inf; int<->fp conversions
            incl. uint64 > 2^63                            → FP isel (parity
            flag handling), cvt* selection, if_convert
  switch    dense/sparse/negative/clustered case sets with fall-through and
            GNU case ranges                               → switch lowering
  memops    memcpy/memmove/memset/struct-assign chains with constant sizes
            0..96 and unaligned offsets                    → aggregate lowering,
            DSE, aggregate_copy_forward, load_forward

The ABI family lives in ``abi_family.py`` because it needs two translation
units and a cross-compiler link matrix.
"""
from __future__ import annotations

import random
import struct
from dataclasses import dataclass, field

import cemu
from cemu import (I8, I16, I32, I64, I128, U8, U16, U32, U64, U128, IntTy, STD_TYPES,
                  Undefined, binop, convert, promote, unop)


@dataclass
class Param:
    cty: str          # C type spelling
    value: str        # C literal for the value fed from main
    kind: str = "int"  # "int" | "double" | "float"
    raw: object = None  # python value, for emulation / reporting


@dataclass
class Case:
    name: str
    family: str
    ret: IntTy | str          # IntTy, or "double"/"float"/"bits64" spelled via u64
    params: list[Param]
    body: str                 # function body (statements; must `return`)
    expected: int             # value of the return type (Python int / bits)
    desc: str = ""
    decls: list[str] = field(default_factory=list)   # file-scope helper decls


# ---------------------------------------------------------------------------
# helpers
# ---------------------------------------------------------------------------

EDGE_INTS = [0, 1, 2, 3, 7, 8, 15, 16, 31, 32, 63, 64, 100, 127, 128, 255, 256,
             1000, 4095, 32767, 32768, 65535, 65536, 0x7fffffff, 0x80000000,
             0xffffffff, 0x100000000, 0x7fffffffffffffff, 0x8000000000000000,
             0xffffffffffffffff]


def rand_value(rng: random.Random, t: IntTy) -> int:
    """A value of type t biased toward boundaries."""
    r = rng.random()
    if r < 0.30:
        v = rng.choice(EDGE_INTS)
        if rng.random() < 0.5:
            v = -v
    elif r < 0.45:
        v = rng.choice([t.minv, t.maxv, t.minv + 1, t.maxv - 1, 0, 1, -1])
    elif r < 0.70:
        v = rng.randint(-64, 64)
    else:
        v = rng.getrandbits(t.bits)
    return t.wrap(v)


def ret_check(ret: IntTy) -> tuple[str, str]:
    """(printf format, cast) to print a value of `ret` exactly."""
    if ret.bits == 128:
        return ("hi=%llx lo=%llx", "")
    return ("%llu", "(unsigned long long)") if not ret.signed else ("%lld", "(long long)")


# ---------------------------------------------------------------------------
# intexpr — random expression trees with exact C semantics
# ---------------------------------------------------------------------------

BIN_OPS = ["+", "-", "*", "/", "%", "&", "|", "^", "<<", ">>",
           "<", "<=", ">", ">=", "==", "!=", "&&", "||"]


class Expr:
    """Expression node carrying (text, value, type) computed under C rules."""

    __slots__ = ("text", "val", "ty")

    def __init__(self, text: str, val: int, ty: IntTy):
        self.text, self.val, self.ty = text, val, ty


def gen_expr(rng: random.Random, params: list[tuple[str, IntTy, int]], depth: int) -> Expr:
    if depth == 0 or rng.random() < 0.18:
        if rng.random() < 0.7 and params:
            n, t, v = rng.choice(params)
            return Expr(n, v, t)
        t = rng.choice(STD_TYPES)
        v = rand_value(rng, t)
        return Expr(t.literal(v), v, t)
    r = rng.random()
    if r < 0.12:
        a = gen_expr(rng, params, depth - 1)
        op = rng.choice(["-", "~", "!"])
        v, t = unop(op, a.val, a.ty)
        return Expr(f"({op}{a.text})", v, t)
    if r < 0.30:
        a = gen_expr(rng, params, depth - 1)
        t = rng.choice(STD_TYPES)
        return Expr(f"(({t.name}){a.text})", convert(a.val, a.ty, t), t)
    if r < 0.38:
        c = gen_expr(rng, params, depth - 1)
        a = gen_expr(rng, params, depth - 1)
        b = gen_expr(rng, params, depth - 1)
        # ?: applies the usual arithmetic conversions to the arms.
        rt = cemu.usual_arith(a.ty, b.ty)
        v = convert(a.val if c.val != 0 else b.val, a.ty if c.val != 0 else b.ty, rt)
        return Expr(f"({c.text} ? {a.text} : {b.text})", v, rt)
    a = gen_expr(rng, params, depth - 1)
    b = gen_expr(rng, params, depth - 1)
    op = rng.choice(BIN_OPS)
    if op in ("<<", ">>") and rng.random() < 0.6:
        # Bias shift counts into range to reduce the UB reject rate.
        w = promote(a.ty).bits
        cnt = rng.randrange(0, w)
        b = Expr(str(cnt), cnt, I32)
    v, t = binop(op, a.val, a.ty, b.val, b.ty)
    return Expr(f"({a.text} {op} {b.text})", v, t)


def gen_intexpr(rng: random.Random, count: int) -> list[Case]:
    out: list[Case] = []
    attempts = 0
    while len(out) < count and attempts < count * 40:
        attempts += 1
        nparams = rng.randint(1, 4)
        params = []
        for i in range(nparams):
            t = rng.choice(STD_TYPES)
            params.append((f"p{i}", t, rand_value(rng, t)))
        try:
            e = gen_expr(rng, params, rng.randint(2, 4))
            rt = rng.choice(STD_TYPES + (I128, U128)) if rng.random() < 0.15 else rng.choice(STD_TYPES)
            v = convert(e.val, e.ty, rt)
        except Undefined:
            continue
        if len(e.text) > 900:
            continue
        cparams = [Param(t.name, t.literal(v0), raw=v0) for (_, t, v0) in params]
        body = f"return ({rt.name})({e.text});"
        out.append(Case(f"intexpr_{len(out)}", "intexpr", rt, cparams, body, v,
                        desc=f"{e.ty.name} expr -> {rt.name}"))
    return out


# ---------------------------------------------------------------------------
# divmod — constant and variable division/modulo at every width
# ---------------------------------------------------------------------------

DIVISORS = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 15, 16, 17, 24, 25, 31,
            32, 33, 63, 64, 65, 100, 125, 127, 128, 129, 255, 256, 257, 641,
            1000, 1023, 1024, 1025, 3600, 65535, 65536, 65537, 1000000,
            0x7fffffff, 0x80000000, 0xffffffff, 1000000007, 6700417,
            0x100000000, 0x100000001, 0x7fffffffffffffff, 0x8000000000000000,
            0xffffffffffffffff, 1000000000000000003]


def gen_divmod(rng: random.Random, count: int) -> list[Case]:
    out: list[Case] = []
    attempts = 0
    while len(out) < count and attempts < count * 40:
        attempts += 1
        t = rng.choice(STD_TYPES)
        op = rng.choice(["/", "%", "/%"])
        a = rand_value(rng, t)
        variable = rng.random() < 0.25
        if variable:
            tb = rng.choice(STD_TYPES)
            b = rand_value(rng, tb)
            if b == 0:
                continue
        else:
            tb = t
            k = rng.choice(DIVISORS)
            if rng.random() < 0.4 and t.signed:
                k = -k
            b = t.wrap(k)
            if b == 0:
                continue
        try:
            if op == "/%":
                q, qt = binop("/", a, t, b, tb)
                r, _ = binop("%", a, t, b, tb)
                v, vt = binop("^", q, qt, r, qt)
                expr = f"((p0 / {{B}}) ^ (p0 % {{B}}))"
            else:
                v, vt = binop(op, a, t, b, tb)
                expr = f"(p0 {op} {{B}})"
        except Undefined:
            continue
        rt = rng.choice([vt, t, U64, I64])
        v = convert(v, vt, rt)
        if variable:
            params = [Param(t.name, t.literal(a), raw=a), Param(tb.name, tb.literal(b), raw=b)]
            expr = expr.replace("{B}", "p1")
        else:
            params = [Param(t.name, t.literal(a), raw=a)]
            expr = expr.replace("{B}", tb.literal(b))
        body = f"return ({rt.name}){expr};"
        out.append(Case(f"divmod_{len(out)}", "divmod", rt, params, body, v,
                        desc=f"{t.name} {op} {'var' if variable else b}"))
    return out


# ---------------------------------------------------------------------------
# shifts — narrow-type shifts and rotate idioms
# ---------------------------------------------------------------------------

def gen_shifts(rng: random.Random, count: int) -> list[Case]:
    out: list[Case] = []
    attempts = 0
    while len(out) < count and attempts < count * 40:
        attempts += 1
        t = rng.choice(STD_TYPES)
        a = rand_value(rng, t)
        kind = rng.choice(["shl", "shr", "rotl", "rotr", "shl_mask", "shr_shl", "sar_ext"])
        pt = promote(t)
        try:
            if kind in ("shl", "shr"):
                n = rng.randrange(0, pt.bits)
                op = "<<" if kind == "shl" else ">>"
                v, vt = binop(op, a, t, n, I32)
                var = rng.random() < 0.5
                params = [Param(t.name, t.literal(a), raw=a)]
                if var:
                    params.append(Param("int32_t", I32.literal(n), raw=n))
                    expr = f"(p0 {op} p1)"
                else:
                    expr = f"(p0 {op} {n})"
                rt = rng.choice([vt, t, U64])
                body = f"return ({rt.name}){expr};"
                v = convert(v, vt, rt)
            elif kind in ("rotl", "rotr"):
                ut = cemu.unsigned_of(t)
                ua = convert(a, t, ut)
                n = rng.randrange(1, ut.bits)          # n != 0 keeps the idiom UB-free
                if kind == "rotl":
                    v = cemu.rotl(ua, n, ut)
                else:
                    v = cemu.rotl(ua, ut.bits - n, ut)
                var = rng.random() < 0.5
                W = ut.bits
                params = [Param(ut.name, ut.literal(ua), raw=ua)]
                if var:
                    params.append(Param("uint32_t", U32.literal(n), raw=n))
                    nn = "p1"
                else:
                    nn = str(n)
                if kind == "rotl":
                    expr = f"(({ut.name})((p0 << {nn}) | (p0 >> ({W} - {nn}))))"
                else:
                    expr = f"(({ut.name})((p0 >> {nn}) | (p0 << ({W} - {nn}))))"
                rt = ut
                body = f"return {expr};"
            elif kind == "shl_mask":
                # x << (n & (W-1)) — the masked-count form backends fold to a bare shift.
                ut = cemu.unsigned_of(pt)
                ua = convert(a, t, ut)
                n = rng.randint(-200, 200)
                cnt = n & (ut.bits - 1)
                v = ut.wrap(ua << cnt)
                params = [Param(ut.name, ut.literal(ua), raw=ua), Param("int32_t", I32.literal(n), raw=n)]
                rt = ut
                body = f"return (p0 << (p1 & {ut.bits - 1}));"
            elif kind == "shr_shl":
                # (x >> k) << k  ==  x & ~((1<<k)-1)  for unsigned x
                ut = cemu.unsigned_of(pt)
                ua = convert(a, t, ut)
                k = rng.randrange(0, ut.bits)
                v = ut.wrap((ua >> k) << k)
                params = [Param(ut.name, ut.literal(ua), raw=ua)]
                rt = ut
                body = f"return (p0 >> {k}) << {k};"
            else:  # sar_ext: sign-extend then arithmetic shift of a narrow type
                st = cemu.signed_of(t)
                sa = convert(a, t, st)
                k = rng.randrange(0, 32)
                v, vt = binop(">>", sa, st, k, I32)
                rt = rng.choice([I32, I64, st])
                v = convert(v, vt, rt)
                params = [Param(st.name, st.literal(sa), raw=sa)]
                body = f"return ({rt.name})(p0 >> {k});"
        except Undefined:
            continue
        out.append(Case(f"shifts_{len(out)}", "shifts", rt, params, body, v, desc=f"{kind} {t.name}"))
    return out


# ---------------------------------------------------------------------------
# builtins
# ---------------------------------------------------------------------------

def gen_builtins(rng: random.Random, count: int) -> list[Case]:
    out: list[Case] = []
    attempts = 0
    suffix = {32: "", 64: "ll"}
    while len(out) < count and attempts < count * 40:
        attempts += 1
        kind = rng.choice(["clz", "ctz", "popcount", "parity", "ffs", "bswap",
                           "add_ov", "sub_ov", "mul_ov", "add_ov", "mul_ov", "clrsb"])
        try:
            if kind in ("clz", "ctz", "popcount", "parity", "ffs", "clrsb"):
                t = rng.choice([U32, U64, I32, I64])
                bt = cemu.unsigned_of(t) if kind != "ffs" else cemu.signed_of(t)
                a = rand_value(rng, t)
                if kind in ("clz", "ctz") and a == 0:
                    a = 1
                fn = {"clz": cemu.clz, "ctz": cemu.ctz, "popcount": cemu.popcount,
                      "parity": cemu.parity, "ffs": cemu.ffs}.get(kind)
                if kind == "clrsb":
                    st = cemu.signed_of(t)
                    sv = convert(a, t, st)
                    # number of leading redundant sign bits
                    if sv >= 0:
                        v = st.bits - 1 - sv.bit_length()
                    else:
                        v = st.bits - 1 - (~sv).bit_length()
                    v = int(v)
                    name = "__builtin_clrsb" + suffix[t.bits]
                    params = [Param(st.name, st.literal(sv), raw=sv)]
                else:
                    v = fn(a, bt)
                    name = "__builtin_" + kind + suffix[t.bits]
                    params = [Param(t.name, t.literal(a), raw=a)]
                # Narrow-type argument variants exercise the implicit promotion.
                if kind in ("popcount", "parity") and rng.random() < 0.3:
                    nt = rng.choice([U8, U16])
                    na = convert(a, t, nt)
                    v = fn(na, nt)
                    params = [Param(nt.name, nt.literal(na), raw=na)]
                    name = "__builtin_" + kind
                rt = I32
                body = f"return {name}(p0);"
            elif kind == "bswap":
                t = rng.choice([U16, U32, U64])
                a = rand_value(rng, t)
                v = cemu.bswap(a, t)
                rt = t
                params = [Param(t.name, t.literal(a), raw=a)]
                body = f"return __builtin_bswap{t.bits}(p0);"
            else:
                op = kind[:3]
                ta, tb, tr = rng.choice(STD_TYPES), rng.choice(STD_TYPES), rng.choice(STD_TYPES)
                a, b = rand_value(rng, ta), rand_value(rng, tb)
                res, flag = cemu.overflow_builtin(op, a, b, tr)
                # Fold the stored result and the flag into one checked value.
                v = (convert(res, tr, U64) * 2 + flag) & U64.mask
                rt = U64
                params = [Param(ta.name, ta.literal(a), raw=a), Param(tb.name, tb.literal(b), raw=b)]
                body = (f"{tr.name} r; int f = __builtin_{op}_overflow(p0, p1, &r);\n"
                        f"    return ((uint64_t)r << 1) | (uint64_t)f;")
                kind = f"{op}_overflow {ta.name},{tb.name}->{tr.name}"
        except Undefined:
            continue
        out.append(Case(f"builtins_{len(out)}", "builtins", rt, params, body, v, desc=kind))
    return out


# ---------------------------------------------------------------------------
# loops — counted loops over every IV shape, emulated iteration by iteration
# ---------------------------------------------------------------------------

TAB = [((i * 2654435761) ^ (i << 7) ^ 0x9e37) & 0xffffffff for i in range(64)]
TAB_DECL = "static const uint32_t tab[64] = {" + ", ".join(f"0x{v:08x}u" for v in TAB) + "};"
OUT_DECL = "static uint64_t outbuf[1024];"


def gen_loops(rng: random.Random, count: int) -> list[Case]:
    out: list[Case] = []
    attempts = 0
    MAXTRIP = 400
    while len(out) < count and attempts < count * 60:
        attempts += 1
        ivt = rng.choice(STD_TYPES)
        limt = rng.choice([ivt, ivt, rng.choice(STD_TYPES)])
        acct = rng.choice([U32, U64, U16])  # unsigned only: accumulation must not overflow
        down = rng.random() < 0.35
        stride = rng.choice([1, 1, 1, 2, 3, 4, 5, 7, 8, 16])
        cmp_op = rng.choice(["<", "<=", "!=", "<", "<="]) if not down else rng.choice([">", ">=", "!="])
        # Choose an init near a boundary of the IV type some of the time.
        r = rng.random()
        if r < 0.25:
            init = ivt.minv if not down else ivt.maxv
            init = ivt.wrap(init + rng.randint(0, 3) * (1 if not down else -1))
        elif r < 0.5:
            init = ivt.wrap(rng.randint(-20, 20))
        else:
            init = rand_value(rng, ivt)
        trips_wanted = rng.choice([0, 1, 2, 3, 4, 5, 7, 8, 9, 15, 16, 17, 31, 33, 64, 100, 257])
        span = trips_wanted * stride
        lim_math = init + span if not down else init - span
        if cmp_op in ("<=", ">="):
            lim_math -= stride if not down else -stride
            if trips_wanted == 0:
                lim_math = init - 1 if not down else init + 1
        if cmp_op == "!=":
            pass  # exact landing required; lim = init + trips*stride
        if not limt.fits(lim_math):
            continue
        lim = lim_math
        body_kind = rng.choice(["sum", "sum", "tab", "store", "cond", "minmax", "two", "nest", "ptr"])
        inner = rng.randint(1, 4)
        # --- emulate ------------------------------------------------------
        try:
            acc = 0
            acc2 = 0
            k = 0
            i = init
            mn = None
            mx = None
            trips = 0
            outbuf = [0] * 1024
            while True:
                c, _ = binop(cmp_op, i, ivt, lim, limt)
                if not c:
                    break
                trips += 1
                if trips > MAXTRIP:
                    raise Undefined("too many trips")
                ui = convert(i, ivt, U64)
                if body_kind == "sum":
                    acc = acct.wrap(acc + U64.wrap(ui * 7 + 3))
                elif body_kind == "tab":
                    acc = acct.wrap(acc ^ (TAB[ui & 63] + trips))
                elif body_kind == "store":
                    outbuf[k] = U64.wrap(ui * 3 + k)
                    k += 1
                elif body_kind == "cond":
                    if ui % 3 == 0:
                        acc = acct.wrap(acc + ui)
                    else:
                        acc2 = acct.wrap(acc2 + 1)
                elif body_kind == "minmax":
                    sv = convert(i, ivt, I64)
                    mn = sv if mn is None or sv < mn else mn
                    mx = sv if mx is None or sv > mx else mx
                elif body_kind == "two":
                    acc = acct.wrap(acc + ui)
                    acc2 = acct.wrap(acc2 ^ (ui << 1))
                elif body_kind == "nest":
                    for j in range(inner):
                        acc = acct.wrap(acc + ui * (j + 1))
                elif body_kind == "ptr":
                    acc = acct.wrap(acc + TAB[(ui * stride) & 63])
                # i += stride  (compound assignment converts back to ivt)
                nv, nt = binop("+" if not down else "-", i, ivt, stride, I32)
                i = convert(nv, nt, ivt)
            if body_kind == "store":
                acc = 0
                for j in range(k):
                    acc = U64.wrap(acc * 31 + outbuf[j])
                res_t = U64
                v = acc
            elif body_kind == "minmax":
                res_t = I64
                if mn is not None and not I64.fits(mx * 1000003 - mn):
                    raise Undefined("minmax checksum overflow")
                v = 0 if mn is None else mx * 1000003 - mn
            elif body_kind in ("cond", "two"):
                res_t = U64
                v = U64.wrap(convert(acc, acct, U64) * 65599 + convert(acc2, acct, U64))
            else:
                res_t = U64
                v = convert(acc, acct, U64)
        except Undefined:
            continue
        # --- emit ---------------------------------------------------------
        step = "+=" if not down else "-="
        hdr = f"for ({ivt.name} i = p0; i {cmp_op} p1; i {step} {stride})"
        if body_kind == "sum":
            code = f"{acct.name} acc = 0; {hdr} acc += ({acct.name})((uint64_t)i * 7u + 3u); return (uint64_t)acc;"
        elif body_kind == "tab":
            code = (f"{acct.name} acc = 0; uint32_t n = 0; {hdr} {{ n++; acc ^= ({acct.name})(tab[(uint64_t)i & 63u] + n); }} "
                    f"return (uint64_t)acc;")
        elif body_kind == "store":
            code = (f"uint32_t k = 0; {hdr} {{ outbuf[k] = (uint64_t)i * 3u + k; k++; }} "
                    f"uint64_t h = 0; for (uint32_t j = 0; j < k; j++) h = h * 31u + outbuf[j]; return h;")
        elif body_kind == "cond":
            code = (f"{acct.name} acc = 0, cnt = 0; {hdr} {{ if ((uint64_t)i % 3u == 0u) acc += ({acct.name})(uint64_t)i; else cnt++; }} "
                    f"return (uint64_t)acc * 65599u + (uint64_t)cnt;")
        elif body_kind == "minmax":
            code = (f"int64_t mn = 0, mx = 0; int first = 1; {hdr} {{ int64_t s = (int64_t)i; "
                    f"if (first || s < mn) mn = s; if (first || s > mx) mx = s; first = 0; }} "
                    f"return first ? 0 : (uint64_t)(mx * 1000003 - mn);")
        elif body_kind == "two":
            code = (f"{acct.name} a = 0, b = 0; {hdr} {{ a += ({acct.name})(uint64_t)i; b ^= ({acct.name})((uint64_t)i << 1); }} "
                    f"return (uint64_t)a * 65599u + (uint64_t)b;")
        elif body_kind == "nest":
            code = (f"{acct.name} acc = 0; {hdr} for (int j = 0; j < {inner}; j++) acc += ({acct.name})((uint64_t)i * (uint64_t)(j + 1)); "
                    f"return (uint64_t)acc;")
        else:  # ptr
            code = (f"{acct.name} acc = 0; const uint32_t *p = tab; {hdr} acc += ({acct.name})p[((uint64_t)i * {stride}u) & 63u]; "
                    f"return (uint64_t)acc;")
        params = [Param(ivt.name, ivt.literal(init), raw=init), Param(limt.name, limt.literal(lim), raw=lim)]
        out.append(Case(f"loops_{len(out)}", "loops", res_t if body_kind != "minmax" else I64, params,
                        code, v if body_kind != "minmax" else v,
                        desc=f"{body_kind} iv={ivt.name} lim={limt.name} {cmp_op} stride={stride} trips={trips}",
                        decls=[TAB_DECL, OUT_DECL]))
    return out


# ---------------------------------------------------------------------------
# bitfield — random layouts and RMW chains
# ---------------------------------------------------------------------------

BF_STORAGE = [("unsigned char", 8, False), ("signed char", 8, True), ("unsigned short", 16, False),
              ("short", 16, True), ("unsigned", 32, False), ("int", 32, True),
              ("unsigned long long", 64, False), ("long long", 64, True)]


def gen_bitfield(rng: random.Random, count: int) -> list[Case]:
    out: list[Case] = []
    for idx in range(count):
        nf = rng.randint(2, 6)
        fields = []
        for f in range(nf):
            st, bits, signed = rng.choice(BF_STORAGE)
            w = rng.randint(1, bits)
            fields.append((f"f{f}", st, w, signed))
        packed = " __attribute__((packed))" if rng.random() < 0.25 else ""
        sname = f"bf{idx}"
        decl = f"struct {sname} {{ " + " ".join(f"{st} {n}:{w};" for n, st, w, _ in fields) + f" }}{packed};"
        static = rng.random() < 0.5
        if static:
            decl += f"\nstatic struct {sname} g_{sname};"
        # emulate a random op sequence over 3 parameters
        pv = [rand_value(rng, U64) for _ in range(3)]
        vals = {n: 0 for n, *_ in fields}
        ops = []
        for _ in range(rng.randint(3, 9)):
            n, st, w, signed = rng.choice(fields)
            p = rng.randrange(3)
            kind = rng.choice(["=", "+=", "^=", "|=", "&=", "-=", "++", "flip"])
            if kind == "=":
                vals[n] = cemu.bitfield_store(pv[p], w, signed)
                ops.append(f"s.{n} = p{p};")
            elif kind == "++":
                vals[n] = cemu.bitfield_store(vals[n] + 1, w, signed)
                ops.append(f"s.{n}++;")
            elif kind == "flip":
                vals[n] = cemu.bitfield_store(~vals[n], w, signed)
                ops.append(f"s.{n} = ~s.{n};")
            else:
                # compound ops compute in the promoted/usual type: field value
                # (promoted) op uint64 param → uint64 arithmetic, then store.
                cur = convert(vals[n], I64 if signed else U64, U64)
                o = kind[0]
                r = {"+": cur + pv[p], "^": cur ^ pv[p], "|": cur | pv[p], "&": cur & pv[p], "-": cur - pv[p]}[o]
                vals[n] = cemu.bitfield_store(r, w, signed)
                ops.append(f"s.{n} {kind} p{p};")
        # checksum
        h = 0
        for n, st, w, signed in fields:
            h = U64.wrap(h * 1099511628211 + convert(vals[n], I64 if signed else U64, U64))
        chk = " ".join(f"h = h * 1099511628211ull + (uint64_t)(int64_t)s.{n};" if signed else
                       f"h = h * 1099511628211ull + (uint64_t)s.{n};" for n, st, w, signed in fields)
        if static:
            body = (f"struct {sname} *sp = &g_{sname}; __builtin_memset(sp, 0, sizeof *sp);\n"
                    f"    #define s (*sp)\n    " + "\n    ".join(ops) + f"\n    uint64_t h = 0; {chk}\n    #undef s\n    return h;")
        else:
            body = (f"struct {sname} s = {{0}};\n    " + "\n    ".join(ops) + f"\n    uint64_t h = 0; {chk} return h;")
        params = [Param("uint64_t", U64.literal(pv[i]), raw=pv[i]) for i in range(3)]
        out.append(Case(f"bitfield_{idx}", "bitfield", U64, params, body, h,
                        desc=f"{nf} fields{' packed' if packed else ''}{' static' if static else ''}",
                        decls=[decl]))
    return out


# ---------------------------------------------------------------------------
# fpcmp — IEEE compares, selects and conversions
# ---------------------------------------------------------------------------

FP_SPECIALS = ["nan", "-nan", "inf", "-inf", "0.0", "-0.0", "1.0", "-1.0", "0.5", "2.0",
               "1e300", "-1e300", "4.9e-324", "2.2250738585072014e-308", "3.0", "1.5", "-2.5",
               "9007199254740993.0", "4294967296.0", "4294967295.0", "2147483648.0", "-2147483649.0"]


def fp_of(s: str) -> float:
    return float(s.replace("-nan", "nan")) * (-1 if s == "-nan" else 1)


def dbits(x: float) -> int:
    return struct.unpack("<Q", struct.pack("<d", x))[0]


def fbits(x: float) -> int:
    return struct.unpack("<I", struct.pack("<f", x))[0]


def gen_fpcmp(rng: random.Random, count: int) -> list[Case]:
    import math
    out: list[Case] = []
    attempts = 0
    while len(out) < count and attempts < count * 40:
        attempts += 1
        ft = rng.choice(["double", "float"])
        a_s, b_s = rng.choice(FP_SPECIALS), rng.choice(FP_SPECIALS)
        a, b = fp_of(a_s), fp_of(b_s)
        if ft == "float":
            def to_f32(x: float) -> float:
                if math.isnan(x) or math.isinf(x) or abs(x) < 3.4028234663852886e38:
                    return struct.unpack("<f", struct.pack("<f", x))[0]
                return math.copysign(math.inf, x)   # (float)1e300 == inf
            a, b = to_f32(a), to_f32(b)
        kind = rng.choice(["cmp", "cmp", "ncmp", "sel_int", "sel_fp", "branch", "unord", "cvt_i", "cvt_u", "cvt_from", "cvt_from_u", "cmp_zero"])
        ops = {"<": lambda x, y: x < y, "<=": lambda x, y: x <= y, ">": lambda x, y: x > y,
               ">=": lambda x, y: x >= y, "==": lambda x, y: x == y, "!=": lambda x, y: x != y}
        op = rng.choice(list(ops))
        pa = Param(ft, f"__builtin_{'nan' if ft == 'double' else 'nanf'}(\"\")" if math.isnan(a) else
                   (f"__builtin_{'inf' if ft == 'double' else 'inff'}()" if math.isinf(a) and a > 0 else
                    (f"-__builtin_{'inf' if ft == 'double' else 'inff'}()" if math.isinf(a) else
                     (f"{a!r}" if ft == "double" else f"{a!r}f"))), kind=ft, raw=a)
        pb = Param(ft, f"__builtin_{'nan' if ft == 'double' else 'nanf'}(\"\")" if math.isnan(b) else
                   (f"__builtin_{'inf' if ft == 'double' else 'inff'}()" if math.isinf(b) and b > 0 else
                    (f"-__builtin_{'inf' if ft == 'double' else 'inff'}()" if math.isinf(b) else
                     (f"{b!r}" if ft == "double" else f"{b!r}f"))), kind=ft, raw=b)
        if math.isnan(a) and a_s == "-nan":
            pa.value = "-" + pa.value
        if math.isnan(b) and b_s == "-nan":
            pb.value = "-" + pb.value
        bits = dbits if ft == "double" else fbits
        rt = U64
        if kind == "cmp":
            v = 1 if ops[op](a, b) else 0
            body = f"return (uint64_t)(p0 {op} p1);"
            params = [pa, pb]
        elif kind == "ncmp":
            v = 0 if ops[op](a, b) else 1
            body = f"return (uint64_t)!(p0 {op} p1);"
            params = [pa, pb]
        elif kind == "cmp_zero":
            v = 1 if ops[op](a, 0.0) else 0
            body = f"return (uint64_t)(p0 {op} 0);"
            params = [pa]
        elif kind == "sel_int":
            x, y = rng.randint(-100, 100), rng.randint(-100, 100)
            v = U64.wrap(x if ops[op](a, b) else y)
            body = f"return (uint64_t)(int64_t)(p0 {op} p1 ? {x} : {y});"
            params = [pa, pb]
        elif kind == "sel_fp":
            # min/max style select: the *bit pattern* of the chosen operand matters (-0.0!).
            chosen = a if ops[op](a, b) else b
            v = bits(chosen)
            body = (f"{ft} r = p0 {op} p1 ? p0 : p1; union {{ {ft} f; {'uint64_t' if ft == 'double' else 'uint32_t'} u; }} u; "
                    f"u.f = r; return (uint64_t)u.u;")
            params = [pa, pb]
        elif kind == "branch":
            v = U64.wrap((7 if ops[op](a, b) else 3) * 2 + (1 if a != a else 0))
            body = (f"uint64_t r = 0; if (p0 {op} p1) r = 7; else r = 3; r *= 2; "
                    f"if (p0 != p0) r += 1; return r;")
            params = [pa, pb]
        elif kind == "unord":
            v = 1 if (math.isnan(a) or math.isnan(b)) else 0
            body = f"return (uint64_t)__builtin_isunordered(p0, p1);"
            params = [pa, pb]
        elif kind == "cvt_i":
            # only in-range, finite values (out-of-range conversion is UB)
            if math.isnan(a) or math.isinf(a) or not (-2147483648.0 <= a <= 2147483647.0):
                continue
            it = rng.choice([I32, I64, I16, I8])
            tv = math.trunc(a)
            if not it.fits(tv):
                continue
            v = convert(tv, it, U64)
            body = f"return (uint64_t)(int64_t)({it.name})p0;"
            params = [pa]
        elif kind == "cvt_u":
            if math.isnan(a) or math.isinf(a) or a <= -1.0 or a >= 18446744073709551616.0:
                continue
            it = rng.choice([U32, U64, U16, U8])
            tv = math.trunc(a)
            if tv < 0 or not it.fits(tv):
                continue
            v = tv
            body = f"return (uint64_t)({it.name})p0;"
            params = [pa]
        elif kind == "cvt_from":
            it = rng.choice([I32, I64, I16, I8])
            iv = rand_value(rng, it)
            if ft == "float" and abs(iv) > (1 << 53):
                continue
            fv = float(iv)
            if ft == "float":
                fv = struct.unpack("<f", struct.pack("<f", fv))[0]
            v = bits(fv)
            body = (f"{ft} r = ({ft})p0; union {{ {ft} f; {'uint64_t' if ft == 'double' else 'uint32_t'} u; }} u; "
                    f"u.f = r; return (uint64_t)u.u;")
            params = [Param(it.name, it.literal(iv), raw=iv)]
        else:  # cvt_from_u
            it = rng.choice([U32, U64, U16, U8])
            iv = rand_value(rng, it)
            if ft == "float" and iv > (1 << 53):
                continue
            fv = float(iv)
            if ft == "float":
                fv = struct.unpack("<f", struct.pack("<f", fv))[0]
            v = bits(fv)
            body = (f"{ft} r = ({ft})p0; union {{ {ft} f; {'uint64_t' if ft == 'double' else 'uint32_t'} u; }} u; "
                    f"u.f = r; return (uint64_t)u.u;")
            params = [Param(it.name, it.literal(iv), raw=iv)]
        out.append(Case(f"fpcmp_{len(out)}", "fpcmp", rt, params, body, v,
                        desc=f"{kind} {op} {ft} a={a_s} b={b_s}"))
    return out


# ---------------------------------------------------------------------------
# switch — random case sets with fall-through and GNU ranges
# ---------------------------------------------------------------------------

def gen_switch(rng: random.Random, count: int) -> list[Case]:
    out: list[Case] = []
    for idx in range(count):
        shape = rng.choice(["dense", "sparse", "neg", "cluster", "huge", "tiny"])
        if shape == "dense":
            base = rng.randint(-5, 50)
            keys = list(range(base, base + rng.randint(3, 24)))
            keys = [k for k in keys if rng.random() < 0.85]
        elif shape == "sparse":
            keys = sorted(set(rng.randint(-1000, 1000) * rng.choice([1, 3, 7, 16]) for _ in range(rng.randint(3, 14))))
        elif shape == "neg":
            keys = sorted(set(rng.randint(-40, -1) for _ in range(rng.randint(3, 12))))
        elif shape == "cluster":
            keys = sorted(set([rng.randint(0, 9) for _ in range(5)] + [rng.randint(100, 109) for _ in range(5)] +
                              [rng.randint(100000, 100009) for _ in range(3)]))
        elif shape == "huge":
            keys = sorted(set(rng.choice([-2147483648, -2147483647, 2147483647, 2147483646, 0, 1, -1, 65536, -65536])
                              for _ in range(rng.randint(3, 8))))
        else:
            keys = sorted(set(rng.randint(0, 3) for _ in range(2)))
        if not keys:
            keys = [0]
        # Assign actions.  action: ("ret", n) | ("fall",) | ("range", hi, n)
        labels: list[tuple] = []
        used = set()
        for k in keys:
            if k in used:
                continue
            r = rng.random()
            if r < 0.15 and shape in ("dense", "sparse") and k + 3 not in keys and k + 1 not in keys and k + 2 not in keys:
                labels.append(("range", k, k + rng.randint(1, 3), rng.randint(1, 99)))
            elif r < 0.35:
                labels.append(("fall", k))
            else:
                labels.append(("ret", k, rng.randint(1, 99)))
        has_default = rng.random() < 0.8
        default_val = rng.randint(100, 199)
        default_pos = rng.randint(0, len(labels)) if has_default else None
        # emulate: value for any x
        def evaluate(x: int) -> int:
            # locate entry index
            entry = None
            for i, lab in enumerate(labels):
                if lab[0] == "range":
                    if lab[1] <= x <= lab[2]:
                        entry = i
                        break
                elif lab[1] == x:
                    entry = i
                    break
            if entry is None:
                if not has_default:
                    return -1
                entry = ("default",)
            # walk from entry following fall-through
            i = default_pos if entry == ("default",) else entry
            seq = []
            for j, lab in enumerate(labels):
                if has_default and j == default_pos:
                    seq.append(("default", default_val))
                seq.append(lab)
            if has_default and default_pos == len(labels):
                seq.append(("default", default_val))
            # find position of entry in seq
            pos = None
            for j, lab in enumerate(seq):
                if entry == ("default",) and lab[0] == "default":
                    pos = j
                    break
                if entry != ("default",) and lab is labels[entry]:
                    pos = j
                    break
            while pos < len(seq):
                lab = seq[pos]
                if lab[0] == "ret":
                    return lab[2]
                if lab[0] == "range":
                    return lab[3]
                if lab[0] == "default":
                    return lab[1]
                pos += 1  # fall
            return -1  # fell off the end → after switch
        # Emit C
        lines = []
        for j, lab in enumerate(labels):
            if has_default and default_pos == j:
                lines.append(f"    default: return {default_val};")
            if lab[0] == "ret":
                lines.append(f"    case {I32.literal(lab[1])}: return {lab[2]};")
            elif lab[0] == "fall":
                lines.append(f"    case {I32.literal(lab[1])}:")
            else:
                lines.append(f"    case {I32.literal(lab[1])} ... {I32.literal(lab[2])}: return {lab[3]};")
        if has_default and default_pos == len(labels):
            lines.append(f"    default: return {default_val};")
        # probe inputs: every key, neighbours, and randoms; fold into a hash
        probes = set()
        for k in keys:
            probes.update([k, k - 1, k + 1])
        for _ in range(6):
            probes.add(rng.randint(-2147483648, 2147483647))
        probes = sorted(I32.wrap(p) for p in probes)
        h = 0
        for p in probes:
            h = U64.wrap(h * 1000003 + convert(evaluate(p), I32, U64))
        probe_decl = f"static const int32_t sw_probes_{idx}[{len(probes)}] = {{" + ", ".join(I32.literal(p) for p in probes) + "};"
        helper = (f"static __attribute__((noinline)) int sw_fn_{idx}(int32_t x) {{\n  switch (x + 0) {{\n" +
                  "\n".join(lines) + "\n  }\n  return -1;\n}")
        body = (f"uint64_t h = 0; for (uint32_t i = 0; i < {len(probes)}; i++) "
                f"h = h * 1000003u + (uint64_t)(int64_t)sw_fn_{idx}(sw_probes_{idx}[i] + (int32_t)p0); return h;")
        out.append(Case(f"switch_{idx}", "switch", U64, [Param("int32_t", "0", raw=0)], body, h,
                        desc=f"{shape} {len(labels)} labels{' default' if has_default else ''}",
                        decls=[probe_decl, helper]))
    return out


# ---------------------------------------------------------------------------
# memops — memcpy/memmove/memset/struct assignment chains
# ---------------------------------------------------------------------------

def gen_memops(rng: random.Random, count: int) -> list[Case]:
    out: list[Case] = []
    N = 96
    for idx in range(count):
        buf = bytearray((i * 37 + 11) & 0xff for i in range(N))
        ops = []
        decls = []
        for _ in range(rng.randint(2, 7)):
            kind = rng.choice(["cpy", "move", "set", "struct", "cpy", "move"])
            n = rng.choice([0, 1, 2, 3, 4, 5, 7, 8, 9, 12, 15, 16, 17, 24, 31, 32, 33, 40, 48, 63, 64, 65, 80])
            if kind == "set":
                d = rng.randint(0, N - n)
                c = rng.randint(0, 255)
                buf[d:d + n] = bytes([c]) * n
                ops.append(f"__builtin_memset(buf + {d}, {c}, {n});")
            elif kind == "cpy":
                # disjoint ranges only
                if 2 * n > N:
                    continue
                d = rng.randint(0, N - n)
                tries = 0
                while tries < 20:
                    s = rng.randint(0, N - n)
                    if s + n <= d or d + n <= s:
                        break
                    tries += 1
                else:
                    continue
                buf[d:d + n] = buf[s:s + n]
                ops.append(f"__builtin_memcpy(buf + {d}, buf + {s}, {n});")
            elif kind == "move":
                d = rng.randint(0, N - n)
                s = rng.randint(0, N - n)
                buf[d:d + n] = bytes(buf[s:s + n])
                ops.append(f"__builtin_memmove(buf + {d}, buf + {s}, {n});")
            else:
                if n == 0:
                    continue
                d = rng.randint(0, N - n)
                s = rng.randint(0, N - n)
                if not (s + n <= d or d + n <= s):
                    continue
                sname = f"blob{idx}_{len(decls)}"
                decls.append(f"struct {sname} {{ uint8_t b[{n}]; }};")
                buf[d:d + n] = buf[s:s + n]
                ops.append(f"*(struct {sname} *)(buf + {d}) = *(const struct {sname} *)(buf + {s});")
        h = 0
        for i in range(N):
            h = U64.wrap(h * 131 + buf[i] + 0)
        # p0 is a volatile-sourced zero so the buffer initialisation is not foldable away
        body = (f"uint8_t buf[{N}]; for (int i = 0; i < {N}; i++) buf[i] = (uint8_t)(i * 37 + 11 + (int)p0);\n    " +
                "\n    ".join(ops) + f"\n    uint64_t h = 0; for (int i = 0; i < {N}; i++) h = h * 131u + buf[i]; return h;")
        out.append(Case(f"memops_{idx}", "memops", U64, [Param("uint32_t", "0", raw=0)], body, h,
                        desc=f"{len(ops)} ops", decls=decls))
    return out


FAMILIES = {
    "intexpr": gen_intexpr,
    "divmod": gen_divmod,
    "shifts": gen_shifts,
    "builtins": gen_builtins,
    "loops": gen_loops,
    "bitfield": gen_bitfield,
    "fpcmp": gen_fpcmp,
    "switch": gen_switch,
    "memops": gen_memops,
}
