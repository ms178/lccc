# Response to the second Review AI audit (PR #695)

**Verdict up front: this audit is much better than the first one.** No blocker
was claimed, and nothing in it was a false positive. Of the 5 medium findings
I agree with 4 and disagree with 1 — and the one I disagree with would have
destroyed test coverage. Of the 11 low findings I agree with 9, disagree with
1, and corrected 1 that was based on a wrong premise. I also found a fourth
blind spot in the volatile ratchet that neither the audit nor I had identified
before.

Everything below was verified against the merged tree (`ec08e6a6`), by
reading the code, running the gates, and mutation-testing. Where I say a
finding is real, I have the failing command.

---

## Medium

### 1. The fold's memory-operand test is too weak — **AGREE, and it was a real latent miscompile**

The test was

```rust
if a.contains('(') || b.contains('(') || a.contains('[') || b.contains('[') {
```

Three independent confirmations:

1. **It was the only such test in the tree that omitted `:`.** Every other
   memory-operand test rejects positively: `load_op_fuse::accesses_memory`
   (`:239`, `:259`, `:1116`), `dead_writes.rs:218`, `:552`,
   `local_patterns.rs:3420`, `memory_fold.rs:2742`, `assembler/parser.rs:1666`.
   The audit's "the repo already has the correct test" is right, and the
   outlier framing is the sharper version of the point.
2. **The TLS fast path is real.** `emit.rs:9359-9360`
   (`try_emit_tls_direct_load` / `..._store`), plus
   `emit_tls_global_addr` and `tls_direct_type_ok`, so `%fs:sym@TPOFF` really
   is emitted.
3. **The consequence chain holds.** `register_family_fast` answers `REG_NONE`
   for both `%fs:x@TPOFF` and a bare `counter`, so `fam_a`/`fam_b` were
   `REG_NONE` and the two clobber checks were skipped *entirely*. A store
   between two textually identical compares folded away.

The audit says "no miscompile is possible today, but it's one change away". I
agreed it was a real hazard and disagreed with the framing I first wrote,
which called it "a *reproducible* miscompile, one line of C away". That was
an overclaim and I am withdrawing it. I never produced a C program that
compiles to wrong output; what I have is a unit test over a synthetic asm
fixture that shows the pass deleting the *wrong* compare when the memory
operand defeats the negative heuristic. That is a demonstrated
precondition violation, and it is what the fix and its test rest on — but
"reproducible miscompile" implies an end-to-end demonstration I do not have,
and the difference matters when someone decides how urgently to backport it.
The accurate statement: the old heuristic could not see a memory operand at
all, so both clobber checks were skipped and any store between two
textually identical compares was folded; the unit test pins that the wrong
delete happens.

**Fixed** by replacing the negative heuristic with the same positive allowlist
the rest of the tree uses: an operand is accepted only if it is an immediate
or a plain GP register (`plain_gp_operand`, already imported in that file).

Mutation-verified: reverting to the old heuristic fails the new test
(55 passed / 1 failed); with the allowlist, 56 pass.

**One correction to the audit's proposed test.** Instruction 1 asked for a
test in the shape `cmp; cmov; mov; cmp; cmov` asserting the *first* compare
survives. That shape does not fold, and must not: `cmovb %rsi, %rcx` writes
`%rcx`, the register the compare reads, so the second compare is genuinely
live. The pass gets this right via `reg_refs`, not `preserves_eflags` — a
`cmov` really does leave EFLAGS alone. The emitted shape therefore has its
`cmov`s on the **tail** (`cmp; cmp; cmov; cmov`), which is what
`check_redundant_flags_compare.sh` asserts ("one comparison feeds the cmov
chain"). I wrote the test the audit asked for, watched it fail, and then wrote
the three tests that pin the real contract instead:

- keeps the **first** of the pair, asserted on **order** (a count cannot tell
  the two compares apart, so the pre-existing test passed even if the pass
  folded backwards — the audit's low finding, and it is right about that);
- refuses across a `cmov` that writes the compared register;
- folds across a `cmov` that writes a *different* register, keeping the first.

### 2. Orphaned test file — **DISAGREE with the prescription, agree with the observation**

The file is genuinely orphaned: `check_loop_preheader.sh` builds the
**top-level** `loop_preheader_shapes.c`, the corpus runner's glob is
non-recursive, and all four docs naming it point at the top-level path. Its
own header — "Only check_loop_preheader.sh builds it" — is false.

But it is **not a stale duplicate**, and "delete the file and its folder" would
have silently deleted real coverage. The two inventories do not overlap at all:

| file | shapes |
| --- | --- |
| live (155 lines) | `invariant_ptr`, `guarded_sum(struct box*)`, `switch_entered`, `computed_goto`, `already_dedicated`, `main` |
| orphan (60 lines) | `dowhile_sum`, `while_sum`, `guarded_sum(const char*, int)`, `alloca_sum` |

The orphan holds the only **do/while** shape (entered by *falling in* rather
than by a back edge) and the only **alloca-only** shape — and the alloca case
is the profitability regression the pass's own docstring cites as the reason
the pass exists at all.

**What I did instead:** moved the four shapes into the live file (renaming the
orphan's `guarded_sum` to `null_guard_sum` to avoid colliding with the
struct-box one), gave them runtime cross-checks in `main`, and wired them into
the gate — `dowhile_sum` as a **second positive shape** with its own negative
control, the other three into the refuse list (four shapes → seven).

**Honest limits.** The `dowhile_sum` effect check is mutation-proven: disabling
the pass in the ON arm alone fails it. For `while_sum`, `null_guard_sum` and
`alloca_sum` I could **not** construct a mutation each uniquely catches —
removing the alloca profitability veto still left `alloca_sum` byte-identical.
So they are restored coverage asserted byte-identical, not three new
independent tripwires, and the commit says so.

### 3. The preheader counters report wrong numbers — **AGREE, and it is worse than described**

Measured, not inferred. Before:

```
guarded_sum: loops=2 inserted=1 ... already_dedicated=1     <- ONE natural loop
f:           loops=2 inserted=1 ... profitability=2          <- TWO loops, one refused
g:           loops=2 inserted=1 ... profitability=4          <- THREE loops
```

Three separate defects:

- **Double counting.** The fixpoint re-walks every loop every round and the
  counters accumulated, so one loop was counted twice — and in round 2 as
  `already_dedicated`, i.e. simultaneously reported as inserted *and* refused.
- **`loops` excluded the loops it rejected.** `census.loops += 1` sat in
  `plan_insertions`, downstream of the profitability filter. The headline
  number silently omitted exactly the loops it was explaining.
- **`out_of_range` did not exist.** Two `continue`s (header index, preheader
  index, both outside `func.blocks`) counted a loop in `loops` and in no
  bucket — so the buckets could not sum to `loops` *even in principle*. The
  audit noted the header case; there is a second one, on the preheader.

**Fixed** so the buckets always partition the loops, by construction: `planned`
joins the rejections as a sixth category, `steady` is rebuilt each round and
only the last is reported, and `inserted` is the one field that accumulates
(assigned *after* the loop — my first attempt assigned it per round and the
per-round reset silently zeroed it, which is why I measure the output rather
than trusting the diff).

**Where I went past the suggestion.** The audit proposes a unit test asserting
the partition. I made the invariant *structural* first — `Census::accounted()`
plus an assert — and then found that a `debug_assert!` there was inert in the
builds that actually ship and in the default test run: `fastbuild` inherits
`release`, whose `debug-assertions` is off, so it enforces nothing in the
emitted binary and nothing in a plain `cargo test --profile fastbuild`. I
verified that, made it a hard assert (one add and one compare per round,
against a CFG rebuild), and *also* added the check to the gate on the
**reported** numbers, which is profile-independent.

To be precise about the scope, because I overstated it the first time: CI is
not blind to debug-only asserts. `.github/workflows/ci.yml` runs the suite a
second time with `--config 'profile.fastbuild.debug-assertions=true'`, so in
*that* job a `debug_assert!` is live. The claim is only that the shipping
binary and the default local fastbuild test run do not execute one — which is
precisely why the invariant could not be left to a debug-only assert, and why
it also lives in the gate.

Mutation-verified both ways: dropping the reachable `already_dedicated += 1`
trips the assert; a mutant that double-counts a bucket *and* neuters the assert
is still caught by the gate.

Also folded in: the `Census` doc claimed it was a `thread_local` while the
comment 13 lines below said it deliberately was not (the audit is right); the
env var is read through a `OnceLock`; and the format string's two runs of ten
spaces — manual column alignment, which breaks the moment a label changes
width — are now single spaces.

### 4. The 674-line volatile lint gives false confidence — **AGREE on the mutation results; PARTIAL on the fix**

I reproduced all three mutations against the real tree. All three pass the
gate silently at exit 0, including the one its own docstring promises to
catch. That is worse than having no gate.

- **Blind spot 1 (debug-print-only use): CLOSED.** `_is_debug_only_use` no
  longer counts a `volatile` that only reaches a logging macro. Verified on the
  real `emit.rs` site (exit 1), and verified that a negated guard and a use in
  a non-logging macro both still pass (exit 0 — no false positives).
- **Blind spot 2 (`volatile: _`): DISAGREE — by design.** The docstring calls
  it "the explicit, reviewable way to say I looked at volatility and it does
  not matter here". That is a deliberate choice, not an oversight, and it is a
  different claim from "nobody looked". It is a hole only for a reviewer who
  reads past it.
- **Blind spot 3 (`..` arm): AGREE, structurally out of reach.** It binds
  nothing, so the rule is never invoked. Closing it means requiring every
  Load/Store pattern in the memory-motion passes to name `volatile`, and 353
  existing `..` patterns in `src/` would have to be made to.

**Blind spot 4 — mine, not the audit's.** The window is a line distance, not
an arm boundary, so in a two-arm `match` where the `Load` arm's only use is a
debug print and the `Store` arm has a real `*volatile`, **the sibling arm's use
satisfies the Load arm's window** and the gate reports clean. I found this
while fixing blind spot 1 — my first M1 mutation still passed, which is what
led me to it. Fixing it means re-deriving the arm boundary: a parser, not a
regex query. Named in the docstring as the highest-value thing the gate could
grow.

All four are now pinned in `--self-test` as **expected results** (19 cases, up
from 15), so the gap is written down rather than assumed away, and the
docstring no longer promises what the code cannot do.

**On switching on the compiler's own lint — right direction, so I measured it
before deferring.** Lifting the crate-wide
`#![allow(dead_code, unused_variables, ...)]` in `src/lib.rs` surfaces **87**
warnings: 18 in `vectorize.rs` (27k lines), 6 in `codegen/memory.rs`, 3 in
`local_patterns.rs`, 2 in `memory_fold.rs`; **LICM, GVN, DSE and if-convert
are already clean**. Scoping it to the memory-motion modules is ~29 fixes
concentrated in one huge file — its own change, not something to smuggle into a
codegen fix. Recorded as `VOLATILE-1` in `backlog.md` with the full table.

The measurement is mildly reassuring: **none** of the 18 `vectorize.rs` unused
variables is a `volatile` binding, so no instance of this bug class is hiding
in the most volatile-heavy pass in the tree.

### 5. Linker cross-check is weaker than stated — **AGREE on both actionable parts**

This is the one that actually bit me: on this host `apt-get install lld` did
not register, `ld.lld` was absent, and the script-path cross-check fell back to
a single opinion — still PASS. The suite's own comment already describes this
exact hazard.

- **`--require-oracles NAMES` added**, wired into `ci.yml` and `ci_local.sh`
  as `bfd,lld`. The run fails before executing anything if a named oracle is
  not registered, naming what *was* registered and what to do about it.
  Verified end-to-end: a no-op on this stocked host; with `ld.lld` hidden from
  the probe, exit 2 naming `lld`. `missing_required_oracles` is module scope
  and unit-tested (6 cases) for the same reason `reloc_oracle_agreement` is.
- **The "structural probe" claim was in `ci.yml`, not the docstring** — and
  the docstring is honest. `_oracle_rejected_the_script` is a regex over the
  linker's stderr, i.e. a negative inference from a diagnostic. I left the
  mechanism alone and made the comment describe it accurately, because a real
  probe is a larger change than this fix should carry and the honest comment
  plus the hard requirement gate closes the fail-open that actually occurs.

**One correction.** The audit calls the one-opinion unit test a contradiction
of the CI comment. It is not: that test pins the quorum **fold** — which must
not pass on zero oracles and must behave sanely on one — which is a different
layer from host configuration. The gap was never the fold; it was the absence
of a host-level check, and that is what is now added.

---

## Low

Agreed and fixed: the stale `preheader_would_unlock_a_hoist` doc (it claimed
to track LICM's rule automatically; it hard-codes the header, justified by a
proof — so the coupling is one-directional and needs a human, and the doc now
says that); the unmeasured "~25M walks" figure (now labelled as arithmetic from
the stated parameters and a worst case, with the things that *are* measured
named instead); the one-item `std::slice::from_ref` loop (straight-line now,
census output byte-identical, so the refactor is behaviour-preserving); the
`Census` thread-local contradiction; the two runs of ten spaces; the duplicate
`fcomip` (replaced with `fucomip`, the commuting variant — verified to block
the fold, so it is new coverage rather than a cosmetic de-duplication); six
`b"…scriptb"` typos (the trailing `b` meant the fixture pinned a string no
oracle produces); the brittle byte-register regex, which matched only
`%r8b..%r15b` and so would have failed on correct code if register allocation
picked `%al`/`%cl`/`%dl`/`%bl`/`%sil`/`%dil`/`%spl`/`%bpl`; the ci.yml comment
describing PR-branch churn as history; the Godbolt catalog fetch.

**The Godbolt one is worth more than "a bit wasteful".** `_compiler_version`
fetched the *entire* Compiler Explorer catalog once per compiler id, and
`_run_one` calls it while building the record — *before* the cache check. So a
five-compiler sweep issued five full catalog downloads, concurrently and
unsynchronised, and a sweep that was 100% cache hits and **completely offline**
still paid for all five, each up to `_VERSION_TIMEOUT` seconds, twice, because
`_get` retries. The docstring's "a sweep run offline must behave exactly as it
did before this existed" was false. Now: one lock-guarded fetch per process,
with the flag set even on failure so a dead network is not retried per
compiler, plus `--no-version-probe`. Measured — 5 compilers offline: 5 → 1
fetch; with the flag, 0; 5 concurrent threads, still exactly 1.

**Disagree with one premise (the cmov shape)** — covered under Medium 1.

**Corrected one:** the `Census` comment. I initially thought the audit had this
backwards; re-reading the file, line 142 really did say "Thread-local because
the pass runs under `--rank`'s worker threads" while line 155 said it
deliberately was not. The audit was right and I was too quick to dismiss it.

---

## What I did not do

- **A real script-capability probe** for the linker suite (link a trivial
  object with a minimal `t.ld` once per oracle, cache `script_capable`). It is
  the better design and removes a whole class of false `incapable`, but it is a
  larger change than the fail-open fix should carry, and the honest comment plus
  `--require-oracles` closes the failure that actually occurs.
- **Arm-boundary tracking in the volatile ratchet** (blind spot 4). Needs a
  parser; documented as the highest-value next step.
- **`VOLATILE-1`**, the `unused_variables` scoping — ~29 fixes, its own change,
  measured and recorded rather than guessed at.

## Result

`bash scripts/ci_local.sh --fast` → **119 passed, 0 failed, 5 skipped, ALL
GATES GREEN**, including clippy and rustfmt. The re-run was done on a clean
worktree so the pass stamp is valid; an earlier run was also green but was
unstamped because I appended to `backlog.md` while it was in flight.

Seven commits, one per finding, each with its mutation evidence in the message.
