# PR #719 review-audit adjudication (F1–F8)

Date: 2026-10-01.  Scope: the review AI's audit of PR #719 (ms178-1.patch
S01/S02: EDG-transplant miners, callgrind pinning, knowledge artifacts).
The audit was **not taken on faith**: every finding was reproduced
empirically against a fresh EDG clone and the patched lccc tree before any
fix was applied.  Verdicts below record agreement/disagreement with
evidence, the implemented fix, and where this round goes *beyond* the
audit's prescriptions.

Method: each finding got a reproducer (numbers below are measured, not
estimated).  Two of the audit's *prescriptions* were independently rejected
or improved where evidence contradicted them.  All fixes are selftested
(`selftest` gates in both mining scripts) and gated by sanity checks that
fail loudly instead of emitting corrupted artifacts.

## Summary

| # | Finding | Verdict | Fix |
|---|---|---|---|
| F1 | `src/Changes` parse lost the entire pre-2008 era; 75.5% of the extract was one mega-entry | **CONFIRMED** (2 935 158 B blob; 0 pre-2008 entries in artifacts) | dual-era header grammar + wrap handling + entry-count sanity gate |
| F2 | C-relevance scoring double-counted (`init` vs `initialization`) and unanchored patterns (`in_c` hit `C++`/`include`) | **CONFIRMED** (GNU C++ entry score +1 from double count; FP counts in_c 519 / gnu_c 378 / include 85 among kept) | word-boundary regex scoring, merged overlap groups, `(?!\+\+)` guards |
| F3 | `#include` line ended parsing too early / `//type:` leaked across preprocessor lines; `expected-*` in headers and preprocessor lines dropped | **CONFIRMED** (28 of 200 sampled files had `expected-` text with zero extracted) | header-block state machine (comments only), expectations from every line |
| F4 | `-verify` extraction stopped at the first `}}`; `@±N`, `-re`, counts, custom prefixes, multi-annotation lines unsupported | **CONFIRMED** (3 064/12 082 = 25.4% empty diagnostic messages across 314 files in the S02 artifacts) | full `-verify` grammar: finditer, balanced-brace messages, line recording |
| F5 | sidecar `.meta.json` files duplicated `corpus-index.json` (repo bloat); `differential_corpus.sh` swept `tests/corpus/` | **CONFIRMED** (100% of sidecar data in index; the sweep would double-count curated tests) | sidecars opt-in only; corpus pruned in the diff harness |
| F6 | `//options:` `:`-separated *invocations* conflated into one set; option args not stripped | **CONFIRMED** (163 multi-invocation files, e.g. `clang-c/C/n1285.c`; 443 unstripped args) | `option_sets` per invocation + `.strip()` + `gcc_flags_per_set` translation |
| F7 | `stats.json` emitted year `"127"`; index lacked a table header; extract had trailing-space hard breaks | **CONFIRMED** (`year: "127"` in S02 stats; 1 521 trailing-2-space lines) | documented date-typo table (4 typos), index header, all output rstripped |
| F8 | `callgrind_ab.py` didn't print geometry, omitted D1 misses, `agg["ir"]` KeyError-prone; EDG license text missing from `third_party_licenses/` | **CONFIRMED** (all four) | geometry+versions in `manifest.json`, D1 columns, `agg.get`, license file + LICENSING.md |

## F1 — `src/Changes` pre-2008 era lost (the big one)

**Confirmed.**  The S02 extractor contained one `8/7/08 · EDGcpfe/9120`
entry of 2 935 158 bytes (75.5% of the 3.89 MB artifact) swallowing
thousands of older entries; zero entries dated before 2008-08-07 were
parseable from the shipped artifacts.  Root cause: the parser only accepted
bracketed bug-ID headers, and EDG used those only from 2008 onward.

Evidence (fresh clone, independent of the parser under test): raw
`src/Changes` has 7 416 bracketed headers, 4 979 padded plain headers
(title column 7–9), and 1 169 one-space plain headers after blank lines —
~13 564 candidate headers, reverse-chronologically consistent from
`5/4/92` to 2026.

**Prescription adopted, one part rejected.**  The audit proposed a
column-padding rule for plain headers; measured evidence rejected it: the
title column is **not constant** across eras (`6/7/93 Variable size array
representation` pads to column 7, `9/25/04 C++-generating back end` to
column 8, later entries to column 9+).  A rigid `col >= 9` rule rejected
626 real headers.  The implemented discriminators are: blank line before,
plausible date, and a negative check against body-wrap connector words
(`for`, `and`, `entry`, `describing`, ...) — the wrap lines
(`4/14/17 for EDGcpfe/17414 and EDGcpfe/17706).`) all start with such
words.  The parser now yields **13 562 entries** (ground truth ±2
counting overlap), 1 559 C-relevant, 914 KB extract, and refuses to emit
below a 13 000-entry sanity gate.

**Beyond the audit:** dual-era selftest corpus with adversarial cases
(wrapped bracketed IDs, wrap references after blank lines, typo dates);
typo adjudication via reverse-chronological neighbors (see F7).

## F2 — scoring double counts and unanchored matches

**Confirmed.**  Measured on S02 scoring: the GNU C++ entry scored +1 from
`init` matching inside `initialization` while both rules carried full
weight; among kept entries, `in_c` fired on `C++`-only and `include`
contexts 519/85 times.  Fix: regex patterns with word boundaries
(`(?!\+\+)` guards on C-mode rules), merged overlap groups so `init|initialization`
contributes once.  Selftest pins both properties.

## F3/F4 — `-verify` extraction

**Confirmed both.**  F3: 28/200 sampled corpus files contained
`expected-` text but zero extracted diagnostics (preprocessor lines ended
parsing); the review's read of the `.sft` grammar (directive header = leading
comment block; `#`-lines are code; `expected-no-diagnostics` lives in
comments) matched the upstream `$EDG/tests/README.md`.  F4: 3 064 of 12 082
captured diagnostics (25.4%) had empty messages across 314 files in the S02
artifacts — the classic `}}`-terminator bug with nested-brace messages.

Fix: expectations are extracted from **every** line; directive scanning
stops at the first code line; the message is consumed with balanced-brace
scanning; `@±N`/`@*` offsets (recorded as absolute source lines), `-re`
regex form, repeat counts (`2`, `1+`, `0-1`), custom prefixes
(`c-error`, `c23-warning`, `-verify=expected,c,c11,c23`) all parsed.
Verified on the real corpus after re-mining: **0** files with `expected-`
text and zero captured (was 28/200), 347 files with advanced annotations
(`@loc`/`-re`/counts) captured, 1 114 files with expectations.

## F5 — sidecars and the differential sweep

**Confirmed.**  Sidecar content was 100% duplicated in `corpus-index.json`;
`scripts/differential_corpus.sh`'s `find "$ROOT" -name '*.c'` (ROOT
default `tests/`) would sweep the curated corpus into the small diff
harness.  Fix: sidecars now opt-in (`--emit-sidecars`); the diff harness
prunes `*/tests/corpus`.  Going forward the index is the single source of
truth (also absorbs SHA-256, so nothing is lost by dropping sidecars).

## F6 — `//options:` semantics

**Confirmed.**  EDG `//options:` is a list of separate invocations split on
`//options_sep:` (default `:`) — confirmed against upstream
`$EDG/tests/README.md`; 163 corpus files exercise multi-invocation form and 443
args had unstripped whitespace in the S02 artifacts (`' --c11'`).
Fix: `option_sets` (one list per invocation, args stripped) — never
flattened; `gcc_flags_per_set` translates EDG dialect switches to suggested
GCC/lccc flags (a deterministic EDG→GCC map, documented in the miner);
`untranslated_edg_flags` names gaps explicitly.  **Beyond the audit:** the
flag translation map and the corpus runner built on it
(`tests/corpus/run_clang_c_corpus.py`).

## F7 — generated artifacts

**Confirmed.**  `stats.json` contained `"year": "127"` (upstream typo
`10/25/127`, an EDGcpfe/18864 entry from 2017); the index lacked its
markdown table header; 1 521 extract lines had trailing-2-space
"hard-breaks".  Fix: a documented `DATE_TYPOS` table — `10/25/127`→2017,
plus three pre-2008 typos discovered and adjudicated by
reverse-chronological neighbors (`10/1/37`→1997 after `10/2/97`;
`5/9/69`→1996 after `6/1/96`; `8/3/82`→1992 after `8/4/92`) — index table
header, and all generated lines rstripped (zero whitespace warnings from
`git apply` expected on these files).  **Beyond the audit:** the three
extra typos and the neighbor-based adjudication method.

## F8 — callgrind harness + licensing

**Confirmed all.**  `callgrind_ab.py` never printed its pinned geometry
(so a reader could not tell what cache model produced the miss counts),
the table had no D1 miss columns, `agg["ir"]` raised `KeyError` on empty
runs, and `third_party_licenses/` had no EDG entry.  Fix: geometry +
compiler `--version` strings + env overrides recorded in
`manifest.json` (an A/B without its cache model is not evidence), D1
columns in the table, `agg.get("ir")`, and
`third_party_licenses/EDG-Clang-Apache-2.0-LLVM.txt` (verbatim upstream
license + provenance header) referenced from `LICENSING.md` and
`third_party_licenses/README.md` (F8b).

## Findings NOT accepted as stated

* **F1's column-padding rule** (see F1): rejected — the padding column
  varies by era; adopting it would have kept rejecting 626 real headers.
  Blank-line + connector-word discrimination is empirically correct.
* **F4's implication that `expected-no-diagnostics` must appear in
  `//`-comments specifically**: our extractor accepts it from any line,
  which is strictly more permissive and cannot lose information; the
  header-comment placement is what the real corpus uses and is handled.

Everything else in the audit was verified and adopted.  The audit's
numbers were, where measurable, accurate to the digit — its weakness was
prescribing mechanics (column rules) from a smaller sample than a full
era-by-era ground-truth count provides.

## Reproducers

```sh
# F1 ground truth (independent of the parser):
grep -cE '^[0-9]{1,2}/[0-9]{1,2}/[0-9]{2,4}\s{2,}(\[|$)' $EDG/src/Changes   # 7 416
# new parser totals + gates:
python3 scripts/edg_changes_mine.py selftest                                  # 18/18
python3 scripts/edg_changes_mine.py --edg-root $EDG                           # 13 562 entries
# F3/F4 on the real corpus:
python3 scripts/edg_corpus_mine.py selftest                                   # 18/18
python3 scripts/edg_corpus_mine.py clang-c --edg-root $EDG --force
# 0 files with expected- text but zero captured; see tests/corpus/README.md
```
