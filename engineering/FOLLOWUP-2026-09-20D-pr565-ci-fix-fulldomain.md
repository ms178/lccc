# Follow-up 2026-09-20D — the PR #565 CI root cause, the slot-operand escape economics, and the full-domain range fold

Base: upstream `main` `7b628baa` (PR #566 merged). This session's commits on
top: `cfb2af39` (the CI fix), `3158374c` (oracle evidence),
`098eee34` (the full-domain fold). Deliverable: `ms178-1.patch`
(APPLIES-CLEAN vs `7b628baa`; includes PR #565's unmerged content rebased
on main — the arena bot's patch application makes it the complete
revision).

## 1. The PR #565 CI failure — root cause and fix

PR #565's new hosted-CI step ("Verify remaining fast local contracts")
ran `check_phi_acyclic_order.sh` in GitHub's environment for the first
time and failed the rot() size contract: default(acyclic) 59 insns/4
stkref vs the runner gcc's 56. The escape-off arm already reached 55/2 —
the resolver was healthy; the DEFAULT allocator policy was losing the
shape.

**Root cause** (traced with a temporary escape-decision log, removed
after diagnosis): the cost-ratio escape fired at the sign-extended loop
bound's admission — `victim v25 prio=1 | incoming v142 bar=20,
escapes=true`. The bound's two remaining uses are integer compare RHS
operands, which the x86-64 emitter serves from the spill slot with ZERO
instructions (`cmpq N(%rsp), %r10`). Register residency buys such a value
one saved def store; the steal displaced the d-entry web and held the
register across the whole loop, cascading the e-web (`d + t1`) through a
per-iteration stack round trip: +4 instructions, +2 stack refs.

**Fix** (`cfb2af39`), three coordinated changes:

1. `mark_loop_spanning` classifies `slot_operand_only` ranges — every
   use of the range and of every coalesced web member is an integer
   compare RHS (x86-64 only; I128/float/alloca-derived excluded; the
   compare LHS never folds). Computed before the loop-extent bail-out:
   the escape is loop-independent.
2. `select_evict_victim`'s escape refuses slot-operand-only incomings;
   clean evictions and the legacy modes are untouched.
3. `emit_int_cmp_replay_insn` gains the slot-direct fold the inline
   compare path already had, so a spilled compare RHS costs the same on
   both emitter paths (the classification's zero-instruction claim holds
   everywhere it is consumed).

The gate pins `default <= escape-off` on rot() so the class cannot
silently return. **Result: 59/4 → 55/2**, beating the hosted runner's
gcc-56 and every Godbolt oracle on the same census (see
`engineering/evidence/ra/2026-09-20-rot-escape/`): LCCC 57, GCC 16.2 60,
Clang 23.1 85, ICC 79, ICX 136.

**Corpus safety**: census A/B (old vs new, whole corpus at -O2/-O3/-Os)
— every function unchanged. The refusal is invisible outside the
defective shape.

## 2. The full-domain range fold

A range covering the compare domain's every value makes the membership
test a constant (inside TRUE, outside FALSE). GCC folds the family;
lccc rejected it — the signed full domain exceeded the
span-representability cap, and the unsigned full domain paid a dead
Sub+Cmp.

`RangePlan` became an enum (`Test` | `FullDomain`) so the shared domain
argument decides once and the four emission paths cannot diverge:
Select and bitwise And/Or become a `Copy` of the constant; the phi
diamond rewrites the phi in place (uses keep their reference — no
dangling bare-Value position is possible); the branch chain becomes an
unconditional `Branch` with the dead side's arms dropped and, under a
two-phase audit (defs unused elsewhere — phi arms count; every surviving
successor keeps a predecessor besides the dying set, on the canonical
CFG), the dead block deleted. Full-domain detection runs before the
span cap (the signed full domain needs no span constant) and again
after narrowing (a u8 range [0,255] behind a widening cast is the
tautology at the narrowed domain).

Measured on the natural spellings: the value forms compile to ONE
instruction (`movl $7` / `movl $9` — was a sub+cmp+select chain); the
branch forms keep dissolving through the earlier type-domain tautology
folds. Corpus census: every function unchanged (no full-domain shapes
occur there at -O2). The A12 battery oracle comment is updated (the i64
full domain is now the constant fold, not the rejection); the gate pins
the value forms fully constant plus the wrapped-span absence.

## 3. The set_membership domain audit (s60 follow-up #4 — CLOSED, clean)

`norm_const` fails closed on every type beyond U8/U16/U32/I32
(`_ => None`), `root_ty_ok` excludes 64/128-bit roots, and every
normalization lands in [0, 2^32) which i64 represents exactly. No
wide-domain arithmetic exists in the pass. The range-fold's new
FullDomain output (a Copy-of-constant) does not match
`suffix_parses`' Sub+Cmp Ule shape, so a full-domain chain member
fails closed to a skip. **No exposure; no change needed.**

## 4. The bitcount IV-widening design (PR #565 follow-up #1 — diagnosed, next session's first item)

`branch_index_store` remains one instruction behind GCC/ICX at baseline
(17 vs 16): the per-iteration `movslq %edx, %rdx` re-extending the
`int c` IV for `branch_slots[c]`. The `iv_widen` pass exists with the
full provenance machinery and runs at -O2 iter 0 — it widens IVs in
OTHER functions of the same TU but rejects this one. The complete
diagnosis:

- **The rejection site**: the select `(n == c) ? c : load(slots + n)`
  uses `c` as select DATA with a NON-member other arm (the load).
  The Select classification admits member-vs-const/member pairs only;
  anything else falls to `narrow_escape_or_bail`, which returns `None`
  for IN-LOOP uses (`lp.body.contains(&bi)`) — the whole plan is
  rejected. (iv_widen.rs, the `Instruction::Select` arm of the closure
  walk; `narrow_escape_or_bail` at the `fn` of that name.)
- **The compare is already handled**: `n` is loop-variant, so the
  `n == c` compare takes the existing `CmpAction::Trunc` path (a
  narrow compare against the member's truncation).
- **The backend elision question is ANSWERED, and it is the real epic**:
  a trunc probe (`(int)c` flowing into a compare, a select arm, a store
  index, and a return) shows the x86-64 backend already elides the
  final sub-register trunc (`movl %r10d, %eax` — free), BUT it also
  left a fully dead `movslq %r10d, %rdi; shlq $2, %rdi; leaq g(%rip),
  %rcx` GEP chain beside the SIB form `(%rcx, %r10, 4)` it actually
  used — three dead instructions from the (trunc → shl → lea) address
  computation that the SIB fold supersedes but nothing cleans.

**The design for the next session**, in dependency order:

1. Backend: teach the address-materialization cleanup (or the GEP CSE /
   narrow-copy-fold machinery) that a `Cast → Shl → LEA` chain whose
   consumer folded into SIB addressing is dead. The probe above is the
   regression test. This is worth doing FIRST — it is pure win
   independent of iv_widen.
2. iv_widen: admit in-loop NARROW escapes for select-data and compare
   uses as truncation actions when the target elides reg→reg truncs
   (x86-64: sub-register discipline, free). The escape's cost model
   must stay target-aware: on ARM/RISC-V a narrowing IS an instruction,
   and the in-loop rejection remains correct there.
3. Re-measure the bitcount kernel against the pinned oracles (17 → 16
   baseline, 16 → 14 at v3) and re-run the full-domain battery (the
   range fold and the IV widening interact only through the shared
   compare-classification discipline, but the census gate is the
   contract).

## 5. Verification matrix (final tree, `098eee34`)

| Gate | Result |
|---|---|
| check_phi_acyclic_order | PASS — rot() 55/2 default; new `d <= k` pin |
| check_range_fold_branch | PASS — full-domain value forms fully constant |
| ci_local.sh --fast | 51 passed, 0 failed |
| cargo test | 2991 passed, 0 failed (7 pre-existing ignores) |
| clippy (lib/bins/tests, -D warnings) | green |
| rustfmt | green |
| Regression suite (SSA validation) | 763 PASS / 3 pre-existing i686 multilib environmental |
| Benchmark output oracle | 204/204 |
| Census A/B (both changes, -O2/-O3/-Os) | every corpus function unchanged |
| Godbolt oracle (rot()) | LCCC best of five compilers |

## 6. Environmental notes

- The 4 GB host's cargo-test leg needs ~3.5 GB free: the sandbox's
  Next.js preview stack (next-server + postcss) holds ~800 MB and had
  to be stopped for the cold-cache DEBUG=0 lib-test compile; with it
  running, rustc is SIGKILLed mid-compile. The mitigation in
  ci_local.sh (DEBUG=0 + INCREMENTAL=0 + -j1) is correct but not
  sufficient alone on this box when the page cache is cold AND the
  preview stack is resident.
- The harness wiped `/home/z/lccc` and `~/.cargo` between sessions;
  recovery is the documented rustup-install + GitHub clone (the bundle
  in artifacts/ is stale — main has advanced past it).
