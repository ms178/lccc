# Red-team audit: PR #682 (`ed61e915f392`) vs PR #681

Scope: audit the competing PR **and** this branch's own prior work, with
programmatic tests and measured data. No claim below rests on reading code
alone; every finding names the experiment that produced it.

## 1. Headline: the two PRs ship the same compiler

`--compile-only` aside (see §3), the substantive question is whether #682 is a
different compiler from #681. It is not.

**Evidence A — file identity.** Nine core implementation files sha256-identical
across the two heads:

```
vectorize.rs   loop_preheader.rs   x86 IR/intrinsics   prologue
slot_census    hot_loop_metric.py  godbolt_cache.py    run_linker_tests.py
```

**Evidence B — emitted code identity.** All **51/51** corpus `.c` files
compiled at `-O2 -march=x86-64-v3` with each compiler: **51 byte-identical
assemblies, 0 differ.**

So #681 and #682 differ *only* in hardening and gates. Any claim that either
compiles better code than the other is false, and neither advances the
performance mandate relative to the other.

## 2. Where B is right and I was wrong

**B's `--compile-only` corpus gate is a genuine improvement over my #681, and
I accept it outright.** I moved the offending fixtures into subdirectories
(fixing BLOCKER-1) but installed *no gate* to stop recurrence. The corpus runner
globs `tests/regression/*.c` non-recursively and compiles every hit standalone,
so a multi-file driver dropped back in that directory fails to link — and the
only gate that would have caught it is `slow`. B adds the cheap half (compile +
link, no execution, no GCC reference build): **859 files in 60s**. That is the
right instrument at the right cost.

B's `_compile_fail_detail` is also well-judged: the raw failure is a wall of
`undefined reference to ...` naming symbols the reader has never seen. Naming
the cause turns a triage into a one-line fix.

**B's `volatile_licm.c` is better placed than my fixtures** — it is
self-contained with its own `main()`, so the corpus runner exercises it for
runtime *and* coverage for free, rather than living in a directory only my
bespoke gate can reach.

**B's volatile gate is sound and I verified it rather than assuming.** It
asserts the volatile load is *positionally* inside the loop and pairs it with a
non-volatile load in the same shape that **must** be hoisted, so a compiler
hoisting nothing cannot pass vacuously. Mutation-tested: setting the
`licm.rs` guard to `if false` and rebuilding makes the gate fail with
`HOISTED OUT OF THE LOOP` and an assembly dump. Restored afterwards.

I adopt all of the above into the union.

## 3. Where B is wrong (both defects proven, both fixed in the union)

**B-1 — `--compile-only` crashes on the exact failure it exists to catch.**
`tests/regression/run_regression.py:193` calls `pathlib.Path(src).name`, but the
module imports only `from pathlib import Path` (line 50). Reproduced: plant a
non-self-contained file, run B's gate, and it dies with
`NameError: name 'pathlib' is not defined` instead of reporting the defect.
A gate that cannot report its own detection is worse than no gate, because it
converts a red build into a confusing traceback. Fixed to `Path(src).name`;
`pyflakes` is clean. (The unrelated `nonlocal passed` warning is pre-existing
on main, not a #682 regression.)

**B-2 — `check_volatile_licm.sh` cannot print its failure summary.**
`set -euo pipefail` plus `sys.exit(fail)` aborts the script at the heredoc, so
the `rc=$?` and the `FAIL: volatile LICM structural contracts` summary are
unreachable. Demonstrated side by side: B's wrapper exits 1 having printed no
summary; the fixed wrapper prints it. The per-check output still reaches stdout,
so impact is cosmetic rather than a lost detection — but a CI log whose gate
line never appears is a real triage cost. Fixed idiomatically with
`cmd || rc=$?` (immune to `set -e`), plus removing the now-clobbering trailing
`rc=$?`.

**B-3 — B did not fix the MED/LOW findings.** `run_linker_tests.py`,
`godbolt_oracle.py` and `src/passes/mod.rs` in #682 are the *unfixed* baseline:
MED-1 (empty-oracle set passes vacuously), MED-2 (CE cache not version-pinned,
so a stale-toolchain cache is served silently), LOW-1/2 (cache decode /
temp-file handling), LOW-4 (raw `set_late_minmax_only` rather than an RAII
scope guard). `godbolt_cache.py` still decodes with `errors="replace"` and keys
temp files on `endswith(".tmp")`.

`pyflakes` across B's changed Python finds the `pathlib` undefined name that
would have caught B-1 pre-merge. Worth adding to the audit loop.

## 4. Union delivered (strictly better than either PR)

| Component | Source |
|---|---|
| `--compile-only` fast corpus-link gate + `_compile_fail_detail` | B, with B-1 fixed |
| `volatile_licm.c` self-contained fixture | B |
| `check_volatile_licm.sh` | B, with B-2 fixed |
| `volatile-spin-loop`, `volatile-pointer-subscript`, `volatile-destructuring` | mine |
| `loop_preheader` end-to-end gate + `lib_loop_bounds.sh` | mine |
| MED-1 fail-closed empty oracle; MED-2 version-aware cache; LOW-1/2 cache strictness; LOW-4 RAII guard | mine |
| `linker-oracle-verdict` pure-logic gate | both (deduped — appears once) |

Gates are wired into **both** `scripts/ci_local.sh` and
`.github/workflows/ci.yml`; `regression-corpus-link` is local-only by design,
since GitHub CI already runs the full corpus.

## 5. Performance: neither PR addresses the mandate

The shared compiler measured against system GCC and Clang, `scripts/bench_kernels.py`
(7 reps, best-of, CPU-pinned, interleaved arms, volatile sink so no arm can
delete the work; this VM exposes no PMU, `perf` or `valgrind`):

```
geomean vs gcc   1.157x      geomean vs clang  1.018x
```

That is a genuine win, driven almost entirely by one kernel — but it is not
uniform, and the losses are systematic:

| kernel | vs gcc | vs clang | shape |
|---|---|---|---|
| adler32_do8 | **9.28x** | **9.05x** | vectorized fold; LCCC wins by a lot |
| matchlen | 1.01x | 1.40x | |
| hashmix | 0.99x | 1.00x | |
| map64_sub | 1.49x | 1.00x | |
| memchr | 0.89x | **0.53x** | data-dependent branches |
| namechars | 0.87x | **0.56x** | data-dependent branches |
| adler32 | 0.85x | **0.59x** | data-dependent branches |
| classify | 0.73x | **0.56x** | data-dependent branches |
| varint | **0.64x** | 0.72x | serial branchy chain |

**Static instruction count is not a speed proxy here, and the data says so
loudly:** `adler32_do8` has 107 static instructions to GCC's 38 and runs 9.28x
*faster*, because the extra instructions are SIMD. The repo's own
`bench_kernels.py` docstring makes this point; the table above is why I did not
optimize against instruction counts.

### 5.1 A concrete, general defect this exposed

`varint` is the worst case at 0.64x. The emitted inner block is:

```asm
cmpb $-128, %r10b
movl %r11d, %edx
cmovbl %r12d, %edx
cmpb $-128, %r10b      ; <-- dead
movl $3, %ecx
movl $4, %ebx
cmovbl %ecx, %ebx
```

`cmov` **preserves flags**, and only flag-preserving instructions sit between
the two compares, so the second is provably dead. Reduced to six lines of C it
reproduces on its own, which is what makes it a real bug rather than a quirk of
this kernel:

```c
int g(unsigned char a, unsigned char b, unsigned char c, unsigned x) {
    unsigned r = x;
    if (c < 0x80) { r = a | c; x = 3; } else { r = a | (c & 0x7f); x = 4; }
    return (int)(r + x);
}
```

LCCC: 50 insns / 6 cmp / 2 cmov. GCC: 42 / 3 / 0. Clang: 34 / 1 / 1.
**GCC and Clang both decline to if-convert this at all**; LCCC converts *and*
duplicates the compare.

Mechanism: `lower_select` (`src/backend/x86/codegen/isel.rs:1157`) emits an
unconditional `Test` for the condition, and the compare/select fusion in
`src/backend/x86/codegen/comparison.rs` deliberately re-emits the compare **at
every consumer** (so the `Cmp` itself materializes nothing). With N selects
sharing one condition that is N identical compares, even though only the first
is needed. This is not varint-specific: it is a per-select tax on every
if-converted conditional in the compiler.

Fix direction: emit the replay compare once per *flag-live region* rather than
once per consumer — intervening `cmov`/`mov`/`lea`/`setcc` do not clobber flags,
so later consumers can reuse them. This is the standard RTL `compare`-CSE that
GCC performs in `combine`. It is a real, general win, and neither #681 nor #682
contains it.

**Status: diagnosed and root-caused, not yet landed.** It is recorded here
rather than shipped because it touches the shared backend lowering path and
needs a full corpus re-verification; the honest statement is that the
opportunity is quantified and the mechanism is identified, not that the
compiler is fixed.
