# Follow-Up Work: Red-Team Audit of the “Perfected Rust Modernization & Benchmark Corpus Expansion” Patch

**Date:** 2026-09-06 (audit session)
**Author:** LCCC Research & Performance Engineering Agent
**Repo:** `ms178/lccc`
**Audited artifact:** `0001-Perfected-Rust-modernization-benchmark-corpus-expans.patch` (1.28 MB, 122 files)
**Upstream base at audit time:** `6af3436` (Merge PR #430)
**Session commits:** `25938ce0`…`e6c6106d` (9 commits on top of `6af3436`, see `git log 6af3436..HEAD`)

---

## 1. Executive audit verdict

**The patch is 100 % merged upstream already.** It applies cleanly to `c8116df`
(Merge PR #428, its exact base) and the resulting tree is **byte-for-byte
identical** to upstream `main` at `6af3436` — i.e. upstream commit `a459716`
(“Modernize Rust usage, expand benchmark corpus to 39 workloads, …”) *is* this
patch. As a *delta* it is therefore **obsolete**: re-applying it to current
main is a no-op (it would not even apply, since main already contains it).

**However, “merged” ≠ “beneficial”.** The red-team audit of its *content*
(found empirically, each item reproduced on this machine) found that the patch
carries **four genuine regressions** that are now in `main`, plus several
documentation-integrity defects. All regressions found are fixed and validated
in this session’s commits (see §3); the good parts (§2) are kept.

### Scorecard

| Patch section | Verdict | Evidence |
|---|---|---|
| Rust modernization (`LazyLock`, `is_some_and`/`is_none_or`, `split_at_checked`, `div_ceil`, `align_up`, `format!` labels) | **Keep** — behavior-preserving, warning-clean, 2019 unit tests + 676-case regression corpus pass | `cargo test` 2019/0; regression 676/0 pre- and post-fix |
| `strip_circumfix` → manual prefix/suffix slicing | **Keep, but rationale wrong** — `str::strip_circumfix` is *stable* in 1.98.1 (verified by compiling it); the “unstable API” justification is false, the replacement is merely harmless | test program compiled+ran on stable 1.98.1 |
| `gcc_linker` cfg-gating of linker re-exports + sysroot helpers | **REVERTED (was a pure regression)** — broke `cargo check --features gcc_linker` (3 hard errors) while fixing zero warnings; base compiled `-D warnings`-clean with the feature | controlled experiment: base passes, patched main fails, fixed main passes (all feature combos) |
| Benchmark corpus +6 kernels (chacha20, sha256, rbtree, zstd_count, lz4, strstr) | **Keep** — provenance-documented, self-checking (RFC 7539 / FIPS vectors), all pass the runner’s differential oracle | full-corpus run in this session |
| `fuzz_diff.py` “unification” of csmith/yarpgen testers | **BROKEN — repaired** — csmith/yarpgen engines were unimplemented stubs printing “Engine requested.” and exiting **0**; wrappers therefore validated nothing; also only `refs[0]` was compared | reproduced: `csmith_diff.py --count 3` → `TOTAL: 0`, exit 0; now 3/3 PASS with real csmith 2.3.0 |
| ci-bench.py / bench.yml rewrite | **REVERTED (accidental revert of 19447a0)** — restored the historical five-benchmark standalone runner, silently undoing the full-corpus CI design; the six new kernels were never run in CI | `diff` against `19447a0^` shows byte-identical revert |
| `verify_fib_perf.sh` rewrite | **REVERTED (accidental revert of 19447a0)** — restored the `bc`-dependent timing that 19447a0 had removed for bc-less CI images; usage line pointed at a nonexistent script | script runs again with Python clock; 141× fib speedup verified |
| README rewrite | **Keep with corrections** — honest loss table retained, but oracle table cherry-picked comparators and cited non-reproducible instruction counts; benchmark table had no provenance | oracle re-run: see §3.2 |
| STATE.md rewrite | **Keep** — links verified to resolve from `engineering/`; kill-switch documentation retained |
| Evidence-dir deletion (31 k-line results.json) | **Acceptable** — but it removed the backing data for the README’s 39-workload table; fresh screening evidence regenerated this session | `engineering/evidence/benchmarks/2026-09-06-audit-6af3436/` |
| chmod +x sweep (58 files) | **Keep** — all files carry shebangs | checked programmatically |

---

## 2. What the audit agrees with (and why)

1. **Rust modernization is fine.** Every `map_or(true, ..)` → `is_none_or`,
   `map_or(false, ..)` → `is_some_and`, `OnceLock::get_or_init` → `LazyLock`,
   `split_at(rest.len().checked_sub(n)?)` → `split_at_checked`, and the
   `NumBuffer` → `format!` label change is semantics-preserving. The removed
   `fresh_label` spelling test is a small coverage loss, but the behavior is
   now trivially visible in one line.
2. **The six workload kernels are exemplary corpus citizens**: each has an
   embedded known-answer check (RFC 7539 §2.3.2 vector for ChaCha20, FIPS
   180-4 “abc” for SHA-256, …), prints a checksum for the runner’s
   byte-for-byte differential oracle, and carries full provenance
   (`WORKLOAD_PROVENANCE.md`: package selector, source digests, licenses,
   adaptation boundary).
3. **The docs’ honesty about losses** (README lists chacha20 5.5×/lz4 3.1×/
   sha256 1.95× slower than best reference) is exactly the right research
   posture — wins *and* losses, with root causes in the follow-up backlog.
4. **Script unification as a goal is sound** (one comparison engine, one
   reporting schema, wrappers for compatibility) — the execution was defective
   (§1), not the idea. The repaired `fuzz_diff.py` keeps the architecture and
   makes it real.
5. **mold X86/i686 preset**: `tests/linker/setup_oracles.sh` already encodes
   the exact option (`-DMOLD_TARGETS='X86_64;I386'`) plus bfd 2.47 pinning
   and git-HEAD mold/wild policy (stale oracles previously produced two
   *false* lccc failures — see `engineering/DECISIONS.md`). This session adds
   the missing **lld 23.1** oracle with the analogous LLVM trick
   (`LLVM_TARGETS_TO_BUILD=X86`).

## 3. What the audit disagreed with (all fixed & validated this session)

### 3.1 `gcc_linker` feature build was broken (commit `d5b4c0cb`)

Controlled experiment on this machine:

| Tree | `cargo check --features gcc_linker` |
|---|---|
| `c8116df` (patch base) | **passes, even with `-D warnings`** |
| `6af3436` (patch merged) | **3 hard errors** (`linker_entry.rs:158,175` cannot find `link_builtin`/`link_shared`; `i686/linker/shared.rs:14` unresolved `exists_with_sysroot`/`with_sysroot_prefix`) |
| `6af3436` + fix | **passes `-D warnings`** — and so do `gcc_assembler` and `gcc_linker,gcc_assembler` combos |

The gates were “hygiene” with no warning to fix — `lccc-ld`’s driver
(`src/linker_entry.rs`) and the always-compiled i686 shared-library linker
legitimately consume those items under every feature combination. Fix: gates
removed with explanatory comments at both sites.

### 3.2 CI benchmarked 5 of 39 kernels (commit `d5b4c0cb`)

The patch restored `.github/scripts/ci-bench.py` and `bench.yml` **byte-for-byte
to their pre-`19447a0` state** (verified by diff against `19447a0^`) — the old
standalone runner with a hardcoded five-program list (`arith_loop, fib,
matmul, qsort, sieve`). Consequences had it stood: the six new workload
kernels (the patch’s own headline feature) would never run in CI, and the
drift-prevention rationale documented in `19447a0` (“preventing a second,
permanently smaller benchmark list from drifting”) was silently discarded.
Fix: restored the exact `19447a0` thin-wrapper + workflow (corpus count
comment refreshed 33 → 39), validated locally (`--list` shows all 39).

### 3.3 Differential testers were silent no-ops (commit `d5b4c0cb`)

`csmith_diff.py` and `yarpgen_diff.py` were replaced by wrappers into
`fuzz_diff.py`, whose engine dispatch ended in `else: print(f"Engine {args.engine} requested.")`
— zero tests, `TOTAL: 0`, **exit 0**. Reproduced empirically before the fix.
This is the worst failure class a validation tool can have: it *reports
success while validating nothing*. Additional defects in the same file:
only `ref_compilers[0]` was ever compared (clang silently ignored);
no compile/run timeout knobs; the stress-suite engine invoked
`gen_fp_stress.py` without its required seed argument and treated the
self-contained `unroll_stress.py` (which needs `--lccc` and is itself a
differential tester, not a generator) as a stdout generator — 2 of 4 stress
cases always skipped.

Fix (validated):
- real csmith engine (csmith 2.3.0 from Debian + `libcsmith-dev`: 3/3 PASS vs
  gcc+clang across `-O0..-O3`);
- real yarpgen engine (generation ported; loud error when the binary is
  absent — yarpgen is not packaged, build it from
  https://github.com/VoR0n0k/yarpgen to use);
- **all** selected references compared, and references must agree with each
  other before LCCC is judged (disagreeing references ⇒ SKIP, not a bogus
  oracle);
- `--compile-timeout`/`--run-timeout`; legacy flag translation
  (`--ccc`→`--lccc`, `--clang`/`--gcc`→`--refs`, `--jobs`→`-j`,
  `--tests`→`--count`, `--seed-start`→`--seed`, `--out-dir`→`--repro-dir`,
  accepted-and-reported no-ops for retention flags) so every invocation in
  `DIFFERENTIAL_TESTING.md` keeps working;
- `--count 0`/`--tests 0` = infinite (bounded in-flight futures);
- stress generators invoked with their real CLIs; `unroll_stress` run as a
  self-contained tester with a bounded `--limit 12 --batch 12` sample
  (full-space sweeps remain a direct invocation);
- `--check-engines` wiring guard added **and wired into `ci.yml`** together
  with a 4-case synthetic differential smoke against the freshly built
  compiler, so a future stub regression cannot pass CI.

### 3.4 `verify_fib_perf.sh` regressions (commit `d5b4c0cb`)

Restored the `19447a0` Python-monotonic-clock implementation (the patch had
reverted to `bc`-based timing, which fails on bc-less hosts — `bc` is absent
here too) and fixed the usage line that referenced the nonexistent
`tests/bench_fib_verify.sh`. Verified: **PASS — LCCC 141× faster than GCC**
on recursive fib (3 reps), plus the comprehensive rec2iter check including
fib(90).

### 3.5 Documentation integrity (commits `5e51b140`)

- **README oracle table**: previous numbers were not reproducible with the
  checked-in kernel sources and `codegen_oracle.py` defaults. Re-verified
  live against the pinned Compiler Explorer channels (GCC 16.2 = `cg162`,
  Clang 23.1 = `cclang2310`, ICC 2021.10 = `cicc2021100`, ICX = `cicxlatest`):
  - `glibc_strstr`/`two_way_short_needle` @ `-O3 -march=x86-64-v3`:
    **LCCC 75** vs GCC **144** (1.92× smaller — genuine LCCC win);
    clang/icc/icx inline the static callee (not comparable at function
    granularity). At **`-O2` GCC emits 58 and beats LCCC’s 75** — the win is
    flag-dependent, and the README now says so.
  - `sha256_transform` whole-TU @ `-O3`: Clang **267** (44 vector insns),
    GCC **275** (37 vector), ICC 438, **LCCC 444 (0 vector insns, 194
    spills)**, ICX 1145 — LCCC is 0.60× the best. The old “beats ICX by
    4.6×” framing cherry-picked the weakest comparator.
  - `zstd_count`/`ZSTD_count`: only LCCC emits the static function (62
    insns) at `-O3`; all references inline it — the old row was not a
    comparable measurement.
- **README benchmark table**: added provenance/interpretation note (bare-metal
  origin, screening-matrix caveat: the 70×/68×/32× ratios are rec2iter
  specialization GCC deliberately does not perform; codec/parser kernels are
  honest losses with documented root causes).
- **README quickstart**: replaced the false “sub-second incremental” claim
  (~2–3 min cold on a 2-core VM, seconds incremental) and the mold claim
  (gcc/bfd unless clang+mold present).
- **`scripts/DIFFERENTIAL_TESTING.md`** rewritten for the unified harness with
  the legacy-flag translation table; **`scripts/README.md`** wrapper entries
  updated.
- **`engineering/FOLLOWUP-2026-09-06-…md`** gained a correction addendum
  recording the re-verified oracle data (history preserved, errors corrected).

## 4. Validation performed this session (all on the fastbuild/-O1/-j2 + 8G swap policy)

| Gate | Result |
|---|---|
| `cargo test --profile fastbuild --all-targets --locked -j 2` (RUSTFLAGS `-D warnings`) | **2019 passed, 0 failed, 6 ignored** |
| `cargo check --features gcc_linker` / `gcc_assembler` / both, `-D warnings` | **all pass** (gcc_linker was broken pre-fix) |
| Regression corpus (`run_regression.py`, `CCC_VALIDATE_SSA=1`, i686 multilib installed) | **676 passed, 0 failed, 13 gcc-side skip-compares** — identical pre/post fixes |
| GCC torture x86-64 (`--filter '^2000'`, `-O2`) | **88/88 pass** |
| GCC torture i686 (wrapper, default binary, `--filter '^pr1'`) | **101 pass + 10 reference-side skips** |
| `fuzz_diff.py` synthetic (vs gcc+clang, `-O0..-O3`) | **PASS** (5–6 cases) |
| `fuzz_diff.py` csmith (real csmith 2.3.0 + runtime headers) | **3/3 PASS** |
| `fuzz_diff.py` stress_suite | **4/4 PASS** (fp/gep/slot generators + bounded unroll sweep) |
| `fuzz_diff.py --check-engines` | **4/4 engines implemented** (CI guard) |
| `verify_fib_perf.sh` | **PASS, 141× speedup** |
| `ci-bench.py --list` (restored wrapper) | **39 kernels visible** |
| Godbolt codegen oracle (4 pinned channels) | glibc_strstr / sha256 / zstd numbers re-verified (§3.5) |
| Full 39-kernel benchmark corpus, 3 compilers × 9 reps + warmup, `--strict` | **39/39 correct** (byte-for-byte oracle vs GCC 14.2 *and* Clang 19.1); LCCC/GCC geomean **0.8025**, LCCC/best-ref geomean **0.8342**; worst: chacha20 11.27× vs clang; best: fib 119.9× vs gcc. Evidence: `engineering/evidence/benchmarks/2026-09-06-audit-6af3436/` |

Environment: Debian 13, GCC 14.2.0, Clang 19.1.7, Rust 1.98.1 stable,
csmith 2.3.0, 2-core VM, 8 GiB swap (no PMU — screening evidence only).

## 5. Prioritized follow-up backlog (for future sessions)

1. **SHA-256 / ARX vectorization + spill elimination (top codegen gap).**
   LCCC emits **zero** vector instructions on `sha256_transform.c` where
   Clang/GCC emit 37–44, and 194 spills vs Clang’s 35 (whole-TU `-O3`). The
   message-schedule expansion and the 8-word state are classic SLP/SIMD
   candidates; the chacha20 case (4 parallel quarter rounds) is the same
   family. Suggested path: SLP-style packing of independent 32-bit lanes in
   `src/passes/vectorize.rs` + `vpslld/vpsrld/vpor` (AVX2) rotate emission;
   spill placement review for 16-live-u32 windows.
2. **`glibc_strstr` at `-O2` — root-caused this session.** GCC’s 58-insn
   `-O2` form (verified in local GCC 14.2 assembly:
   `two_way_short_needle.constprop.0`) fills the 256-byte shift table with an
   SSE broadcast (`movd/punpcklbw/punpcklwd/pshufd` + 16 × `movaps`) — 20
   static instructions. LCCC’s 75-insn form keeps a **256-iteration scalar
   byte-store loop** (`movb %r8b, 24(%rsp,%r9)` + `addq` + `cmp` + `jb` ≈
   ~1024 dynamic instructions), plus a loop-invariant `leaq 24(%rsp), %rcx`
   inside the fill-by-value loop (missed LICM of the alloca base), plus dead
   `movl %eax, %r11d` moves right after `movzbl` loads. Fix sketch:
   (a) recognize constant-fill loops (`for(i=0;i<N;i++) a[i]=c;`) in the
   vectorizer/map path and emit SSE2 broadcast + unaligned `movups` stores
   (or lower to memset when the runtime has it); (b) LICM the stack-base
   materialization; (c) peephole the zero-extend-then-move pair. Reproducer:
   `./target/fastbuild/lccc -O2 -S tests/benchmark/programs/glibc_strstr.c`
   vs `gcc -O2 -S` — `.LBB1/.LBB2` (fill) and `.LBB5` (invariant LEA).
3. **LZ4 `read32` load forwarding** (3.1× runtime gap): teach load forwarding
   to hoist the 4-byte hash input into a register and emit direct
   `movl (%base,%index,scale)` SIB forms (FOLLOWUP doc Gap 2).
4. **gzip_crc32 SIB folding of global-symbol bases** (`xorl table(,%reg,4), %eax`,
   FOLLOWUP doc Gap 4): GCC 15 static insns vs LCCC 36.
5. **Loop rotation default-enable** (PF-17): audit the 15 edge cases, add
   dominance verification, then flip `CCC_LOOP_ROTATE` default.
6. **lld as a third linker differential oracle**: this session provisions
   lld 23.1 (X86-only build) in `setup_oracles.sh`; wiring it into
   `run_linker_tests.py`’s oracle set (mold + wild today) is the follow-up.
7. **Corpus growth from `archpkgbuilds`** (candidates not yet covered):
   xz/LZMA match-finder loop, libjpeg-turbo IDCT/upsampling integer kernel,
   nghttp2/cur Huffman decode, PostgreSQL btree page split, Redis sds
   string ops, FFmpeg fixed-point DSP. Each needs the provenance discipline
   of the existing six.
8. **Benchmark-table evidence binding**: make the README benchmark table cite
   the evidence directory inline (or generate the table from the runner’s
   Markdown output) so numbers can never again outlive their raw samples.
9. **`NumBuffer` label test**: optionally restore a one-line spelling test for
   `fresh_label` (cheap insurance against future cleverness in label
   formatting).
10. **`fuzz_diff.py` yarpgen smoke in CI**: build yarpgen in CI (or cache a
    binary) so the yarpgen engine is exercised, not just the wiring guard.

## 6. Session artifacts

- `ms178-1.patch` — canonical deliverable (squashed, base `6af3436`)
- `/home/user/artifacts/` — snapshots S01–S04 (patch + series + tarball +
  bundle + ledger), regression JSONs (pre/post fix), full-corpus benchmark
  evidence, oracle manifests
- `engineering/evidence/benchmarks/2026-09-06-audit-6af3436/` — fresh
  39-kernel screening evidence (this VM; no PMU)

## 7. Operational lessons (harness hygiene)

1. **Never let a timed-out test tree survive its parent.** `timeout N python3
   unroll_stress.py` kills only the direct child; the generated benchmark
   binaries kept running (some for 20+ minutes at ~50 % CPU each) and
   contaminated a concurrent benchmark-corpus run before being discovered via
   `ps --sort=-%cpu`. Rules adopted: (a) `fuzz_diff.run_command` now runs
   children in a fresh session and SIGKILLs the whole group on timeout
   (verified with a parent+child probe); (b) when killing background
   validation manually, `pkill -9 -f` the *tool* pattern, not just the
   pipeline PID; (c) check `ps --sort=-%cpu` for stragglers before starting
   any timing-sensitive run.
2. **A wrapper that exits 0 is not a test.** The stub-engine incident (§3.3)
   is the canonical example: `TOTAL: 0 / exit 0` from a differential tester
   must be treated as a wiring failure. The `--check-engines` CI guard
   generalizes this: advertised capability ⇒ verified dispatch.
3. **Accidental reverts look like new work in a big patch.** Both
   `ci-bench.py`/`bench.yml` and `verify_fib_perf.sh` were regenerated from a
   stale pre-`19447a0` session snapshot, silently undoing merged fixes. Any
   future “perfected” patch must be diffed against what it *claims* to build
   on (`git diff <claimed-base>..HEAD -- <paths>`), not just applied cleanly
   to some base.
4. **Measure with an idle host.** Even the corrected run shares the VM with
   nothing, but earlier screening numbers (perf_ab during the corpus run)
   were taken under contention and are wiring checks only, not measurements.
