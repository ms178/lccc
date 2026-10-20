# FOLLOWUP 2026-10-19: REGEHR 2124 transplant, YARPGen resurrection, untrusted-patch salvage

Design/evidence narrative: `engineering/REGEHR_2124_TRANSPLANT.md`.
All numbers below were produced on the 2-CPU / 1.9 GiB harness VM (GCC
14.2.0 reference, no hardware PMU): timings are **screening only**.

The work reached `main` as one squashed change; the numbered items below
are the *stages* it went through, not commits that exist in this
repository, and they are named by what they did so they stay findable in
the history of the files they touched. Section "State of the merged
tree" at the end records what this repository actually measures.

## Accomplished (stages, oldest first)

1. **mutpair + slice_superopt (perfected salvage).**
   Missed-alarm pair engine (`scripts/mutpair.py`, `fuzz_diff --engine
   mutpair`) and Minotaur-lite superoptimiser. Red-team fixes vs the
   draft: shift-swap mutation emitted unmasked shift counts (real C UB
   in ≈1.1 % of pairs — excluded, `validate_ub_free` audit + negative
   controls added); `judge_pair` re-materialises sources; superopt
   variable-shift specs flagged masked-shift. Evidence: 17 unit tests,
   self-tests incl. collapsing-optimiser control, 120/120 vs GCC
   `-O0..-O3`, 2295-pair UB prober clean.
2. **six bench kernels** (crc32 slice-8, base64, jhash,
   SQLite varint, UTF-8 validate, Horspool) + `bench_verify` oracle;
   checksums equal GCC. VM screening geomean 0.795× vs GCC (base64
   0.51× and sqlite-varint 0.55× worst — see perf TODOs).
3. **YARPGen resurrection + `_Bool` miscompile fix.**
   `ensure_yarpgen.sh` (pinned intel/yarpgen `1adb290…`, probe-gated);
   `fuzz_diff --engine yarpgen` repaired (multi-file cases incl.
   `init.h`, `--seed=N`, `--max-array-dims=3`, full repros);
   `yarpgen_mutpair.py` + unit tests + CI gates. First sweeps found
   seed 20261023 wrong at `-O0`: constant `(_Bool)` conversion folded
   by truncation-then-normalise at four sites — fixed by value
   (C11 6.3.1.2), regression
   `regress_const_bool_cast_zero_lowbyte.c`.
4. **redundant sign-extension peephole, hardened (F-1).**
   Salvaged pass made sound: extension facts cleared at *every*
   rewrite site (the `movq`→`movl` site was a **pre-existing latent
   miscompile** on main), low-subregister-exact consumers,
   parenthesis-aware destination classification with conservative
   clearing. 36 unit tests in the pass itself at the time (43 now);
   mutation negative
   control binds; base64 `movsbq` 36→24, instructions 419→402,
   1.12× A/B vs `CCC_NO_SEXT=1` (VM screening).
5. **alu_torture oracle UB removal** (`clz(0)` guards,
   `__builtin_memcpy` punning). The claimed divergence does not
   reproduce under GCC 14.2; kept as hygiene, documented as such.
   MATCH at `-O0..-O3/-Os`.
6. **Statement-expression conditional composite type (seed 20261020).**
   `get_expr_ctype_with_scope` typed `a ? b : c` as `typeof(b)`,
   truncating mixed-type kernel `min`/`max` statement expressions.
   Diagnostic path worth reusing: per-global dump → counter
   instrumentation → loop-header extraction → dual-oracle delta-debug
   (compare compilers, never "0 trips" alone) → `sizeof` type probing
   → sema exonerated by instrumentation → lowering scope inference.
   Regression `regress_stmt_expr_conditional_composite_type.c`;
   seeds 20261020/20261023 green at all `-O`; whole lib suite green.
7. **Seed-101 `-(unsigned)` fold.** Single-global dump localisation
   again (var_419): constant folding computed unary minus on a 32-bit
   unsigned in its 64-bit carrier (mod 2^64, not mod 2^32) at both
   const-eval sites — fixed by masking; regression
   `regress_const_neg_unsigned_wrap.c`.
8. **cmov soundness (audit find).** Pre-existing: the pass treated a
   32-bit cmov as an unconditional zero-extending write, establishing
   upper32_zero for a possibly-unwritten register. Now preserve-only;
   unit-tested both halves. See
   `engineering/AUDIT-2026-10-19-regehr2124-redteam.md`.
9. **Function-length refactor of redundant_ext** (census ratchet):
   sign-extension consumer + movq/cmov producers extracted; full red-
   team verdicts (own delta + other merged work) in the AUDIT doc.
10. **Toolchains provisioned**: gcc-16.2 (sid, CI oracle) + multilib,
    gcc-multilib, clang 23.1.4, ICX 2026.2.0, valgrind 3.24, gdb 16.3,
    pahole 1.30. godbolt.py audit: all pins current. Host quirk:
    i386 PIE binaries segv at 0x1 on this kernel (-no-pie fine).


## Validation state at hand-off

* `cargo test --lib` (fastbuild): whole suite green, 10 ignored.
* YARPGen classic engine, seeds 20261019..28: 7 PASS, 2 SKIP,
  1 timeout (seed 20261019 `-O1` — compiles correctly standalone,
  83 s on this VM; see compile-time TODO).
* mutpair differential 60/60 post-F-1; 120/120 at salvage time.
* `ci_local.sh --fast`: see "Open" — must be re-run on the final tree
  before this is called done.

## Open / next (priority order)

1. **Compile time of YARPGen mega-TUs at -O1+ (measured).** Seed 101
   (63,020 asm lines, one function): -O0 15 s, -O1 580 s,
   CCC_NO_PEEPHOLE=1 -O1 94 s => peephole = 486 s (84 %);
   CCC_NO_PEEPHOLE_RERUN=1: 571 s (not the reruns). gdb samples:
   liveness straight_effect + implicit-operand oracles + LineStore
   string churn. NEGATIVE RESULT: memoizing classify_line +
   implicit_*_refs by line text made it WORSE (718 s) — the cost is
   solve volume, not classification; and straight_effect cannot be
   text-memoized (pinning passes mutate LineInfo.kind for unchanged
   text). Do not retry memoization. Real fixes: batch passes per
   liveness solve; windowed solves with conservative live-at-boundary;
   structured-IR peephole long-term. Seed 20261019 -O1 83 s, same cause.
2. **Re-run `yarpgen_mutpair` pairs at scale** (first pre-fix run:
   3/5 failures, all explained by the two bugs above; post-fix
   confirmation run recorded at the end of this file). Grow the seed
   range in CI once (1) improves.
3. **Attribute/qualifier differential axis** (the blog's named blind
   spot): systematic `__attribute__`/qualifier variation over mutpair
   cases (e.g. `const`, `restrict`, `may_alias`, packed/aligned,
   `noinline`/`always_inline`, section/visibility where observable),
   judged against GCC at several `-O` levels. Both bugs found this
   session lived in under-stressed construct corners; this is the
   deliberate version of that luck.
4. **Perf, base64 (Cachegrind-measured 1.83× vs GCC).** Ir
   (identical output, -O3 -march=x86-64-v3): GCC 9.83M, Clang23
   10.24M, ICX 10.24M, lccc 18.03M; godbolt static GCC16.2 37 insns
   vs lccc 68. Causes ranked: (1) no RIP-relative+index addressing
   (leaq tab(%rip)+addq x4/iter vs movsbl tab(%rdx)); (2) byte
   extraction movq/shrl/movzbl/movb vs %dh-part stores; (3)
   per-element leaq k(%i) vs pointer-bump; (4) movsbq vs movsbl for
   u32-consumed s8 loads. (1) is an isel addressing-mode project —
   biggest single win across all table-driven code (crc32 same
   shape). Do that first; re-census with godbolt compare +
   cg_annotate.
5. **Perf, SQLite varint (0.55×) and crc32 (0.85×).** CRC32 first
   hypothesis: table-index extension/zeroing pattern. Remember the
   crc32 lesson from this session: check ASM identity before
   believing a VM delta (a −7 % "regression" was identical assembly).
6. **Slice-mining pass**: run `slice_superopt` over the bench kernels'
   hot expressions; any candidate must pass the 16/32/64 re-gate and a
   `bench_kernels` A/B before a peephole/IR rule is written.
7. **Snapshot-script parity check**: verify the claim that
   `scripts/lccc-snapshot.sh` (used for S01..S06 here, with
   `LCCC_SNAPSHOT_UNGATED=1`) is a strict superset of the user's
   standing snapshot semantics (bundle + series + tarball + ledger
   under `/home/user/artifacts`); it produced APPLIES-CLEAN patches
   each time, but the superset claim itself was never diffed.
8. **Host toolchains**: this session validated against GCC 14.2 only.
   Re-verify the two miscompile regressions and alu_torture on the
   godbolt pins (GCC 16.2 `cg162`, Clang 23.1 `cclang2310`, ICC
   `cicc2021100`, ICX `cicxlatest`) — in particular the alu_torture
   `clz(0)` claim, which may well reproduce on GCC 16.

## Traps recorded for the next agent

* `cargo test` for unit tests needs `--lib -- <filter>`; without
  `--lib` only bin targets run (0 tests, looks green).
* Delta-debug predicates must compare two compilers on the *same*
  variant; "still 0 trips" happily minimises to a legitimately empty
  loop (happened here, cost one round-trip).
* Per-global dump localisation beats `c_reduce` line-ddmin on dense
  YARPGen files (72 KB → 50 KB in 400 predicates here). Counter
  instrumentation of loop bodies finds trip-count bugs in one run.
* When a constant fold looks value-correct, test inputs whose low
  byte/word is zero (`256`, `65536`, `2^24`) — truncation-first folds
  pass every small-value test.
* VM bench deltas < ~10 % need an ASM-identity check before they are
  believed in either direction.
- `lccc-snapshot.sh` does `git add -A`: untracked fuzz repros get
  committed (observed: 1.97 MB patch). Always clean
  `artifacts/repros/*` and check `git status` BEFORE snapshotting.
- Files may be externally restored/torn mid-session (observed twice on
  redundant_ext.rs): read-before-edit, count-asserted anchors, TEST
  COUNT deltas as loss detectors, commit per validated step.
- The audit census counts comment/blank lines toward function length,
  and main itself was ratchet-red from PR #871 (panic +3, fn300 +1) —
  see AUDIT doc section 4b before re-baselining anything.

## State of the merged tree (measured, not recalled)

Every number below was produced by running the command shown, on the
tree that carries this document, with GCC 16.2.0 as the reference:

| Claim | Command | Result |
|---|---|---|
| Unit suite | `cargo test --profile fastbuild --lib` | 5393 passed, 0 failed, 10 ignored |
| Real YARPGen differential | `fuzz_diff.py --engine yarpgen --count 3 --opts=-O0,-O2` | 3/3 PASS |
| Immutable pair judgements | `yarpgen_mutpair.py --ci-smoke` | 3 judged, 0 failures |
| `_Bool` / negation / conditional regressions | `run_regression.py` + `gcc-16` oracle | byte-identical to GCC |
| Whole corpus vs GCC 16.2 | `run_regression.py --gcc gcc-16 -j 2` | 1033 passed, 9 failed (all `-m32` link: no multilib on this host), 4 skipped-compare, 28 skipped-run |
| Provisioner contract | `check_ensure_yarpgen.sh` | 20 assertions, all passing |
| Gate parity | `check_ci_gate_parity.py` | PASS (315 commands) |
| Formatting / lints | `cargo fmt --check`, `cargo clippy --all-targets` | clean |

The two defects the follow-up list named as structural are closed in the
same tree. The extension peephole now asks the backend's one architectural
model of unnamed register writes (`types::classify_implicit_operands`)
instead of keeping a partial private table: at the PR tip the pass had no
implicit-write invalidation at all beyond `call`. The two constant folders
share one definition of the `_Bool`-by-value and modular-negation rules
(`common::const_eval::negate_const_promoted`, `bitnot_const_promoted`,
`c_bool_const`) instead of asking the same question two different ways.

Two pre-existing gate defects were also fixed: `check_ci_gate_parity.py`
reported a gap that had been red on `main` (the TLS runtime oracle ran
hosted but not locally), and `check_implicit_operand_matrix.py` had never
run anywhere -- it extracted a function without the helper it calls, so the
probe did not compile. Both are wired into the hosted and local gates now.
