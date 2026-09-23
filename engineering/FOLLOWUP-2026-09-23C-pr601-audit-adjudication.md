# Follow-up S09 — external Review-AI audit of PR #601 (F1–F8): full adjudication

Session date: 2026-09-23 (second turn of the day). Base: upstream `93596224`
(PR #600 re-push; main unchanged). Start of turn: head `efb71616` (row 13),
working tree broken mid-F7-fix. End of turn: audit fully adjudicated, all
repairs landed and validated.

## Verdicts on the external audit

The reviewer has no testing capability (zero compile tests were run); every
claim was verified here failing-first before acceptance. Result: **the review
is vindicated on every confirmed finding.** My prior S08 in-house red-team
audit (REDAUDIT Part 2) missed F1/F2/F3/F4/F7; where the review and my doc
conflicted, the review won and this document corrects the record.

| # | Claim | Verdict | Evidence |
|---|-------|---------|----------|
| F1 | Pair fold substitutes extension DESTINATIONS (stale bytes), falls through to dest substitution for non-base-only memory, ignores order dependency | **CONFIRMED, fixed** | 13 failing-first tests; rewrite substitutes extension SOURCES with dependency guards |
| F2 | js/jns/sets/setns in the sext allowlist | **CONFIRMED, fixed** | 2^16 exhaustive model: SF not width-invariant (127−(−1): SF=0@32, SF=1@8) |
| F2b | (new, found here) jo/jno/seto/setno also not sext-invariant | **CONFIRMED, fixed** | Model: 0−(−128): OF=1@8, OF=0@32 — caught during repair, before shipping |
| F3 | `dead_after(first reader)` does not prove no-use; store/copy/addr-use between AND and reader breaks the deletion | **CONFIRMED, fixed** | Span scan [i+1, r] (r included when r is not a flags reader); mid-test-aware window start |
| F4 | fold 1 and (pre-existing) `eliminate_redundant_test_i686` alias %ah with %al via `register_family` | **CONFIRMED, fixed** | Verbatim-destination-text law in both passes; upstream's own older pass had the same bug the review's argument predicted |
| F5 | asm-path freshness gap | **CONFIRMED, fixed** | `emit_inline_asm`/`_with_segs` never noted clobbers — now evict homes via `home_phys_by_asm_name`; 5 fail-closed tests (round-trip, sibling law, class isolation, liveness gate) |
| F6.1 | Vacuous test asserts (bless `cmpw %ax,%di`; unobservable `movzwl 24(%esi)` assert) | **CONFIRMED, fixed** | All pair-fold tests now assert source-substituted operands with role-order justification comments |
| F6.2 | Regalloc fixture vacuous when `phi_chain` empty | **CONFIRMED, fixed** | `phi_chain_published_for_applied_coalesce` asserts nonempty, v10→7 published, rep-not-key invariant, determinism (an empty RA-3 publication now fails the suite) |
| F6.3 | Tests bless wrong output | **CONFIRMED, fixed** | 8 stale expectations updated WITH justification comments (never output-matched silently) |
| F7 | `is_barrier`/flag-window/next_instruction treat ALL directives as transparent (`.byte 0x9e` ≡ sahf) | **CONFIRMED, fixed** | Whitelist `.cfi*/.loc/.file/.line/.align/.p2align` via `directive_is_transparent`; opaque directives fail windows and folds exactly like inline asm |
| F8 | Snapshot script prints wrong paths; cross-fs `mv` breaks atomicity; bundle published into the wipe-excluded zone the restore script reads it from | **CONFIRMED, fixed** | mktemp in dest dir; bundle → durable `artifacts/` (the .git-recovery artifact); summary prints real paths; restore gains a legacy-path fallback |

Rejected: nothing — every claim survived empirical verification. Points where
this round went beyond the review: the OF (not just SF) non-invariance for
sext pairs; the same F4 aliasing in `eliminate_redundant_test_i686`
(upstream's own pass, predates this fork's work); corner (a)
(`andl; je; jmp; .Lx: js`) verified sound and pinned.

## The repair set (what landed)

1. **F7**: `directive_is_transparent(&str)` whitelist; `next_instruction`
   (signature now `(store, infos, idx)`) and `prev_instruction` skip only
   transparent directives; `flags_reader_window_ok` splits `Directive` out of
   the flag-blind catch-all and fails opaque ones.
2. **F1**: pair fold operand law rewritten — each compare side contributes the
   extension's SOURCE (`%al`/`%bx` verbatim identity; memory base-only forms
   re-read verbatim); refusal rules: high-byte/esp/ebp sources, complex
   addressing (never falls through to dest substitution), side-i source
   depending on ext-k's write (order dependency), REG_NONE families,
   two-memory compares. Liveness cache invalidated on every rewrite.
3. **F2**: `PAIR_JCC_SIGNED_SAFE`/`PAIR_SETCC_SIGNED_SAFE` = exactly the
   flag-EXPRESSION-invariant set {je jne jp jnp jb jae jbe ja jl jge jle jg}
   (SF and OF rows removed with the counterexamples in the doc comment);
   zext sets unchanged. Window oracle: exact first-token matching (`jecxz`
   no longer prefix-matches `je`; lists trim-tolerant).
4. **F3**: value-deadness law — every register reference in [i+1, r] must
   belong to the mid zero-test the pass itself deletes; r included when r is
   not the first flags reader; no-reader arm starts the window scan after the
   deleted mid test (a Cmp would otherwise end the window early and hide
   readers).
5. **F4**: verbatim byte identity in fold 1 (`tj == format!("{mnem} {dst_text}, {dst_text}")`)
   and in `eliminate_redundant_test_i686` (both parsers return destination
   TEXT; family equality never crosses bytes).
6. **F5**: `home_phys_by_asm_name` reverse table (homes only; width aliases
   resolve to the same physical home); both inline-asm emitters evict named
   homes after the asm block. Tests: full round-trip vs `phys_reg_name`
   (incl. EGPRs), scratch/pseudo-clobbers → None, sibling/class/liveness laws.
7. **F6.2**: RA fixture strengthened to positive evidence (see verdicts).
8. **F8**: snapshot/restore path agreement (see verdicts).
9. **Pins added this round**: 27 new tests total across peephole.rs
   (counterexamples + corrected laws + corner (a)) and emit.rs (5).

## Verification ledger (this session)

- Unit: `cargo test --lib` **3262 passed / 0 failed** / 7 ignored
  (3237 + 27 new − reworked). fmt clean, clippy silent.
- Correctness suite: **57/0**. m32 differential fuzz: **357/0**;
  alias fuzz 59/0; slot-RMW fuzz **177/0**; ALU torture MATCH.
- Kernel: full build EXIT=0, QEMU boot **16/16 ALL CHECKS PASSED**.
- Census: lccc **5021** insns vs gcc 3183; all-three-passes-off **5032**
  (−11 = the passes' remaining sound win; the audit removed the unsound part
  of the previous −25). Boot size TOTAL 21613 / 12598.
- x86-64 corpus: 853 units, **byte-identical asm, 0 status diffs** (F5's
  asm-path is the only x86-64-visible change and the corpus has no inline
  asm in that position).
- **m32 corpus A/B vs the pre-fix binary**: 853 units, 54 asm diffs — all
  47 runnable diff units behaviorally 3-way checked: the PRE-fix binary
  diverges from GCC on **28** (incl. one SIGSEGV, `arm_gep_regbase_acc_cache`,
  and wrong exit codes on `expat_xml_scan`, `sqlite_varint`,
  `bool_thread_fse_tail_overrun`, …); the POST-fix binary diverges on **1** —
  the pre-existing `glibc_memcmp.c` LP64-assumption case (identical on both
  binaries, logged below).
- ci_local `--fast`: see ledger row 14 comment (green at commit time).

## Open items carried forward

- **glibc_memcmp.c -m32** (pre-existing, both binaries): `check_kernel`
  takes the `return 2` path under lccc while gcc prints a checksum — the
  test program assumes LP64 byte ordering in its bytewise sign extraction;
  needs a dedicated reduced repro to assign (compiler vs test-program bug).
- RA roadmap unchanged: RA-3 already satisfied (`phi_chain` published;
  now positively tested), RA-5, F-1 leaf prologue, branchless-select/cmov.
