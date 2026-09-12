# FOLLOWUP-2026-09-12-i686-cfg-slot-forwarding-and-a-narrow-store-miscompile.md

Session S19. Branch `s19`, commit `8d4ad8fc`, rebased onto `origin/main`
`2e6e04d4` (PR #511, *Global Location Allocation Phase 1*). Deliverable:
`ms178-1.patch` (94,126 B, one commit, `git am`-clean on pristine main).

Files touched: `src/backend/i686/codegen/peephole.rs` (+1,785/−39),
`scripts/godbolt.py` (+15/−4), and a new runtime regression test
`tests/regression/i686_narrow_store_alu_slot_source.{c,flags}`.

---

## 1. What was asked, and what "superior" had to mean

The mandate was explicit: S17's deletion of `global_store_forwarding` was a
mistake to be remedied, not defended; bring the capability back for i686 with a
**superior** design, fix the floating-point problems **properly**, and make it
as fast as possible — not the windowed `forward_slot_loads` substitute.

The capability that was lost is *cross-basic-block, loop-carried and
cross-call* frame-slot store→load forwarding. A windowed peephole cannot
express it: the proof that a register still holds a stored value has to survive
joins, back edges and calls, and that is a dataflow problem, not a pattern
problem. So "superior" was given a concrete meaning here:

1. **Sound by construction, not by enumeration of the cases that happened to
   break.** The deleted pass had four independent soundness defects (§2). Each
   is closed by a single shared mechanism that every forwarding decision goes
   through, and each is pinned by a test that fails if the mechanism is
   reverted.
2. **CFG-wide**, with a real meet operator at joins and correct handling of
   `%esp` movement, callee-pop, calls, `setjmp` and escaping addresses.
3. **Cheap enough to run unconditionally**: one linear dataflow pass per
   function, 40k-line budget, 48-slot cap, no fixpoint iteration over the CFG.
4. **Measured against the oracle compilers**, not against itself (§6).

## 2. The four defects of the deleted pass, and how each is closed

| # | Defect in the deleted `global_store_forwarding` | Closure in the new design | Pinned by |
|---|---|---|---|
| a | Byte-range invalidation was wrong: an overlapping write of a *different* width did not kill the cached value | one shared `frame_write_extents` / `frame_kill_extents` core, width-exact, mechanically enumerated against **all 47 frame-operand shapes** the corpus emits | `wide_x87_store_kills_overlapping_integer_slot_forwarding`, `wide_sse_store_kills_overlapping_integer_slot_forwarding`, `slot_load_not_forwarded_across_indexed_frame_store` |
| b | The write check only recognised `(%ebp)`; every `%esp`-relative write was invisible | both bases tracked in one slot table | `esp_adjustment_fences_esp_slot_forwarding`, `alu_slot_forward_handles_negative_ebp_slots` |
| c | `-off-1` vs `ESP_SLOT_BIAS` disagreed, so **all 100,442 `%esp` operands** in the corpus were rejected and the pass was a no-op | the bias is the single source of truth; `%esp` keys are renumbered on every stack move | `cfg_fwd_*` suite + the corpus A/B itself (the pass now fires on 139 TUs) |
| d | `%esp` movement was never fenced: a `push`/`pop`/`call`/`ret $N` silently renumbered every slot | `shift_esp(-pop)` — a callee pop **raises** `%esp`, so displacements **shrink**; only `%esp`-biased keys (≥ `ESP_SLOT_BIAS`) are shifted, testing the base *before* shifting; `%ebp` slots never move | `esp_adjustment_fences_esp_slot_forwarding`, `cfg_fwd_push_does_not_forward_the_pre_push_displacement` |

Defect (a) is worth dwelling on because of *how* it was found. A hand-written
mnemonic table of "instructions that write the frame" is unfalsifiable: it
looks complete until it isn't. Instead every distinct frame-operand shape in
the corpus was enumerated mechanically (47 of them), and each was classified by
what it does to memory. That is how `fstenv`/`fnstenv` turned up — a **28-byte
write** that no mnemonic table listed, and which would have silently corrupted
any forward across it. The rule adopted: never trust a hand-written table about
the corpus; enumerate the corpus.

Three further bugs were found while making the dataflow correct, each of which
had defeated an earlier attempt:

* **Entry stamps.** A register live at function entry must be stamped
  `REG_DEF_ENTRY`, not `CONFLICT`. Forwarding only has to prove "not redefined
  since the store", which is independent of where the value came from;
  `CONFLICT`-stamping entry refused essentially every forward whose source
  register was not defined inside the function. `CONFLICT` is meaningful only
  at a join where predecessors disagree.
* **Label leaders.** A label *heads* its block (`starts.push(j)`). Pushing
  `j+1` split every function into at least two blocks and defeated the
  single-block bail-out, so the pass ran (and cost time) where it could not
  possibly win.
* **Metadata directives.** `.cfi_*`, `.loc` and `.p2align` follow every call
  and every prologue. Killing forwarding state at a directive destroys exactly
  the cross-call forwards the pass exists for. Only real data directives
  (`.byte`, `.long`, `.ascii`, `.section`) kill state now, via
  `is_metadata_directive`.

## 3. Design

Per function: split into blocks at labels and branch targets; run one forward
dataflow pass; state is a slot table (value + defining register + width) plus
eight register stamps. Meet at a join is **intersection** (a slot is known only
if every predecessor agrees). Terminators: calls kill caller-saved registers
but *not* slot state; `push`/`pop`/`ret $N` renumber `%esp`-biased keys;
`setjmp` in the function bails out entirely (longjmp can restore any `%esp`);
an address of a slot escaping (or any indirect memory write when the frame is
not proven local) kills conservatively, while indexed frame accesses kill
unconditionally. Budget 40k lines, cap 48 slots — over either, the function is
skipped, which costs completeness and never costs correctness.

Only GP `movl`/`movw`/`movb` participate. `fstpl S; fldl S` is **never**
collapsed: the x87 store family is audited as writes (§2a), and an FP
round-trip through memory can change the value (double-extended precision,
exception flags), so there is no sound fold to be had at this level.

**Placement matters and is not a style choice.** The pass nops redundant
reload lines. Any pass that deletes lines other passes pattern-match on must
run *after* them: an earlier placement deleted anchors that downstream folds
needed, and the resulting failures looked like bugs in the folds. It runs at
the head of phase 4.

**Kill switches:** `CCC_NO_GLOBAL_SLOT_FWD`, `CCC_NO_SLOT_FWD_PRECISION`. Both
are honoured by every harness used here, and the off-arm was verified to be
byte-identical to base on 793 of 794 TUs (§5) — i.e. the harness really does
measure the pass and not a rebuild artefact.

### 3.1 The windowed pass was deliberately left alone (measured negative result)

Upgrading the historical windowed `forward_slot_loads` to the precise model was
implemented and measured, and it is **net-negative**: worth 417 of 462 modelled
sites in theory, but −5 instructions corpus-wide and **+2 on `loop_rotate`**,
because it stole the load-op-store anchor that `collapse_slot_rmw_i686` needs.
The windowed pass keeps its historical point window; precision lives only in
the CFG pass. This is recorded so nobody re-derives it.

## 4. A silent miscompile the new fuzzer found on `main`

This was not the target of the session and it is not caused by the new pass —
both lccc arms (pass on and pass off) disagreed with gcc, which is precisely
what proved it predates the work.

**Symptom.** `movw %ax, 8(%esp)` followed by `addl 8(%esp), %esi` was rewritten
to `addl %eax, %esi`. A 16-bit store wrote two of the slot's four bytes; the
ALU operand reads all four; `%eax`'s upper half has nothing to do with the
slot's upper half. In C: `u.h[0] = acc & 0xffff; acc += u.w;`.

**Root cause.** `rewrite_alu_slot_source` matched any of eight 32-bit ALU
mnemonics whose first operand was the slot text and rewrote it to the store's
source register. `forward_slot_loads` computed `store_bytes` and used it for
every overlap check — but the ALU-source arm never consulted it. The store's
width was simply not part of the decision.

**Scope.** i686 only, wrong at `-O1`, `-Os`, `-O2` and `-O3`; `-O0` is correct.
x86-64 was never affected: its store forwarder rewrites *loads* and requires
exact width, or the sound `qword→dword` case where the stored eight bytes do
define the low four. arm/riscv have no ALU-memory-source rewrite at all.

**Fix.** The width is now a parameter of `rewrite_alu_slot_source`
(`store_size != MoveSize::L` refuses), so no caller can forget it. Making it a
parameter rather than a check at the call site is the point: the call site is
where the bug was, and a guard there can be removed by the next refactor.

**Minimisation, and a trap worth recording.** A line-level ddmin reduced the
20-line fuzzer case to 10 lines — and the result was worthless: it had deleted
the *initialising* store, so the program read an uninitialised union. gcc and
lccc may differ legally on UB, so the "minimal reproducer" proved nothing. The
minimiser now pins initialising stores (`tools/minimize.py`), which yields a
13-line, fully defined reproducer. Lesson: a minimiser for a miscompile must
preserve definedness, not just the divergence.

**Tests.** `alu_slot_use_not_forwarded_from_narrow_store` (unit, both `movw`
and `movb`) — **verified to fail when the guard is neutered**, so it pins the
fix rather than passing vacuously — plus
`tests/regression/i686_narrow_store_alu_slot_source.c`, which runs against the
gcc oracle: gcc PASS, pre-fix lccc FAIL (`s1=50964873` vs `1124575625`),
post-fix PASS. Reference values agree across gcc `-O0/-O1/-Os/-O2/-O3`.
`word_pun` is the discriminating case; `byte_pun` and `cmp_pun` were already
correct and stay as guard-rails for the `MoveSize::B` path and for a narrow
store to the *high* half feeding `cmpl`/`xorl`/`andl`.

**Cost of the fix: nothing measurable.** `base` vs the both-kill-switches-off
arm over 794 TUs: **+0 instructions, +1 slot reference, 0 TUs whose instruction
count changed**; byte-identical everywhere except the reproducer itself. A
narrow store could in principle be merged with an older full-width store of the
same slot to recover the wide value, but that needs a per-byte slot model; for
one idiom in 809 TUs that is a bad trade, and the refusal is exact.

## 5. Evidence

Corpus: 809 C/C++ TUs under `tests/`, i686, fastbuild; 794 compile in every
arm (the same 15 fail in all four arms, so they are not arm-dependent).

### 5.1 Static, default configuration (Location Allocation Phase 1 off)

| arm | insns | slot refs |
|---|---|---|
| `base` (main `2e6e04d4`) | 270,945 | 102,007 |
| `off` (both kill switches) | 270,945 | 102,008 |
| `prec` (precision switch only) | 270,945 | 102,008 |
| **`both` (pass on)** | **270,698** | **101,602** |

**−247 instructions (−0.0912%), −405 slot references, 108 TUs improved, 0 TUs
regressed.** `off ≡ prec ≡ base` except for the single TU the miscompile fix
changes; the numbers are identical to the pre-rebase measurement, i.e. the
rebase did not perturb them.

### 5.2 Static, with upstream's Location Allocation Phase 1 **on in every arm**

PR #511 landed `CCC_RA_GLOBAL_LOCATION` (default off) while this work was in
flight, so the interaction was measured rather than assumed:

| arm (all with `CCC_RA_GLOBAL_LOCATION=1`) | insns | slot refs |
|---|---|---|
| `base` | 270,957 | 101,835 |
| **`both`** | **270,697** | **101,420** |
| **delta** | **−260** | **−415** |

112 TUs improved, **0 regressed**. Phase 1 alone is worth +12 instructions and
−172 slot references on i686; the two are **complementary, not redundant** — my
pass recovers slightly more on top of Phase 1 than it did without it.

### 5.3 Runtime

`lz4_compress` (the binding −40.53% constraint) and seven further benchmarks
come out **hash-identical** between arms, so no regression is possible there by
construction — a stronger guarantee than any timing. Eight benchmarks have
differing assembly and were timed.

Two methodologies were run, and they disagree in a way that matters:

| statistic | value | verdict |
|---|---|---|
| sequential min-of-N (N=9), all 8 timed benchmarks | geomean **+1.33%** | unusable — see below |
| sequential min-of-N, floor-passing 6 only | geomean +1.69% | still unusable |
| **paired-alternating (96 rep-pairs), floor-passing 6** | **median 0.9993 (−0.07%), geomean 1.0042 (+0.42%)** | **neutral** |

The sequential number is an artefact of two harness defects, both now fixed in
`tools/slot_fwd_runtime.py`:

* **No timing floor.** `fib` runs in 1.2–1.3 ms and `vecreg_new_ops` in
  1.3–1.5 ms — below process-spawn and scheduler-quantum scale on this box.
  They reported +7.8% and +12.5% and moved the geomean by more than a percent
  between two runs of the *same binaries*. Rows under `RT_FLOOR_MS` (default
  20 ms) are still printed but labelled `SUB-FLOOR` and excluded, so the
  exclusion can never be silent.
* **Sequential arms.** Timing one arm N times then the other lets drift,
  frequency scaling and page-cache warming land on one arm only. `RT_PAIRED=1`
  interleaves them within every repetition (off,on,on,off) and takes a
  per-repetition ratio.

The instability is visible per benchmark: `sqlite_varint` shows a min-of-N ratio
of **1.0982** and a paired median of **1.0016** — the min-of-N figure is nearly
10% and the paired figure is 0.2%. Per-benchmark paired medians:
`fannkuch` **0.9805** (3.1 s per run, the only benchmark long enough to time
cleanly, and it is ~2% *faster*), `zlib_ng_adler32` 0.9913, `sqlite_varint`
1.0016, `tls_seg_access` 1.0036, `sieve` 1.0084, `arith_loop` faster on both
statistics (min 0.9571).

**Verdict: neutral — no measurable effect in either direction.** Instruction
count is not runtime, and a −0.09% code-size change has no reason to show up in
wall clock. This is reported as such rather than dressed up as a win, and the
one benchmark with enough runtime to resolve a 2% effect went the favourable
way.

### 5.4 Correctness

* **Differential fuzzer** (`tools/slot_fwd_fuzz.py`, 12 generators: punning,
  dynamic index, cross-block, loop-carried, across-call, `%esp` movement,
  volatile, `setjmp`, x87 overlap, escape, wide/narrow, struct fields): **864
  cases** this session. The last three 240-case seeds are **240/240 clean** —
  zero divergences between the pass-on and pass-off arms and zero disagreements
  with gcc. Before the §4 fix, 36 cases had *both* arms disagreeing with gcc;
  after it, none.
* Two fuzz generators were themselves unsound and were fixed before their
  verdicts were trusted: an unterminated buffer made `strlen` read out of
  bounds, and cast-based punning violated strict aliasing. A fuzzer whose cases
  are UB cannot validate anything.
* **Execution differential** over the corpus against gcc `-m32`: no new
  miscompiles introduced (new-vs-oracle equals base-vs-oracle), and the two
  off-vs-base differences are benchmarks that print elapsed wall-clock time
  (verified stable across three runs of the same binary).
* **Test suite (post-rebase, commit `8d4ad8fc` on `2e6e04d4`):**
  `cargo test --profile fastbuild` **2,606 passed / 0 failed**; the `peephole`
  filter reports **980 passed / 0 failed** (946 before the rebase plus the
  x86-64 peephole tests PR #511 added), of which **32 are new here** (31 tests
  and one CFG driver). All 202 test/helper functions present at HEAD are still
  present (set-difference checked by name, not by count). `cargo fmt --check`
  clean, `cargo clippy --profile fastbuild --all-targets` clean (0 warnings).
* **CI gate:** `scripts/ci_local.sh --fast` → **ALL GATES GREEN, 30 passed /
  0 failed / 3 skipped, rc=0**, including the regression corpus and
  `codegen-quality-gate` (all golden workloads within tolerance).
* **Regression harness:** `run_regression.py --filter i686_narrow_store` →
  1 passed, 0 failed, gcc comparison executed (0 skipped-compare).

## 6. Oracle comparison (Compiler Explorer, `-O2 -m32`, AT&T)

`gcc -m32` disables SSE2 and is therefore the only honest i686 oracle; clang,
icc and icx emit SSE2 for FP at `-m32`. Both kernels below are pure integer, so
all four are comparable here.

| kernel | lccc | gcc 16.2 | clang 23.1.0 | icc 2021.10.0 | icx latest |
|---|---|---|---|---|---|
| `word_pun` (the §4 reproducer) | 30 | 15 | **9** | 19 | **9** |
| `main`, `narrow_shift_count_ge_width.c` (biggest forwarding win) | 498 | **291** | 337 | 304 | 300 |

Artifacts and manifests: `results/godbolt_narrow_store/`,
`results/godbolt_fwd_kernel/`. `scripts/godbolt.py audit` confirms every pinned
oracle alias resolves and is current.

**What this says, plainly: lccc loses here, and not because of forwarding.** On
`word_pun`, clang and icx keep the union in *registers* and constant-fold the
whole punning chain into nine instructions; lccc homes the union in a stack
slot and pays 30. Forwarding makes the slot traffic cheaper; only location
allocation removes it. On the 498-vs-291 kernel the same mechanism dominates.

A tool bug was fixed to get these numbers honestly: ICC labels its entry block
`word_pun.:` (a trailing dot, not a valid C identifier), and the body scanner
treated it as a new symbol, truncating the function to one instruction and
printing **icc=1 against clang=9**. A bogus "optimal" oracle row is worse than
a hard failure because it can make a bad compiler look perfect — the exact
failure mode `_label_is_function` was written to prevent. icc is 19.

## 7. Red-team audit of this session's own work

**Where I disagree with my own framing.**

1. *"−247 instructions is a win."* Only in the weakest sense. It is 0.09%,
   below the noise floor of every benchmark in the corpus, and §30 says a P0
   counts only if a realistic benchmark wins and nothing relevant regresses.
   By that standard this is **not** a P0 completion; it is a code-size and
   memory-traffic hygiene improvement plus a correctness fix. The claim I stand
   behind is narrower: 108–112 TUs strictly smaller, zero larger, zero runtime
   regressions, one miscompile class closed.
2. *"Restoring the pass was the highest-value work available."* I did it
   because it was mandated, and the evidence says it was **not** the
   highest-value work. Frame-slot forwarding is now ~93% exhausted (431 of 462
   modelled sites realised). The remaining ~33k GP slot loads are not a
   forwarding problem at all — they are locals living in memory that the
   oracles keep in registers (§6). The lever is location allocation
   (`memory.rs:470` for the x87 location model; `location_alloc.rs` for the
   general case), and PR #511's Phase 1 is the right direction.
3. *"The pass is sound."* Soundness rests on the 47-shape enumeration, which is
   strong *evidence* over the corpus but not a proof over all inputs; a frame
   operand shape the corpus never emits could still be mishandled. The
   mitigations are structural (every decision goes through one shared
   width-exact extent function; unknown shapes kill state) and empirical (the
   fuzzer), and the failure mode of a missed shape is a wrong forward — the
   class §4 shows is worth pinning with tests, not with confidence.
4. *"The soundness fix is free."* True on this corpus (+0 instructions, 1 TU).
   It is not free in general: it deletes an optimisation that a per-byte slot
   model would keep. I accept that trade and record it as a follow-up rather
   than claiming the opportunity does not exist.
5. *"My pass is not obsoleted by Location Allocation."* Measured, not assumed
   (§5.2) — but with a falsifiable prediction attached: as Phase 2+ promotes
   more locals to registers, my delta **should shrink toward zero**. If it
   grows instead, my model of the interaction is wrong and one of the two
   passes is doing something other than what its documentation claims.
6. **Test coverage of the pass itself is thinner than the case count suggests.**
   864 fuzz cases produced only ~20 instruction-count differences per 240
   cases: roughly 8–12% of generated programs exercise a forward at all. The
   suite is broad in *shape* and shallow in *hit rate*. Raising the fire rate
   (more multi-block slot idioms per generator) is the single highest-value
   improvement to the harness.
7. **A maintenance hazard I am leaving behind.** The pass nops reload lines and
   must run after passes that pattern-match them (§3). That invariant lives in
   a comment and in one test, not in the architecture. A future pass added to
   phase 4 after this one can reintroduce anchor theft, and the symptom will
   look like a bug in the *other* pass.

**Where I agree with the mandate and the earlier adjudications.** S17's
deletion was the right call *at the time* (the pass was unsound in four
independent ways) and the wrong call *as a final state* (the capability was
real and is now restored soundly). The S18 x87 GP-pair folds remain correct as
an interim mitigation; they are not in conflict with this pass, and
`fstpl/fldl` is still deliberately not folded here.

## 8. Follow-ups, in priority order

1. **P0-A location allocation** — the real lever (§6, §7.2). Target the ~33k
   remaining GP slot loads and the x87 location model at `memory.rs:470`;
   continue PR #511's Phase 1 rather than adding more peepholes.
2. **Raise the fuzzer's fire rate** (§7.6) so the pass's own coverage is
   proportional to its risk.
3. **Per-byte slot model** — would recover the narrow-store→wide-ALU forward
   refused in §4 and would make the 47-shape enumeration a byte-range
   computation instead of a table.
4. **Phase-ordering invariant** (§7.7) — make "nops reloads, so runs last" a
   property the pipeline can check, not a comment.
5. Re-run the §5.2 interaction A/B after each Location Allocation phase
   lands; the prediction in §7.5 makes the result interpretable either way.

## 9. Harnesses added this session (kept outside the repo tree)

| path | purpose |
|---|---|
| `tools/slot_fwd_fuzz.py` | 12-generator differential fuzzer: lccc-on vs lccc-off vs gcc, with instruction counts and a fire-rate metric |
| `tools/minimize.py` | line-level ddmin for a gcc-vs-lccc divergence, **pinning initialising stores** so the minimal case stays defined |
| `tools/fwd_ab.py` | 4-arm static A/B (base/off/prec/both) with byte-identity and per-TU regression detection; now accepts extra env merged into every arm (used for the Location Allocation interaction) |
| `tools/slot_fwd_runtime.py` | 3-arm runtime A/B, hash-first, only times benchmarks whose assembly actually changed; `RT_PAIRED=1` interleaves the arms within every repetition and `RT_FLOOR_MS` (default 20) labels and excludes sub-floor benchmarks from the geomean |
| `tools/exec_diff.py` | corpus execution differential against the gcc `-m32` oracle |
