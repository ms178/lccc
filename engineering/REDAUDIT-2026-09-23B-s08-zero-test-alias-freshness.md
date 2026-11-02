# S08 deep red-team audit — upstream adace81a choices, and this session's work

Auditor: arena-agent. Scope: (1) upstream's merged RA work (`adace81a`,
re-pushed as PR #600) — do we agree with their choices; (2) this session's
four code commits (`0f666c8b` zero-test fold + terminal sweep, `7308462f`
alias-aware home freshness, `ab955e1c` RA-6 + dead-AND→testb) attacked with
the same standards we hold upstream to. Every verdict carries its evidence
and its residual risk.

## Part 1 — upstream's choices

### AGREE

1. **Widths from context evidence, not spelling.** Spelling-derived widths
   are unsound by construction (a `u32` seeded from a const can legally
   widen); their fix closed a real miscompile class and measured −160 B
   boot. This is the correct direction and the measurement discipline we
   require.
2. **Phi chaining (Part 2).** Back-edge copy elimination with veto checks
   (call windows, other consumers) — the wins are measured and the veto set
   is the right conservative shape.
3. **`flags_reader_window_ok` — the whole-window reader law.** A second
   reader (`je` then `jl`) binds exactly like a lone `jl`; single-reader
   vetting was a latent miscompile. We ported the law into our pair-fold
   windows and pinned it (4 pins). Upstream was right and we adopt it.
4. **Linker ICF non-text discriminant.** Correct classification of the
   miscompile; regression wired into CI.

### DISAGREE / CRITIQUE

1. **Phi chaining shipped without completing the emission-side freshness
   contract.** The coalescer hands whole chains one slot-less register home
   under the contract "a value's uses never follow a home clobber" — but
   the emitter's freshness bookkeeping stayed per-SSA-id, and the in-place
   chain update (`cmovnel %r10d,%r13d`) freshens only the newest id while
   evicting the older ids of the SAME logical value. Result: the session-26
   hard gate refused (fail-closed ICE) on kernel `workqueue.o` — blocking
   **100 % of kernel builds from current main** (reproduced on the
   pre-session binary; introduced with `adace81a`). The design flaw is not
   the coalescing itself but shipping a producer whose correctness depends
   on an invariant the consumer side does not maintain. `7308462f` completes
   the contract: the verifier's blessed same-value classes are published
   (`RegAllocResult::phi_chain`) and the read-side gates are alias-aware.
   **Lesson for RA-3:** any new coalescer MUST feed the same blessed-class
   map the verifier unions — the alias gate and the verifier are one
   contract from now on.
2. **The i686 roadmap cites x86-64 encodings.** RA-6's spec line is
   `movl %esi,%eax; movzbl %al,%eax → movzbl %sil,%eax` — but i686 (32-bit
   mode) has no REX prefix: `%sil/%bpl/%dil` do not exist. 22 of 29 boot
   census sites must refuse; 7 fold (a/b/c/d sources). An encoding-
   availability law that obvious belongs in the lever spec, not in the
   implementer's head; our pass documents it where the refusal happens
   (`narrow_reg_name → None`).
3. **PR #600 was a re-push of a tree-identical commit.** No technical
   content; process noise. Noted only so the next re-base does not waste a
   cycle "absorbing" it twice.

### RA-3 (pressure-aware greedy scan) — direction agreed, with one addition

The video-bios +20 B regression they cite is the right evidence that homing
more values needs a pressure model. Our addition: the scan's coalescing must
publish `phi_chain` (see DISAGREE 1), and its placement decisions must honor
the byte-addressability demand of narrow consumers — that is the
allocation-side root cause of the staging copies RA-6 cleans up post hoc.

## Part 2 — red-team of this session's work

### `0f666c8b` — zero-test fold + terminal adjacency sweep

- **No-reader-gate law attacked:** a logical op clears CF/OF and sets
  ZF/SF/PF from its result; `cmpl $0` over that result produces the same
  set. Every reader class consumes identical flags — the law is closed
  under the ISA flag definition, so no reader gate is needed. Pinned incl.
  the `cmpl $1` refusal and the width refusals.
- **Placement attacked:** the first three wirings (global, changed2,
  changed8-in-loop) were dormant on the real pipeline because phase 3.8's
  fixpoint has an entry gate that never opens for these functions. Root-
  caused with `LCCC_DEBUG_PEEPHOLE_IN` + a temp trace (removed before
  commit); the fix is the unconditional post-3.8 + terminal sweeps. Rule
  recorded: *a pass whose window other passes open must run unconditionally
  after the last window-opener.*
- Residual risk: none known. Validated by corpus A/B (−11), QEMU 16/16,
  runtime battery.

### `7308462f` — alias-aware home freshness (soundness-critical)

Attack attempts and outcomes:

1. *Fresh sibling, clobbered register (non-member write inside the class's
   live range):* impossible — the RA overlap verifier unions exactly these
   classes and fails the build on any other overlap, so every write to the
   shared register within the class interval is a class member's
   definition. The verifier is what makes the alias check sound.
2. *Vacuously-fresh sibling (defined later in emission order, never
   freshened):* a stale member can only exist if a write happened after its
   def; per (1) that write is a member def, which freshens ITS id — so a
   genuinely-fresh sibling always exists when the alias path fires. The
   vacuous arm is unreachable as the sole qualifier (argued in the code
   comment beside `home_readable_via_alias`).
3. *Write-accounting completeness:* same trust base as the pre-existing
   freshness design (`note_reg_clobbered` at every clobbering site). If a
   write path skips accounting, the old design miscompiles the same way —
   no new trust added.
4. *Non-alias targets:* they receive an ignored empty map; behavior is
   bit-for-bit the pre-alias rules (CI + torture green).
- Residual risk: the argument depends on the overlap verifier running
  (it does, under the production config) and on `phi_chain` being fed by
  future allocators — the roadmap note in Part 1 pins that duty.

### `ab955e1c` — RA-6 copy+subreg fold; dead-AND→testb

**RA-6 attacked:**
- *Copy has a reader between copy and extend:* impossible — the window is
  strictly adjacent (nops/directives skip only); a label between refuses
  (a path entering at the label never executed the copy) — pinned.
- *Extend reads a different byte:* the verbatim `%al`-form comparison
  refuses `%ah` (different byte) — pinned by construction of
  `parse_narrow_ext32`'s verbatim source contract.
- *Extend overwrites the dest partially:* impossible — all four
  movz*/movs* forms write the full 32-bit destination; that is what makes
  the copy's value unobservable.
- *Encoding availability:* `narrow_reg_name → None` refuses %esp/%ebp/
  %esi/%edi byte forms (no REX on i686) and all byte forms of %esp —
  refusal IS the correctness mechanism; 22/29 census sites refuse, 7 fold,
  −N bytes measured in the census.
- *Word forms from %esp:* `movl %esp,%eax; movzwl %ax,%eax` would be sound
  but is not worth special-casing; byte forms from %esp refuse (no %spl).

**testb narrowing attacked:**
- *SF divergence:* `js/jns/jg/jge/jl/jle` refuse via the pair-fold's
  ZF/CF-safe sets — pinned (`dead_and_refuses_sign_flag_reader`, which also
  documents that the plain testl deletion still fires: its law is exact
  flag equality).
- *`jecxz` prefix collision ("je"-prefix, but flag-blind):* FOUND BY THIS
  AUDIT, FIXED: explicit `jecxz`/`jcxz` exclusion; the window oracle past
  it still vetoes foreign readers.
- *Value not actually dead:* `GprLiveness::dead_after` (the pair fold's
  oracle, already trusted) gates every rewrite; the live-value pin refuses.
- *Re-processing/idempotence:* after narrowing, the line is `testb` — no
  pass matches it (fold 1 matches and/or/xor only; the testb pass's mid-
  check only matches testl/cmpl $0; re-scan hits `dead_after` false because
  testb reads the register). Terminates.
- *imm parsing:* decimal + 0x-hex only; `$-4` refuses (parse fails) —
  correct, since negative masks need the 32-bit form. 256+ refuses (pin).

**Cross-pass interactions:** fold 1 deletes the redundant test; the testb
pass then sees `andl;reader` and narrows; the narrowed line re-classifies
and no pass re-matches. The corpus diff (pre-session binary vs current)
plus the kill-switch matrix (all three switches → hash identical to the
GCC-built oracle battery at -O2 and -Os) bound the change to its intent.

### Harness findings

- `ci_local.sh` and `build_kernel_vm.sh` must run SERIALLY: CI's cargo
  rebuild replaces `target/fastbuild/lccc` while make execs it. The first
  "kernel failure" this session was exactly that race; the second was the
  real upstream ICE.
- The stale-object class (Kbuild fingerprints command lines, not compiler
  content) is already closed upstream by `kernel_tool_identity.sh`
  (31e73b71) — our earlier claim that S07 "silently skipped" objects is
  thereby explained and future builds invalidate on compiler change.

## Part 3 — oracle ledger (hard data, godbolt.py/codegen_oracle.py)

Source: `/tmp/oracle_s08.c` (distilled lever shapes), flags `-Os -m32`,
oracles gcc16.2 (cg162) + clang 23.1.0 (cclang2310); local lccc via
`godbolt.py compare`.

| shape | gcc16.2 | clang 23.1 | lccc before | lccc after |
| --- | --- | --- | --- | --- |
| selB masked branch (reg value) | `testb $2,mem`+cmov | `testb $2,%al`+lea/cmov | load;andl;testl;je | load;**testb $2,%al**;je |
| selC dead mask | `or` dropped | dropped | orl kept | **dropped** |
| ext_byte (homed value) | extend from mem | `movb mem,%al` | copy+extend (5 B) | **extend from home (3 B)** |
| boot census | — | — | 5021 insns | **5007 insns** |

Remaining standings gap on `ext_byte`/`selB` is the leaf-prologue + arg-
homing lever (F-1, upstream's open item 2) — 3–5 insns of frame/callee-
save overhead per leaf, NOT addressable at peephole level; tracked, not
hacked around.

## Verdict

- Upstream: adopt all four landed choices; complete the freshness contract
  (done); demand `phi_chain` publication from RA-3 (roadmap); fix the RA-6
  spec's encoding law (done in our implementation).
- Ours: two audit findings fixed (jecxz guard; the SF-reader pin law), one
  design law recorded (terminal sweep placement), zero known residual
  soundness risks. Runtime battery hash-verified at -O2/-Os × 4 kill-switch
  combos; corpus diff clean; kernel builds; QEMU 16/16.
