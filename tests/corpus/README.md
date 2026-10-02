# Corpus tooling: inventory, declarations, observations—not coverage by file count

This directory contains imported test material and compiler-free infrastructure.
The governing current contract is [`tools/corpus/`](../../tools/corpus/), not the
older per-file `lccc_expect`/`verify_prefixes_per_set` summaries. The independent
[audit and backlog](../../engineering/PR724-AUDIT-2026-10-02.md) documents the
validation boundary. **No native corpus execution or compiler-performance win
was established in this audit.**

## Inventory and provenance

| Material | Count | Meaning |
|---|---:|---|
| Clang C source records | 1,529 | 1,434 copied files + 95 explicit skipped sources |
| Planned invocation records | 5,130 | 4,982 actual Clang `RUN` declarations + 148 opaque EDG-only inventory placeholders |
| GNU manifest | 18,188 | Test names/directive facts only; no redistributed GNU test bodies and no GNU execution coverage |
| EDG Changes | 13,562 | Historical entries; 1,567 heuristic selections, not executable tests or a language classification |

The Clang source pin is
`edgcpp/compiler@0ac366374c06e54612ddbac397c74ffa40e5182c`, subtree
`6626681cf444dc0bd258aef2bb3aafbfd0f162ae`. The index records Git blob IDs,
SHA-256, source/transform provenance, expected diagnostics and per-invocation
contracts. The generator identity covers the shared schema/directive/diagnostic
implementation. It is strict compact JSON (`lccc-c-corpus-v2`): duplicate keys,
NaN/Infinity and malformed control fields are errors; unbounded counts use
`count_max: null`.

All 1,434 copied `.c` files remain **byte-identical to the original PR**. After
removing the declared three-line attribution prefix, 1,433 bodies also match
pinned upstream Git blobs. The exception, `C/C23/n2927.sft.c`, came from an
upstream invalid UTF-8 byte `0x92` that the original importer replacement-decoded.
That transformation is explicitly bound to known hashes and unsupported for
execution. Do not “repair” it, rewrite upstream whitespace or change diagnostic
expectations to make a run pass.

The retained [Clang subtree notice](../../third_party_licenses/EDG-Clang-Apache-2.0-LLVM.txt)
contains the byte-exact pinned upstream notice, including Legacy NCSA terms.
`source.license_sha256` identifies the **retained attribution/notice file**;
per-record license evidence separately identifies the upstream notice digest.
A subtree SPDX declaration is not per-file clearance. The `/usr/include`
text-origin indicator appears in 148 inventory records, only 53 copied files:
it is a review signal, **not** a mixed-origin copyright classification. These
records stay blocked by the conservative execution policy. GNU metadata is an
inventory, not a GPL-body import or completed GNU provenance/conformance audit.

## Compiler-free checks

Use this entrypoint during a no-build audit:

```sh
bash scripts/ci_corpus_tools.sh
python3 scripts/edg_corpus_mine.py verify-index
python3 tests/corpus/run_clang_c_corpus.py --plan \
  --compiler-family lccc --arch x86_64 --report /tmp/corpus-plan.json
```

It checks miner selftests, every `tests/corpus/test_*.py`, strict index/fixture
integrity, generated-document integrity, differential path selection, script
imports, workflow shell syntax, CI parity and documentation links. It does not
install toolchains, query/build a compiler or run native C. The local and hosted
CI mirror includes these tooling checks, **not** native LCCC execution of this
corpus. `ci_local.sh --fast` is NOT compiler-free.

The validated x86_64/LCCC planning run selects 1,529 files and records 5,130
planned invocations plus 95 unplanned-source skips. It has 102 eligible
plan-only invocations, 5,028 UNSUPPORTED invocations and **zero executions**.
Plan-only eligibility is reported as SKIP, never a PASS or C coverage.

Do not run `clang-c --force` against the frozen imported fixtures in this
review. Index-only `upgrade-index --tree-manifest FILE` is an explicit migration
operation: it verifies/recomputes the pinned Git trees and source/body identities
before atomic publication. Repeat upgrades preserve EDG facts, declared runs
and rights indicators. Fresh mining elsewhere requires the pinned clean Git
checkout, exact candidates/blobs and retained notice; publication must verify
the complete staged generation and fail without deleting the live generation.

## Invocation and diagnostic contract

* Actual Clang `RUN` commands bind their own prefixes to explicit expectation
  IDs. EDG `//options:` facts are independent, not zipped or broadcast onto
  those commands. EDG-only rows remain opaque unsupported inventory—case/
  `TEST_NUMBER`/phase/options execution has not been invented.
* Supported compiler-free declaration parsing covers physical/presumed lines,
  `@N`, `@±N`, `@*`, literal `#line` filenames (including empty filenames and
  line zero), severities and active prefixes. Labels, conditional/spliced
  declarations, GNU marker stacks and unresolved source-manager cases stay
  explicit unsupported.
* Located diagnostic headers must match file, line, severity, message and
  cardinality. Echoed annotations, repeated words, inactive prefixes and one
  diagnostic reused twice cannot certify a rejection.
* Ordinary Clang declarations have **literal** messages. Only explicit `-re`
  declarations use nested `{{ERE}}` fragments, in a bounded ASCII POSIX-ERE
  subset. Matching budgets and a killable per-verification Python worker bound
  backtracking; timeout/worker/resource failures are ERROR, never XFAIL.
  This is not a full native Clang/lit/SourceManager-equivalence certificate.
* Repeated `-verify` policy, malformed no-diagnostics/count/prefix declarations,
  optional-error outcome ambiguity, unresolved external inputs/include graphs,
  unestablished default dialect, unknown flags/capabilities, C++/other languages,
  runtime/link/codegen/output oracles and FileCheck remain unsupported rather
  than silently becoming a generic `-fsyntax-only` acceptance test.
* `--arch` passes a prerequisite filter; it does **not** retarget unqualified
  commands or prove the compiler's default architecture. Target/default/m32
  translation evidence is separate. Unknown or different contracts are
  NONCOMPARABLE in the advisory differential.

Every source retains every invocation observation and independent unplanned
source skips. Reports (`lccc-c-corpus-report-v2`) separate command/translation,
raw exit code/output hashes/resource state, diagnostic policy and final verdict.
Sources are SHA-checked before and after spawn and again before publication.
Bounded process execution closes stdin, pins locale, bounds combined output and
kills the process group on timeout/budget failure. JSON reports publish by
fsync/rename; failed publication does not replace an older complete report.

Standard XFAIL policy: clean policy failure becomes XFAIL, unexpected success
becomes XPASS. CRASH/TIMEOUT/ERROR remain hard failures. `--xfail-as-pass` is
removed. `--mode outcome-only` explicitly carries **no diagnostic certificate**;
without declared `-verify`, an outcome-only pass must not be counted as
verified diagnostics. Differential pairing is advisory, per ID, and only
compares clean executed ACCEPT/REJECT observations with the same established
translation/architecture. Missing, skipped, unsupported and terminal observations
remain visible and NONCOMPARABLE.

## First-party gates stay isolated

[`corpus_selection.py`](../../scripts/corpus_selection.py) resolves physical
identity, prunes `tests/corpus` before sampling, avoids directory-symlink
traversal and deduplicates aliases. Whitespace/differential/identity/AArch64
consumers share it. The every-15th whitespace list has 64 first-party sources:
56 regression, six benchmark, two other, **zero corpus paths**. Assembly output
identities do not rely on basename uniqueness. Codegen identity requires
explicit regression/benchmark roots and rejects broad `tests/`.

## Generated Changes and simulated benchmark evidence

```sh
# Offline generator/output/denominator integrity, no raw EDG checkout needed:
python3 scripts/edg_changes_mine.py verify-artifacts
# Full reproducibility when pinned src/Changes is available:
python3 scripts/edg_changes_mine.py check --edg-root /path/to/pinned-edg
```

Offline integrity is not full regeneration. Full regeneration independently
succeeded from the exact 7,416,817-byte source. Excerpts are presentation-
normalized, not byte-verbatim fixture copies. Hash-linked statistics publish
last; multi-file interrupted publication is detected, not called an atomic
multi-file transaction.

Callgrind tooling retains equal-length `aa`/`bb`, pinned I1/D1/LL geometry,
visible overrides, read+write misses, exit+stdout checks and strict manifests.
Discovery is lazy; missing/zero/malformed Ir is failed evidence, not a zero
speedup, division error or successful partial run. Instrumented output is checked
against ordinary output. These are **simulated instruction/cache-model counts,
not Raptor Lake uops, PMU counters, cycles or measured speedups**. Match ISA and
software/library/startup contracts before interpreting them; no Callgrind run
or actual performance measurement was performed in this audit.
