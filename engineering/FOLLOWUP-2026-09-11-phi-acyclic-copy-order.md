# Phi elimination follow-up — cycle-accurate parallel-copy resolution, gated on a measured allocator prerequisite

**Date:** 2026-09-11

**Base:** `4f527199b9f4b94dfdae2ad52638caebf973483a` (latest `ms178/lccc` main merge, PR #495)

**Scope:** how `src/ir/mem2reg/phi_eliminate.rs` resolves the *simultaneous* assignment denoted by one predecessor edge's phi copies — specifically which copies get a shared temporary and in what order the rest execute. Not a peephole, not a backend change, and deliberately not a loop transform.

---

## UPDATE (same day, post-rebase onto `25ed36de`) — the blocker diagnosis below is SUPERSEDED

Everything after this line was written against base `4f527199` and its central
claim is **wrong**: the resolver's runtime cost was *not* caused by a
loop-carried range being lengthened across the back edge, and RA-06 location
pieces are *not* the prerequisite for shipping it.

The real cause was an allocator bug the resolver merely exposed.
`live_range::mark_loop_spanning` derives `span_has_in_loop_use` by summing
`uses_in_extents` over a coalesce leader and its members, but `uses_in_extents`
was populated by a pass over `ranges` — and a merged member owns no `LiveRange`.
Every member lookup missed, so the sum the code documents as "web-wide" was a
silent no-op, and the flag degraded to *does the leader have an in-extent use*.
For a phi web led by a cold preheader definition — every loop-carried recurrence
— that is false, so the in-loop-USELESS-span admission rule demoted the hottest
values in the loop. On `sha256_transform` those are exactly `leader=v166
members=[166,389]` (`Load state[0]`) and `leader=v182 members=[182,392]`
(`Load state[4]`), reloaded 7× per iteration each.

That is fixed and landed (web-wide in-loop-use supply, kill switch
`CCC_NO_WEB_INLOOP_USE`, gate `tests/regression/check_ra_web_inloop_use.sh`).
Re-measured amplified on the rebased base, the factorial **inverts** the
disposition recorded below:

| leg | 51 rounds | 41 rounds | p |
|---|---|---|---|
| base → RA fix alone | **+3.63 %** | **+4.33 %** | 0.0000 / 0.0000 |
| RA fix → + this resolver | −1.99 % | −0.71 % | 0.0008 / 0.0288 |
| base → both | +1.88 % | +1.98 % | 0.0000 / 0.0002 |

So the resolver stays opt-in, but for a *different and much smaller* reason: on
top of the allocator fix it costs ~1–2 %, not ~5–7 %. The `+8.21 %` recorded for
"both together" earlier the same day is **withdrawn** — it was an un-amplified
(~55 ms/arm) median that disagreed with its own min ratio (+3.8 %), violating the
harness's `median_and_min_agree` criterion, and it did not survive the rebase
onto a main that changed eviction in `select_evict_victim` (`evict_short_k`).

Current numbers, corrected: `sha256_transform` is 198 insns / 61 stack refs
shipping (base 210 / 62) and 178 / 28 with the resolver; gcc is **142 / 8** and
**44.10 % faster** at runtime (median ratio 1.4410, min 1.4361, p=0.0000). Both
compilers emit the same number of loops here (2 backward jumps each), so the gap
is not loop structure — it is 54 extra frame-relative stack references, ~40 extra
`mov`s, 9 extra labels and 6 extra compares.
An earlier reading of this table reported gcc at 240 / 31 and concluded LCCC won;
that came from a function extractor matching `.size <name>` with a literal space,
which silently fell back to the whole translation unit on gcc's tab-separated
`.size\tsha256_transform`. Both gates now fail loudly instead of falling back.

Raw record: [`evidence/ra-web-inloop-use-2026-09-11/`](evidence/ra-web-inloop-use-2026-09-11/).

---

## Decision

**Landed, opt-in** (`CCC_PHI_ACYCLIC_ORDER=1`). **Not the default.**

The cycle-accurate resolver is semantically superior, is exhaustively validated,
and is a large static win wherever the region is not register-saturated. It is
nonetheless gated because it costs **~5–7% runtime on `sha256_transform`**, the
most register-saturated kernel in the corpus, *while lowering* that function's
static instruction and stack-reference counts. Shipping it default-on would
trade a static win for a runtime loss on the kernel that matters most.

The blocker is characterised below and is an **allocator** problem, not a
phi-elimination problem. *(The characterisation below blamed a lengthened
back-edge live range; see the UPDATE above — the actual cause was a no-op
web-wide in-loop-use aggregation in `mark_loop_spanning`, now fixed.)*

**Zero-risk property, proved not assumed:** the shipping default arm emits
**byte-identical assembly to base `4f527199` across 450 translation units**
(51 `tests/benchmark/programs/*.c` + 399 further `tests/**/*.c`; 1 file is not
compilable by either arm). The measured regression is therefore entirely
contained behind the flag.

---

## Root cause of the original defect

`eliminate_phis` lowers a block's phis to a *simultaneous* assignment
`dest_i <- src_i`. Executing that as an instruction sequence is only sound when
every copy reads its source before another copy overwrites it. The classical
solution decomposes the copy graph into **cycles**, which need one shared
temporary each, and **chains**, which need none and are simply executed in
reverse-topological order.

The decomposition this pass performed was much coarser: it treated *"my source
is somebody's destination"* as a cycle. That predicate is true of **every** copy
in a rotation, so an acyclic chain such as SHA-256's eight-word state rotation
(`h=g; g=f; f=e; …; b=a`) was routed through the two-phase temporary scheme in
full.

The cost is not one redundant instruction — it is a redundant **web**. Each
temporary is a fresh value with its own live range, so the register allocator
sees twice as many loop-carried values and twice as many unconditional copies
per iteration. SHA-256's rotation is the extreme case: 16 webs and 8 header
relay copies per round where the rotation needs 8 webs and 6 moves. Over budget,
the allocator staged the whole rotation through the stack.

Measured on the reduced rotation kernel (`rot()` in the CI gate):

| | instructions | stack refs |
|---|---|---|
| base `4f527199` | 73 | 27 |
| gcc `-O2` | 71 | 0 |
| **opt-in resolver** | **55** | **0** |

---

## The fix

`plan_edge_copies_acyclic` builds the precedence graph `i -> j` meaning *"copy
`i` must execute before copy `j`"* — which holds exactly when `i` reads the
destination `j` writes — and runs **Kahn's algorithm** over it. Every copy whose
prerequisites are satisfied is emitted in order; whatever remains when the queue
drains is blocked by a genuine cycle and gets a shared temporary. Self-copies
(`dest == src`) write nothing observable and are excluded from both sets.

Cycle detection is therefore **exact**, and a true rotation still keeps its
temporaries — which is the only correct resolution for one.

`plan_edge_copies_legacy` retains the historical over-approximation verbatim as
the production default and as the A/B reference.

### Policy is threaded, not probed

The gate is read **once per function** into `PhiElimCtx::copy_order`
(`CopyOrderPolicy::{Legacy, Acyclic}`) rather than probed from the environment
inside `plan_edge_copies`, which runs once per (block, edge). This removes a
per-edge `getenv` and — the reason it matters here — makes
`eliminate_phis_with_policy` reachable from the unit tests, so the end-to-end
rotation pin can exercise the opt-in arm **without mutating the process
environment**, which would race the parallel test threads (the hazard `MS-04`
exists for).

---

## Why it is not the default: the measured regression

### Runtime, paired and interleaved

`scripts/perf_ab.py`'s aggregate verdict is unusable here (see *Harness
finding*). The measurements below come from a paired harness — interleaved
rounds, arm order alternated each round, same binary pair — since promoted into
the repo as **`scripts/paired_ab.py`**, which additionally hashes both arms and
refuses to emit a verdict when they are byte-identical, and screens stdout/exit
status agreement *before* timing anything. It reports a paired sign test and
flags when `min` and `median` disagree on direction — which they did on an
earlier 21-round run and do not on the retained 25-round one, a reminder that
the ratio alone is not the evidence. The mechanism below is:

`sha256_transform` at `-O2 -DPASSES=4 -DBLOCK_COUNT=32768`, 25 counted rounds
after 2 discarded warm-ups (raw samples and logs frozen under
[`evidence/phi-acyclic-copy-order-2026-09-11/`](evidence/phi-acyclic-copy-order-2026-09-11/)):

| arm | min | median | mean | sd |
|---|---|---|---|---|
| default | 38.68 ms | **40.30 ms** | 40.91 ms | 1.84 |
| opt-in | 40.65 ms | **42.34 ms** | 43.19 ms | 2.60 |

Opt-in is **5.08% slower**, `min` ratio `1.0511` agreeing with `median` ratio
`1.0508`, paired sign test **`p = 0.0003`**. The direction reproduces across
independent runs (7.05% at 25 rounds, 5.46% at 21 rounds, 5.08% here) — the
magnitude moves with VM load, the direction does not.

The one comparison that came out *neutral* is recorded because it is the
hypothesis that was falsified: rewriting the round loop's ten `movq` moves to
32-bit gives 339.2 vs 345.5 ms, i.e. 1.8%, noise.

`fib` is the only other corpus kernel whose code changes under the knob; at
~1 ms per invocation it is below this harness's resolution and its 4.84% delta
is not significant (`p = 0.23`). It is not claimed as a win.

### Mechanism: the copy was a live-range splitter

Slot-traffic census of the round loop:

| | memops/iter | distinct slots | max reads/slot |
|---|---|---|---|
| default | 25 | 11 | 2 |
| opt-in | **14** | **2** | **7** |

The opt-in arm issues *fewer* memory operations but concentrates them on the two
recurrence words (`a` = slot 32, `e` = slot 24), each re-read 7 times per
iteration — because a destructive rotate needs a fresh destination register per
use, so every use reloads.

`CCC_DEBUG_RA_PHASES` explains why those two specifically lose:

```
[RA-P1]     fn=sha256_transform pool=[PhysReg(1..6)] candidates=0 assigned=[]
[RA-FINAL]  fn=sha256_transform assigned=[…114 ids…] spilled=[2, 166, 182, 309]
```

The recurrence words appear in **neither** `assigned` **nor** `spilled`: they are
left stack-homed and *unassigned*.

* Under the two-phase scheme the loop-carried word `a` exists as **two short
  complementary ranges** — the phi input (latch → header) and the body value
  (header → latch) — and the allocator assigns both.
* Resolving the rotation acyclically **merges them into one range spanning the
  back edge**, and that longer range loses.

So the allocator's decision is sensitive to *live-range length*, and eliminating
redundant copies is **not monotone in code quality** while ranges are split only
at phi boundaries. Note the default arm has *higher* register pressure (more
live values) yet wins — count is not the operative variable, range structure is.

### Falsified alternatives — do not retry

* **Store-to-load width mismatch.** The rotation emits 64-bit `movq` stores
  feeding 32-bit `movl` loads. Hand-rewriting the round loop's ten moves to
  32-bit, assembling, and re-measuring changes runtime by **1.8% (noise)** and
  leaves the gap at 5.5%. All three binaries are output-identical
  (`054db5f638d89d8b`). **Not the cause.**
* **`CCC_EVICT_MODE=6`** and **`CCC_NO_TIER2_GRAPH=1`** do not change the victim
  set. Victim choice is made elsewhere (scan/coalescing).
* **Loop unrolling** does not address it; the defect is the copy-graph
  decomposition, upstream of the allocator.

---

## Harness finding (affects every future A/B here)

At the time of measurement the resolver was the default and the flag was its
kill switch, so the run was
`scripts/perf_ab.py --env CCC_NO_PHI_ACYCLIC_ORDER=1 --reps 9`; **that knob no
longer exists** — the polarity was inverted when the resolver became opt-in, and
the equivalent screen today is `--env CCC_PHI_ACYCLIC_ORDER=1`. It reported
*"A 2.08% slower"* as a corpus geomean. MD5-comparing the two arms' binaries
shows **6 of 8 benchmarks compile byte-identically** — their deltas are pure
noise, and the noise floor on this 2-core VM is **±4%**. Only `sha256_transform`
and `fib` change code at all.

An aggregate verdict computed over arms that are byte-identical is meaningless.
**Always MD5-compare arm binaries before trusting any ratio**; assert on
per-kernel deltas only where the binaries differ.

`scripts/paired_ab.py` now enforces this rather than leaving it to discipline:
byte-identical arms exit 3 with *UNINFORMATIVE* and no verdict.

`--allow-identical` reproduces the noise floor on demand, and the result is
worse than "noisy" — it is *nominally significant*. Two **byte-identical**
`base64_enc` arms, whose true effect is exactly zero, measured a **5.67%**
median delta with a paired sign test at **`p = 0.0164`**. Interleaving and order
alternation did not remove it: on a 2-core shared VM the bias is systematic and
correlates within a round, so a paired test inherits it instead of averaging it
out. **Statistical significance does not imply a real effect**, and no amount of
paired-round discipline substitutes for hashing the arms first.

Raw samples and logs for all three kernels are frozen under
[`evidence/phi-acyclic-copy-order-2026-09-11/`](evidence/phi-acyclic-copy-order-2026-09-11/).

---

## Programmatic tests

`src/ir/mem2reg/phi_eliminate.rs` — **32 tests, all passing**:

* **Positive:** chains need no temporary and order the reader first; the
  SHA-style rotation forest (two chains) resolves with zero temporaries;
  self-copies do not manufacture a self-cycle; constant sources impose no
  precedence.
* **Negative / near misses that must NOT fire:** a genuine 2-cycle keeps *both*
  temporaries and orders nothing; a 4-cycle is fully cyclic; a chain hanging off
  a cycle keeps its own order while the cycle members keep theirs.
* **Adversarial:** `resolved_order_is_semantically_equivalent_exhaustively`
  enumerates **18,240 copy graphs** (n = 2..5), applies each plan sequentially
  to a concrete state and compares against the simultaneous-assignment oracle.
  It also asserts **non-degeneracy** — if the resolver starts marking too many
  copies cyclic it fails loudly (the assertion that caught the flag flip:
  *"n=3: only 47/124 graphs resolved without a temporary"*). Both arms are held
  to the same oracle, so `Legacy` is proved *sound*, not merely conservative.
* **Determinism:** 32 repeated plans per policy, both policies.
* **End-to-end IR:** the rotation loop emits ordered direct copies with no
  temporaries and no header copies.
* Policy-agnostic invariants are checked under **both** arms via `BOTH_POLICIES`,
  so the suite cannot silently stop covering whichever one is not the default.

`tests/regression/check_phi_acyclic_order.sh` — wired into `scripts/ci_local.sh`
as gate **`phi-acyclic-copy-order`** (runs in ~1 s). Pins four properties:

1. **The mechanism fires** — under the flag, `rot()` has 0 stack refs, is
   strictly smaller than the default arm, *and* beats `gcc -O2` (55/0 vs 71/0).
2. **Near misses stay correct** — rotation + swap + 3-cycle, 65 trip counts each
   (sweeping across 0 so peel/rotate/exit all run), matching GCC bit-for-bit on
   stdout *and* exit status, **in both arms**.
3. **The gate is wired** — `CCC_PHI_ACYCLIC_ORDER=1` observably changes the
   emitted code.
4. **The opt-in does not leak** — unset, `=0`, empty and `=true` all reproduce
   the default arm byte for byte, so the flag is parsed strictly as `== "1"` and
   production codegen cannot be perturbed by a stray environment.

Its `sha256_transform` section is a **static** contract on purpose (known-answer
digest `ebf7d5612b4881d9` == GCC, plus lower insn/stkref counts under the flag).
A *runtime* assertion there would pin the regression in place.

### Mutation-verified in both directions

| mutation injected into `copy_order_policy()` | gate result |
|---|---|
| `false && …` (resolver never fires) | **FAIL**, 5 diagnostics |
| `true \|\| …` (resolver always on / leaks) | **FAIL**, 3 diagnostics |
| restored | PASS, all 4 properties |

---

## Validation summary

| check | result |
|---|---|
| `cargo test --lib` (full) | **2377 passed, 0 failed**, 6 ignored |
| `phi_eliminate` module | 32 / 32 |
| `scripts/ci_local.sh --fast` | **20 passed, 0 failed, 3 skipped** |
| `cargo clippy --all-targets -- -D warnings` | clean |
| `cargo fmt --check` | clean |
| default arm vs base `4f527199` | **450 / 450 TUs byte-identical** |
| `sha256_transform` known-answer | `ebf7d5612b4881d9`, matches GCC |

---

## Follow-up work — the prerequisite

Enabling this by default requires the allocator to split a loop-carried range
**profitably at the back edge**, so a value can hold a register inside the body
without lengthening the range that crosses the edge. That is the RA-06
"location pieces" item; until it exists the two-phase temps are doing that job
by accident, and only for cyclic-looking phi blocks.

Two further leads found while diagnosing, both independent of this change:

1. **The register budget, checked and closed.** An earlier reading of this
   session recorded `MACHINST_ALLOCATABLE_GPRS`
   (`src/backend/x86/codegen/machinst.rs:532`) as "reserving `rax`/`rcx`,
   leaving 13 homes". **That is wrong and is corrected here.** The constant
   lists **15** entries and its own doc comment states that `rax` (0) and `rcx`
   (7) *are* included, because the window allocator's interference model records
   every physical-register touch per instruction, so a scratch assignment is
   sound exactly when no recorded touch overlaps the scratch's live interval.
   What is true is narrower: those two are "never **main-RA** homed", so the
   main allocator works with 13 while the MachInst window allocator has 15.

   Widening the round loop's budget therefore means routing it through the
   MachInst window allocator — which is a **recorded measured negative**:
   `agent/RULES.md` item 16, "Do not force MachInst on large loops
   (`CCC_MI_MAX_LOOP_INSTS`; gzip −3%)". Pursued no further, per the rule that a
   rejected direction is not rebuilt without new evidence. The premise of the
   sha256 gap being a raw home-count shortage is **not established**; the
   evidence in this file points at range *structure* instead, which is what
   RA-06B addresses.
2. **x86 has no slot-load dedup — verified, and this lead survives.** The ARM
   backend has `eliminate_repeated_slot_loads`
   (`src/backend/arm/codegen/peephole.rs:2742`, gated by
   `CCC_NO_SLOT_LOAD_DEDUP`): it caches `(frame byte, is_word) -> register` and
   turns a repeated `ldr` of an unclobbered slot into a `mov`, dropping all
   state at stores through unknown bases, calls, and control-flow boundaries.
   A grep of `src/backend/x86/` finds **no equivalent**; the only hit is
   `memory_fold.rs`'s `refuses_when_a_slot_load_intervenes`, which is the
   opposite behaviour — x86 declines to fold *because* a slot load intervenes.

   Coalescing N reads of one slot into one load plus N−1 register uses is
   instruction-count-neutral but removes load-port uops and shortens the
   dependency chain, which is precisely the 7-reads-of-one-slot pattern measured
   above. It cannot help `sha256_transform` specifically (the loop has no free
   register to hold the cached value — that is the whole problem), so it is not
   the unblock for RA-06B. It is a real gap for every *other* stack-homed value
   with repeated unclobbered reads, and the ARM implementation is a working
   design to port, including its invalidation discipline and the
   sp-displacement aliasing hazard its comment documents.

Also worth noting for the RA-31 per-use cost work: the allocator's victim
selection is **latency- and foldability-blind**. `a` and `e` carry the highest
use counts in the round loop and were still the values demoted; any cost model
that weighted reads-per-slot would have chosen differently.
