# tests/corpus — mined external test corpora (transplant items E1/E7)

This directory holds **mined third-party test material** for LCCC's frontend
and optimizer, produced by [`scripts/edg_corpus_mine.py`](../../scripts/edg_corpus_mine.py)
from the open-sourced [EDG C/C++ front end](https://github.com/edgcpp/compiler)
test suite.  See [`docs/EDG_TRANSPLANT_ANALYSIS.md`](../../docs/EDG_TRANSPLANT_ANALYSIS.md)
§4.2 for the design rationale and
[`engineering/FOLLOWUP-2026-10-01-edg-transplant.md`](../../engineering/FOLLOWUP-2026-10-01-edg-transplant.md)
for session history.

## What lives here

| Path | License | Content |
|---|---|---|
| `gnu-torture-manifest.jsonl` | **facts only** (GPL source NOT redistributed) | 18 188 GCC-derived test names + `//type:`/`//options:` directives + category, one JSON record per line |
| `gnu-torture-manifest-summary.json` | same | aggregate counts by type/category |
| `clang-c/` | Apache-2.0 WITH LLVM-exception | 1 434 adapted Clang C tests (`C/`, `Parser/`, `Preprocessor/`, `Sema/`) with `.meta.json` sidecars + `corpus-index.json` + `LICENSE-NOTICE.txt`; 95 multi-MB Arm SME/SVE feature-matrix giants deliberately skipped (`skip_reason` in the index) |

## Licensing law (non-negotiable)

* `tests/tests/imported/clang/**` in EDG is **Apache-2.0 WITH LLVM-exception** —
  adapted copies are fine; every copied file keeps an SPDX attribution header.
* `tests/tests/imported/gnu/**` in EDG is **GPL-3.0** — its source must
  **never** enter this tree.  We store testcase *names and directive values*
  (uncopyrightable facts) only; a runner fetches the actual files from a
  locally cloned EDG repo or a GCC checkout at run time.

## Regenerating / verifying

```sh
git clone --filter=blob:none --depth 1 https://github.com/edgcpp/compiler.git /home/user/edg-compiler

scripts/edg_corpus_mine.py selftest                                    # parser unit tests
scripts/edg_corpus_mine.py summary      --edg-root /home/user/edg-compiler
scripts/edg_corpus_mine.py clang-c      --edg-root /home/user/edg-compiler --force
scripts/edg_corpus_mine.py gnu-manifest --edg-root /home/user/edg-compiler
```

Every copied test carries a `sha256` in its sidecar / the index: after a
harness restore, re-mining must reproduce identical hashes for unchanged
EDG input.

## How to drive the mined tests

Each `clang-c/**/*.c` has a `*.c.meta.json` sidecar:

* `lccc_expect`: `accept`, `reject` or `skip` — whether LCCC must compile
  the file (derived from the EDG `//type:`).
* `options` / `options_sep`: recorded flags (`//options_sep:` honoured).
* `expected`: `expected-warning`/`expected-error` annotations extracted from
  LLVM `verify`-style comments, for diagnostic verification.
* `has_main`, `test_number_uses`, `bytes`, `origin`, `sha256`.

The GNU manifest records `has_main`, `type`, `options`, `cases` per name;
the `rp`/`lp` slice (2 876 entries) is the highest-value set for the
differential runtime harness (`run_regression_suite.sh` conventions:
identical stdout+exit vs GCC).  The `tree-ssa/` (2 458) and `vect/`
(1 816) categories are optimizer-legality gold for later phases.
