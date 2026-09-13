# Flag-consumer reachability, a truncated i686 scan, and the PR #512 audit adjudicated

**Date:** 2026-09-13
**Base:** main `fad79262`
**Commits:** `3a4cb6e6` (x86 OF audit), `2e1d9cdb` (x86 flag walk), `480cefc5` (i686 scan
bound), `058e7db7` (docs/gitignore)
**Scope:** soundness of flag reasoning in the x86-64 and i686 peepholes, plus the verdict on
the Review AI's audit of the CFG frame-slot forwarder.

---

## 1. Why this document exists

The audit of PR #512 found four real defects in the CFG frame-slot forwarder. Its findings were
mostly right; two of its *justifications and remedies* were wrong, and both errors were of a
kind that only execution catches. Adjudicating it line by line (§2) and then asking what its
failure mode generalises to (§3) produced three new soundness holes in flag reasoning — two in
x86-64, one in i686 — all reproduced on unmodified main before being fixed.

The failure mode is one thing, repeated: **a structural assumption standing in for a verified
effect.** A worklist assumed ready-gating implies convergence. A successor list assumed "no
target parsed" implies "no successors". A write check assumed "recognised mnemonic" implies
"all writes seen". An escape rule assumed "positive displacement" implies "caller-owned". A
consumer scan assumed "Push kind" implies "flag-neutral", and "no reader in 16 lines" implies
"no reader".

## 2. Verdict on the PR #512 audit

| Item | Verdict | Why |
|---|---|---|
| F1 loop-carried facts never forwarded | **Agree** (finding), **refine** (mechanism) | Not a starved worklist: a *must* analysis cannot bootstrap from bottom. `IN[head] = OUT[pre] ∩ OUT[head]` has two solutions, `∅` and the fact, and upward iteration reaches only `∅`. A ready-gated requeue alone fixes nothing; the seed has to be optimistic (RPO over non-back predecessors) and then iterate down. Shipped that way; pinned by `cfg_fwd_loop_carried_store_is_forwarded`, `…back_edge_redef_stops_forwarding`, `…post_loop_block_forwards`. |
| I-1.2 "the transfer is monotone" | **Disagree — false** | The state-restricted implicit-write probe is *anti*-monotone: adding a fact could remove one, so no least fixpoint was available to iterate to. Fixed by making `source_regs` function-level (`peephole.rs:3637`), i.e. by removing the state dependence rather than by trusting the claim. The audit reached a correct conclusion by a route that would have produced an oscillating pass. |
| F2 `JmpIndirect` had no successors | **Agree** | Shipped as the audit prescribed: `succ[b].extend(0..nb)`, over-approximate, the meet conservatises. Pinned three ways, including a positive control that switch forwarding *survives* when all paths agree. |
| F3 string ops were invisible writes | **Agree** | Shipped as an **enumerated** 28-entry table, not a prefix test: prefix-matching `movs` false-matches `movsbl`/`movsbw`, which write no memory through an implicit pointer. Wired into the shared classifier so the windowed pass sees it too. |
| F4 escape analysis too relaxed | **Agree** (finding), **reject** (remedy) | The prescribed blanket kill on any indirect write costs **166 forwarded loads** against the precise variant, because "positive displacement ⇒ caller-owned" is true for `%ebp` and false for `%esp`: with `ESP_SLOT_BIAS` the function's own locals *are* the positive displacements. Shipped `drop_caller_owned_slots` (`retain(|e| e.slot >= ESP_SLOT_BIAS || e.slot <= 0)`), pinned by a refusal test **and** a positive control that own-frame slots survive. |
| F5 documentation drift | **Agree** | All reconciled but one, found today: §1 bullet 3 of the 2026-09-12 follow-up still claimed "no fixpoint iteration over the CFG" while §10.2 of the same file describes the seed, the worklist and the application sweep. Fixed in `058e7db7`. |
| I-5 six demanded pins | **Agree** | All six present, plus four more: 35 `cfg_fwd_*` tests where there were 25. Two of the additions are positive controls, which are the tests that fail if someone restores soundness by deleting the optimisation. |

**What the audit's lack of a test capability cost it:** it could not price its own F4 remedy
(−166 loads), it asserted monotonicity without running the transfer, and it could not check
that a proposed pin actually fails without the fix. The third one bites even with a test
capability: two candidate pins written for §3 passed for the wrong reason until reshaped (§6).

## 3. Three new holes of the same species

### 3.1 x86-64: `pushf` is a flag reader wearing a stack adjustment's clothes

`pushf*` is classified `LineKind::Push { reg: REG_NONE }`, deliberately, so `%rsp` tracking
stays correct — the classifier records that treating it as `Other` once disabled the whole
global peephole for files like gzip's `deflate.c`. But
`flag_consumers_are_zf_only` skipped `Push`/`Pop` kinds as *"stack adjustments do not touch
EFLAGS"* **before** consulting `flags_effect`, which lists `pushf` among the flag readers. An
instruction that reads every flag was invisible to the scan that decides what may be rewritten
under it. Emitted by unmodified main:

```
    testb $128, %bl        <- was: movl %ebx,%esi ; andl $128,%esi
    pushfq                 <- captures SF as data
    popq %r12
    je .LBB5
    xorl %eax, %eax
    ret
.LBB5:
    shrb $7, %r12b         <- returns the captured SF
    movzbl %r12b, %eax
    ret
```

`testb $128,%bl` puts bit 7 of the masked value in SF; `andl $128,%esi` puts bit 31 there,
which is 0 for every mask ≤ 255. The divergence leaves through a register, so no flag reader in
the block reveals it, and the function *is* flag-block-local, so the block-locality invariant
does not catch it either.

The evidence that this was an oversight rather than a decision: **`flags_dead_after` in the same
file consults `flags_effect` first and skips Push/Pop kinds second**, so `pushf` terminates it
correctly and its structural arm is dead code for the flag forms. Two functions, one file,
opposite orderings, one of them sound.

### 3.2 x86-64: flags travel along conditional edges; a linear walk does not

The same scan stopped at the first flag writer. Flags also flow down the **taken** edge of every
conditional jump, so a writer on the fall-through path proves nothing about the target block:

```
    testb $128, %bl        <- was: movl %ebx,%esi ; andl $128,%esi
    je .LBB5               <- ZF-only consumer, and an edge
    addl $1, %edi          <- kills the flags HERE ONLY
    movl %edi, %eax
    ret
.LBB5:
    js .LBB6               <- reads SF of the flags the taken edge delivered
```

The walk saw one ZF-only consumer, stopped at the `addl`, licensed the narrow form. The `cmpb
$0, mem` rewrite in `dead_writes.rs` is licensed by the same function and diverges in SF for the
same reason, so it was exposed identically.

### 3.3 i686: a truncated scan accepted as a clean bill of health

The `setCC`-bool fusion's post-branch guard scans for "a flags writer before any flags reader"
within 16 lines and, on exhaustion, fell through with the guard still satisfied. With 18 opaque
pointer stores between the branch and a `jc`, unmodified main fused and emitted `jne .L1`,
handing that `jc` the producer's CF (`cmpl $1,%ebx`) instead of the dropped test's (always 0).
This is the budget lesson from F1 in a different backend: a truncated iteration is not a
fixpoint, and a truncated scan is not a proof.

## 4. The fix

One mechanism, because both x86-64 holes are in one function and the project's own rule for the
CFG forwarder applies ("each defect is closed by a single shared mechanism that every
forwarding decision goes through").

`walk_flag_consumers` is forward reachability over the **flag flow**, not the text:

* follows direct branch targets as well as fall-through (§3.2);
* classifies every line through `flags_effect`, so only *real* register pushes/pops are skipped
  and `pushf`/`popf` get the reader/writer treatment they are due (§3.1);
* ends each path at the first writer of the tracked flags, at a call, or at a `ret`;
* merges paths through a visited set — a line is expanded once and its facts are shared by every
  path reaching it. At a join that over-approximates *which* flags a consumer sees, and
  over-approximation here only ever reports more consumers, which fails closed;
* reports `proved = false` for anything it cannot resolve — indirect jump, external symbol, label
  outside the function — and an unproved walk licenses nothing.

Two views on one result: `flag_consumers_are_zf_only` (semantics unchanged, still used where ZF
really is the only surviving flag) and the narrower `flags_reach_an_sf_consumer`.

The narrowing is what turns a soundness fix into a code-quality win, and it is **provable, not
empirical** — SF is the only flag either rewrite can change:

| rewrite | ZF | PF | CF | OF | AF | SF |
|---|---|---|---|---|---|---|
| `testb $imm,%Xb` ← `mov`+`andl $imm` (imm ≤ 255) | same | same | both 0 | both 0 | both undefined | **bit 7 of `imm & X` vs bit 31 = 0** |
| `cmpb $0,mem` ← `movzbl mem,%r`+`testl %r,%r` | same | same | both 0 | both 0 | both 0 | **bit 7 of the byte vs bit 63 = 0** |

For the byte form, SF can differ only when the mask carries bit 7, so that case short-circuits
with no walk at all. A downstream `jc`, `jo`, `jp` or `adc` cannot observe an SF-only divergence
and no longer refuses the fold — the fix is strictly more permissive than the code it replaces.

The i686 fix is two separate changes and only one of them is about soundness: exhaustion now
refuses (soundness — a bound can never be wide enough), and the bound is widened 16 → 64
(precision — it finds writers that were previously out of reach instead of giving up on them).

## 5. Measurements

`scripts/flag_consumer_ab.py` (added by this delivery) runs all three questions in one sweep.

| config | identical | differing | `testb $` | `cmpb $0` |
|---|---|---|---|---|
| x86-64 `-O1` | 816 | **0** | 63 → 63 | 4 → 4 |
| x86-64 `-O2` | 816 | **0** | 22 → 22 | 16 → 16 |
| x86-64 `-O3` | 816 | **0** | — | — |
| x86-64 `-Os` | 816 | **0** | — | — |
| i686 `-O2 -m32 -fno-pic` | 803 | **0** | — | — |

i686 emitted instructions across the corpus: **256 629 → 256 629 (+0, 0.000 %)**.
(2 and 15 "both-failed" TUs are pre-existing: they do not compile with either binary.)

Compile time, paired and interleaved over 818 TUs at `-O2`: pre min 28.10 s / median 28.41 s,
post min **27.83 s** / median 29.09 s → **−0.95 % on min, +2.38 % on median, against a ±9 %
run-to-run spread.** A sequential (non-interleaved) measurement first reported +4.55 %; pairing
showed it was drift. Report the paired number.

**Gates, replicated locally on the delivered tree.** Full suite **2704 passed / 0 failed / 6
ignored**; `cargo fmt --check` clean; `cargo clippy --all-targets` zero warnings. `ci.yml`:
**21/21 gates rc=0, TOTAL FAILED GATES: 0**, warning lines **0** on both the
`RUSTFLAGS="-D warnings" cargo test --all-targets --locked` gate and `clippy -D warnings`.
`bench.yml`: **4/4 rc=0, TOTAL FAILED BENCH GATES: 0**, including the 383 s timed strict corpus,
which reports **n=39 all correct** and LCCC/GCC geometric mean **0.7255** (arithmetic 0.9586;
best `ackermann` 80.53× faster, worst `mandelbrot` 1.52× slower). That geomean is where
output-identity puts it: the previous delivery measured 0.7226 on binaries byte-identical to
these, so the difference is measurement noise — and the agreement is the check, because a geomean
that had *moved* would have falsified the identity claim.

One hygiene defect was caught by the CI run rather than by review: after all 21 gates the tree
still showed `bench-terminal.txt` untracked, and `git check-ignore -v` named no rule for it,
because the pre-existing `bench_*` glob needs an underscore where bench.yml's output name has a
hyphen — the same reason `bench-artifacts/` needed its own entry. The other four CI outputs are
now confirmed matched by real files, not assumed.

**A rejected intermediate, with its numbers.** The first fix for §3.2 was coarser: require
verified flag-block-locality whenever the walk stepped over a conditional edge. Sound, and five
lines long — but it cost 9 differing TUs at `-O2` and 30 at `-O1`, widening byte tests in
`linux_find_bit.c`, `zstd_count.c`, `k10_ffs.c`, `fannkuch.c`, `glibc_strstr.c`,
`sqlite_varint.c` and others, and in `tests/bench/k_memchr.c` it lost a fold outright
(`cmpb $0,(%rdi,%rsi)` → `movzbl` + `testl`, one instruction becoming two). Measured, rejected,
replaced by the walk plus the SF narrowing, which is output-identical on every TU.

## 6. Mutation analysis, and two pins that had to be reshaped

Every fix was reverted in isolation and the failing pins recorded (22 tests in scope):

| reverted | pins that fail |
|---|---|
| `proved` never set false | `walk_reports_unproved_when_it_cannot_resolve_a_target` |
| SF narrowing → old ZF-only rule | `…keeps_byte_form_for_a_carry_consumer`, `…keeps_byte_form_for_a_sign_consumer_when_the_mask_has_no_bit7` |
| bit-7 short-circuit removed | `…keeps_byte_form_for_a_sign_consumer_when_the_mask_has_no_bit7` |
| conditional edges not followed | `…refuses_byte_form_when_a_consumer_lives_past_the_scan_window`, `unsigned_load_test_is_kept_when_the_sign_consumer_is_on_the_taken_edge`, `walk_follows_the_taken_edge_of_a_conditional_jump` |
| `pushf`/`popf` skipped structurally | `…refuses_byte_form_when_pushf_captures_sf_as_data`, `walk_charges_whole_flag_readers_with_sf_but_not_carry_readers`, `walk_stops_each_path_at_a_writer_a_call_and_a_ret` |
| i686 exhaustion-refusal removed | `setcc_fuse_refuses_when_the_post_branch_scan_exhausts_its_bound` |
| i686 bound 64 → 16, refusal kept | none — confirming the widening is precision, not soundness |

Two pins were **vacuous as first written** and only became evidence after reshaping:

* The indirect-jump pin passed for the wrong reason: the emitted assembly showed *no fold at
  all*, because a function whose flag flow reaches an indirect jump, an external symbol or an
  unknown label is already refused upstream by the liveness oracle the fold requires. Diagnosed
  by printing the verdict for six shapes (`ret` block → BYTE, `jmp` to a label → BYTE, `js` on
  the taken edge → WIDE, indirect/external/missing label → NO-FOLD). The pin was replaced by a
  reachable one, and `proved = false` is now pinned **directly on the walk**, where it is
  reachable, with the call-site situation stated in the test's comment instead of implied.
* The i686 truncation pin did not discriminate: with the bound widened, removing the
  exhaustion-refusal left it green, because the reader at line 19 was then inside the bound.
  A second pin puts the reader past even the new bound, and now removing the refusal fails it.

Lesson worth keeping: a pin is evidence only if reverting the fix makes it fail, and that has to
be checked, not assumed.

## 7. Honest limits

* The corpus A/B shows **output-neutrality**, not that a shipping miscompile was fixed: no TU
  under `tests/` triggers these shapes. The defects are demonstrated at the level where they
  live — the passes' own assembly-text interface — with reproducers that print the divergent
  instruction. Claiming "fixed a miscompile in a real program" would need a C-level trigger,
  which was not found, and upgrading the claim anyway is precisely the failure the audit's
  untested remedies illustrate.
* Reachability is concrete rather than hypothetical: `pushfq`/`popfq` come from lccc's own
  `Select` lowering (the classifier comment says so) and from inline asm, and flags crossing a
  branch edge is ordinary codegen.
* `proved = false` is defence in depth at the byte-form site specifically (§6, first bullet). It
  remains load-bearing for the walk's contract and is pinned there.
* The BCD group (`daa`/`das`/`aaa`/`aam`/`aad`/`aas`) appears in the reader tables for
  completeness but is invalid in 64-bit mode; on i686 it is reachable only through inline asm,
  which is pinned and treated as a barrier.

## 8. Follow-ups

1. **One flag model, not three.** x86-64 `flags_effect`, the `relay_and_lea` CF/OF reader and
   clobber sets, and i686 `is_flags_writer`/`is_flags_reader` are three hand-maintained models of
   the same silicon, and they disagree in exactly the places documented above. Unifying them
   behind one silicon-verified table (the f10 sweep already provides the data) is the right
   architecture, but it is a cross-backend refactor of the highest-risk code in the compiler and
   does not belong in a delivery whose subject is three soundness holes.
2. **`flag_consumers_are_zf_only`'s third caller** (the width-mismatched redundant-test
   deletion) still checks `flags_are_block_local` explicitly. That is now redundant with the
   walk and strictly conservative; removing it needs its own measurement.
3. **Value homing and the push-based outgoing-argument ABI** remain the measured i686 gap to GCC
   (slot-heavy kernel at `-fno-pic`: lccc 402 instructions, gcc 291).
