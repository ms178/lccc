# PR #824: empirical follow-up and limits

Date: 2026-10-14. Compiler repair commit: `6b933af`; initial infrastructure follow-up: `e7d397b`.

This supersedes the **validation status**, not the historical observations, in
[the initial audit](PR824-AUDIT.md). The user's subsequent instruction explicitly
requested local compilation and execution; the earlier no-local-build restriction
no longer applies. No universal superiority, exhaustive bug absence, or 10/10
certification is claimed.

## Current continuation

[The rebased architectural-effects audit](PR824-REBASE-AUDIT.md) records the
latest counter-domain repair, copy/spill follow-through, native negative
controls, genuine Callgrind measurements and current acceptance blockers.
Treat earlier numbers below as stage-labeled historical measurements, not
proof that the final source or local fast CI has passed.

## What execution changed about the audit

The baseline Test Suite at `81f9930` actually failed the five predicted unit tests:
VEX upper zeroing, pinned dead-FP moves, location-counter CSE, pinned CSE, and the
`vmaskmovdqu` forwarding barrier. This is stronger evidence than the original
source-only prediction. Baseline run: [38555750182](https://github.com/ms178/lccc/actions/runs/38555750182).

The repairs preserve the original optimization direction rather than disabling
passes globally:

- **Region-aware FP liveness:** retain the existing low-128-bit and frame-slot
  identities, add independent 128..256/256..512 region facts, and require all
  written regions to be dead before deleting a VEX move. Legacy low-only DCE
  remains available. Tests cover preservation and profitable deletion, wider
  observations, branch paths, masked writes, full VEX kills and explicit upper
  zeroing. Destructive EVEX families retain destination dependencies.
- **Shared stable-address recognition:** both symbol-LEA CSE implementations
  reject location-counter/numeric RIP expressions. Ordinary symbols, addends and
  `.LC0` remain eligible. Pinned lines are boundaries, not rewrite candidates.
- **MAY-write invalidation:** handle both exchange operands, partial writes,
  multi-output instructions and implicit string outputs. A new negative test
  caught a defect in my initial repair: the reused MAY helper's documentation
  claimed unknown instructions wrote everything, but its fallback trusted their
  final operand. The implementation now refuses that inference for unknown
  families. Tests also caught the string-comparison exception to the flag-only
  `cmp*` classification.
- **Memory barriers and assertion integrity:** recognize `vmaskmovdqu`,
  `movdir64b` and `enqcmd*`; preserve pinned FP moves; assert active line metadata,
  not immutable source text that remains after a deletion.
- **Immutable IVSR indexes:** build definitions, canonicalized address-use lists
  and raw value-use membership once per discovery scan. Preserve the distinction
  between alias-resolved address uses and raw `has_uses` identities. Analysis
  does not mutate IR while those indexes are live.

## Additional executed counterexample: flag-only VEX operations

The follow-up red team found another architectural classification defect beyond
those five baseline failures: `vucomisd`, `vucomiss`, `vcomisd`, `vcomiss`,
`vptest`, `vtestpd` and `vtestps` have no vector destination. The generic VEX
full-definition rule treated their final READ operand as a kill, allowing the
feeding move to be deleted even when the flags were subsequently observed.

The new optimizer regression was first run against the uncorrected classifier
and **failed** at `vucomisd` (`flag input deleted`). After correction, all seven
forms are checked as read dependencies. The independent, checked-in
[architectural witness](../tests/regression/fixtures/peephole-vex-flags/README.md)
was assembled with GCC and actually executed on AVX2 hardware: all seven pairs
returned `original=1 removed=0`. This witnesses the real ISA effect, not just an
expectation encoded in the optimizer test. It is also wired into hosted/local CI
and the Debian exact-version lane. No production C-level automatic reproduction
is inferred from a handwritten assembly witness.

## Executed local checks

| Check | Result |
| --- | --- |
| Rust 1.99 `cargo test --profile fastbuild --all-targets --locked -j 2` | 4,684 library tests passed, 0 failed, 8 ignored; other targets passed, including one bin test |
| Strict Clippy wrapper | PASS; 76 policy lint names probed, zero unknown |
| Rust 1.99 formatter | PASS |
| Native VEX flag witnesses | PASS; 7 original/deleted pairs differ as predicted |
| Canonical native corpus, `-O2`, explicit local GCC reference | 45/45 output matches |
| Same complete corpus with `-fwrapv` in both arms | 45/45 output matches |
| Whole-TU static baseline/candidate comparison, workload-specific macros retained | 45/45 assembly listings identical |
| GCC provisioning mocks | 6 tests passed |
| Registry/codegen evidence mocks | 5 tests passed |
| CI/local parity after new registrations | PASS; 219 commands, 22 invocation contracts |
| Pipefail/SIGPIPE and conflict-marker checks | PASS; 165 wired shell scripts checked |

Build/test flags included `-D warnings -C link-arg=-Wl,--no-keep-memory`.
The local GCC reference is **Debian GCC 12.2.0**, not GCC 16.2. It is supplementary
execution evidence, not a substitute for the exact-version Debian lane.
The default corpus's integer-overflow assumptions are not silently treated as
language proofs: the separate whole-corpus `-fwrapv` run defines signed wraparound.

### Toolchain and swap

Installed tools report:

- `rustc 1.99.0 (b940084d7 2026-09-28)`;
- `cargo 1.99.0 (5f94df478 2026-08-27)`;
- `rustfmt 1.10.0-stable (b940084d7e 2026-09-28)`;
- `clippy 0.1.99 (b940084d7e 2026-09-28)`.

The restricted network required third-party npm distribution packages under
`@toolchain-bin/*-1.99.0-x86_64-unknown-linux-gnu`. Payloads were checked against
registry SHA-512 integrity metadata; the compiler-reported full commit matches
the official Rust 1.99.0 Git tag. This is **not** independent cryptographic
verification of an official Rust distribution signature. An earlier auxiliary
1.92 nightly was tried, exposed an unsupported language feature and a test-call
signature error, and was replaced; its results are not counted above.

A 4 GiB swapfile is active at `/tmp/lccc-audit.swap`, outside persisted source
artifacts. The initial all-target build actually used about 2.3 GiB of swap;
swap availability is observed, not merely a successful file allocation.

## Correct corpus and oracle use

The registry has **45 workloads**, not 39, backed by 43 distinct selected source
paths. A directory glob is not the registry; neither is a deduplicated list of
source filenames. Two important shared-source variants are:

| Workload | Source | Distinguishing registry flags |
| --- | --- | --- |
| `linux_find_bit_scaled` | `linux_find_bit.c` | `-DPASSES=65536` |
| `lz4_match_extend` | `lz4_compress.c` | `-DMATCH_RICH=1 -DPASSES=2048` |

My first static inventory passed 45 source paths to the batch tool but omitted
per-workload macros. That was **not** full shape coverage and is discarded.
The new `scripts/benchmark_codegen.py` adapter preserves registry IDs, tags,
extra flags, source hashes, matching local include paths and per-case reports.
Its tests ensure shared filenames are not deduplicated, local includes are not
sent to Compiler Explorer, failed compilers fail the survey, stale reports are
removed, and source/flag identity is verified. It uses non-rank batch mode:
`codegen_oracle.py --rank` deliberately exits zero on errors and is not a gate.

Corrected baseline/candidate inventory command, once per compiler binary:

```sh
python3 scripts/benchmark_codegen.py --gcc gcc --oracles '' \
  --local target/fastbuild/lccc --out results/pr824/static-candidate
```

Default flags are `-O2 -march=x86-64-v3 -fwrapv`, plus each workload's extra flags.
The recorded whole-TU totals were identical in both arms: **7,511 instructions,
856 classified loads, 656 stores, 657 explicit stack references, 386 vector
instructions**. These are static classifications, not allocator-spill counts,
retired-instruction counts, or native speedups. Full assembly equality is a
stronger observation than equality of those totals alone.

Other tools have deliberately different evidentiary scope:

- `callgrind_corpus.py` checks the exact registry/size-profile set, uses bounded
  inputs and wrapping arithmetic, and measures simulated whole-program Ir. It
  was read, not executed here: Valgrind is absent. Ir is not a runtime ranking.
- `perf_ab.py` provides controlled paired A/B timing or static-plus-behaviour
  experiments. Its configuration/flag/selection interfaces are not interchangeable
  with `tune_oracle.py`, which classifies tuning decisions, not speed.
- `codegen_oracle.py` is static codegen evidence; a local-only run is not a GCC,
  Clang, ICC or ICX comparison. No live CE compiler freshness claim is made.
- The ordinary hosted benchmark workflow owns repeated native VM timing and
  output comparison. Its result must not be substituted for bare-metal PMU or
  microarchitectural evidence.

## Compile-time experiment

An independent baseline compiler was built from `81f9930`, using the same Rust
1.99 fastbuild profile as the repaired compiler. A synthetic unsigned affine
address kernel was generated with 16/32/64/128 independent terms. Both compilers
and GCC produced identical executable output at `n=17`; baseline/repaired
assembly was also byte-identical for each size.

Each compiler was pinned to the same permitted CPU. One paired warm-up was
excluded, then six pairs alternated AB/BA. Whole-compiler wall-time medians:

| Affine terms | Baseline ms | Repaired ms | Repaired/baseline |
| ---: | ---: | ---: | ---: |
| 16 | 29.782 | 29.891 | 1.004 |
| 32 | 39.694 | 39.256 | 0.989 |
| 64 | 59.426 | 58.565 | 0.986 |
| 128 | 102.698 | 101.604 | 0.989 |

**No measurable speedup is claimed.** These differences are small, startup is
material, and this is a VM whole-pipeline screen, not an isolated IVSR benchmark
or an asymptotic proof. The indexed implementation removes identifiable repeated
scans, but these samples do not establish universal performance superiority.
Raw local evidence is retained under ignored `results/pr824/`; binaries and
large artifacts are outside Git. Hosted artifacts retain the independent run.

## Hosted exact-version validation and provisioning

The added Debian lane uses actual `debian:sid-slim` distribution packages,
checks `gcc-16 -dumpfullversion = 16.2.0`, installs Rust 1.99.0 including formatter
and linter, builds this checkout, and runs the entire canonical registry with
strict compile/run/output failure handling. It succeeded at `e7d397b`:
[run 38561921301](https://github.com/ms178/lccc/actions/runs/38561921301).
GCC 16.2 is therefore demonstrated **on hosted Debian**, not installed locally
in this restricted sandbox.

An empirical reporting issue was caught too: Actions retains only ten notices
per step, so publishing one per workload loses API-visible evidence. The follow-up
groups workloads into at most eight notices and includes a complete-registry
identity check. A real checks-API retrieval then exposed an independent 4096-byte
message truncation limit: most grouped full-detail JSON was cut off. Publication
now keeps each payload below 3900 bytes, retaining all 45 verdicts plus source,
output and command hashes and a whole-report hash. Full raw reports and
binary/disassembly artifacts remain uploaded independently. A new stress test
uses deliberately large outputs/commands to enforce both observed API limits.

GCC setup now has one ownership path, distribution-first selection, Ubuntu-only
PPA fallback, both multilib sets and development objects, installed-package
idempotence, optional exact-version enforcement, and propagated command failures.
Mock tests cover these paths without apt/root/network effects. Ordinary CI keeps
a **rolling major-16 policy**, not a claim of immutable package reproducibility.

## Still open — do not turn this evidence into a blanket approval

- **F8:** the nbody displacement-regex shape gate is still representation-dependent.
  Full output checks and a separate struct-walk gate do not prove a semantic
  mass-load count. A bounded, fail-closed address/loop-aware classifier with
  mutation controls remains needed; simply restoring an obsolete exact text
  count would be wrong.
- **F9:** IVSR indexing is implemented and checked, but the vectorizer's broader
  target-neutral plan extraction is not. A large speculative refactor was not
  mixed into these correctness repairs. The timing screen is limited as above.
- **F10:** GCC provisioning mocks are present; exhaustive swap-helper failure and
  root/sudo/idempotence mocks are not. Real swap activation and real hosted
  multilib execution are distinct evidence, not substitutes for every mock case.
- The peephole effect tables are not a complete formal model of every x86
  extension. Existing and added tests establish the exercised contracts, not
  correctness for every possible handwritten instruction sequence.
- Finite unit/corpus tests do not establish exhaustive input, target, ISA, ABI,
  alignment, masking, remainder or alias coverage. No production C miscompile
  newly reproduced outside the demonstrated fixtures is claimed.

The final required check statuses must be read on the current PR head. Earlier
successful runs, especially before evidence/harness changes, are not a green
status for a later head. Do not weaken or skip correctness gates to obtain green.
