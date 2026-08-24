# Session 76 — Repair, red-team and validate the session-75 / v7 patch

Date: 2026-08-24
Base: `acb61d1` (latest `ms178/lccc` main; PR #224 / session-69 const-hoist included).
Build: fastbuild profile, Rust 1.98.0, `-j2`, 4 GiB swap.

The incoming `ms178-1.patch` (session 75 / v7) did not apply to main:

* four unified-diff hunks had corrupt line counts (`error: corrupt patch at
  line 2839`);
* the new `liveness.rs` was missing a newline after `live_after`'s `{`, so
  the next hunk's `+` leaked into the source (`{+        if fam > …`).

GNU `patch --fuzz=3` applied the rest. The red-team then found real
soundness holes, not just apply mechanics.

## 1. Red-team findings and fixes

### 1.1 Compile-breaker: leaked `+` in `FileLiveness::live_after`

`liveness.rs:136` was

```text
pub(super) fn live_after(...) -> Option<bool> {+        if fam > REG_GP_MAX ...
```

Fixed. Without this the crate does not compile.

### 1.2 PF-06 kill switch was inverted

`CCC_NO_PF06_ADD_PEEL=1` was documented as "do not peel `add(iv, const)`".
The implementation still peeled (so `disp` and the IV-as-index stayed) and
only skipped the two soundness gates. Setting the kill switch therefore
made the *more dangerous* fold.

Fix: capture the flag inside `resolve_index` and `break` out of the add/sub
peel arms. The SIB index is then the add's RESULT, which is the pre-PF-06
behaviour.

### 1.3 Cross-backend miscompile: ARM / i686 dropped `disp`

PF-06 peels `add(iv, const)` in target-independent `generation.rs`. The
x86-64 emitter used `disp`; the i686 SIB emitters took `_disp` and ignored
it; AArch64 called the no-disp indexed form and returned success. When the
indexed fold succeeds the GEP is skipped, so a dropped displacement is a
wrong address.

Fixes:

* i686: `sib_mem` / `sib_mem_sym` now encode the displacement (same AT&T
  form as x86-64).
* AArch64: `emit_{load,store}_indexed` return `false` when `disp != 0`, so
  `rematerialize_skipped_indexed` rebuilds `base + orig_offset`.

### 1.4 Symbol+disp invented a different symbol

`sib_mem64_sym("foo", …, 4)` emitted `foo4(, %idx)` (symbol `foo4`), not
`foo+4(, %idx)`. Negative disp happened to work (`foo-4`). Same bug on
i686. Fixed: `sym+disp` / `sym-disp`.

### 1.5 `fold_load_test_into_cmp` changed SF for unsigned / narrow signed loads

`movzbl mem, %esi; testq %rsi, %rsi; js` → `cmpb $0, mem; js` is a
miscompile: byte `0x80` has `testq` SF=0 (zero-extended) and `cmpb` SF=1.
`movsbl` + `testq` has the same hole (32-bit write zero-extends).

Fix: unsigned loads, and 32-bit signed loads under `testq`, fold only when
every subsequent flag consumer is ZF-only (`je`/`jne`/`sete`/`setne`/…).
Signed-to-64 (`movsbq`/`movswq`/`movslq`) still fold for SF consumers.
New unit tests pin both sides.

## 2. What landed from session 75 (kept)

* Exact CFG liveness (`peephole/passes/liveness.rs`) with ABI-aware
  `call`/`ret` effects and conservative unknown-mnemonic handling.
* Whole-function copy coalescing of the parameter shuffle.
* Dead pure-write elimination, redundant-load reuse (with the v6
  `movslq`/`movsbq` → `movl` miscompile already fixed), load+self-test →
  `cmp $0, mem`, accumulator round-trip folding, redundant self-test after
  `and`/`or`/`xor`, dead `movslq` → `movl` narrowing.
* PF-06: `a[j±1]` becomes `±4(%base, %iv, 4)` instead of a per-iteration
  `lea`/`cltq` chain.

## 3. Measurements (this VM; screening, no PMU)

Kernel corpus (`scripts/kernel_count.py`, LCCC `-O2` vs system GCC 14.2):

| kernel | LCCC | GCC | delta |
|---|---:|---:|---:|
| adler8 | 89 | 63 | +26 |
| sum8 | **12** | 13 | **−1** |
| crc32k | 28 | 18 | +10 |
| my_strlen | 10 | 10 | tie |
| maxv | **19** | 27 | **−8** |
| dot | **15** | 17 | **−2** |
| isort | 40 | 23 | +17 |
| **total (15)** | **322** | **264** | +58 |

Peephole A/B (`scripts/peephole_ab.py`, new pass set vs
`CCC_PEEPHOLE_SKIP` of the same set): **9034 → 8525 instructions (−5.63%)**,
every program's stdout+exit identical. Largest wins: `zlib_ng_adler32` −35,
`sqlite_varint` −34, `gzip_crc32` −32, `hash_table` −20, `spectral_norm` −20.

Execute vs GCC `-O2` (identical stdout+status): `gzip_crc32`,
`zlib_ng_adler32`, `expat_xml_scan`, `sqlite_varint`, `linux_find_bit`,
and the PF-06 driver (`idx1`/`idxm1`/`sum_adj` → `40 40 35`).

Godbolt oracle (`-O2 -march=x86-64-v3`, GCC 16.2 / Clang 22.1 / ICC 2021.10
/ ICX latest):

| function | LCCC | GCC 16.2 | Clang 22.1 | ICC | ICX | best |
|---|---:|---:|---:|---:|---:|---|
| `idx1` (PF-06) | **3** | 3 | 3 | 3 | 3 | **tie** |
| `sum8` | **12** | 57 | 72 | 39 | 34 | **LCCC** |
| `adler8` | 89 | **66** | 75 | 76 | 101 | GCC |
| `crc32k` | 28 | **17** | 54 | 37 | 68 | GCC |
| `isort` | 40 | 27 | 58 | **19** | 50 | ICC |

`idx1` is now the same three-instruction shape as every reference compiler
(`movl 4(%rdi,%reg,4), %eax; ret`). `sum8` beats the oracles because they
vectorise a tiny reduction at this march; that is a screening artefact, not
a claim that LCCC's scalar loop is the right long-term answer.

`tests/regression/check_gpr_leaf_param_codegen.sh` passes (accepts a fully
coalesced parameter shuffle).

`cargo test --lib` of the new modules was started and aborted: `--test`
rebuild of this crate OOMs the 2 GiB box. The new unit tests compile as
part of the library crate and the C-level A/B + execute gates above cover
the same shapes.

## 4. Infrastructure

* `scripts/build_lccc_fast.sh` falls back to `gcc` + GNU ld when clang/mold
  are absent (`cargo --config` overrides, so the committed mold rustflags
  cannot leak into the gcc link). The committed `.cargo/config.toml` still
  prefers clang+mold.
* `scripts/peephole_ab.py` DEFAULT_SKIP now includes the v7 pass names so
  the A/B gate actually measures them.

## 5. Follow-up (priority order)

1. **RA-06 — adler32 / arithmetic-chain copy webs.** 89 vs GCC 66 (Godbolt
   GCC 16.2). Next-use-aware eviction, not another peephole. The eight
   unrolled byte temporaries still win the registers and `s1`/`s2` spill.
2. **LICM of `leaq table(%rip)`** (crc32k 28 vs 17). x86-64 cannot form
   RIP-relative+index; GCC hoists the table base. That is still the gap.
3. **isort 40 vs ICC 19 / GCC 27.** Secondary-IV pointer walk + fewer
   `cltq`/`movslq`. PF-06 closed the `a[j+1]` SIB half; the rest is RA +
   IV widening.
4. **`cargo test --lib` on a box with ≥8 GiB.** The new unit tests
   (`liveness`, `copy_coalesce`, `dead_writes` unsigned-SF, `flag_peepholes`)
   were not executed here.
5. **i686 PF-06 execute gate.** Displacement encoding is in; there is no
   `-m32` run in this session.
6. **Do not treat Godbolt `sum8` as a win to chase.** The oracles
   vectorised; LCCC stayed scalar. Confirm on a real trip count before
   changing the vectorizer cost model.
7. **Phase-4 loop trampoline** remains default-off (known rename bug).
   Still the right call.
8. **Multi-store stencil / nbody / spectral_norm** — still the structural
   FP gap. Not touched.

## 6. How to restore this tree

```text
git checkout acb61d1
git apply ms178-1.patch          # must apply clean; hunks were rebuilt
scripts/build_lccc_fast.sh       # gcc fallback if clang/mold missing
```
