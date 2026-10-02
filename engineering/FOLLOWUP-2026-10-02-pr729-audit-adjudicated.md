# The PR #729 review, adjudicated: what testing found that reading did not

An external reviewer audited the merged S17 series (PR #729: the row-scoped
dead-segment law, the APX×AVX-512 EVEX fold pins, the both-sides encdiff
gate parity) and returned seven findings (`D1`–`D7`) plus a six-item
follow-up list (`F-1`…`F-6`), rating the PR 8.5/10 with the APX evidence
independence as the 10/10 blocker. The reviewer re-derived the PR's claims
empirically in their own harness — a materially better audit than reading —
but still without the ability to run the pinned toolchain. This document is
the repo's half: every finding was re-derived here against the *pinned*
binutils 2.47 pair, all six follow-ups landed, and two defects were found
that the review did not have.

## 0. Method

Re-based per standing instruction: the series sits on `f0124632` (the merge
of PR #729; its content is the S17 series squashed as `3bc6b7c3`, plus
upstream's PR #728 vectorizer work). Baselines re-established before any
edit: `test_encdiff.py` 46/46, `test_ci_gate_parity.py` 23/23,
`check_ci_gate_parity.py` PASS, corpus gate 194 rows BEATS=166
ORACLE-INVALID=10 ok=18 (−519 B) against the pinned 2.47 pair. The
objdump half of the pair was built from the same 2.47 tree this session
(`make` resumed; `binutils/objdump` installed beside `gas/as-new`).

## 1. Verdicts on the seven findings

| # | Review's claim | Verdict | What decided it | Disposition |
| --- | --- | --- | --- | --- |
| D1 | the 10 new APX pins have no independent oracle — lccc==lccc twins plus hard-coded bytes; all three corpora contain zero EGPR rows; the GAS byte-identity was a one-time manual probe CI cannot reproduce | **AGREE** | `rg` over the corpora: zero r16–r31 rows; the pins are fold-vs-base twins + literals; the "probed" evidence lived only in prose | F-1: the EGPR rows now run in the corpus gate against the pinned as+objdump pair on every push (fold rows BEATS over GAS's 11-byte SIB forms, round-trip verified; base twin ok); the mitigating fact the review itself verified — `registers.rs` untouched, the EGPR fold predates #729 — re-confirmed (`git diff af4971e9 3bc6b7c3 --stat -- src/backend/x86/registers.rs` is empty) |
| D2 | the disassembler oracle is unpinned while the PR makes the encdiff invocation a byte-truth contract: `_OBJDUMP` is env-default `objdump`, nothing ever pins it, and `decides_same` — the BEATS/ok authority — runs on whatever the runner image ships | **AGREE** | `rg LCCC_OBJDUMP` across scripts/.github: never set; the exact unpinned-tool class the 2.47 `as` pin exists to prevent | F-1: `--objdump` CLI flag (mirroring the `--objcopy` precedent — a CLI flag, not an env prefix, so the parity parser sees it); `ensure_gas_247.sh` installs the PAIR from one build and its early-exit guard now requires BOTH binaries (an as-only cache is an interrupted provision, not a working oracle); both mirrors and `check_encdiff_gate_parity` enforce the pin |
| D3 | latent over-match: corpus rows are read comment-inclusive, `classify` matches `_ROW_SEG_FOLD` on the raw row, so a trailing comment quoting a flip-shaped operand opts a non-flip row into the strip | **AGREE — empirically, with a sharpening** | the first reproduction attempt FAILED to match: the regex requires the `%`-prefixed spelling, and a comment writing `ds:4(,...)` bare does not match. The realistic case — a comment quoting the row spelling exactly as this corpus's own documentation style does (`# unlike %ds:4(,%rbp,1), ...`) — matches, and `decodes_same(seg_dead64=True)` then launders a dropped-prefix regression into a pass. Latent (no instruction line in the corpora carries a trailing comment today) but real, and the dangerous direction | F-2: `classify` matches on `row.insn.split("#")[0]`, exactly like `_canon_insn`; the test asserts the regex DOES match the raw adversarial line (non-vacuous) and that classify threads `seg_dead64=False` for it while the real flip row still opts in |
| D4 | the occurrence-owner dispatch is a substring `rfind` that also matches `test_encdiff.py`; correct today only because the asmdiff command line sits closer — reordering `ci_local.sh` flips it | **AGREE** | read + the review's own empirical dispatch trace | F-3: token-based — the owner is the last `python3 scripts/(enc|asm)diff.py` program token before the pin, asserted non-empty and one of the two drivers; a `test_*` gate can never own an `--as` pin |
| D5 | the engineering doc's "rejected by exactly its owning checker" was verified only in an ad-hoc harness; the committed test asserts one direction | **AGREE** | the test asserted only `checker(mutated) == 1` | F-4: the test now asserts the full non-vacuity triple — owning checker rejects the mutation (1), the NON-owning checker accepts it (0, proving the dispatch is load-bearing), and both checkers pass the un-mutated tree (0, proving the 0-legs aren't vacuous) |
| D6 | no aggregate verdict baseline: the exit contract counts only WRONG-BYTES/UNVERIFIED-*/REJECTS-VALID/LONGER, so BEATS→ok-best drift is invisible; "verdict-identical to the S15 record" was a manual claim | **AGREE** | the exit block counts exactly those five classes | F-5: `--expect-histogram` + the checked-in `expected-verdicts.txt` (counts only, all 15 verdict classes pinned including zeros); the gate fails on any count change — drift, new rows, deleted rows, a class appearing from zero — until the baseline is consciously re-recorded. Wired into both mirrors and the parity contract |
| D7 | the ebp half of the claimed flip set {rbp, ebp} is unpinned — the 64-bit pins cover only rbp | **AGREE — and understated** | `rg ebp` in the pin region: zero; the 32-bit pins use rax/eax indexes, so an ebp INDEX was unpinned in BOTH modes, not just 64-bit | F-6: the 64-bit ebp quad pinned (ss/ds fold + base twins, the 0x67 addr32 form — `67 48 8b 45 04` / `3e 67 48 8b 45 04`, GAS-2.47-cross-checked); the ebp flip rows + base twins added to the corpus (they exercise the ebp half of `_ROW_SEG_FOLD` end-to-end: BEATS over GAS's 9–10-byte raw-view SIB forms, exactly the rbp pattern) |

## 2. Where the review was right about the remedy, and where it under-asked

- The review asked for "one ebp flip row" in the corpus and one Rust pin.
  The corpus got the full ebp QUAD (ss/ds fold + ss/ds base twins) — the
  same twin structure the rbp family has, so the fold is proven by
  byte-identity to the base spelling, not just by a shorter-vs-GAS verdict.
- The review asked for 7 EGPR corpus rows; the fold rows land BEATS and the
  base twin ok, exactly as it predicted (`205 rows: BEATS=174
  ORACLE-INVALID=10 ok=21, −555 B`, from 194/166/10/18/−519).
- The review's "Do not" list (no CLI-flag law, no skipping EGPR rows on
  undecodable oracles, no env-prefixed gate command, no loosening
  `_ROW_SEG_FOLD` to any-segment+fold) was followed in full.

## 3. What testing found that the review did not

**F-A (caught before commit): the histogram comparison's zero-projection
trap.** The first implementation compared `actual == expected` as raw dict
equality — and the checked-in baseline deliberately lists zero-count
classes the actual dict has no key for, so the CORRECT baseline failed the
gate on its first run. Fixed by comparing through the nonzero projection on
both sides (a class recorded at 0 is documentation; anything appearing from
zero still mismatches). The committed test pins this exact semantics
(equal-with-zeros passes; drift, from-zero, and unparseable baselines fail).

**F-B (design note, no change): `--objcopy` stays unpinned deliberately.**
The review's D2 logic ("the runner image's objdump is not an oracle")
applies to `as` (source of the reference bytes) and `objdump` (arbiter of
the BEATS/ok verdicts). `objcopy -O binary --only-section=.text` is a
byte-mechanical section dump with no version-sensitive semantics in this
pipeline — pinning it would grow the contract without protecting a truth
decision. Recorded here so the asymmetry is a decision, not an oversight.

**F-C (bookkeeping): the corpus's opt-in count is now four rows, not two.**
The review measured "exactly 2 opt in" (the rbp pair). The ebp flip rows
added by F-6 legitimately opt in too — 2 rbp + 2 ebp = the complete flip
set the law's comment always claimed ({rbp, ebp}). The 8 formerly blind
segment rows remain fully discriminating; the count change is the ebp half
of the law finally being exercised rather than asserted.

## 4. Verification (final tree)

- `python3 -m unittest scripts/test_encdiff.py`: **48/48** (46 + the
  comment-strip law + the histogram semantics).
- `python3 scripts/test_ci_gate_parity.py`: **23/23** — with the
  occurrence loop now mutation-checking all three `--as` pins AND the
  `--objdump` pin, the full non-vacuity triple, and ten hosted-side
  mutations + three local-side mutations for the extended encdiff
  contract.
- `python3 scripts/check_ci_gate_parity.py`: PASS — the encdiff contract
  now requires the pinned 2.47 pair and the histogram baseline on BOTH
  mirrors.
- Corpus gate vs the pinned as+objdump 2.47.20260726 pair:
  **205 rows, BEATS=174, ORACLE-INVALID=10 (partitioned), ok=21,
  lccc −555 B** — the 11 new rows all landed their predicted verdicts
  (8 BEATS: six EGPR folds + two ebp flips; 3 ok: the base twins).
  Baseline drift is gate-fatal: a BEATS-174/173 tamper exits 1.
- Every new pin's bytes: produced by the built encoder via `encode_local`
  and cross-checked against GAS 2.47's own encodings (the ebp base twins
  are byte-identical to GAS's; the fold rows are GAS's 9–11-byte SIB
  forms against lccc's 6-byte folds).
- `cargo test --profile fastbuild --lib`, `cargo fmt --all --check`,
  `cargo clippy --all-targets -j1 -- -D warnings`: see the delivery
  record (all green; the counts are in the S18 worklog).
- Slow gates (asm-diff whole corpus, remote-oracle encdiff, benchmark,
  Callgrind) remain hosted-CI-only per policy. The review's merge gate
  ("Test Suite green") is hosted evidence; this record covers everything
  reproducible against the pinned toolchain locally.
