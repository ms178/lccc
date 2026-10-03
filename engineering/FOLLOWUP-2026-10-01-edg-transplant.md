# FOLLOWUP-2026-10-01 — EDG Open-Source Deep Dive & Transplant Program

**Session base:** upstream `ms178/lccc` `bc251d99` (main, 2026-10-01, PR #717 —
the session re-based onto latest main after a workspace wipe).
**Primary deliverable:** [`docs/EDG_TRANSPLANT_ANALYSIS.md`](../docs/EDG_TRANSPLANT_ANALYSIS.md)
— full EDG-vs-LCCC subsystem analysis with the prioritized transplant
register (E1–E14). This file records *what was accomplished this session*,
*the mid-session harness wipe and recovery*, and *what future agents must do
next*.

---

## Session 2026-10-02 — PR #721 second review audit + CI-red round

**Accomplished (details: [`docs/REVIEW_721_ADJUDICATION.md`](../docs/REVIEW_721_ADJUDICATION.md)):**

1. **CI-red root cause found and fixed**: first failing subcommand of
   "Verify remaining fast local contracts" = `check_env_test_hygiene.sh`
   (deferred-marker scan hit 87 verbatim Clang fixtures with upstream
   `FIXME`s).  Narrow, documented exclusion of exactly `tests/corpus/clang-c/`;
   all 72 block commands reproduced locally and green.
2. **All 6 P1 + 5 P2 + 1 P3 audit themes verified and fixed**: typed
   verdict architecture (CRASH never passes), per-invocation schema
   (mixed C/C++ runs kept, `%-D` pass-through, target values consumed,
   8 empty defaults preserved, `options_all` applied, honest `phase`), Clang
   `{{regex}}` template oracle + counts + active prefixes + absolute `@N`
   lines, outcome-level GCC differential, realpath-canonical corpus
   exclusion, regeneration stage/atomic-swap, callgrind empty-run failure +
   D1 write misses + rc checking, Changes-scoring case fix (extract now
   1 567 entries — audit's predicted number), subtree license with Legacy
   NCSA + GPL-3 mapping + mixed-origin classification.
3. **Contract tests written first, now green, and CI-gated**: 44 mocked
   runner/miner contracts + 5 path-enumeration tests + both miner
   selftests, wired into ci.yml AND ci_local.sh (parity checker green,
   129 commands mirrored).

**To-do next:** run `run_clang_c_corpus.py --cc target/fastbuild/lccc`
against the real compiler and triage failures into `tests/bugs/` +
`backlog.md`; curate `docs/edg_changes_c_extract.md` (1 567 entries) into
regression tests (E7 remainder); keep S04+ snapshots flowing.

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
4. **E1 landed — `scripts/edg_corpus_mine.py`** (red-team-hardened +
   PR #719 review-fixed: `--selftest` 18/18 covering mid-file-directive-leak,
   `//options_sep:`, multi-invocation `//options:`, full `-verify` syntax
   (`@±N` offsets, `-re`, counts, nested braces, custom prefixes);
   SPDX attribution on every copied file; sha256 per file in
   `corpus-index.json`; giant Arm-SME/SVE matrices capped at 256 KB with
   reasons recorded):
   - `tests/corpus/clang-c/` — 1 434 adapted Apache-licensed Clang C tests
     (`C/` incl. `C89`/`C99`/`C11`/`DR` subdirs, `Parser/`, `Preprocessor/`,
     `Sema/`) with metadata **only** in `corpus-index.json` (expect
     accept/reject, per-invocation `option_sets` + `gcc_flags_per_set`,
     `expected-*` diagnostics with source lines) + `LICENSE-NOTICE.txt`;
   - `tests/corpus/gnu-torture-manifest.jsonl` — 18 188 GPL-safe test
     records (names, categories, directives; zero source text) incl. the
     optimizer gold (`tree-ssa/` 2 458, `vect/` 1 816) and 2 876
     runtime/link execution tests;
   - `tests/corpus/run_clang_c_corpus.py` — corpus runner (accept/reject/
     xfail, timeouts, per-category, `--limit`, GCC differential as an
     oracle, arch/lang skips from metadata).
5. **E7 accelerated — `scripts/edg_changes_mine.py`**: dual-era lossless
   parser for EDG's `src/Changes` (**13 562** entries: 7 416 bracketed bug-ID
   headers + ~6 146 padded pre-2008 headers, wrap-aware; wrap-reference
   lines rejected by connector-word rules; 4 upstream date typos
   adjudicated).  Keyword-scored C-relevance filter (word-boundary anchored,
   merged overlap groups — no double counting); outputs
   `docs/edg_changes_c_extract.md` (914 KB, **1 559** C-relevant entries with
   full bodies), `docs/edg_changes_c_index.md` (all 13 562), stats JSON.
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
| `src/Changes` parser missed ALL pre-2008 plain headers (75% of extract was one mega-blob) | catastrophic parse loss | dual-era header grammar + entry-count sanity gate + selftest (PR #719 review F1) |
| C-relevance scoring double-counted `init`/`initialization`, unanchored `in_c` matched `C++`/`include` | scoring noise | word-boundary patterns, merged overlap groups (F2) |
| `#include`-line `//type:` leak + `expected-*` dropped when scanning stopped at preprocessor lines | misparse | header-block state machine; expectations extracted from every line (F3) |
| `-verify` extraction stopped at the first `}}` and dropped `@±N`/`-re`/counts/prefixes | 25.4% empty diagnostics | full `-verify` grammar, balanced-brace messages, source-line recording (F4) |
| sidecars duplicated the index; `differential_corpus.sh` swept `tests/corpus/` | repo bloat, double counting | index-only metadata (`--emit-sidecars` opt-in); corpus pruned from the diff harness (F5) |
| `//options:` `:`-separated runs conflated into one set; args unstripped | wrong invocations | `option_sets` per run + `gcc_flags_per_set` translation (F6) |
| `stats.json` emitted year "127"; extract had 1 521 trailing-space lines | false data, apply warnings | documented date-typo table, all output lines rstripped (F7) |
| `callgrind_ab.py` printed no geometry, table lacked D1, `agg["ir"]` could KeyError | unverifiable A/B | geometry+versions in `manifest.json`, D1 columns, `agg.get` (F8) |
| third-party license texts missing for the mined EDG corpus | license gap | `third_party_licenses/EDG-Clang-Apache-2.0-LLVM.txt` + LICENSING.md section (F8b) |

## To-do for the next session (priority order)

The register in `docs/EDG_TRANSPLANT_ANALYSIS.md` §6 is authoritative.

- [ ] **CI gate:** run `./scripts/ci_local.sh --fast` (exempted this session
      by explicit user order — not done, not claimed). Full `--slow` before
      any snapshot claims `mode=full`.
- [x] **E1 remainder (done this session):** `tests/corpus/run_clang_c_corpus.py`
      drives the corpus (expect-accept/reject/xfail per index, timeouts,
      per-category reporting, GCC-differential oracle mode).  Remaining:
      run it against a built LCCC, triage real frontend failures into
      `tests/bugs/` + `backlog.md`.  The `Preprocessor/` (257) and `Sema/`
      (1 027) slices are where lccc is weakest today.
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

---

# Session 2026-10-03 — E1 corpus-driven hardening, #embed, Review-AI audit response

Snapshots: S02 (E5 budget), S03 (embed + pragma deps), S04 (audit hardening).

## 1. E1 corpus results (clang-c, outcome-only, c-driver-contract +
clang-default-c-dialect capabilities)

| Slice        | executed | PASS | FAIL | notes |
|--------------|----------|------|------|-------|
| Preprocessor | 26       | 24   | 2    | was 16/10 pre-session |
| Sema         | 193      | 124  | 68   | baseline; 64 = "expected reject, observed ACCEPT" (missing semantics diagnostics), 4 over-rejects, 1 runner ERROR |

Residual Preprocessor FAILs (documented in tests/bugs/backlog.md):
`pragma_assume_nonnull` (needs `_Nonnull` keyword + -Wnonnull warning),
`pushable-diagnostics` (needs diagnostic-state machine + unused-comparison
warning). Both multi-subsystem; deferred for corpus breadth.

## 2. Features landed (corpus-pinned, EDG-derived)

- **C23 `#embed` / `__has_embed` / `__STDC_EMBED_*`** — full parameter
  grammar: `limit` accepts C integer pp-numbers (hex/octal/binary/char
  literals incl. escapes), `prefix/suffix/if_empty`, `clang::offset(N)`;
  duplicate-param detection; literal-aware paren balance; strict trailing
  validation in whole-line forms; mid-line (standard) form preserves
  trailing tokens (`{#embed "f"};`). `__has_embed` probes silently yield
  NOT_FOUND for unsupported/vendor parameters (feature-detection channel)
  and diagnose true syntax errors. 8/9 corpus embed tests pass;
  `embed_constexpr` needs an aux data file the corpus does not ship.
- **`__has_warning`** — truthful verdicts against lccc's actual
  WarningKind registry (no blanket 1s); full Clang contract diagnostics
  (missing paren, zero args, non-string argument, non-`-W` option name,
  unterminated literal).
- **`#pragma GCC dependency`** — existence check, mtime staleness warning
  with optional trailing message, delimiter validation, no macro expansion
  of the filename (GCC contract; corpus pins both error shapes).
- **`_Pragma` created by expansion** — rescan pass handles operators that
  only come into existence through macro expansion (`DO_PRAGMA (...)`
  kernel pattern) without resurrecting blue-painted macros.
- **Stringize validation** — literal-aware: `#define F(x) "#"` and
  `'#'` bodies are valid; `#z` with non-parameter z still diagnosed.

## 3. Review-AI audit response (PR #734) — all 9 finding groups resolved

- **High-1 parser termination (CONFIRMED LIVE, fixed):** `({ ... })` chains
  beyond the frame budget hung forever (compound-statement loop retrying a
  non-consuming placeholder). Fix: budget-exhaustion bail in the compound
  loop + universal progress guarantee + silent unwind (expect_after /
  emit_error suppress cascade once the budget diagnostic fired). Gate now
  pins `stmtexpr_11000`.
- **High-2 embed offset underflow (fixed):** `current_line_start` captured
  before the running offset advances; no reconstruction from rewritten
  line lengths.
- **High-3 stringize literals (fixed):** scanner skips string/char
  literals (audit's exact examples probed pre/post).
- **High-4 gate fail-closed (fixed):** requires rc==1 exactly, exactly
  one budget diagnostic, explicit signal/negative-rc rejection, timeout
  rejection; acceptance logic extracted into `evaluate_budget_run` with
  10 mocked unit cases in `--selftest`.
- **High-5 unary fixture (fixed):** whitespace-separated negations
  (`- - - 1`), maximal-munch-safe, odd/even invariants asserted.
- **Med-6 embed grammar (fixed):** see §2.
- **Med-7 __has_warning truthfulness (fixed):** see §2.
- **Med-8 GCC dependency completeness (fixed):** see §2.
- **High-9 diagnostic windowing coordinates (fixed):** byte-column from
  resolve_span converted to char indices before windowing/caret/squiggle
  math; UTF-8 caret alignment verified; no panics possible (no byte
  slicing).
- **DISAGREED (documented):** inline/mid-line `#embed` IS the C23
  6.10.15 standard form, not an extension (comment now cites the clause);
  the gate already rejected signal exits via negative-rc fall-through,
  but is now explicit anyway.

## 4. E7 curation (docs/edg_changes_c_extract.md -> tests/regression)

Nine differential regression tests committed (all pass lccc vs GCC where
GCC is a valid oracle): `edg_c23_empty_initializer`,
`edg_gnu_null_constexpr`, `edg_dr423_const_return_qualifier`,
`edg_anon_member_designated_init`, `edg_c23_compatible_tag_redefinition`
(-std=c2x), `edg_typeof_statement_expr_cast`, `edg_inline_embed_tokens`
(lccc-only), `edg_stringize_in_literals`, `edg_has_warning_registry`
(lccc-only; GCC lacks __has_warning).

## 5. E6 first pin + open bugs

`tests/bugs/bitfield_generic_effective_type.c` — GCC's _Generic selects
the EFFECTIVE bit-field type (`unsigned u:8` -> unsigned char); lccc uses
the declared type. First pinned divergence for the E6 differential corpus.
`tests/bugs/compound_literal_alignas.c` — _Alignas on compound literals
loses its alignment in emission (DR444 syntax accepted, placement not).

## 6. Verification at S04

ci_local.sh --fast **150/150 green** (first fully green run); cargo test
4011/0; deep-nesting gate 7/7 incl. stmtexpr termination; Preprocessor
corpus 24/26; clippy + rustfmt clean; Sema baseline 124/193 recorded for
the next session's triage (report: work/corpus_sema.json equivalent at
/home/user/work/corpus_sema.json).

## 7. Next-session starting points

1. Sema triage: 64 "expected reject, observed ACCEPT" = missing semantic
   diagnostics (group by error class); 4 over-rejects:
   c2x-bool/c2x-nodiscard (C23 keywords `bool`/`true`/`false` +
   `[[nodiscard]]` prefix attrs), overloaded-func-transparent-union,
   undefined-internal-typeof-c23. C23 bool keywords are the highest
   leverage (kernel/glibc -std=c23 readiness).
2. E6 bit-field differential corpus build-out from the pinned divergence.
3. E3 builtin signature scrape (kernel/glibc/zlib-ng/expat zero unresolved).

---

# Session 2026-10-03 (cont.) — rebase + full red-team audit of the patch

Snapshots S06–S08. Base moved: be8b8569 (PR #731) -> **27cffb8 (PR #732)**;
patch replays cleanly (zero shared files with #732).

## Rebase facts

- PR #732 touched only tooling/tests (callgrind_ab, lccc_recover, workload
  extraction safety, peephole whitespace gate, gzip run.py) — zero overlap
  with the session patch; `git apply` clean, no conflicts.
- Post-wipe environment restored: rustup/rustc 1.99.0 reinstalled
  (.cargo/.rustup are snapshot-excluded), swap re-enabled via
  scripts/ensure_swap.sh, and the wiped i686 multilib stack
  (gcc-multilib, g++-multilib, libc6-dev-i386) reinstalled — proven
  environmental by host GCC failing `gcc -m32` identically before the fix.

## Red-team findings (audited line-by-line) and fixes

| ID | Severity | Defect | Fix |
|----|----------|--------|-----|
| R1 | bug | `probe_embed` reported directories as Found (metadata succeeds; the #embed read then fails) — inconsistent `__has_embed` verdicts | require `file_type().is_file()` |
| R2 | bug | only the FIRST inline `#embed` per line was spliced; a second occurrence survived as raw text | splice loop scans the unconsumed remainder left-to-right; expansion text is never re-scanned (no recursion via prefix/suffix containing `#embed`) |
| R3 | perf | per-byte `b.to_string()` = 1 heap allocation per embedded byte | `push_u8_decimal`: branch-on-digit-count fixed divisors, zero allocations |
| R4 | perf | whole-file `fs::read` even with `limit(N)` | `read_embed_bounded`: prefix read capped at offset+limit; short-read/EINTR-tolerant loop; 50 MB + limit(16) measured **4 ms** (vs 41.7 s full expansion of the same file) |
| R5 | debt | resolve/read/diagnose block duplicated across expand forms | extracted `read_embed_resource` |
| R6 | dead | leftover `let _ = bytes;` | removed |
| R7 | bug | **double offset**: bounded read seeked by `offset` while `expand_embed_bytes` sliced by `offset` again → truncated output (caught by the multi-embed differential) | single arithmetic owner: I/O reads the offset+limit prefix, slicing stays in `expand_embed_bytes` |
| R8 | msg | `clang::offset` failures said "invalid embed limit" | `InvalidLimit(param, value)` → "invalid embed parameter 'clang::offset' value ''" |
| R9 | diag | windowing caret/squiggle now fully char-based with byte→char conversion at one point (verified on multibyte windowed snippets) | (from the prior round; re-verified) |

## Verification evidence (no guesswork)

- **Byte oracle**: 256-byte fixture (all byte values) embedded via
  `#embed`; FNV-1a `0x4242dc5249c33625` / size 256 identical for
  lccc-#embed and a GCC-compiled reference array — baked as the constant
  in `tests/regression/edg_embed_byte_oracle.c` (+ .bin fixture, sha256
  40aff2e9…).
- **i686 architecture**: the same oracle compiles and passes with
  `-m32` (frontend is target-independent; verified anyway).
- **Grammar battery** (`edg_embed_grammar_battery.c`): two embeds on one
  line, offset+limit, hex limit, prefix/suffix, if_empty past EOF.
- **Unit tests**: `embed_unit_tests` (4) — radices/suffixes, malformed
  rejection incl. u64 overflow boundary, char-literal escapes, bounded
  read contract. Total suite 4015/0.
- **Fuzz sweeps, zero panics/hangs**: 27 embed edge cases
  (unterminated params, overflow limits, nested-paren prefix, vendor
  params, chevron/quote truncation), 17 `__has_warning` forms
  (concatenated literals -> 1, -Wno- mapping, macro-arg non-expansion,
  unterminated strings), 22 garbage-token recovery contexts (progress
  guarantee without budget exhaustion).
- **Verdict checks**: `__has_warning("-W" "return-type")` = 1,
  `-Wno-return-type` = 1, `-Wextra` = 1; unknown flags = 0.
- **CI**: `ci_local.sh --fast` 150/150 green on the rebased base;
  linker suite 302/302 (after multilib restore); deep-nesting gate 7/7;
  clippy 0 warnings; rustfmt clean; Preprocessor corpus 24/26
  (unchanged); Sema corpus 124/193 identical to pre-red-team baseline
  (no semantic drift).

## Performance notes (14700KF-relevant)

- Hot embed path is branch-light integer formatting into a pre-sized
  String (no allocations per byte, one reserve of take*5 + affixes).
- I/O bounded by offset+limit; the only remaining O(file) cost is an
  UNBOUNDED embed of a huge file, which is inherent (the byte list
  itself is the output). Preprocessing a 50 MB resource stays bounded
  (limit(16) on 50 MB: ~9 ms); the downstream cost is the giant
  initializer it emits (see backlog B5) and on this 2 GB harness a
  full 50 MB embed exceeds the memory cgroup (SIGKILL) — measured
  identical on S08, i.e. not a regression.
- Compound-loop termination guard costs one pos compare per statement —
  below measurement noise on real TUs.

# Session 2026-10-03 (cont. 2) — PR #739 Review-AI audit remediation

The Review AI audited PR #739 (11 findings, "request changes", self-scored
6.5/10) with the explicit caveat that it CANNOT execute code. Every finding
was therefore verified with reproducers against the built compiler before
fixing; this ledger records the verdicts and the measured evidence.

## Finding-by-finding verdicts (all verified empirically)

| # | Audit claim | Verdict | Evidence / action |
|---|-------------|---------|-------------------|
| F1 | `#embed` limit/offset can size an allocation to u64::MAX -> ICE/abort | **AGREE** | `read_embed_bounded` allocated `vec![0u8; offset+limit]` unclamped; `limit(18446744073709551615)` parsed fine. Fixed: clamp to `f.metadata().len()` BEFORE allocating + `usize::try_from` reject. Gate pins 3 adversarial inputs (u64::MAX limit, 1 TiB limit, u64::MAX offset+limit(1)); all compile clean in ~ms. |
| F2 | queued `_Pragma` diagnostics report line 0 | **AGREE** | Drain passed `(0,0)`. Fixed: queue now carries `(content, 1-based line)` resolved at enqueue through the current line resolver (`#line`-aware). Repro: macro-expanded `_Pragma("GCC dependency ...")` reports the macro-use line; `#line 9` override honored. Corpus `_Pragma-dependency.c` d0001 (line 17) and `_Pragma-dependency2.c` d0000 (line 9) now PASS in diagnostics mode. |
| F3 | `__has_warning("foo")` (non-`-W` name) rejects the TU | **AGREE** | Severity split: well-formed call with non-flag argument is now a WARNING via a new `warn!` path; missing-paren/zero-args/non-string/unterminated stay errors per Clang. Corpus `invalid-__has_warning2.c` flipped FAIL->PASS. |
| F4 | stale blanket-policy doc contradicts the registry | **AGREE** | Doc block deleted; `is_recognized_warning_flag` now documented as the registry predicate; contract block re-validated line-by-line against the code. |
| F5 | budget only covers expressions; other recursion classes crash | **AGREE — and worse than claimed** | Measured stack-overflow aborts (rc=134) on VALID C: 200k nested blocks, 200k brace-initializer levels, 50k nested struct definitions (named AND anonymous), plus 20k-level stmt chains. Budget generalized to four frame classes with class-specific diagnostics (see below). |
| F6 | diagnostic cap is error-only | **AGREE** | `MAX_RENDERED_WARNINGS = 200` added (rendering cap; counting continues past it, so `-Werror` and summaries stay exact). 200 vs Clang's 20 justified in-code: corpus pins exact warning output and real TUs must not be truncated at 20; the cap still bounds pathological floods. |
| F7 | compound-loop `continue`s skip the progress check | **AGREE** | Restructured to a labelled `'item` block; every path (incl. the former `continue` exits) now flows through the single tail progress check; outer loop labelled `'body` for the labeled break. |
| F8 | `resolve_has_macros_in_code` gets 0-based lines | **AGREE** | 5 call sites now pass `source_line_num + 1` (matching the `process_directive` convention); contract documented on the function. Repro: code-line `__has_warning("foo")` warning now reports the correct 1-based line. |
| F9 | splice re-scans and clones | **AGREE (minor)** | Single-scan rewrite: first `find_inline_embed` result reused, head cloned only from the first `#` onward, remainder offsets derived from the suffix-slice guarantee of `parse_embed_params`. |
| F10 | `push(c as char)` byte->char assumption | **AGREE, deferred** | Correct that ~20 pragma-de-escape sites assume 1 byte = 1 char; today inputs are pre-validated to the source charset and the sites are hot. Tracked below as a backlog item; no behavioral bug demonstrated. |
| F11 | stray `e.txt`, stale FOLLOWUP base-ref line | **AGREE** | Both removed. |

## F5 deep-dive: generalized parser frame budgets

The audit's preferred option was implemented: one shared live-frame counter
(`parser_frame_depth`, renamed from the expression-only counter) with
class-specific budgets and diagnostics:

| Class | Entry points counted | Budget | Measured ceiling |
|-------|----------------------|--------|------------------|
| expressions | `enter_expr_frame` sites | 32 768 | ~4 800 paren levels overflow |
| statements/blocks | `parse_stmt` + `parse_compound_stmt` | 8 192 | ~14–16k block levels overflow |
| brace initializers | `parse_initializer` | 32 768 | (same order as expressions) |
| record definitions | `parse_struct_or_union` | 2 048 | ~50k levels overflow; post-parse phases are quadratic in record depth, so the lower bound also caps hostile-but-legal compile time (~0.6 s worst case) |

Supporting hardening discovered while testing the budget itself:
- **Termination guarantees**: struct-field loop gained the same
  diagnosed-break + progress-guard pair the block loop has; without it the
  post-budget unwind of deep record chains spun forever (rc=124) — found
  by repro, fixed, and now pinned by the gate.
- **alignof memoization**: `alignof_type_spec` now resolves previously
  defined named records through `struct_tag_alignments` instead of
  re-walking the chain; deep NAMED struct parsing drops from quadratic
  (depth 8000: 11.6 s) to linear.
- Known remaining algorithmic gap (backlog): sema/codegen layout cost is
  quadratic in ANONYMOUS record-nesting depth (parser itself is linear
  there; budget fires before sema runs, so crash-class inputs never hit
  it). Real TUs nest single digits deep.
- Gate (`scripts/check_deep_nesting_robustness.py`) extended: embed
  adversarial class (3 cases, host-width-aware evaluator) + block /
  initializer / record crash classes (4 cases) + stmtexpr re-pinned to the
  block budget (statement expressions parse as compound statements).
  Full gate: 14/14 PASS.

## Corpus numbers, diagnostics mode (the honest bar)

Preprocessor slice, `run_clang_c_corpus.py --mode diagnostics`
(c-driver-contract + clang-default-cialect):

| State | PASS | FAIL |
|-------|------|------|
| Before this session (S08) | 15/26 | 11 |
| After fixes | **18/26** | 8 |

Fixed: `_Pragma-dependency` (F2), `_Pragma-dependency2` (F2),
`invalid-__has_warning2` (F3). ZERO regressions (fail-set is a strict
subset; verified by rebuilding the pre-fix tree and diffing the reports).
Note: the audit's claimed "true figure 22/26" was an overestimate — the
measured pre-fix baseline was 15/26.

Remaining 8 FAILs are pre-existing diagnostic-feature gaps (each verified
against the pre-fix binary): `_Pragma.c` (malformed `_Pragma` error text +
unknown-pragma warnings), `extension-warning.c`, `macro-reserved.c`,
`pushable-diagnostics.c`, `suggest-typoed-directive.c` (x2) — all
`-pedantic`/suggestion warnings lccc does not implement yet — plus
`invalid-__has_warning1.c` (EOF-truncated call message text) and
`pragma_assume_nonnull.c`. These belong to the E1 diagnostics backlog.

## Backlog (from this session)

- **B1 (audit F10)**: audit the byte->char decode sites for the
  1-byte-1-char assumption — measured: 5 literal `push(c as char)`
  (2x expr_eval.rs, 3x macro_defs.rs) plus 35 sibling byte-push sites
  in the preprocessor; move to `Vec<u8>` or UTF-8-aware decoding if
  non-ASCII source charsets are admitted.
- **B2**: memoize sema/codegen struct layout for anonymous record chains
  (quadratic today; bounded by TYPE_FRAME_BUDGET=2048 => ~0.6 s worst
  legal case).
- **B3**: implement the 8 pre-existing Preprocessor diagnostics gaps
  listed above (E1 scope).
- **B4**: C23 `bool`/`true`/`false` keyword gap under `-std=c2x`
  (`TokenKind::from_keyword` needs a dialect parameter).
- **B5**: giant initializer lists go super-linear past ~500k elements
  (measured, PRE-EXISTING at S08, embed NOT involved: plain
  `unsigned char a[] = {0,1,...}`: 480k elems = 1.6 s, 960k elems =
  21 s on both trees). Suspected sema initializer-eval threshold;
  huge `#embed` expansions surface it because they emit one
  million-element initializer. Full 50 MB embed additionally exceeds
  the 2 GB harness cgroup (SIGKILL) — bounded on real hosts by RAM,
  same complexity class as any implementation of the directive.
