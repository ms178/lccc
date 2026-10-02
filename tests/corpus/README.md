# tests/corpus — mined external test corpora (transplant items E1/E7)

This directory holds **mined third-party test material** for LCCC's frontend
and optimizer, produced by [`scripts/edg_corpus_mine.py`](../../scripts/edg_corpus_mine.py)
from the open-sourced [EDG C/C++ front end](https://github.com/edgcpp/compiler)
test suite.  See [`docs/EDG_TRANSPLANT_ANALYSIS.md`](../../docs/EDG_TRANSPLANT_ANALYSIS.md)
§4.2 for the design rationale,
[`engineering/FOLLOWUP-2026-10-01-edg-transplant.md`](../../engineering/FOLLOWUP-2026-10-01-edg-transplant.md)
for session history, and
[`docs/REVIEW_719_ADJUDICATION.md`](../../docs/REVIEW_719_ADJUDICATION.md)
for the PR #719 review adjudication that reshaped this layout.

## What lives here

| Path | License | Content |
|---|---|---|
| `gnu-torture-manifest.jsonl` | **facts only** (GPL source NOT redistributed) | 18 188 GCC-derived test names + `//type:`/`//options:` directive values + category, one JSON record per line |
| `gnu-torture-manifest-summary.json` | same | aggregate counts by type/category |
| `clang-c/` | Apache-2.0 WITH LLVM-exception | 1 434 adapted Clang C tests (`C/`, `Parser/`, `Preprocessor/`, `Sema/`; EDG `C/C89`, `C/C11`, `C/C99`, `C/DR`, ... subdirectories preserved) + `corpus-index.json` + `LICENSE-NOTICE.txt`; 95 multi-MB/oversized tests deliberately skipped (`skip_reason` in the index) |
| `run_clang_c_corpus.py` | LCCC contribution | corpus runner (accept/reject/xfail, timeouts, GCC differential, per-category) |

**`clang-c/corpus-index.json` is the single source of truth for metadata**
(one record per test: expectations with source lines, `option_sets`, SHA-256,
language/arch tags, skip reasons).  Per-file `.meta.json` sidecars are gone;
`--emit-sidecars` exists only for tooling that needs them.

## Licensing law (non-negotiable)

* `tests/tests/imported/clang/**` in EDG is **Apache-2.0 WITH LLVM-exception** —
  adapted copies are fine; every copied file keeps an SPDX attribution header.
  Full text: [`third_party_licenses/EDG-Clang-Apache-2.0-LLVM.txt`](../../third_party_licenses/EDG-Clang-Apache-2.0-LLVM.txt).
* `tests/tests/imported/gnu/**` in EDG is **GPL-3.0** — its source must
  **never** enter this tree.  We store testcase *names and directive values*
  (uncopyrightable facts) only; a runner fetches the actual files from a
  locally cloned EDG repo or a GCC checkout at run time.

## Regenerating / verifying

```sh
git clone --filter=blob:none --depth 1 https://github.com/edgcpp/compiler.git /home/user/edg-compiler

scripts/edg_corpus_mine.py selftest                                    # parser unit tests (18 cases)
scripts/edg_corpus_mine.py summary      --edg-root /home/user/edg-compiler
scripts/edg_corpus_mine.py clang-c      --edg-root /home/user/edg-compiler --force
scripts/edg_corpus_mine.py gnu-manifest --edg-root /home/user/edg-compiler
```

Every copied test carries a `sha256` in the index: after a harness restore,
re-mining must reproduce identical hashes for unchanged EDG input.

## How to drive the mined tests

```sh
tests/corpus/run_clang_c_corpus.py --cc lccc                   # corpus pass
tests/corpus/run_clang_c_corpus.py --cc lccc --gcc-cmd gcc -j2 # + outcome diff
tests/corpus/run_clang_c_corpus.py --category Preprocessor --limit 40
```

Verdict model: every run records an OBSERVATION (`ACCEPT/REJECT/CRASH/
TIMEOUT/ERROR/UNSUPPORTED` — signals and ICE text are CRASH, never a
passing rejection), a DIAGNOSTIC policy (template-regex `{{…}}` matching
with counts and active prefixes) and a VERDICT (`PASS/FAIL/XFAIL/XPASS/
SKIP`).  XFAIL covers semantic divergence only; crashes, timeouts and
infrastructure errors hard-fail under every policy.  `--mode
diagnostics` enforces declared `-verify` expectations; `--mode
outcome-only` is accept/reject only and is labeled as such in reports.
The GCC differential compares observed OUTCOMES, not verdicts.

Contract tests (mocked subprocesses, no compiler):
`python3 -m unittest discover -s tests/corpus -p 'test_*.py'` — wired into
CI and `ci_local.sh` together with both miner self-tests and
`scripts/test_differential_corpus_paths.py`.

Per-test metadata in `corpus-index.json` (schema v2, PR #721 review round):

* `lccc_expect`: `accept`, `reject` or `skip` — whether LCCC must compile
  the file (derived from the EDG `//type:`).
* `option_sets`: each `//options:` invocation kept separate (`:`-split,
  `//options_sep:` honoured, args stripped, **empty segments preserved as
  default invocations**); `languages_per_set` marks C vs C++ per run;
  `options_all` args are prepended to every run; `gcc_flags_per_set`
  translates EDG dialect switches (`--c99`, `--gnu`, `--strict`, `%-D…`
  pass-through) to suggested GCC/lccc flags, with target/march values
  consumed as data (`arch_tags`) and `untranslated_edg_flags` naming every
  gap.  The last standard flag wins per run.
* `phase`: `compile` / `preprocess` / `execute` / `filecheck` (from the
  `// RUN:` lines); the runner executes `compile`, records `filecheck` as
  UNSUPPORTED with a reason — it never fakes coverage.
* `expected[]`: Clang `-verify` diagnostics with `kind`/`prefix`/`-re`
  form, `msg` (raw template) **and `msg_regex`** (Clang `{{regex}}`
  template splicing, generated once in the miner), `count`/`count_min`/
  `count_max`, `@±N` relative / `@N` absolute / `@*` any / `@#label`
  locations recorded as source `line`/`any_line`/`label`.
  `verify_prefixes_per_set` lists each run's `-verify=` active prefixes.
* `contains_expanded_system_headers`: mixed-origin classification flag
  (see `LICENSE-NOTICE.txt`); `language`, `arch_tags`, `has_main`,
  `test_number_uses`, `cases`, `bytes`, `origin`, `sha256`.

The GNU manifest records `has_main`, `type`, `option_sets`, `cases` per
name; the `rp`/`lp` slice (2 876 entries) is the highest-value set for the
differential runtime harness (`run_regression_suite.sh` conventions:
identical stdout+exit vs GCC).  The `tree-ssa/` (2 458) and `vect/`
(1 816) categories are optimizer-legality gold for later phases.

## Design limits (deliberate)

* Giant generated Arm SME/SVE feature-matrix tests are skipped by name
  pattern (`arm_sme_*`, ...) and a 256 KB size cap; reasons are recorded per
  entry.  `--include-giants` overrides.
* Diagnostic *matching* policy lives in the runner, not the miner: the
  index records what the test expects; how strictly LCCC must reproduce it
  is a runner concern (so a young compiler can xfail early and tighten).
