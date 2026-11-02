# Cross-domain comparisons: the unsigned reading of a sign extension

`simplify`'s `cross_domain_sign_test` (§17 of
`engineering/llvm-missed/SCALAR-PREDICATE-FOLDS.md`). The refusal §16 recorded
(`keep_schar_ult`) is now a proof, and the probe is faster than both oracles.

## Reproducing

```sh
# the shapes and their instruction counts, per backend
LCCC=../../../target/fastbuild/lccc
for t in "" -i686 -arm -riscv; do
    $LCCC$t -O2 -S ../../../tests/regression/cross_domain_sext_cmp_asm.c -o on$t.s
    CCC_SIMPLIFY_SKIP=cmprange $LCCC$t -O2 -S ../../../tests/regression/cross_domain_sext_cmp_asm.c -o off$t.s
    python3 asm.py count on$t.s off$t.s
done

# the values (also run, at five optimization levels, by the gate)
$LCCC -O2 ../../../tests/regression/cross_domain_sext_cmp.c -o /tmp/xd && /tmp/xd
```

`asm.py` counts instructions inside a function body (`func:` to `.size`), which
is the unit every table here is in: the `ret` is counted, the directives and the
`.L` labels are not, and `#` is a comment only where it is not an instruction
immediate (AArch64/RISC-V spell immediates `#0`).

## The measurements (2026-11-02, sandbox: GCC 12.2.0, Clang 22.1.8 via `zig cc`,
## no ICX; CI's oracles are GCC 16.2 / Clang 23.1.4 / ICX 2026.2)

x86-64, `-O2`, shape file, `ret` included:

| shape | off (`CCC_SIMPLIFY_SKIP=cmprange`) | on | gcc 12.2 | clang 22.1.8 |
| --- | --- | --- | --- | --- |
| `(unsigned)(signed char)x < 300` | 5 | **4** | 4 | 4 |
| `(unsigned)(signed char)x <= 300` | 5 | **4** | 4 | 4 |
| `(unsigned)(signed char)x >= 300` | 5 | **3** | 3 | 3 |
| `(unsigned)(signed char)x > 300` | 5 | **3** | 3 | 3 |
| `(unsigned)(short)x >= 70000` | 5 | **3** | 3 | 3 |
| `(unsigned long long)(int)x >= 2^64-2^31` | 5 | **3** | 3 | 3 |
| `(unsigned long long)(int)x > 2^64-2^31-1` | 6 | **3** | 3 | 3 |
| `(unsigned)(signed char)x < 5` (control) | 5 | 5 | 4 | 4 |
| `(unsigned)(short)x < 300` (control) | 5 | 5 | 4 | 4 |
| `(unsigned)(signed char)x >= 0xffffff81` (control) | 5 | 5 | 5 | 5 |
| `(unsigned long long)(int)x >= 2^64-2^31+1` (control) | 5 | 5 | 5 | 5 |
| `(unsigned long long)(int)x <= 2^64-1` (control) | 5 | 5 | 5 | 5 |

The controls are one step outside the proven band; a guard that widened by one
would fold them here. The two "inside a half" controls are the 1-instruction gap
that stays: GCC reads the low bits unsigned (`cmpb $4, %dil; setb`), which the
IR cannot express without an unsigned compare of a signed-typed narrow value.

Other backends, same shapes (on / off): **no shape gets longer**.

| backend | byte shapes | 16-bit shape | 64-bit shapes |
| --- | --- | --- | --- |
| x86-64 | 5 -> 4 (and 5 -> 3) | 5 -> 3 | 5 -> 3, 6 -> 3 |
| i686 | 10 -> 10 | 10 -> 10 | 16 -> 12 |
| AArch64 | 12 -> 12 | 15 -> 13 | 14 -> 12, 13 -> 12 |
| RISC-V | 16 -> 16, 17 -> 15 | 17 -> 15 | 11 -> 9, 10 -> 9 |

AArch64's byte shapes are at parity, not a win, and that is the point of the
table: before the narrow compare took its constant as an immediate they were
12 -> **14**, a regression the gate's per-backend length check refuses. The
`cmp w1, #imm` path is a win of its own for every narrow compare against a
constant.

## What the test caught

The first version of the guard used `hi < C` / `C <= U-lo'` for all four
predicates. `(unsigned)(signed char)-128 <= 0xffffff80` is true; the sign test
said false. One value per source width, invisible to an instruction count and
to a probe sweep that did not sit exactly on the boundary — the whole-domain
unit test found it on its first run, and the C battery grew the boundary probes
(`*_le_nl`, `*_ge_nl`, `*_gt_nl1`) so a runtime differential catches it too.
