#!/usr/bin/env python3
"""x86-64 128-bit integers: narrow multiplies and the GPR-pair relay.

Three contracts, each with its negative control.

1. **Narrow multiply.**  ``(unsigned __int128)a * b`` with 64-bit operands is
   ONE widening ``mulq`` (the signed twin: one-operand ``imulq``), not the
   generic three-product sequence.  The emitter proves the operands' high
   halves from their unique defining instruction; a value with several
   definitions, a loaded value or a merged value has no fact and takes the
   generic product.  ``CCC_NO_I128_NARROW_MUL`` restores the generic form,
   which is how this module shows its shape assertions can fail.

2. **Pair relay.**  A 128-bit integer lives in a GPR pair, so its slot is
   written by two 8-byte stores.  A typed Load/Store therefore relays it as
   two 8-byte halves; the former ``movdqu`` relay read 16 bytes across two
   8-byte stores, which cannot be store-forwarded (measured 7.2-7.6 ns per
   transfer against 0.6-0.7 ns).  Functions that carry ``_Float128`` values
   keep the 16-byte relay, because there the producers and consumers are XMM
   registers and the pair relay would create the opposite stall.
   ``CCC_NO_I128_PAIR_RELAY`` restores ``movdqu`` everywhere.

3. **Function-order independence.**  The parameter classes used to decide
   whether a 128-bit parameter's home is coherent were read from emitter
   state that still described the PREVIOUSLY emitted function.  A function's
   code must not depend on what precedes it in the translation unit.

Every shape is also run: all optimisation levels, both kill switches, against
GCC 16.2 and Clang 23.1.5 (and ICX when present).
"""

from __future__ import annotations

import os
import platform
import re
import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
LCCC = Path(os.environ.get("LCCC", REPO / "target/fastbuild/lccc"))
GCC = os.environ.get("GCC16") or shutil.which("gcc-16") or shutil.which("gcc")
CLANG = os.environ.get("CLANG") or shutil.which("clang-23")
ICX = os.environ.get("ICX") or shutil.which("icx")

X86_HOST = platform.machine() in ("x86_64", "amd64")
READY = bool(X86_HOST and LCCC.is_file() and GCC)

KERNELS = r'''
#include <stdint.h>
typedef unsigned __int128 u128;
typedef __int128 i128;
#define NOINLINE __attribute__((noinline))

/* ---- narrow multiplies ------------------------------------------------ */
NOINLINE u128 mul_uu(uint64_t a, uint64_t b) { return (u128)a * b; }
NOINLINE i128 mul_ss(int64_t a, int64_t b) { return (i128)a * b; }
NOINLINE u128 mul_u32(uint32_t a, uint32_t b) { return (u128)a * b; }
NOINLINE i128 mul_s32(int32_t a, int32_t b) { return (i128)a * b; }
NOINLINE u128 mul_u8u16(uint8_t a, uint16_t b) { return (u128)a * b; }
NOINLINE i128 mul_s8s16(int8_t a, int16_t b) { return (i128)a * b; }
NOINLINE u128 sq_u(uint64_t a) { return (u128)a * a; }
NOINLINE i128 sq_s(int64_t a) { return (i128)a * a; }
/* A zero-extended next to a sign-extended operand is NOT a widening multiply. */
NOINLINE u128 mul_us(uint64_t a, int64_t b) { return (u128)a * (u128)(i128)b; }
NOINLINE u128 mul_su(int64_t a, uint64_t b) { return (u128)(i128)a * (u128)b; }
NOINLINE i128 mul_s8u16(int8_t a, uint16_t b) { return (i128)a * b; }
/* One narrow operand: a single cross term remains. */
NOINLINE u128 mul_fu(u128 a, uint64_t b) { return a * b; }
NOINLINE u128 mul_uf(uint64_t a, u128 b) { return (u128)a * b; }
/* A lone sign extension keeps both cross terms: the generic product. */
NOINLINE u128 mul_fs(u128 a, int64_t b) { return a * (u128)(i128)b; }
NOINLINE u128 mul_ff(u128 a, u128 b) { return a * b; }
/* Facts from shifts and masks. */
NOINLINE u128 mul_shr(u128 a, u128 b) { return (a >> 64) * (b >> 70); }
NOINLINE i128 mul_sar(i128 a, i128 b) { return (a >> 64) * (b >> 100); }
NOINLINE u128 mul_and(u128 a, u128 b) { return (a & 0xffffffffffffffffULL) * (b & 0xffffffffULL); }
NOINLINE u128 mul_shr63(u128 a, u128 b) { return (a >> 63) * (b >> 1); }
/* Constant multipliers. */
NOINLINE u128 mul_kz(uint64_t a) { return (u128)a * 0x9E3779B97F4A7C15ULL; }
NOINLINE u128 mul_kzh(uint64_t a) { return (u128)a * (((u128)0x1234567 << 64) | 0x9E3779B97F4A7C15ULL); }
NOINLINE u128 mul_kf(u128 a) { return a * 0x9E3779B97F4A7C15ULL; }
NOINLINE u128 mul_kfh(u128 a) { return a * (((u128)0x1234567 << 64) | 0x9E3779B97F4A7C15ULL); }
NOINLINE i128 mul_ks(int64_t a) { return (i128)a * 1000003; }
/* Values with more than one definition must carry no fact. */
NOINLINE u128 loop_acc(uint64_t a, int n) {
    u128 acc = a;
    for (int i = 0; i < n; i++) acc = acc * acc + (unsigned)i;
    return acc;
}
NOINLINE u128 loop_narrow(const uint64_t *p, int n) {
    u128 acc = 1;
    for (int i = 0; i < n; i++) { u128 w = p[i]; acc += w * w; w = acc; acc ^= w * 3; }
    return acc;
}
NOINLINE u128 phi_mix(uint64_t a, u128 w, int c) { u128 x = c ? (u128)a : w; return x * x; }
NOINLINE u128 phi_mix2(uint64_t a, int64_t b, int c) {
    u128 x;
    if (c) x = a; else x = (u128)(i128)b;
    return x * x;
}
/* The bignum inner step this work started from. */
NOINLINE uint64_t mac(uint64_t *r, uint64_t a, uint64_t b, uint64_t carry) {
    u128 t = (u128)a * b + *r + carry;
    *r = (uint64_t)t;
    return (uint64_t)(t >> 64);
}

/* ---- typed load / store relay ----------------------------------------- */
NOINLINE u128 pass(u128 a) { return a; }
NOINLINE void store_param(u128 *d, u128 a) { *d = a; }
NOINLINE u128 ext_id(u128 a) { return a + 0; }
NOINLINE void store_call(u128 *d, u128 a) { *d = ext_id(a); }
NOINLINE void copyu(u128 *d, const u128 *s) { *d = *s; }
NOINLINE u128 addu(const u128 *p, const u128 *q) { return *p + *q; }
NOINLINE void incu(u128 *p) { *p += 1; }
NOINLINE void swapu(u128 *a, u128 *b) { u128 t = *a; *a = *b; *b = t; }
struct __attribute__((packed)) packed_u128 { char pad; u128 v; };
NOINLINE void store_packed(struct packed_u128 *p, u128 v) { p->v = v; }
NOINLINE u128 load_packed(const struct packed_u128 *p) { return p->v; }
'''

F128_KERNELS = r'''
typedef _Float128 f128;
typedef unsigned __int128 u128;
#define NOINLINE __attribute__((noinline))
NOINLINE f128 addq(const f128 *p, const f128 *q) { return *p + *q; }
NOINLINE void scaleq(f128 *d, const f128 *s) { *d = *s * *s; }
/* An integer sibling in the SAME translation unit keeps the pair relay: the
   regime is decided per function. */
NOINLINE void store_param_i(u128 *d, u128 a) { *d = a; }
'''

DRIVER = r'''
#include <stdio.h>
static void show(const char *name, u128 v) {
    printf("%s %016llx%016llx\n", name, (unsigned long long)(v >> 64), (unsigned long long)v);
}
static const uint64_t edges[] = {
    0, 1, 2, 3, 0x7fffffffULL, 0x80000000ULL, 0xffffffffULL, 0x100000000ULL,
    0x7fffffffffffffffULL, 0x8000000000000000ULL, 0xffffffffffffffffULL,
    0x9E3779B97F4A7C15ULL, 0xC2B2AE3D27D4EB4FULL, 0x0123456789abcdefULL,
};
enum { NE = sizeof edges / sizeof edges[0] };
int main(void) {
    uint64_t state = 0x243F6A8885A308D3ULL;
    for (int i = 0; i < NE; i++) {
        for (int j = 0; j < NE; j++) {
            uint64_t a = edges[i], b = edges[j];
            state = state * 6364136223846793005ULL + 1442695040888963407ULL;
            u128 wa = ((u128)state << 64) | a, wb = ((u128)b << 64) | (state >> 7);
            show("uu", mul_uu(a, b));
            show("ss", (u128)mul_ss((int64_t)a, (int64_t)b));
            show("u32", mul_u32((uint32_t)a, (uint32_t)b));
            show("s32", (u128)mul_s32((int32_t)a, (int32_t)b));
            show("u8u16", mul_u8u16((uint8_t)a, (uint16_t)b));
            show("s8s16", (u128)mul_s8s16((int8_t)a, (int16_t)b));
            show("us", mul_us(a, (int64_t)b));
            show("su", mul_su((int64_t)a, b));
            show("s8u16", (u128)mul_s8u16((int8_t)a, (uint16_t)b));
            show("fu", mul_fu(wa, b));
            show("uf", mul_uf(a, wb));
            show("fs", mul_fs(wa, (int64_t)b));
            show("ff", mul_ff(wa, wb));
            show("shr", mul_shr(wa, wb));
            show("sar", (u128)mul_sar((i128)wa, (i128)wb));
            show("and", mul_and(wa, wb));
            show("shr63", mul_shr63(wa, wb));
            show("phi1", phi_mix(a, wb, 1));
            show("phi0", phi_mix(a, wb, 0));
            show("phi2a", phi_mix2(a, (int64_t)b, 1));
            show("phi2b", phi_mix2(a, (int64_t)b, 0));
            uint64_t limb = b;
            uint64_t carry = mac(&limb, a, b, state);
            printf("mac %016llx %016llx\n", (unsigned long long)limb, (unsigned long long)carry);
        }
        uint64_t a = edges[i];
        u128 wa = ((u128)edges[(i + 5) % NE] << 64) | a;
        show("squ", sq_u(a));
        show("sqs", (u128)sq_s((int64_t)a));
        show("kz", mul_kz(a));
        show("kzh", mul_kzh(a));
        show("kf", mul_kf(wa));
        show("kfh", mul_kfh(wa));
        show("ks", (u128)mul_ks((int64_t)a));
        show("loop", loop_acc(a, 5));
        show("loopn", loop_narrow(edges, i));

        u128 slot = 0, other = ~wa;
        show("pass", pass(wa));
        store_param(&slot, wa); show("store_param", slot);
        store_call(&slot, other); show("store_call", slot);
        copyu(&slot, &wa); show("copyu", slot);
        copyu(&slot, &slot); show("copyu_self", slot);
        show("addu", addu(&wa, &other));
        show("addu_self", addu(&wa, &wa));
        incu(&slot); show("incu", slot);
        swapu(&slot, &other); show("swap_a", slot); show("swap_b", other);
        swapu(&slot, &slot); show("swap_self", slot);
        struct packed_u128 box = { 7, 0 };
        store_packed(&box, wa); show("packed", load_packed(&box));
    }
    return 0;
}
'''

# A: parameter index 5 is a 128-bit REGISTER pair (%rcx:%r8).
# B: parameter index 5 is a 128-bit value passed ON THE STACK (only %r9 left).
ORDER_A = r'''
typedef unsigned __int128 u128;
__attribute__((noinline)) void order_a(double f0, double f1, long a, long b, long c, u128 p5, u128 *out)
{ (void)f0; (void)f1; (void)a; (void)b; (void)c; *out = p5; }
'''
ORDER_B = r'''
typedef unsigned __int128 u128;
__attribute__((noinline)) void order_b(long a, long b, long c, long d, long e, u128 p5, u128 *out)
{ (void)a; (void)b; (void)c; (void)d; (void)e; *out = p5; }
'''
# The mirrored hazard: the register pair FOLLOWS the stack-passed one.
ORDER_C = r'''
typedef unsigned __int128 u128;
__attribute__((noinline)) void order_c(double f0, double f1, long a, long b, long c, u128 p5, u128 *out)
{ (void)f0; (void)f1; (void)a; (void)b; (void)c; *out = p5 + 1; }
'''


def function_body(asm: str, name: str) -> str:
    """Instruction lines of `name` (labels, directives and comments dropped)."""
    match = re.search(rf"^{re.escape(name)}:\n(.*?)^\s*\.size\s+{re.escape(name)}\b", asm, re.S | re.M)
    if match is None:
        raise AssertionError(f"function {name} not found in assembly")
    lines = []
    for line in match.group(1).splitlines():
        line = line.strip()
        if not line or line.startswith(("#", ".")) or line.endswith(":"):
            continue
        lines.append(line)
    return "\n".join(lines)


def mnemonic_count(body: str, mnemonic: str) -> int:
    return sum(1 for line in body.splitlines() if line.split()[0] == mnemonic)


def one_operand(body: str, mnemonic: str) -> list[str]:
    return [
        line for line in body.splitlines()
        if line.split()[0] == mnemonic and "," not in line
    ]


class Base(unittest.TestCase):
    def compile_asm(self, source: str, opt: str = "-O2", env_extra: dict[str, str] | None = None,
                    extra: tuple[str, ...] = ()) -> str:
        env = os.environ.copy()
        for knob in ("CCC_NO_I128_NARROW_MUL", "CCC_NO_I128_PAIR_RELAY"):
            env.pop(knob, None)
        env.update(env_extra or {})
        with tempfile.TemporaryDirectory(prefix="i128-relay-") as td:
            src = Path(td) / "unit.c"
            out = Path(td) / "unit.s"
            src.write_text(source)
            proc = subprocess.run(
                [str(LCCC), opt, *extra, "-S", str(src), "-o", str(out)],
                capture_output=True, text=True, timeout=180, env=env,
            )
            self.assertEqual(proc.returncode, 0, proc.stderr[-2000:])
            return out.read_text()


@unittest.skipUnless(READY, "x86-64 host, fastbuild LCCC and GCC 16.2 required")
class NarrowMultiplyShape(Base):
    def test_zero_extended_operands_are_one_widening_mulq(self) -> None:
        for opt in ("-O1", "-O2", "-O3"):
            asm = self.compile_asm(KERNELS, opt)
            for name in ("mul_uu", "mul_u32", "mul_u8u16", "sq_u", "mul_shr", "mul_and"):
                with self.subTest(opt=opt, function=name):
                    body = function_body(asm, name)
                    self.assertEqual(len(one_operand(body, "mulq")), 1, body)
                    self.assertEqual(mnemonic_count(body, "imulq"), 0, body)

    def test_sign_extended_operands_are_one_widening_imulq(self) -> None:
        asm = self.compile_asm(KERNELS)
        for name in ("mul_ss", "mul_s32", "mul_s8s16", "sq_s", "mul_sar"):
            with self.subTest(function=name):
                body = function_body(asm, name)
                self.assertEqual(len(one_operand(body, "imulq")), 1, body)
                self.assertEqual(mnemonic_count(body, "imulq"), 1, body)
                self.assertEqual(mnemonic_count(body, "mulq"), 0, body)

    def test_one_narrow_operand_keeps_exactly_one_cross_term(self) -> None:
        asm = self.compile_asm(KERNELS)
        for name in ("mul_fu", "mul_uf", "mul_us", "mul_su", "mul_s8u16"):
            with self.subTest(function=name):
                body = function_body(asm, name)
                self.assertEqual(len(one_operand(body, "mulq")), 1, body)
                self.assertEqual(mnemonic_count(body, "imulq"), 1, body)

    def test_no_fact_means_the_generic_product(self) -> None:
        asm = self.compile_asm(KERNELS)
        for name in ("mul_ff", "mul_fs", "mul_shr63", "phi_mix", "phi_mix2", "loop_acc"):
            with self.subTest(function=name):
                body = function_body(asm, name)
                self.assertEqual(len(one_operand(body, "mulq")), 1, body)
                self.assertEqual(mnemonic_count(body, "imulq"), 2, body)

    def test_constant_multiplier_with_a_zero_extended_operand(self) -> None:
        asm = self.compile_asm(KERNELS)
        body = function_body(asm, "mul_kz")
        self.assertEqual(len(one_operand(body, "mulq")), 1, body)
        self.assertEqual(mnemonic_count(body, "imulq"), 0, body)
        # A constant with a high half keeps that one term; a general left
        # operand keeps the other.
        self.assertEqual(mnemonic_count(function_body(asm, "mul_kzh"), "imulq"), 1)
        self.assertEqual(mnemonic_count(function_body(asm, "mul_kf"), "imulq"), 1)
        self.assertEqual(mnemonic_count(function_body(asm, "mul_kfh"), "imulq"), 2)

    def test_kill_switch_restores_the_generic_product(self) -> None:
        """Negative control: the shape assertions above can fail."""
        asm = self.compile_asm(KERNELS, env_extra={"CCC_NO_I128_NARROW_MUL": "1"})
        for name in ("mul_uu", "mul_ss", "mul_fu", "mul_uf"):
            with self.subTest(function=name):
                body = function_body(asm, name)
                self.assertEqual(mnemonic_count(body, "imulq"), 2, body)
                self.assertEqual(len(one_operand(body, "mulq")), 1, body)
        self.assertEqual(mnemonic_count(function_body(asm, "mul_kz"), "imulq"), 1)

    def test_generic_product_never_uses_the_stack_or_r8_r9(self) -> None:
        for env in ({}, {"CCC_NO_I128_NARROW_MUL": "1"}, {"LCCC_NO_PEEPHOLE": "1"}):
            asm = self.compile_asm(KERNELS, env_extra=env)
            body = function_body(asm, "mul_ff")
            for needle in ("pushq %rdx", "popq %rdi", "%r8", "%r9"):
                self.assertNotIn(needle, body, f"{env}: {body}")


@unittest.skipUnless(READY, "x86-64 host, fastbuild LCCC and GCC 16.2 required")
class PairRelayShape(Base):
    INTEGER_FUNCTIONS = (
        "pass", "store_param", "store_call", "copyu", "addu", "incu", "swapu",
        "store_packed", "load_packed", "mac", "mul_ff",
    )

    def test_integer_functions_never_relay_through_xmm(self) -> None:
        for opt in ("-O1", "-O2", "-O3"):
            for env in ({}, {"LCCC_NO_PEEPHOLE": "1"}):
                asm = self.compile_asm(KERNELS, opt, env_extra=env)
                for name in self.INTEGER_FUNCTIONS:
                    with self.subTest(opt=opt, env=env, function=name):
                        body = function_body(asm, name)
                        self.assertNotIn("movdqu", body, body)
                        self.assertNotIn("%xmm", body, body)

    def test_kill_switch_restores_the_sse_relay(self) -> None:
        """Negative control: the stalling shape is what the switch brings back."""
        asm = self.compile_asm(KERNELS, env_extra={"CCC_NO_I128_PAIR_RELAY": "1"})
        for name in ("pass", "store_param", "store_call"):
            with self.subTest(function=name):
                body = function_body(asm, name)
                self.assertIn("movdqu", body, body)
                # ...and it is the store-forwarding failure: a 16-byte load of
                # a slot whose two halves were stored separately just above.
                lines = body.splitlines()
                load = next(i for i, l in enumerate(lines) if re.match(r"movdqu \S+\(%r[sb]p\), %xmm0", l))
                slot = re.match(r"movdqu (-?\d+)\(%r[sb]p\)", lines[load]).group(1)
                half_stores = [
                    l for l in lines[:load]
                    if re.match(rf"movq %r\w+, {re.escape(slot)}\(%r[sb]p\)", l)
                ]
                self.assertTrue(half_stores, body)

    def test_float128_functions_keep_the_16_byte_relay(self) -> None:
        asm = self.compile_asm(F128_KERNELS)
        for name in ("addq", "scaleq"):
            with self.subTest(function=name):
                body = function_body(asm, name)
                self.assertIn("movdqu", body, body)
                # No 8-byte half store may feed a 16-byte XMM load of the
                # same slot: that is the stall in the opposite direction.
                half_stored = set(re.findall(r"movq %r\w+, (-?\d+)\(%rsp\)", body))
                xmm_loaded = set(re.findall(r"movdq[ua] (-?\d+)\(%rsp\), %xmm", body))
                self.assertFalse(half_stored & xmm_loaded, body)
        # The integer sibling in the same translation unit is unaffected.
        self.assertNotIn("movdqu", function_body(asm, "store_param_i"))

    def test_no_sse_builds_never_emit_an_xmm_relay(self) -> None:
        for flag in ("-mno-sse", "-mgeneral-regs-only"):
            asm = self.compile_asm(KERNELS, extra=(flag,), env_extra={"CCC_NO_I128_PAIR_RELAY": "1"})
            for name in ("pass", "store_param", "copyu", "addu"):
                with self.subTest(flag=flag, function=name):
                    self.assertNotIn("%xmm", function_body(asm, name))


@unittest.skipUnless(READY, "x86-64 host, fastbuild LCCC and GCC 16.2 required")
class FunctionOrderIndependence(Base):
    def test_a_function_compiles_the_same_whatever_precedes_it(self) -> None:
        for opt in ("-O1", "-O2"):
            alone_b = function_body(self.compile_asm(ORDER_B, opt), "order_b")
            after_a = function_body(self.compile_asm(ORDER_A + ORDER_B.split("\n", 2)[2], opt), "order_b")
            self.assertEqual(alone_b, after_a, f"{opt}: order_b depends on order_a")

            alone_c = function_body(self.compile_asm(ORDER_C, opt), "order_c")
            after_b = function_body(self.compile_asm(ORDER_B + ORDER_C.split("\n", 2)[2], opt), "order_c")
            self.assertEqual(alone_c, after_b, f"{opt}: order_c depends on order_b")

    def test_whole_unit_is_independent_of_function_order(self) -> None:
        """Every kernel compiled alone equals the kernel inside the full unit."""
        asm = self.compile_asm(KERNELS)
        prelude = KERNELS.split("/* ---- narrow multiplies", 1)[0]
        for name, definition in (
            ("store_param", "NOINLINE void store_param(u128 *d, u128 a) { *d = a; }"),
            ("pass", "NOINLINE u128 pass(u128 a) { return a; }"),
            ("mul_uu", "NOINLINE u128 mul_uu(uint64_t a, uint64_t b) { return (u128)a * b; }"),
            ("mul_fu", "NOINLINE u128 mul_fu(u128 a, uint64_t b) { return a * b; }"),
        ):
            with self.subTest(function=name):
                self.assertIn(definition, KERNELS)
                alone = function_body(self.compile_asm(prelude + definition + "\n"), name)
                self.assertEqual(alone, function_body(asm, name))


@unittest.skipUnless(READY, "x86-64 host, fastbuild LCCC and GCC 16.2 required")
class RuntimeDifferential(unittest.TestCase):
    def test_every_shape_agrees_with_the_reference_compilers(self) -> None:
        references = [str(GCC)]
        if CLANG:
            references.append(str(CLANG))
        if ICX:
            references.append(str(ICX))
        with tempfile.TemporaryDirectory(prefix="i128-diff-") as td:
            root = Path(td)
            source = root / "probe.c"
            source.write_text(KERNELS + DRIVER)
            expected: str | None = None
            for reference in references:
                with self.subTest(reference=Path(reference).name):
                    exe = root / f"ref-{Path(reference).name}"
                    proc = subprocess.run(
                        [reference, "-O2", "-w", str(source), "-o", str(exe)],
                        capture_output=True, text=True, timeout=180,
                    )
                    self.assertEqual(proc.returncode, 0, proc.stderr[-2000:])
                    run = subprocess.run([str(exe)], capture_output=True, text=True, timeout=60)
                    self.assertEqual(run.returncode, 0, run.stderr)
                    if expected is None:
                        expected = run.stdout
                    self.assertEqual(run.stdout, expected)
            self.assertIsNotNone(expected)
            self.assertGreater(len(expected.splitlines()), 4000)

            arms: list[tuple[str, tuple[str, ...], dict[str, str]]] = []
            for opt in ("-O0", "-O1", "-O2", "-O3", "-Os"):
                arms.append((opt, (opt,), {}))
            arms += [
                ("-O2 fpo", ("-O2", "-fomit-frame-pointer"), {}),
                ("-O2 fp", ("-O2", "-fno-omit-frame-pointer"), {}),
                ("-O2 raw", ("-O2",), {"LCCC_NO_PEEPHOLE": "1"}),
                ("-O2 no-narrow", ("-O2",), {"CCC_NO_I128_NARROW_MUL": "1"}),
                ("-O2 no-pair", ("-O2",), {"CCC_NO_I128_PAIR_RELAY": "1"}),
                ("-O3 both-off", ("-O3",), {"CCC_NO_I128_NARROW_MUL": "1", "CCC_NO_I128_PAIR_RELAY": "1"}),
                ("-O2 v3", ("-O2", "-march=x86-64-v3"), {}),
                ("-O2 no-sse", ("-O2", "-mno-sse"), {}),
            ]
            for index, (label, flags, env_extra) in enumerate(arms):
                with self.subTest(arm=label):
                    env = os.environ.copy()
                    for knob in ("CCC_NO_I128_NARROW_MUL", "CCC_NO_I128_PAIR_RELAY"):
                        env.pop(knob, None)
                    env.update(env_extra)
                    exe = root / f"lccc-{index}"
                    proc = subprocess.run(
                        [str(LCCC), *flags, str(source), "-o", str(exe)],
                        capture_output=True, text=True, timeout=180, env=env,
                    )
                    self.assertEqual(proc.returncode, 0, proc.stderr[-2000:])
                    run = subprocess.run([str(exe)], capture_output=True, text=True, timeout=60)
                    self.assertEqual(run.returncode, 0, run.stderr)
                    if run.stdout != expected:
                        got, want = run.stdout.splitlines(), expected.splitlines()
                        first = next(
                            (i for i, (g, w) in enumerate(zip(got, want)) if g != w),
                            min(len(got), len(want)),
                        )
                        self.fail(
                            f"{label}: line {first}: got {got[first:first + 1]} "
                            f"want {want[first:first + 1]}"
                        )


if __name__ == "__main__":
    unittest.main()
