# The PR #726 review, adjudicated: what testing found that reading did not

An external reviewer audited the merged S15 series (PR #726: the AVX-512 EVEX
fold, the folded-view segment decision, and the encdiff-corpus gate) and
returned six findings plus a five-item follow-up list. The reviewer had no
ability to run anything. This document is the other half: every finding was
re-derived by running code — the shipped canonicaliser against mutation
models, the built encoder through `encode_local`, GAS through both the host
2.44 (acceptance smoke) and the provisioned 2.47.20260726 (oracle) — and the
verdicts below are *confirmed with evidence*, *confirmed with a correction*,
or *rejected*, plus two defects the review did not find (§3).

## 0. Method

Re-based per standing instruction: the series sits on `af4971e9` (the merge
of PR #726), the latest `origin/main` at audit time. Baselines were
re-established before any edit: `test_encdiff.py` 43/43, `test_ci_gate_parity.py`
22/22, `check_ci_gate_parity.py` PASS (128 commands), corpus gate 194 rows
BEATS=166 ORACLE-INVALID=10 ok=18 (−519 B vs GAS 2.47). The toolchain and
oracle were re-provisioned after another harness wipe (rustup 1.99.0, GAS
2.47.20260726 via the atomic-extraction ensure script, sysroot32).

## 1. Verdicts on the six findings

| # | Review's claim | Verdict | What decided it | Disposition |
| --- | --- | --- | --- | --- |
| F1 | MAJOR: the encdiff corpus gate cannot detect the defect it was added to protect — `_SEG_DEAD64` unifies ds/es/ss with absent, so 8/10 segment rows are blind to the prefix byte | **AGREE (empirically)** | mutation models through the *shipped* `_canon_insn` and `decodes_same`: correct-ds / wrong-es / wrong-ss / no-prefix all compare equal; only cs/fs/gs differ | fixed three ways: 64-bit Rust pins (the review's remedy, extended), the corpus comment now states exactly what the gate verifies per row, and the F2 scoping below makes 8/10 rows gate-discriminating |
| F2 | MEDIUM: `_SEG_DEAD64` is global, not scoped — every 64-bit comparison loses ds/es/ss discrimination forever | **AGREE with the direction; the severity framing is rejected** | the law was semantically correct globally (dead bytes never change the program in 64-bit flat mode); the real cost is tripwire sensitivity, not comparison correctness | fixed with a better mechanism than proposed: the opt-in is derived **per row** from the row source (`_ROW_SEG_FOLD`: a segment override on an index-only scale-1 **rbp/ebp** operand — exactly the view-flip set), not from a corpus registry or CLI flag. Future corpora and ad-hoc `--insn` runs discriminate automatically; a fold row that legitimately needs the law matches it by shape |
| F3 | MEDIUM: the APX index fold on the AVX-512 EVEX path is an unpinned behaviour expansion — zero pins for a vector mnemonic with an EGPR index-only fold | **AGREE** | test-read confirmed zero pins; probes: lccc's fold output is **byte-identical to the base-form spelling** on every class probed (r16, r18, r20 rsp-class, r24 classic-B+B4, disp8 FVM compression, W=1, `{evex}` promoted) and identical to GAS's own base-form bytes | pinned: `index_fold_tests::fold_moves_the_egpr_index_through_avx512_evex` (10 asserts incl. the requested `(,%r16,1)` / `(,%r20,1)`, B4 set / X4 clear); the honest evidence basis (no folding oracle knows EGPR — justification is address equivalence + GAS base-form byte-identity) recorded in the test and the `evex_addr_bits` doc |
| F4 | LOW: dead guard and discarded clone in `folded_base` — `mem.base.is_none() &&` is unreachable-false, and a whole `MemoryOperand` is built to test `.is_some()` | **AGREE** | direct read of `core.rs` | fixed: `folds_index_into_base` predicate; `fold_index_into_base` delegates; `folded_base` calls the predicate. No behaviour change: cargo test 3999/0/7, corpus verdicts byte-identical |
| F5 | LOW: `assertEqual(count, 3)` then `for i in range(2)` — the third pin occurrence is never mutation-checked | **AGREE with the fact; the remedy was insufficient** | the loop's `range(2)` existed because `check_asmdiff_gate_parity` only parses asmdiff invocations — naively iterating 3 with that checker would fail occurrence 2 *falsely* | fixed by giving the encdiff occurrence a checker that owns it (see §3 F-A): the loop now mutation-checks all 3 occurrences, dispatching each to the parity checker for the differential program the occurrence's command runs |
| F6 | LOW: "EVEX P0 byte" contradicts Intel SDM numbering | **AGREE with the review's own dismissal** | the byte-naming matches the file's pre-existing convention (avx.rs "Byte 1: R X B R' B4 mmm", the P0/P1 pair in `evex_addr_bits`' doc) | no change — renaming two bytes across a 6.9k-line file would churn every comment for zero information gain |

## 2. Corrections to the review

The review was right that the law needed byte-level protection, but two of its
statements did not survive testing:

1. *"These four asserts are the only thing standing between the law and a
   silent regression."* Not quite: the 32-bit `movl` pins in
   `fold_decides_segment_elision_on_the_folded_view` already protect the
   elision **decision** — `folded_base` and the `rbp` name-match run
   identically for `movl %ds:(,%rbp,1)` — so a view regression was already
   caught by `cargo test` before this round. What was genuinely unpinned: the
   64-bit surface (REX.W + disp8), the never-a-default class through an
   extended base (`ss:8(,%r10,1)`), and the fs/gs fold rows. All pinned now.
2. The 2/10-protective tally is correct **under the law as shipped**. Under
   the scoped law the tally inverts: 8/10 rows compare their prefix byte in
   the gate (the three explicit-base twins, the two non-flip rax folds, the
   two inline-rendered fs/gs rows, and the r10 never-default fold); only the
   two rbp-flip rows are canon-blind — and they are *necessarily* blind: the
   law exists precisely because their folded bytes differ from GAS's by the
   dead byte. Those two are Rust-pinned.

## 3. What testing found that the review did not

**F-A (MEDIUM): the hosted encdiff command was enforced by nothing.** The
review's F5 looked at the local mutation loop only. The hosted side was worse:
`check_asmdiff_gate_parity` parses asmdiff invocations, path-level mirroring
only proves the script *path* appears on both sides, and the local block pin
never looked at the workflow — so a hosted-only edit of the encdiff step
(depinned `--as`, dropped `--offline`, a swapped corpus file, the wrong
compiler, or removing the step outright) passed every check in the tree. The
test comment even claimed the protection existed ("hosted CI must run the
identical `encdiff.py ... --as <pinned>` command, so a local depinning breaks
that equality") — it did not. Fixed: `check_encdiff_gate_parity` requires the
identical invocation (offline, quiet, pinned 2.47 oracle, x86-64 compiler,
both 64-bit law corpora, fast-gate registration) on BOTH mirrors; wired into
`main()`; mutation-tested from both sides (7 hosted mutations + local depin,
each rejected; dispatch verified non-vacuous — each occurrence is rejected by
exactly its owning checker).

**F-B (LOW): the corpus comment implied the gate verified the prefix bytes.**
Fixed alongside F1: the comment now states per row class what the gate
verifies (fold + length everywhere; the prefix byte everywhere except the two
rbp-flip rows) and where the flip rows' bytes are pinned.

## 4. Verification

On the final tree (all commits):

- `cargo test --profile fastbuild --lib`: **3999 passed / 0 failed / 7 ignored**
  (S15: 3998 — the new APX test adds one; the segment pins extend an
  existing test).
- `cargo fmt --all --check` clean; `cargo clippy --all-targets -j1 --
  -D warnings` clean.
- `python3 -m unittest scripts/test_encdiff.py`: **46/46** (43 + 3 new:
  the flip-set predicate, the classify threading, the decodes_same
  discrimination).
- `python3 scripts/test_ci_gate_parity.py`: **23/23** (22 + the hosted-side
  contract test).
- `python3 scripts/check_ci_gate_parity.py`: PASS, now including the encdiff
  corpus gate parity.
- Corpus gate (GAS 2.47.20260726, offline): **194 rows, BEATS=166,
  ORACLE-INVALID=10 (partitioned), ok=18, lccc −519 B** — verdict-identical
  to the S15 record; the scoping changed no verdict, exactly as designed
  (the strip was a no-op for every non-flip row's correct bytes).
- Probes: every new pin's bytes were produced by the built encoder
  (`encode_local`) and cross-checked against GAS's own base-form encodings;
  the APX×EVEX base forms additionally smoke-checked on host binutils 2.44
  before the 2.47 oracle run.

Slow gates (asm-diff whole corpus, remote-oracle encdiff, benchmark,
Callgrind) remain hosted-CI-only per policy.
