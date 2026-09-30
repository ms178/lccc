# The oracle metric was ranking us against a mirage — measured correction

**Date:** 2026-09-30 · branch base `71e3b1ae` (main, PR #686 merged)
**Flags:** `-O2 -march=x86-64-v3` throughout.

## 0. Why this document exists

The standing work queue was ordered by `scripts/codegen_oracle.py --rank`, which
said we were 1899 instructions behind across 100 functions, worst first
`zlib_ng_adler32::main` at **204 behind `icc=73`**. I went to close that gap and
found instead that **the metric was measuring inlining policy, not code
generation.** Every number below came from a tool call; where a number of mine
turned out to be wrong, the wrong one is recorded next to the right one.

## 1. The #1 gap is an artifact of who inlines

`scripts/oracle_asm.py` on `zlib_ng_adler32.c`, function `main`:

```
icc main: 73 instructions, 4 calls:
    call __intel_new_feature_proc_init
    call printf
    call zlib_ng_adler32_c..0
    call zlib_ng_adler32_c..1
```

ICC's `main` is small because ICC left `zlib_ng_adler32_c` **out of line as two
specialised copies** (`..0`, `..1`). The single-function oracle never measured
those copies, so 73 is not the size of ICC's work — it is the size of ICC's
*stub*. LCCC inlines everything into `main`, so 277 is the size of the whole
computation. The two numbers are not comparable, and their difference is not a
codegen gap.

Whole translation unit (`codegen_oracle.py --all-functions`), same file:

| compiler | insns | calls | stack refs |
|---|---:|---:|---:|
| **lccc** | **261** | 1 | 15 |
| gcc16.2 | 156 | 1 | 2 |
| clang | 148 | 0 | 0 |
| icx | 147 | 0 | 3 |
| icc | 73 | **4** | 9 |

`zlib_ng_adler32_c` is not emitted as a separate function by anyone who inlines
it — the oracle reports "function not emitted" for all five, which is the direct
confirmation. Against the three compilers that fully inline (gcc 156, clang 148,
icx 147) the honest gap is **~105–114 instructions, not 204.** Roughly half the
headline number was an inlining decision.

## 2. Whole-TU on the six "worst" files: we are second, not sixth

`codegen_oracle.py --all-functions --totals` over `zlib_ng_adler32`, `nbody`,
`moving_stats`, `i686_alu_chains`, `glibc_strstr`, `matmul` — the six the
per-function rank put at the top of the deficit table:

| compiler | insns | loads | stores | spills | branches | vectors |
|---|---:|---:|---:|---:|---:|---:|
| gcc16.2 | **1169** | 175 | 92 | 87 | 134 | 184 |
| **lccc** | **1613** | 379 | 111 | 151 | 157 | 83 |
| icc | 2303 | 514 | 136 | 315 | 138 | 54 |
| clang | 2716 | 636 | 76 | 139 | 201 | 328 |
| icx | 4104 | 796 | 200 | 430 | 270 | 708 |

ICC — the compiler the rank table said was 204 instructions ahead of us on
adler32 — is **43 % larger** than us across these six files. Clang is 68 %
larger, ICX 2.5×. Per file, `glibc_strstr` whole-TU is lccc 194 / gcc 182 /
icc 197 / icx 261 / clang 319: we are within 7 % of GCC and ahead of the rest,
where the per-function rank had us 70 behind.

**Conclusion:** on whole-translation-unit static size we are a clear second
behind GCC and well ahead of ICC, Clang and ICX. The per-function `--rank`
ordering systematically flatters any compiler that declines to inline, because
it charges the callee's body to nobody.

## 3. But the corpus-wide total flatters *us* — the honest number is +15.8 %

I then measured the whole corpus myself (51 programs, whole TU, same flags) and
got a headline that looked like a win:

| set | n | LCCC | GCC | delta |
|---|---:|---:|---:|---:|
| all benchmark programs | 51 | 6869 | 7851 | **−12.5 %** |
| the 4 recursion benchmarks | 4 | 172 | 2068 | −91.7 % |
| **the other 47** | 47 | **6697** | **5783** | **+15.8 %** |

The −12.5 % is entirely carried by `binary_trees` (lccc 132 / gcc **1583**),
`fib` (24/238), `ackermann` (7/123) and `constant_recursion` (9/124) — four files
where GCC explodes. Remove them and **we are 15.8 % larger than GCC, larger on
37 of 47 files.** The robust statistic is the per-file median:

```
median LCCC/GCC instruction ratio over the 47 = 1.17
LCCC smaller on 10 of 47
```

A total is the wrong statistic for this corpus because four files can dominate
it in either direction. The median ratio, and the larger/smaller counts, are the
numbers to quote. **This applies to my own earlier "−12.5 %" as much as to the
oracle's "204": both are totals over a set with heavy outliers.**

## 4. Two hypotheses of mine that the data rejected

Recorded because the rejections are more useful than the confirmations.

**(a) "Spills are the dominant defect."** The rank table's spill column looked
decisive (`i686_alu_chains` 75, `sha256_transform` 61, `struct_copy` 60).
Measured over the 77 functions behind:

```
corr(gap, spills)        = +0.453
corr(gap, spills/insns)  = +0.305
corr(gap, loads+stores)  = +0.739
median spill fraction    = 0.0 %
functions with >=20 % spill refs: 5 of 77, holding 242/1899 = 13 % of the gap
```

Spills are a real but *minority* effect. Total memory traffic is the correlated
quantity, and most functions do not spill at all. `nbody` — the worst real gap —
has **zero** stack references.

**(b) "A register-coalescing peephole (`leaq K(%r),%t; movq %t,%r` → `addq $K,%r`)
is a systemic win."** I found it in adler32's `len_16` loop and generalised too
fast. Corpus-wide it occurs **12 times in 6869 instructions**, and
register-to-register `movq` occurs 448 times against GCC's 396. The pattern is
real, it is worth fixing, and it is not where the 15.8 % lives.

## 5. What the 15.8 % actually is

Corpus-wide mnemonic census, 51 programs, `-O2 -march=x86-64-v3`:

| pattern | LCCC | GCC | ratio | reading |
|---|---:|---:|---:|---|
| `movsd` (scalar FP move) | **171** | 54 | **3.2×** | redundant FP reg-to-reg traffic |
| `leaq sym(%rip)` (static base) | **145** | 91 | **1.6×** | invariant bases re-materialised |
| `imul $const,` (index scaling) | **87** | 55 | **1.6×** | index form, not strength-reduced |
| `movq %r,%r` | 448 | 396 | 1.13× | coalescing, minor |
| stack refs | 655 | 773 | 0.85× | we are *better* here |

The same three patterns, concentrated, are visible in the worst single case.
`nbody`'s inner loop (5 bodies × 5 000 000 iterations — the hottest loop in the
corpus):

```
GCC  .L2 : 14 instructions
    vsubsd 8(%rax), %xmm7, %xmm2      displacement addressing
    vsubsd (%rax), %xmm8, %xmm1
    addq   $1, %rdx
    addq   $56, %rax                  stride bump, one instruction
    ...
    cmpl   $5, %edx
    jne    .L2

LCCC .LBB7: 110 instructions   (7.9x)
    cmpl   $5000000, -72(%rbp)        outer bound re-read from a stack slot  x3
    movslq %r13d, %r9
    imulq  $56, %r9, %r15             index*56 by MULTIPLY                   x2
    leaq   bodies(%rip), %rcx         static array base re-materialised      x2
    ...
    movsd  %xmm15, -160(%rbp)         FP value spilled                       x2
    mnemonic census: movsd=29 movupd=17 movq=10 -> 56 of 110 are data movement
```

GCC walks the array with one `addq $56, %rax` and reaches every field through a
displacement. We keep the loop in **index form**, recompute `i*56` with a
multiply twice per iteration, re-materialise the static base twice, and move the
results around with 56 scalar FP moves. That is the signature of a missing
**induction-variable strength reduction / IVopts** stage plus weak loop-invariant
address hoisting, and it is the same signature in `spectral_norm` (+114 %),
`moving_stats` (+66 %), `struct_copy` (+62 %) and `matmul` — i.e. in the
struct-array and FP kernels that dominate the top of the real deficit table.

## 6. Landed: the metric now says when it cannot be trusted

Counting call sites is cheap and it is the whole tell, so `--rank` now carries a
`calls` column, marks each row comparable or not, and prints a warning naming
the rows it cannot rank. Run over the corpus after the change:

```
WARNING: 16 of the 77 'behind' rows compare functions with DIFFERENT call
counts, so their gap measures an inlining decision, not code generation:
  zlib_ng_adler32:main (lccc 1 calls vs icc 4)
  nbody:main (lccc 2 calls vs gcc16.2 4)
  i686_alu_chains:main (lccc 7 calls vs icc 8)
  glibc_strstr:main (lccc 1 calls vs gcc16.2 2)
  ...
```

**Those 16 rows carry 781 of the 1899-instruction headline deficit — 41 % — and
include both of the top two.** The comparable remainder is 1118. The marker
discriminates rather than blanket-flagging: `moving_stats::main` is 1 call on
both sides, so its 90-instruction gap is real and stays unmarked.

Deliberate detail: `jmp`/`b`/`j` are **not** counted as calls. A compiler that
specialises a callee out of line often reaches it with a tail jump, and counting
that as a call would mark the row comparable and hide precisely the artifact the
guard exists to expose.

Covered by `scripts/test_codegen_oracle.py` (11 known-answer cases, pure logic,
no network), registered as `gate "codegen-oracle-metric"` in
`scripts/ci_local.sh` and mirrored in `.github/workflows/ci.yml`. Both mutations
were checked: counting plain jumps as calls fails 1 test, and forcing the
comparability flag to "comparable" fails 2.

## 7. Consequences for the tooling

1. `--rank`'s per-function deficit must not be used to order work on `main`
   (or any function whose size depends on inlining). Either compare whole-TU
   totals, or report the call count beside the instruction count so a reader can
   see that the two compilers are not computing the same thing in the same
   place.
2. Corpus aggregates must be reported as a **median ratio plus larger/smaller
   counts**, not a total, on a corpus with recursion blow-ups.
3. Static instruction count is a screening metric only. Nothing here is PMU
   evidence; the runtime claims in this tree continue to come from
   `bench_kernels.py` / `perf_ab.py`.

## 8. Reproduction

```sh
# 1. the inlining artifact, one function, all oracles
python3 scripts/oracle_asm.py tests/benchmark/programs/zlib_ng_adler32.c \
        --function main --flags "-O2 -march=x86-64-v3" --local target/fastbuild/lccc

# 2. the honest whole-TU comparison on the six "worst" files
python3 scripts/codegen_oracle.py --all-functions --totals \
        tests/benchmark/programs/{zlib_ng_adler32,nbody,moving_stats,i686_alu_chains,glibc_strstr,matmul}.c \
        --local target/fastbuild/lccc --flags "-O2 -march=x86-64-v3" \
        --local-flags "-O2 -march=x86-64-v3"

# 3. corpus totals; note the four recursion files before quoting any total
python3 scripts/codegen_oracle.py --all-functions --totals \
        tests/benchmark/programs/*.c --local target/fastbuild/lccc \
        --flags "-O2 -march=x86-64-v3" --local-flags "-O2 -march=x86-64-v3"
```
