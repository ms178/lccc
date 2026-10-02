# PR #721 review-audit adjudication (P1-1..P1-6, P2, P3) + CI-red root cause

Date: 2026-10-02.  Scope: the second Review AI audit (PR #721 = PR #719's
S03 content) and the red "Test Suite" job
(run 36929303098, job 110594329541).  As with the first audit, **every
claim was reproduced empirically** against the restored tree and the mined
corpus before any fix was written; verdicts below record the measured
evidence, not agreement by default.  All new behavior is pinned by
mocked-subprocess contract tests (`tests/corpus/test_runner_contracts.py`,
`tests/corpus/test_miner_contracts.py`,
`scripts/test_differential_corpus_paths.py`) that were written first, run
red, and are now green — and wired into CI + ci_local.sh parity (P1-6).

## CI red — root cause, found and reproduced

**First failing subcommand: `bash tests/regression/check_env_test_hygiene.sh`**
(step "Verify remaining fast local contracts", block command 63 of 72).
Reproduced by building `target/fastbuild/lccc` and executing the block's
commands in order; commands 1–62 passed, the hygiene gate failed exactly
as on CI, 64–72 pass once fixed.

Mechanism: the gate's invariant 4 greps `src/ tests/ scripts/` for
deferred-work markers (`FIXME`).  The PR imports 1 434 **verbatim** Clang
C fixtures; 87 of them carry upstream `FIXME` comments.  Byte-faithful
preservation of third-party fixtures is deliberate (attribution +
provenance doctrine, and the audit itself rejects mass-formatting upstream
fixtures).  The gate conflated *our* deferred work with *upstream prose
under test*.

Fix: the marker scan excludes exactly `tests/corpus/clang-c/` with a
written justification, and nothing else — the rest of `tests/` (including
our own corpus tooling), `src/` and `scripts/` stay fully covered (0 marker
hits outside the import tree; verified).  The gate was narrowed on a
principled boundary, not weakened.

Secondary findings from the repro: the block's first 30 commands also
proved `test_ci_gate_parity.py` is green with `.github/` present (an
earlier local failure was a harness-wipe artifact, not the CI failure);
`check_peephole_whitespace.sh` (phase 3 sweeps `tests/**/*.c`) survived the
corpus-skewed selection and passed.  A local-only prerequisite (`gcc-multilib
libc6-dev-i386`, installed by CI step 4) was needed for two link gates.

## Summary of audit verdicts

| # | Finding | Verdict | Fix |
|---|---|---|---|
| P1-1 | crash counts as passing rejection (`rc != 0`) | **CONFIRMED** (rc=-11 → PASS) | typed outcomes `ACCEPT/REJECT/CRASH/TIMEOUT/ERROR/UNSUPPORTED`; signals + ICE text are CRASH; CRASH/TIMEOUT/ERROR hard-fail even under XFAIL |
| P1-2 | invocation metadata silently drops/changes coverage | **CONFIRMED, to the digit** (25 mixed files/67 C runs lost; `%-DFIRST_WAY`→arch; 8 empty defaults dropped; `options_all` unapplied; 222 FileCheck files faked as compile) | per-invocation schema (`languages_per_set`, value-consuming translator, `%-D/-U/-I` pass-through, empty defaults kept, `options_all` applied, `phase` classification with UNSUPPORTED reasons) |
| P1-3 | diagnostics + GCC differential unreliable | **CONFIRMED** (108 template-`{{regex}}` msgs/44 files; 293 counts/105 files unenforced; 133 absolute `@N` lines wrong; verdict-level differential) | `msg_regex` template splicing (single source: miner), `count_min/max` enforcement, active-prefix selection per invocation, `@N`→absolute/`@*`→any; differential compares OBSERVED outcomes |
| P1-4 | corpus isolation fails for default root | **CONFIRMED, to the digit** (`tests`→2 382 files incl. 1 434 imports; `./tests`→948) | realpath-canonical prune + `--list` mode; enumeration tests drive the REAL script |
| P1-5 | licensing exceeds provenance | **CONFIRMED** (bundled text lacked the subtree's 45-line Legacy UIUC/NCSA section; README mapped GPL-3 material to an LGPL-3 text; 53 files with `/usr/include` expansion) | subtree `LICENSE.txt` bundled verbatim (with NCSA); GPL-3 mapping corrected (no text bundled, facts-only); `contains_expanded_system_headers` flag + LICENSE-NOTICE mixed-origin classification (no blanket assertions) |
| P1-6 | new behavior not CI-gated | **CONFIRMED** (nothing ran the miners/runner) | 4 gates wired into ci.yml **and** ci_local.sh, parity-checked green (129 commands mirrored) |
| P2 | CLI: `--no-reject` spawns first; `--xfail-as-pass` dead; bad category exits 0 | **CONFIRMED** | skip-before-spawn; `--xfail-as-pass` collapses XFAIL→PASS; unknown category exits 2 with valid list |
| P2 | profiling: empty run "succeeds"; D1 column lacks write misses; stdout-only correctness | **CONFIRMED** | exit 2 + manifest failures on empty runs; D1 = reads+writes; LL = IL+DL r+w; rc checked; failures recorded |
| P2 | Changes scoring: dead `_Generic`/`_Complex`/`_Static_assert` rules | **CONFIRMED** (lowercased hay vs case-sensitive literals) | lowercase literals + pinned selftest (7 = 3+1+3); re-mined extract is **1 567** entries — exactly the audit's predicted +8 |
| P2 | `--force` can clobber good corpus from empty input | **CONFIRMED** (live in tests) | empty input refused; stage → validate → atomic swap; old corpus preserved on any failure |
| P2 | reporting: untranslated flags omitted; undecodable stderr aborts | **CONFIRMED** | per-test `untranslated_edg_flags` in results/reports; bytes decoded `errors="replace"` |
| P3 | entry points non-executable; metric labeling | **CONFIRMED** | `chmod +x` on 4 entry points (git mode 100755); historical vs current counts labeled in docs |

## Where I disagree with the audit (and why)

* **"216 files have custom prefixes"** — measured 144 files with
  non-`expected` prefixes in the shipped index (their count likely includes
  un-shipped giants or per-RUN counting).  The class is real; the exact
  figure differs by filter.  Fixed regardless.
* **"52 files contain expanded system-header provenance"** — measured 53
  (`/usr/include` mentions).  We adopt their policy exactly: classification
  only, no infringement claims, no invented blanket licensing.
* **"332 accept files contain diagnostic expectations"** — measured 476
  accept files with any `expected[]` entry (their figure may exclude
  `no-diagnostics`-only files).  The audit's *policy* is adopted verbatim:
  outcome-only accept checking is never described as diagnostic coverage;
  `--mode diagnostics` enforces declared expectations, `--mode outcome-only`
  is labeled as such in reports.
* **`{{regex}}` semantics**: we verified their reading against the raw
  corpus (`'(unnamed struct at {{.*}})' …` on `-re` lines, 108/108) and
  against match semantics (whole-message `re.search` cannot match the real
  diagnostics) — Clang's template-splice interpretation is the only
  consistent one.  Implemented as `msg_regex` at mining time.
* Their instruction "leave actual compiler execution to GitHub CI" was
  necessary here *inverted*: the CI root cause could only be identified by
  reproducing the block locally with the built compiler.  No compiler
  semantics were changed.

## Reproducers

```sh
# CI root cause (block order preserved; first failing command):
awk '/name: Verify remaining fast local contracts/,/^      - name: (Verify kernel/' \
  .github/workflows/ci.yml | grep -E '^\s+(python3|bash) '   # then run in order

# contract suites (no compiler, mocked subprocess):
python3 -m unittest discover -s tests/corpus -p 'test_*.py'    # 44 tests
python3 scripts/test_differential_corpus_paths.py              # 5 tests
python3 scripts/edg_changes_mine.py selftest && python3 scripts/edg_corpus_mine.py selftest

# the fixed data:
#   corpus-index.json: 25 mixed files keep C runs, 10 empty defaults,
#   0 garbage arch tags, 133/133 absolute @N lines correct,
#   15 346 msg_regex expectations, 53 provenance flags
#   extract: 13 562 entries / 1 567 C-relevant
```
