# The PR #731 review, adjudicated: what testing found that reading did not

An external reviewer audited the merged S18 series (PR #731: the pinned
oracle pair, the comment-stripped fold law, the verdict histogram, the
token-based owner dispatch, the EGPR/ebp corpus rows) and returned three
medium findings (`M1`–`M3`), five low findings (`L1`–`L5`) and an
overclaim ledger, rating the PR 8.0/10 with M1 ("the fix is scoped to 1
of 3 gates") as the one item that must not rot. The reviewer ran what
their harness allowed — zero-fuzz application, arithmetic checks, the
shipped `classify` with synthetic rows and a real objdump, pin-vs-corpus
bookkeeping — a materially better audit than reading, but without the
pinned toolchain and without a built `lccc`. This document is the repo's
half: every finding was re-derived here against the pinned binutils
2.47 pair and the built encoder, all six follow-ups landed, and the
letter of four of them was deliberately exceeded where the finding's own
logic demanded more than its remedy asked for.

## 0. Method

Re-based per standing instruction: the series sits on `be8b8569` (the
merge of PR #731 = the S18 series squashed as `2a45e806`, plus upstream
PR #730's corpus-tooling interpreter fixes). Baselines re-established
before any edit: `test_encdiff.py` 48/48, `test_ci_gate_parity.py`
23/23, `check_ci_gate_parity.py` PASS (132 commands), the encdiff
corpus gate **205 rows BEATS=174 ORACLE-INVALID=10 ok=21, lccc −555 B**
against the pinned as+objdump 2.47.20260726 pair, and both asm-diff
corpora green (1418 + 672 cases) with the unpinned objdump of the day.

The review's two headline quantitative claims were re-derived here
before accepting them: the betterok exposure (its 2,211 rows) and the
M2 laundering (its synthetic ds row) — both confirmed, with the exact
numbers recorded below.

## 1. Verdicts on the eight findings

| # | Review's claim | Verdict | What decided it | Disposition |
| --- | --- | --- | --- | --- |
| M1 | the objdump fix is scoped to 1 of 3 gates: `asmdiff.py:533` resolves objdump from `LCCC_OBJDUMP`/`PATH`, no `--objdump` flag exists, neither mirror passes one, and up to 2,211 rows in 7 `betterok` casefiles ride the runner image's binutils for their BETTER verdicts | **AGREE** | read the resolution site; counted with the real `load_cases` parser: **25 `allow_better` groups across 7 casefiles (5 x86-64 + 2 i686), ~2,236 instruction lines** — the review's 2,211 is the same class (its count predates PR #730's tooling edits) | **landed, exceeded**: `--objdump` flag mirroring encdiff's pattern, threaded into the `allow_better` path; both mirrors pin it on both asm-diff gates; the parity checker requires it on both sides for both modes; the parity tests owner-dispatch-mutate all three `--objdump` pins (non-vacuity triple) and depin both hosted gates two ways each. **Beyond the letter**: the audit's own verdict text names a narrow false-pass the instruction did not ask to fix — `semantically_equal` accepted `(bad)`/`.byte` renderings, and objdump prints `(bad)` WITHOUT the bytes, so two different garbage streams can render to identical text and launder a size-only BETTER. Closed (undecodable = no semantic evidence, encdiff's `decodes_same` discipline), verified non-breaking: both corpora re-run green against the pinned pair (1418 + 672, 0 failed) |
| M2 | the four new ebp flip-law rows cannot falsify the segment-byte policy: `_SEG_DEAD64` strips ES/DS/SS on both sides for opted-in rows, so the distinguishing byte is definitionally invisible — a dropped `3e` on the ds flip row still classifies BEATS with "round-trip verified" | **AGREE** | reproduced here against the REAL pinned pair before any fix: GAS 2.47's raw SIB view of `mov %ds:4(,%ebp,1), %rax` is `67 48 8b 04 2d 04 00 00 00` (9 B, ds elided — no base, DS default), and both the correct fold (`3e 67 48 8b 45 04`, 6 B) and the regressed fold (`67 48 8b 45 04`, 5 B) classify **BEATS** through the shipped `classify` | **landed, exceeded**: new `# byte-exact <hex>` row annotation, stripped from the assembled source and enforced in `classify()` BEFORE any law, partitioning or round-trip — nothing downstream can launder a pinned byte difference. A malformed annotation is a hard error, not prose. Annotated the **complete flip set** `{rbp, ebp} × {ss, ds}` plus the ebp base twins (the audit asked for the ebp quad; the two rbp flip rows carry the identical blind spot). Verified in BOTH directions against the real encoder: the corpus is green with the pins live, and a tampered pin (weakened to the 3e-less bytes) fails the gate WRONG-BYTES. Prose in the corpus and in encoder `mod.rs` now states precisely what the BEATS verdicts prove (addressing-fold equivalence, disp economy) and what the annotations prove (the segment byte itself) |
| M3 | 7 of 10 EGPR pins are corpus-recorded; missing `(%r20)`, `(%r24)` (the SIB-escape base twins) and `16(,%r16,1)` (the positive compressed-disp8 sign) | **AGREE** | extracted the pins from `fold_moves_the_egpr_index_through_avx512_evex` programmatically: exactly the 7/10 split the review names | **landed, exceeded**: the three rows appended (verdicts vs the pinned pair: ok, ok, BEATS — the review's 175/23/10 prediction held exactly), **all ten** EGPR rows now carry `# byte-exact` annotations (the fold's byte-identity to the base-form spelling — the law's step-(b) correctness argument — becomes part of the CI record, not a Rust-only pin), and the 10/10 coverage is a committed test contract: `test_encdiff.py` extracts the pins from `mod.rs` and requires a corpus annotation with equal bytes for each, so a new pin without its corpus row fails the suite. Baseline re-recorded in the same commit from the gate's own drift table: **208 rows, BEATS=175, ORACLE-INVALID=10, ok=23, −559 B** |
| L1 | the histogram is a net-count contract: a compensating delete+add of same-verdict rows nets to zero and is invisible; "a deleted row forces a baseline update" was true only net | **AGREE** | read `check_verdict_histogram`; the comparison is the nonzero-projection dict equality | **landed** (the review's "better" option): `rows_digest()` — sha256 over the sorted comment-stripped row texts, each carrying its byte-exact pin — recorded as a `# rows-sha256:` line the baseline MUST carry and the gate verifies. Falsified end-to-end at the gate level: a corpus with one BEATS row swapped for a different BEATS row (counts identical, 208/175/10/23) **exits 1 on the digest alone**. Verdict-only changes keep the digest (BEATS staying BEATS through better bytes still needs no re-record). The baseline header and the mirrors' contract comments now make the stronger, now-true claim |
| L2 | `--expect-histogram` undocumented in `scripts/README.md` | **AGREE** | `rg` — the patch that introduced it had documented the CLI oracle flags but not the baseline flag or its update rule | **landed**: new "The corpus-gate contracts (encdiff)" README section (pinned pair, counts-only semantics, nonzero projection, same-commit re-record rule, the rows digest, the `# byte-exact` annotation and its malformed-is-an-error grammar), plus the pair discipline stated for `asmdiff.py` and its table row |
| L3 | `ensure_gas_247.sh` validates presence, not version: a stale 2.46 pair under the 2.47-named prefix satisfies the guard, contradicting the gate's premise | **AGREE** | read the fast path: `-x` on both binaries only | **landed**: the fast path re-derives what it is about to trust — `--version`'s first line of BOTH binaries must say 2.47 — with the invariant stated in place ("the prefix name is not the version"). Falsified offline: a planted 2.46 pair is rejected (proceeds to provision, fails loudly with no mirror reachable), an as-only cache is rejected, the real 2.47.20260726 pair still short-circuits printing both versions |
| L4 | the drift printer fires its "update the baseline" hint even when the baseline file is missing/unparseable, where the advice is misleading | **PARTIAL — an adjacent real defect, imprecisely described** | reproduced each leg: a MISSING baseline was an unhandled `FileNotFoundError` traceback (no hint at all — arguably worse); the unparseable-line branch returns before any hint; the misleading hint fired only for an EMPTY baseline (parses to no counts, mismatches, prints drift + update advice) | **landed**: every failure mode now gets advice accurate to itself — missing file ("a missing baseline is not drift — create it…"), empty baseline ("an empty baseline is not a contract"), digest-less baseline (add this line, with the actual digest printed), and "update the baseline" only fires on genuine drift. All four legs pinned by tests |
| L5 | the adjudication doc mixes reproduced-with-pinned-toolchain claims and soft citations in one list | **AGREE** | read the S18 doc's §4 | **landed by construction**: this document separates them (§3 hard, §4 soft) |

**The overclaim ledger** — all three entries confirmed, all three now
closed at the artifact level, not the prose level: (a) "the EGPR
evidence is oracle-recorded" is now true for the bytes as well as the
addressing fold (F-3's annotations; 10/10 by committed test); (b) the
flip-law rows now prove the segment byte too (F-2's pins, checked
before any law); (c) "a deleted row forces a baseline update" is now
true of row identity, not just net counts (F-6's digest).

## 2. Where the review under-asked

The review's remedies were scoped to its findings; four places demanded
more than the letter:

- **The rbp flip rows carry the identical M2 blind spot.** The audit
  asked to annotate "the four ebp rows"; the laundering demo it ran
  works verbatim on `mov %ds:4(,%rbp,1), %rax`. The complete flip set —
  both halves, both segments — is annotated.
- **M1's false-pass was diagnosed but not remedied.** The verdict text
  proves the `(bad)`/`.byte` same-rendering hole in
  `semantically_equal`; the F-1 instruction stops at the flag. The hole
  is closed here, with the both-corpora re-run proving no legitimate
  betterok case rests on undecodable renderings.
- **10/10 coverage as a one-time audit is 10/10 until the next pin.**
  The review asked for a re-run of the pin-vs-corpus audit and a
  recorded sentence; the coverage is instead a committed test that
  fails when a new `mod.rs` pin lands without its corpus row (and when
  the two records drift apart in either direction).
- **A weakened pin is a compensating swap at the annotation level.**
  The digest deliberately covers each row's byte-exact pin, so editing
  `3e67488b4504` down to `67488b4504` in the corpus changes the digest
  and forces a conscious baseline re-record — the L1 discipline applied
  to F-2's own mechanism.

## 3. Reproduced against the pinned toolchain (hard evidence)

Everything in this section was run here, against
`~/.cache/gas-2.47-x86_64-linux-gnu/bin/{as,objdump}`
(2.47.20260726, the same build) and `target/fastbuild/lccc-x86`:

- **M1 exposure quantified with the real parser**: 25 `allow_better`
  groups across 7 casefiles (`apx`, `avx`, `lea`, `misc`, `modrm`,
  `i686/addressing`, `i686/basic`), ~2,236 instruction lines.
- **M2 reproduced pre-fix**: with GAS's real 9-byte raw view as the
  oracle, the 3e-dropped fold classifies BEATS through the shipped
  `classify`; post-fix the same row is WRONG-BYTES with the pin
  violation in the note (pinned by `test_encdiff.py`).
- **M3 gap verdicts measured, not predicted**: GAS's own bytes for the
  three gap rows — `(%r20)` → `62 f9 7f 08 6f 04 24`, `(%r24)` →
  `62 d9 7f 08 6f 00`, `16(,%r16,1)` → the 11-byte SIB+disp32 form —
  and the corpus gate's own drift table for the re-record:
  **208 rows, BEATS=175, ORACLE-INVALID=10, ok=23, −559 B**.
- **Both asm-diff corpora green under the new flag and the
  undecodable-rendering refusal**: x86-64 1418/0, i686 672/0.
- **Corpus gate green with all 16 byte-exact pins live** (6 flip-set
  rows + 10 EGPR rows) and the digest baseline accepted; tamper tests
  exit 1 in both directions (weakened pin; compensating row swap with
  identical counts).
- **ensure_gas fast-path falsification**: planted 2.46 pair → rejected
  (exit 1 at the unreachable-mirror stage, not exit 0 at the guard);
  as-only cache → rejected; real pair → exit 0 with both versions.
- `test_encdiff.py` **55/55**, `test_ci_gate_parity.py` **23/23**,
  `check_ci_gate_parity.py` **PASS (132 commands)**, `rg` audit: three
  pinned `--objdump` occurrences per mirror, zero bare fallbacks inside
  gate commands.
- `cargo check --profile fastbuild --lib` clean; `cargo fmt --all
  --check` clean; `cargo clippy --lib -- -D warnings` clean; the Rust
  unit suite compiled FRESH on this box and ran green through the
  repo's own OOM recipe — **4011 passed / 0 failed / 7 ignored** on the
  full `--all-targets` battery leg (upstream #730 added 11 tests to
  the 4000 of the pre-series tree).

## 4. Soft citations (stated, not re-derived here)

- The **fresh-compile** line above needs its recipe stated plainly:
  bare `cargo test` invocations on this 4 GB box OOM (three attempts
  died at 2.6–3.5 GB anon RSS — the link of the monolithic lib-test
  binary spikes past what the cgroup leaves free). The repo already
  knows: `ci_local.sh`'s cargo-test leg applies `debug=0` +
  `incremental=0` + `-j1` + `-Wl,--no-keep-memory` on small hosts
  (the flags `build_lccc_fast.sh` publishes through
  `target/lccc-rustflags`), and under exactly that recipe the full
  suite compiles and passes here (429 s). My bare invocations simply
  hadn't used it.
- `cargo clippy --all-targets` OOMs on this box for the same reason
  (its `--test` leg compiles with `debuginfo=2` + incremental); the
  lib leg re-ran clean here, the Rust test code is untouched by this
  series (all test changes are Python), and the pre-series
  `--all-targets` run was green.
- Hosted CI legs (the workflow's own runs of the new pinned commands)
  are beyond this harness; the parity checker and its mutation tests
  are the local proxy for them, per the standing gate-parity contract.
- The review's own harness results (zero-fuzz application, its
  arithmetic, its synthetic-row demonstrations) are its claims; the
  ones this document relies on were independently re-derived (§3).

## 5. Verification summary (final tree)

`test_encdiff.py` 55/55 · `test_ci_gate_parity.py` 23/23 ·
`check_ci_gate_parity.py` PASS (132 commands) · encdiff corpus gate
green vs the pinned pair, 208 rows BEATS=175 ORACLE-INVALID=10 ok=23,
−559 B, digest-verified · x86-64 asm-diff 1418/0 and i686 asm-diff
672/0 under the pinned pair and the undecodable refusal · pin coverage
10/10 by committed test · zero bare-objdump gate commands on either
mirror · `ci_local.sh --fast` per the delivery record.
