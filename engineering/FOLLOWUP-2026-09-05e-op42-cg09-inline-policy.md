# Follow-up — OP-42, CG-09, and measured normal-`-O2` inlining repair

**Date:** 2026-09-05

**Upstream base:** `a3f5a3174328fa1f8facdf99cfec70b828781870` (PR #421)

**Session commits:** `89adb5c3` (OP-42 / CG-09 repair), `59edc453` (A/B tool hardening),
`8072b7f3` (RA census repair), `46120851` (inline/TLS work).

This record separates facts measured in one alternating window from static-code
observations.  The VM has no usable `perf` PMU, so runtime figures are pinned,
interleaved wall-clock screens; the primary statistic is the minimum (with a
low-sample confirmation reported where available).  They are not presented as
metal-cycle claims.

## Delivered optimization items

| Item | Decision and implementation | Evidence |
|---|---|---|
| **OPT-1 / OP-42** | Landed atomic expression-chain sinking only when a transaction is semantics-safe *and* its complete external-input live-range cost is Pareto-profitable. | `89adb5c3`; focused pass tests, verifier-enabled corpus validation; details below. |
| **OPT-2 / CG-09** | Landed longest-common-*suffix* epilogue sharing, narrowed to safe SysV callee-save tails and hardened against CFI/order hazards. | `89adb5c3`; static corpus result `6551 -> 6442` instructions (`-109`, `-1.7%`). |
| **OPT-3 / PERF-41** | Keep a plain, multi-site static wrapper outlined when it hides an inlineable loop descendant; persist the decision across repeated inline-pipeline invocations. | `spectral_norm`: `250.28 -> 205.50 ms`, new/old `0.821`, exact outputs equal. |
| **OPT-4 / RA-pressure inline policy** | Keep a plain multi-site loop helper outlined when its call site is in an enclosing loop at normal `-O2`; retain `-Os`, explicit-inline, section, and PGO overrides. | `hash_table`: `9071.38 -> 7632.42 ms`, new/old `0.841`; `228 -> 174` assembly instructions. |
| **OPT-5 / glibc clone cap** | Cap cloning of an ordinary small loop helper at four direct sites.  This catches a five-way `glibc_memcmp_bytes` clone without broadening the normal policy. | Amplified glibc derived workload: low-5 `78.155 -> 69.889 ms`, ratio `0.8942`; the caller falls from `183` to `93` instructions and `18` stack references to zero. |
| **Additional TLS quality fix** | Merge foldable and must-materialize TLS `GlobalAddr` webs: TLS must materialize a `%fs` base either way, unlike RIP-foldable normal globals. | `tls_pass`: three TLS address sequences to two.  Runtime neutral within noise (low-7 ratio `0.9947`), output equal. |

The inline changes deliberately do **not** treat `CCC_INLINE_SKIP` as a
production interface.  It remains a diagnostic switch used to establish the
direction of each decision; the production conditions derive from IR facts.

## OP-42: length-aware atomic sinking

The first atomic-chain experiment showed why counting values at one source /
target boundary is insufficient.  A chain can have one external value at the
target but extend that value across every instruction between source and
target.  The initial implementation increased corpus pressure (`+22`
instructions, `+1` reg-reg move, `+2` stack references, `+11` pushes), despite
looking neutral under a boundary-only model.

The landed model in `src/passes/expr_sink.rs` therefore:

1. collects a maximal unique-use feeder chain before changing the IR;
2. rejects multi-def/redefined operands, barriers, calls, aliases, volatile and
   observable memory hazards, loop-entry frequency increases, and any invalid
   dominance/liveness shape;
3. evaluates the complete transaction against live-point and call-crossing
   pressure, including the *length* over which every external input becomes
   live;
4. requires a strict Pareto improvement rather than exchanging a saved result
   for an equally expensive extended input; and
5. removes and inserts the complete chain transactionally in preserved program
   order, so no intermediate pass sees a partial migration.

That is an agreement with the **goal** of PR #421's atomic design, but a
rejection of its earlier boundary-only profitability premise.  The negative
experiment is retained in `engineering/DECISIONS.md`; it must not be revived
without a stronger cost model than the one now enforced.

## CG-09 red-team result

The original PR #418 exact-tail cross-jump was a sound base, but matching only
whole identical epilogues left strict suffixes unshared.  CG-09 extends the
match only to a suffix of a longer host restore sequence.  The repair in
`89adb5c3` deliberately refuses everything outside this narrow shape:

- SysV AMD64 callee-saved `popq` (`%rbx`, `%rbp`, `%r12`–`%r15`), optional
  stack adjustment, then `ret` only;
- no `%rax` / return-value restoration or arbitrary text suffixes;
- labels generated uniquely and only at instruction boundaries;
- no cross-function host; and
- fail closed for CFI after control flow has split, because the target PC would
  otherwise inherit another path's unwind state.

The longest compatible host is selected first and shared labels are reused per
suffix length.  Unit coverage includes deepest-suffix selection, label safety,
and path-sensitive-CFI rejection.  This is a qualified **agree** with CG-09:
the code-size idea is beneficial, while unconstrained textual LCS merging is
not acceptable for an ABI epilogue transform.

## Normal `-O2` inlining investigation and repair

### Reproducer and repeated-pipeline bug

`tests/regression/inline_multisite_loop_wrapper.c` has a two-call plain static
`wrapper`, a one-call loop `loop_kernel`, and volatile input to prevent IPCP
from folding away the structural signal.  Before the policy, the inliner chose
`loop_kernel -> wrapper`, then subsequently chose `wrapper -> main` twice.
The first descendant-only guard was inadequate: a later inline invocation
rebuilt the callee map after the kernel had expanded into `wrapper`, so the
wrapper appeared as a direct loop body and escaped its original size envelope.

`CalleeData` now records ordinary-static status, module-wide direct calls,
eligible recursive descendants, and persistent `func.has_inlined_calls`.  A
plain multi-site wrapper remains outlined if it currently hides an eligible
loop descendant **or** acquired a loop during an earlier inliner invocation.
The regression's structural guard requires an emitted `wrapper:` and exactly
two `call wrapper` instructions at `-O2`; the runtime output is also compared
against GCC.

The intended decision is directly confirmed: with the policy enabled,
`CCC_INLINE_SKIP=mul_AtAv` changes `spectral_norm` by exactly `1.000` in a
nine-round screen, because both arms retain the same outer wrapper.

### General nested-loop and clone-pressure gates

The wrapper observation was not the only measured failure mode:

- `lookup` in the workload-derived hash table is a 20-instruction, seven-block
  plain static loop called at two sites inside outer loops.  Diagnostic
  outlining made the old configuration `16.8%` faster in isolation.  Normal
  `-O2` now keeps this exact class outlined, while `static inline`, GNU inline,
  always-inline, one-owner, section-sensitive, and profile-forced callees
  retain their stronger policies.
- `glibc_memcmp_bytes` is a 27-instruction, eight-block loop cloned five times
  into branch exits of `glibc_memcmp_common_alignment`.  The cap permits up to
  four clones but leaves the fifth-and-later case out of line.  It does not
  apply under `-Os`, where the established size policy is independently
  guarded by `check_os_nested_loop_inline_policy.sh`.

The first paired screen was compiled by two LCCC binaries from the same
pre-policy source base (`59edc453`) and the wrapper/nested-loop-policy source,
with every stdout and exit status checked first.  The code is identical to the
final source for every row except the deliberately separate clone-cap memcmp
case below:

| benchmark | old minimum (ms) | new minimum (ms) | new/old | old -> new assembly instructions |
|---|---:|---:|---:|---:|
| `spectral_norm` | 250.28 | 205.50 | 0.821 | 199 -> 136 |
| `hash_table` | 9071.38 | 7632.42 | 0.841 | 228 -> 174 |
| `glibc_memcmp` | 6.77 | 7.42 | 1.096 | 325 -> 325 |
| `expat_xml_scan` | 47.53 | 47.42 | 0.998 | 259 -> 259 |
| `gzip_crc32` | 117.40 | 117.51 | 1.001 | 146 -> 92 |
| `zlib_ng_adler32` | 38.16 | 37.98 | 0.995 | 258 -> 258 |
| `nbody` | 225.93 | 225.90 | 1.000 | 394 -> 394 |

The unamplified memcmp timing is too short to attribute a change.  Its
mechanically amplified `PASSES=65536` version is the relevant same-window
measurement: all outputs were `551707045632`; min, low-5, and median ratios
were `0.8913`, `0.8942`, and `0.8970`, respectively.  Raw samples and the method note are versioned in
`engineering/evidence/op26-inline-policy-2026-09-05/`; working copies also
remain under `/home/user/evidence/`.

## PR #415–#421 audit disposition

| PR | Disposition | Evidence / repair |
|---|---|---|
| #415 — if-convert key injectivity, volatile preservation, RA-06 | **Agree.** RA-06 remains opt-in, appropriately, until its split proof has broader evidence. | The address-key / volatile preservation correction is a hard correctness fix; existing differential coverage and the full verifier suite remain green. |
| #416 — wider copy/phi slot roots | **Agree.** | The root unification is conservative and follows the established width/liveness partition.  No new relaxation was added here. |
| #417 — RA verification documentation | **Agree.** | It records a negative result rather than turning it into an unsupported optimization claim. |
| #418 — exact epilogue cross-jump | **Agree, qualified.** | Exact complete tails are safe under its gates.  CG-09 improves it only with the strict SysV/CFI suffix rules above; arbitrary textual suffix matching is rejected. |
| #419 — latch liveness and LEA invalidation | **Agree.** | The latch footprint closes a real phi-eliminated liveness hole; using the shared may-write oracle avoids an incomplete instruction-name list. |
| #420 — rebase audit, vector-width test, i686 liveness oracle | **Agree.** | The vector test becomes mechanically exhaustive; the i686 memory-fold scan now uses whole-function liveness instead of assuming a barrier ends a value's lifetime. |
| #421 — expression sinking, suffix merge, A/B tooling | **Agree with repairs.** | `89adb5c3` supplies OP-42's length-aware transactional profitability and CG-09's CFI/ABI hardening. `59edc453` hardens interleaved reporting. |

This is not a blanket endorsement based on commit titles: each qualification is
a response to a concrete counterexample or ABI condition described above.

## Validation completed

- `scripts/build_lccc_fast.sh`: passed after every source iteration (the
  mandated `-O1`, `-j2` fastbuild configuration).
- Focused Rust tests: `inline_limit_tests` **8/8** and
  `global_addr_cse::tests` **16/16** passed.
- New wrapper structural test and existing `-Os` nested-loop structural test:
  passed with `CCC_VERIFY_IR=abort`.
- Focused GCC-output / A-B regression filters: inline **23**, PGO sections
  **1**, GNU inline **3**, TLS **2** — all passed with the IR verifier armed.
- Full regression suite: **636 PASS, 0 FAIL, 7 SKIP**, including both
  stack-layout A/B configurations and the verifier gate.
- Full benchmark GCC-oracle gate at every optimization level (`-O0` through
  `-O3`): **156 PASS, 0 FAIL, 0 SKIP**, run with `CCC_VERIFY_IR=abort`.

## Follow-up work deliberately not claimed complete

1. Replace direct-call-count and loop containment proxies with profile-aware
   benefit versus post-allocation pressure feedback.  The new gates are narrow
   and evidence-backed, not a claim that every multi-site loop should outline.
2. Validate PGO force decisions with representative profile data; the policy
   explicitly preserves them but this host run did not synthesize a new profile.
3. Implement direct `%fs:symbol@TPOFF` load/store/GEP lowering.  TLS CSE removes
   one redundant materialization, but does not yet reach the desired segment
   memory operand.
4. Continue the high-prize backlog: verified live-range splitting (RA-PRESSURE-1),
   derived-IV closure only with provenance proofs, classifier if-conversion
   after critical-edge splitting, Adler recurrence work, and non-reduction FP
   vectorization.
5. Re-run the full timing corpus and a metal PMU protocol before claiming
   hardware cycles, IPC, or branch-miss improvements.
