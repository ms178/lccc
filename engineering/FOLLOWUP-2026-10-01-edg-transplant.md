# FOLLOWUP-2026-10-01 — EDG Open-Source Deep Dive & Transplant Program

**Session base:** upstream `ms178/lccc` `bc251d99` (main, 2026-10-01, PR #717 —
the session re-based onto latest main after a workspace wipe).
**Primary deliverable:** [`docs/EDG_TRANSPLANT_ANALYSIS.md`](../docs/EDG_TRANSPLANT_ANALYSIS.md)
— full EDG-vs-LCCC subsystem analysis with the prioritized transplant
register (E1–E14). This file records *what was accomplished this session*,
*the mid-session harness wipe and recovery*, and *what future agents must do
next*.

---

## ⚠ Mid-session harness wipe — what happened and how it was beaten

Between turns the harness truncated the 3.7 GB / 111 k-file EDG clone to
104 MB / 5 423 files, **deleted lccc's `.git` entirely**, and lost the mined
`tests/corpus/` — the documented 10 k-file / 128 MB snapshot cap plus the
"`.git` can vanish" failure mode from `scripts/arena_session_restore.sh`.
Consequences and doctrine (do not re-learn these the hard way):

1. **The EDG clone cannot survive a turn boundary.** Re-clone
   (`git clone --depth 1 https://github.com/edgcpp/compiler.git`) is ~30 s;
   what must survive is the *extracted value*. Everything mined from EDG
   (`tests/corpus/`, `docs/edg_changes_*`) lives inside the lccc tree and in
   `ms178-1.patch` + `artifacts/` (the small, user-facing files that persist).
2. **Never end a turn without a fresh snapshot.** A previously interrupted
   CI command lost an entire turn's mining work. After every validated chunk
   this session the snapshot script runs (DIRECT STANDING ORDER).
3. **Deliverable staging:** intermediate copies live in
   `/home/user/staging-backup/` (4 files, tiny — always inside the snapshot
   budget) so even a `.git`-eater cannot take the work products.
4. Observed counts before/after re-clone matched exactly (19 717 `.sft.c`,
   1 573 339 `src` LOC, 7 415 `Changes` entries), so the earlier measurements
   were real, not truncation artifacts.

## Accomplished this session

1. **Environment bootstrapped twice from near-empty state**: swap
   (`/swapfile` 4 G active), rustup 1.99.0 via `arena_session_restore.sh`,
   green `target/fastbuild/lccc` (14.2.0) on first bootstrap.
2. **Complete EDG code-base survey** (1.57 M LOC core, 47 K lines docs,
   45 109 single-file tests, 985 MB dev tooling). Findings, evidence paths,
   pros/cons and the T0–T3 transplant register are in the analysis document.
   Key discovery for VM measurement doctrine: **EDG pins Valgrind Cachegrind
   geometry** (`I1=32768,8,64 D1=32768,8,64 LL=33554432,16,64`) so simulated
   cache/insn counts are host-independent — the correct no-PMU methodology
   for this harness.
3. **Licensing adjudication:** EDG core + the imported **Clang** corpus are
   Apache-2.0 WITH LLVM-exception (safe to adapt with attribution — spot-
   checked: no GPL outside `tests/tests/imported/gnu/**` and one vendored
   pylib LICENSE); the imported **GNU** corpus is **GPL-3** and must never
   enter the lccc tree — mined by *name/directive facts* only. §5 of the
   analysis is the standing law.
4. **E1 landed — `scripts/edg_corpus_mine.py`** (red-team-hardened:
   `--selftest` 8/8 including mid-file-directive-leak and `//options_sep:`
   cases; SPDX attribution on every copied file; sha256 per file; giant
   Arm-SME/SVE matrices capped at 256 KB with reasons recorded):
   - `tests/corpus/clang-c/` — 1 434 adapted Apache-licensed Clang C tests
     (`C/`, `Parser/`, `Preprocessor/`, `Sema/`) + `.meta.json` sidecars
     (expect accept/reject, options, `expected-*` diagnostics) +
     `corpus-index.json` + `LICENSE-NOTICE.txt`;
   - `tests/corpus/gnu-torture-manifest.jsonl` — 18 188 GPL-safe test
     records (names, categories, directives; zero source text) incl. the
     optimizer gold (`tree-ssa/` 2 458, `vect/` 1 816) and 2 876
     runtime/link execution tests.
5. **E7 accelerated — `scripts/edg_changes_mine.py`**: lossless parser for
   EDG's `src/Changes` (7 415 entries incl. multi-line bug-ID headers; an
   early revision silently dropped 72 wrapped headers — caught and fixed in
   red-team); keyword-scored C-relevance filter; outputs
   `docs/edg_changes_c_extract.md` (3.9 MB, 1 422 C-relevant entries with
   full bodies), `docs/edg_changes_c_index.md` (all 7 415), stats JSON.
   E7 is now "curate extract → regression tests", not "mine manually".
6. **E2 landed — `scripts/callgrind_ab.py`**: Callgrind now runs with pinned
   `--I1/--D1/--LL` geometry (env-overridable `LCCC_CG_{I1,D1,LL}`),
   smoke-tested against Valgrind (events `Ir Dr Dw I1mr D1mr D1mw ILmr
   DLmr DLmw` confirmed). No-PMU measurements are now host-reproducible.
7. **Oracle pinning verified** (no changes needed): `scripts/godbolt.py`
   already pins GCC 16.2 (`cg162`), Clang 23.1 (`cclang2310`), ICC
   2021.10 (`cicc2021100`), ICX via `cicxlatest` with resolved-id
   manifests — matches the standing version policy.
8. `backlog.md` carries the EDG-TRANSPLANT register; `tests/corpus/README.md`
   documents regeneration + licensing law.

## Red-team audit results (this session's self-review)

| Finding | Severity | Fix |
|---|---|---|
| `summary()` counted directories via `rglob("*")` | wrong stats | counts files only |
| `//options_sep:` honoured only if listed before `//options:` | misparse | two-pass header handling |
| mid-file `//type:` comment could reclassify a test | misparse | directive scan stops at first code line |
| `//type: s` (skip) mapped to `lccc_expect: accept` | wrong triage | maps to `skip` |
| `src/Changes` parser dropped 72 multi-line bug-ID headers | silent loss | wrap-aware header parser; verified 0 unique losses |
| Analysis doc test counts mixed tests with recorded outputs | misleading | exact counts re-verified and restated |
| Followup claimed `ci_local.sh --fast` was run | false | corrected: **CI not run this session — user exemption** |
| Giant Arm SME/SVE tests bloated corpus to 331 MB | uncommittable | 256 KB cap + name-pattern skip, reasons recorded |

## To-do for the next session (priority order)

The register in `docs/EDG_TRANSPLANT_ANALYSIS.md` §6 is authoritative.

- [ ] **CI gate:** run `./scripts/ci_local.sh --fast` (exempted this session
      by explicit user order — not done, not claimed). Full `--slow` before
      any snapshot claims `mode=full`.
- [ ] **E1 remainder:** wire `tests/corpus/clang-c/` into a driver
      (expect-accept/reject per `.meta.json`, GCC-differential stdout/exit for
      `has_main` tests); triage real frontend failures into `tests/bugs/` +
      `backlog.md`. The `Preprocessor/` (257) and `Sema/` (1 027) slices
      are where lccc is weakest today.
- [ ] **E7 remainder:** curate `docs/edg_changes_c_extract.md` entries into
      `tests/regression/` reproducers — start with the highest-score entries
      (C23 tag compatibility N3037, empty initializers, bit-field quirks,
      unsequenced-modification semantics).
- [ ] **E5 (T1):** work-stack const-eval + step budget (EDG `interpret.c`
      architecture); `artifacts/repros/crash_synth_*.c` deep-expression
      class must go green and stay green.
- [ ] **E3 (T1):** builtin signature scrape+generate (EDG
      `dev_tools/builtins` design). Target: zero unresolved `__builtin_*`
      on kernel/glibc/zlib-ng/expat with arity checking.
- [ ] **E6 (T1):** bit-field ABI differential corpus (EDG `layout.c`
      comments are the variant checklist).
- [ ] **E4/E8/E9/E13 (T1)**, **E10/E11/E12 (T2)** per the register.

## Defects / debt observed (tracked)

- **D1 (harness):** mid-session wipe cost a full turn of mining output;
      mitigated by staging + early snapshots + re-derivable extracts (see
      top of this file). Keep `/home/user/staging-backup/` current.
- **D2 (pre-existing):** `common/const_eval.rs` recursive evaluation has no
      depth/step budget → E5.
- **D3:** `callgrind_ab.py` geometry now pinned (closed); remaining A/B
      scripts (`perf_ab.py`, `hot_loop_metric.py`) are wall-time based and
      were not audited for frequency discipline — acceptable for same-window
      paired A/B per house doctrine, note in next measurement doc.
- **D4 (upstream-adjacent):** EDG's builtins scraper has hard-coded host
      paths — when porting (E3), make paths configurable; do not copy blindly.
- **D5 (observed, not ours):** fresh `ms178/lccc` main moved to `bc251d99`
      during the session (PR #717). Snapshot base refs are per-branch; always
      rebase the deliverable onto latest main per standing order.

## Session metrics

- EDG tree analyzed: `src/` (1 573 339 LOC), `doc/source` (47 494 lines at
  module granularity), `dev_tools/` (bench/test/bisect/pack tools),
  `tests/` (45 109 single-file tests + licensing trees).
- Knowledge artifacts produced: 1 434 mined tests (+18 188-name manifest),
  1 422-entry Changes extract (3.9 MB), full 7 415-entry index, two mining
  scripts with selftests, this analysis + followup + register.
- Validation performed: miner selftest 8/8; Changes parser loss-proof
  (0 unique entry loss vs 7 416 header lines); SPDX/GPL scan clean;
  Callgrind pinned-geometry smoke test green; corpus integrity asserts
  green. **CI not run (user exemption).**
