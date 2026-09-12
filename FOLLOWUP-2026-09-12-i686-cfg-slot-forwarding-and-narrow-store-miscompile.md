# FOLLOWUP-2026-09-12-i686-cfg-slot-forwarding-and-narrow-store-miscompile.md

Session S19. **The first half of this series has merged upstream**: PR #512
(`bcfeeefe`) carries the CFG slot-forwarding pass, the narrow-store miscompile
fix, the `godbolt.py` icc-label fix, the S17 red-team audit and the first
edition of this document. Branch `s19` is therefore rebased onto `origin/main`
`0e4cf540` and now contains five commits of *new* work: `87ebbef7` (constant
call arguments stored directly instead of routed through `%eax`), `65ceb2a3`
(the oracle-methodology correction in §6 and the constant-fold decision record),
`4a68b490` (final-binary runtime and execution-differential evidence),
`da0000a0` (one shared constant-materialization model for both marshalling
paths, 6 unit tests and an end-to-end regression test — §6.4), and this final
commit (the numbers above, restated on the delivered tree).
Deliverable: `ms178-1.patch`, `git am`-clean on pristine main, six files:
`src/backend/i686/codegen/emit.rs` and `calls.rs` (the fold, its shared pure
helpers and 6 unit tests), `tests/regression/i686_const_stack_args.{c,flags}`
(the end-to-end test), this document, and a one-line stale-hash correction in
`engineering/AUDIT-2026-09-12-S17-redteam.md`.

The instruction-count totals below are stated against `2e6e04d4` (the
pre-PR-#512 baseline) so the whole session's contribution stays visible; the
`origin/main` row is now PR #512's state, i.e. the A/B **base** arm.

Headline numbers, i686, 809-TU `tests/` corpus, fastbuild:

| corpus | CFG slot forwarding | constant-argument fold | total |
|---|---|---|---|
| 794 TUs (`2e6e04d4` → PR #512 → fold) | −247 insns, −405 slot refs | −993 insns | **−1,240 insns (−0.458%)**, 270,945 → 269,705 |
| 795 TUs (delivered tree, binary `0fccd521`) | −246 insns, −406 slot refs | −999 insns | **−1,245 insns**, 271,233 → 270,234 |

The two rows differ by one translation unit — the regression test this patch
adds — and nothing else. That TU is 526 instructions with the fold gated off and
520 with it on, which is why the absolute totals shift by ~+535 between rows and
why the fold's own delta grows by exactly 6; the forwarding delta moves by one
for the same reason. Slot references are unchanged by the fold in both rows
(`movl %eax,N(%esp)` and `movl $imm,N(%esp)` reference the same slot), so all
−406 of them are the forwarder's.

298 TUs smaller and 0 larger from the fold (instruction counts; `.text` bytes
are +580 on a 210-TU sample, +638 of which is two memset TUs — see §6.3), 108
smaller and 0 larger from the forwarder. **16 of the 22-benchmark corpus come
out byte-identical between arms, `lz4_compress` among them**, so no regression
is possible there by construction; the five whose assembly changed are neutral
by paired measurement (median 0.9972, geomean 0.9991, none slower than 2%;
§5.3.1).

Files touched: `src/backend/i686/codegen/peephole.rs` (+1,785/−39),
`src/backend/i686/codegen/emit.rs` and `calls.rs` (+116/−4),
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
| `base` (pre-#512 main, `2e6e04d4`) | 270,945 | 102,007 |
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

### 5.3.1 Runtime, final binary (both transformations, 22-benchmark corpus)

Re-run on the delivered build (`sha256 e6acf596…`, non-zero-immediate fold) with
paired alternation and the 20 ms floor:

| | value |
|---|---|
| benchmarks with **byte-identical** assembly between arms | **16 of 22**, including `lz4_compress`, `nbody`, `matmul`, `mandelbrot`, `binary_trees`, `hash_table`, `strlen_bench`, `i686_alu_chains` |
| timed (assembly differs) | 5 — `fannkuch`, `sqlite_varint`, `sieve`, `arith_loop`, `fib` (sub-floor, excluded) |
| **paired-alternating aggregate (72 rep-pairs)** | **median 0.9972 (−0.28%), geomean 0.9991 (−0.09%)** |
| min-of-N geomean, floor-passing 4 | 0.9768 (−2.32%) — the optimistic statistic, quoted for completeness only |
| benchmarks slower than 2% | **none** |

The identity result is the load-bearing one: 16 of 22 benchmarks cannot regress
because the compiler emits the same bytes. Of the five that changed, the two
with enough runtime to resolve a 2% effect (`fannkuch` at 4.1 s, paired median
0.9975; `sqlite_varint` at 67 ms, paired median 0.9857) are both marginally
faster, and `sieve`/`arith_loop` are inside noise in both directions depending
on the statistic. **Verdict: neutral-to-marginally-favourable, no regression
beyond noise, and the binding `lz4_compress` constraint is satisfied by hash
identity rather than by measurement.**

One harness note: `fp_scalar_webs` fails to *link* under the 32-bit oracle in
this environment (missing `lib32` startup objects) in every arm, including
unpatched `origin/main`; `exec_diff` reports the same 71 link failures for all
four arms, so it is an environment limit, not a result.

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
* **Execution differential** over the corpus against gcc `-m32`, re-run on the
  final binary (`tools/exec_diff.py`, 809 TUs × 4 arms): **666 TUs build and run
  under the gcc oracle**; **21 disagree with gcc — exactly base's 21**, so no new
  miscompile is introduced. The only two new-vs-base differences
  (`i686_alu_chains.c`, `fp_liveness_ptr_deref_alias_negative.c`) also differ
  with every transformation gated **off**, and each printed a *third* distinct
  value across arms (9.03643e+107 / 1.1651e+108 / 1.47702e+108) — they are
  nondeterministic TUs, not patch effects. Stage census per arm: base 719 run /
  71 link / 15 compile / 4 timeout; patched 722 run / 71 link / 15 compile /
  1 timeout; kill-switch arm identical to patched.
* **Test suite** (measured at `8d4ad8fc` on `2e6e04d4`, now upstream as
  `bcfeeefe`, and re-run on the rebased tree at `87ebbef7` on `0e4cf540` —
  the rebased build is byte-identical, `sha256 e6acf596…`, so these are the
  delivered binary's numbers):
  `cargo test --profile fastbuild` **2,606 passed / 0 failed**; the `peephole`
  filter reports **980 passed / 0 failed** (946 before the rebase plus the
  x86-64 peephole tests PR #511 added), of which **32 are new here** (31 tests
  and one CFG driver). All 202 test/helper functions present at HEAD are still
  present (set-difference checked by name, not by count). `cargo fmt --check`
  clean, `cargo clippy --profile fastbuild --all-targets` clean (0 warnings).
* **CI gate:** `scripts/ci_local.sh --fast` → **ALL GATES GREEN, 30 passed /
  0 failed / 3 skipped, rc=0**, including the regression corpus and
  `codegen-quality-gate` (all golden workloads within tolerance).
* **Unit tests for the constant fold** (6 new, `emit.rs`): a bit-exactness
  table over all eleven `IrConst` arms (i64/i128 low-half truncation, `F32`
  bit pattern, `F64` *low half* of the bit pattern, `D32`/`D64` BID patterns,
  `LongDouble` leading bytes, `I8`/`I16` sign extension, `-0.0f` = `0x80000000`
  ≠ zero); which arms emit `xorl` for zero; the fold predicate re-derived from
  byte-count constants at **post-peephole** cost for both displacement forms;
  the arm-independent zero refusal as a tripwire against removing that peephole;
  and every non-zero immediate folding in every arm. Lib tests: **2,618**.
* **End-to-end regression test** `tests/regression/i686_const_stack_args.c`
  (+`.flags`, `-m32 -O2 -fno-pic`): a `noinline` 13-argument callee and an
  8-argument callee, all constant kinds, zeros in every position, 8-byte
  arguments interleaved to shift every following slot offset, plus the operands
  that must keep using `%eax` (a variable, an alloca ADDRESS, a global address,
  a function pointer) and a variadic `printf`. Checksums are over argument *bit
  patterns*. Reference values are gcc `-m32`, identical at -O0/-O1/-O2/-O3;
  lccc passes at all four levels **with the fold on and with it gated off**, so
  both marshalling paths are pinned. The test is not vacuous: the fold fires 36
  times in it against 29 with the gate set, 520 instructions against 526.

## 6. Oracle comparison (Compiler Explorer, AT&T), and a correction to it

`gcc -m32` disables SSE2 and is therefore the only honest i686 oracle for FP;
clang, icc and icx emit SSE2 at `-m32`. The kernels below are pure integer, so
all four are comparable.

### 6.1 The first comparison was unfair, and by how much

The initial run compared lccc's **default PIC** output against the oracles'
**non-PIC** output at `-O2 -m32`: lccc emitted `__x86.get_pc_thunk.bx` and 60
`@GOTOFF`/`@PLT` relocations, the oracles none. Re-run with `-fno-pic` on both
sides:

| kernel (`-O2 -m32`) | lccc | gcc 16.2 | clang 23.1.0 | icc 2021.10 | icx |
|---|---|---|---|---|---|
| `main`, `narrow_shift_count_ge_width.c` — **mismatched PIC** | 498 | 291 | 337 | 304 | 300 |
| same, both `-fno-pic` | **403** | **291** | **295** | — | **300** |
| `word_pun`, `i686_narrow_store_alu_slot_source.c` — mismatched PIC | 30 | 15 | 9 | 19 | 9 |
| same, both `-fno-pic` | **30** | **15** | **9** | **19** | **9** |

`word_pun` is a leaf function that touches no globals, so PIC costs it nothing
and both rows agree — re-measured under matched flags rather than assumed
(`results/godbolt_wordpun_noppic/`). The slot-heavy kernel is the one where the
code model mattered.

**95 of the 207-instruction "gap" was a harness artefact.** The honest gap is
112 instructions (1.38×), not 1.71×, and against clang/icx it is 1.37×/1.34×
rather than the 1.48×/1.66× the unfair numbers implied. Any oracle comparison
that does not pin the code model on both sides is measuring the flag, not the
compiler. Artifacts: `results/godbolt_fwd_kernel/` (mismatched),
`results/godbolt_noppic/` (matched), `results/godbolt_narrow_store/`.

A second tool bug was fixed to get any of these numbers honestly: ICC labels its
entry block `word_pun.:` (a trailing dot, not a valid C identifier), and the
body scanner treated it as a new symbol, truncating the function to one
instruction and printing **icc=1 against clang=9**. A bogus "optimal" oracle row
is worse than a hard failure because it can make a bad compiler look perfect —
the exact failure mode `_label_is_function` was written to prevent. icc is 19
(on `word_pun`, where lccc is 30 and clang/icx are 9).

### 6.2 What the fair numbers actually say, and the second root cause found

Classifying every `movl` in the matched-flag artifacts locates the gap
precisely:

| | lccc | gcc 16.2 |
|---|---|---|
| instructions | 403 | 291 |
| instructions with a memory operand | **148 (37%)** | **4 (1%)** |
| `movl` total | 236 | 22 |
| — reg→slot (homing / argument staging) | 111 | 0 |
| — reg→reg copy | 41 | 10 |
| — imm→reg | 38 | 1 |
| — slot→reg reload | 12 | 0 |
| `pushl` | 4 | 132 |

gcc holds live values in callee-saved registers and passes arguments with
`pushl`; lccc homes values in a 60-byte frame and stages arguments through
`%eax` into `disp(%esp)`. Two distinct defects, in order of what they cost:

1. **Every stack argument was routed through `%eax`** — `operand_to_eax(arg)`
   followed by `movl %eax, N(%esp)` — even when the argument is a constant that
   could be stored directly, which is what gcc/clang/icx all do. Fixed in
   `87ebbef7` at the emission site (§6.3).
2. **Values that gcc keeps in `%ebx`/`%esi`/`%edi` are homed in slots.** This is
   the larger term and it is *location allocation*, not forwarding: upstream's
   `location_alloc.rs` Phase 1 measures at −4 slot references corpus-wide on
   i686 (budget 6 GPRs, reach band 2) and changes this kernel not at all (549
   instructions with `CCC_RA_GLOBAL_LOCATION=1` and 549 without). Slot
   forwarding makes that traffic cheaper; only location allocation removes it.

### 6.3 The constant-argument fold, and an encoding-arithmetic decision

Census first (`tools/dead_materialize_census.py`, 809 i686 TUs): **471** sites
where a value is materialized into a register that is dead immediately after a
slot store — 327 zeros (`xorl %eax,%eax`), 105 non-zero immediates, 39 register
copies. Fixing it at the lowering site rather than as a peephole beat the
census by 2.3×, because a peephole must prove the register dead whereas the
lowering site never materializes it at all — and the removed `%eax` clobber
takes its downstream reloads with it.

Folding **all** constants measured −1,194 instructions but **+1,344 .text bytes
(+0.477%)**. The reason is encoding arithmetic, not a heuristic:

```
non-zero   movl $imm,%eax (5B) + movl %eax,d8(%esp) (4B) = 9B  ->  movl $imm,d8(%esp) = 8B   WIN
zero       xorl %eax,%eax (2B) + movl %eax,d8(%esp) (4B) = 6B  ->  movl $0,d8(%esp)   = 8B   LOSS
```

and 327 of the 432 constant sites are zeros. So zero is excluded, which keeps
83% of the instruction win at 57% less byte growth: **−993 instructions, 298
TUs smaller, 0 larger, +580 bytes (+0.206%)** on a 210-TU sample. +638 of that
+580 is two memset TUs (22,573 → 22,892 each); every other TU nets smaller.
Their growth is *downstream*, not the fold: `movl %ebx, %eax` copies appear
where the removed `%eax` clobber used to supply a scratch definition.

Runtime was neutral for the wider variant too (paired geomean −0.24%, median
+0.13%), so there is no measured performance argument for paying bytes — which
is why the trade was resolved on the static metrics rather than on a story
about which metric "matters more".

Gate: `CCC_NO_CONST_STACK_ARG`. With it set the object file is **byte-identical**
to the parent build (sha256 `220b4c89…` on `cpu_model_memset_inline.c`) and the
corpus counts reproduce exactly, so the harness provably measures the change and
not a rebuild artefact.

### 6.4 The exclusion rule was nearly refined into a regression, and what caught it

The blanket `imm != 0` exclusion looks over-broad, because only three IR arms
materialize zero with `xorl` at the point of emission (`I32`, `I64`, `Zero`);
`I8(0)`, `I16(0)`, `I128(0)`, `D32(0)`, `D64(0)`, `F32(0.0)`, `F64(0.0)` and
`LongDouble` emit `movl $0, %eax` (5 B), for which the 8-byte folded store
*appears* to win a byte. A census (`tools/zero_arm_census.py`, 794 TUs, fold
disabled) reported **0** such sites against 562 `xorl` sites, which read as "the
refinement is harmless either way" — so the per-arm predicate was implemented,
with unit tests, and measured — then reverted in the working tree, never
committed.

It was wrong, and the measurement said so: `tools/asm_identity.py` (per-TU
assembly hash comparison against the already-validated binary) reported **8 TUs
changed**, and every change was

```
xorl %eax, %eax          ->     movl $0, 0(%esp)
movl %eax, 0(%esp)              (-1 instruction, +2 bytes)
```

The cause is a peephole in `peephole.rs` ("`movl $0, %reg` → `xorl %reg, %reg`,
saves 3 bytes") that normalizes *every* arm's zero materialization before the
assembler sees it. The zero that reaches the object file is 2 bytes whichever IR
arm produced it, so the folded 8-byte store loses in all of them. The census
returned 0 because it was counting the **final** assembly, where the `movl $0`
form no longer exists — the data was right and the inference from it was not.

The predicate is therefore the blanket rule, and the reason is now the mechanism
(the peephole) rather than an empirical accident. Three things were hardened so
this cannot silently regress:

* `const_stack_arg_imm` and `const_zero_uses_xorl` are pure functions shared by
  `operand_to_eax` and the fold, so the two marshalling paths cannot drift on a
  constant's bits — bit-exactness is structural, not a duplicated table.
* `const_stack_arg_fold_wins` is pure and its test re-derives the verdict from
  byte-count constants using the **post-peephole** cost, so changing either the
  predicate or an encoding size without the other fails a test.
* `no_zero_folds_in_any_arm` is an explicit tripwire: if that peephole is ever
  removed, the test names the arms that genuinely become 9-byte sequences and
  the predicate must be re-derived instead of left refusing a win.

After the revert the refactor is output-neutral by measurement: **795 of 795
compilable TUs emit byte-identical assembly** to the pre-refactor binary
(`asm_identity.py`, 0 differing), so all numbers above are the delivered
binary's numbers rather than a re-derivation.

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

8. **I published an unfair oracle comparison, and caught it only by asking why
   the excess existed.** The first version of this document reported lccc 498 vs
   gcc 291 as evidence about code generation. 95 of those 207 instructions were
   PIC relocations that gcc was never asked to emit (§6.1). The error was not
   the tooling but the interpretation: I read a *total* without classifying it.
   The habit that caught it — histogram the excess by mnemonic and by operand
   class before concluding anything — is what then found the argument
   marshalling defect, so the correction paid for itself, but the first draft
   was wrong and is corrected here rather than quietly replaced.
9. **I reported an instruction win before measuring bytes, and the measurement
   changed the design.** The unrestricted constant fold was −1,194
   instructions; it was also +1,344 .text bytes. Shipped on the strength of the
   first number, the patch would have made the compiler's output *larger* while
   calling itself an optimisation. Instruction count and code size are different
   metrics with different consumers (uops/decode vs I-cache); when they
   disagree, the honest move is to find the subset that wins both, or else state
   the trade and justify it — which is what §6.3 does. Note that "0 TUs larger"
   is true for instruction counts and **false** for bytes: two memset TUs grow
   by 319 bytes each.
10. **The fuzzer's coverage of the CFG pass is still thin, and I only partly
    fixed it.** `g_cfg_pressure` and per-generator fire-rate reporting were
    added (§9), which makes the thinness visible instead of hiding it behind a
    case count: the CFG pass fires on ~8% of generated programs, the constant
    fold on ~15%. A pass exercised by 8% of cases is not validated by 864 cases.

11. **My byte-count model was evaluated at the wrong point in the pipeline, and
    it took a second measurement to catch it.** The exclusion rule in §6.3 was
    derived from encoding sizes *as emitted*, but the compiler peepholes
    `movl $0, %reg` into `xorl %reg, %reg` afterwards, so the emitted size is
    not the delivered size. I implemented the "more precise" per-arm predicate
    that the emission-time model implied, wrote unit tests for it, and it still
    would have shipped a code-size regression at eight sites — the tests passed
    because they encoded the same wrong model. What caught it was a *differential*
    measurement against the already-validated binary (`asm_identity.py`), not
    reasoning and not a unit test. Two lessons: a cost model must be evaluated
    against final output, and a test derived from the same model as the code
    cannot falsify it — only an independent measurement can. This is why the
    predicate's test now re-derives the verdict from byte constants and why
    `no_zero_folds_in_any_arm` is written as a tripwire naming the peephole it
    depends on.

**Where I agree with the mandate and the earlier adjudications.** S17's
deletion was the right call *at the time* (the pass was unsound in four
independent ways) and the wrong call *as a final state* (the capability was
real and is now restored soundly). The S18 x87 GP-pair folds remain correct as
an interim mitigation; they are not in conflict with this pass, and
`fstpl/fldl` is still deliberately not folded here.

## 8. Follow-ups, in priority order (each with the measurement that ranks it)

1. **Push-based outgoing arguments.** lccc pre-allocates the outgoing area with
   `subl $N,%esp` and stores each argument through `disp(%esp)` at 4–11 bytes;
   gcc pushes at 1–6 (`pushl %reg` = 1, `pushl $0` = 2). On a 7-argument
   `printf` that is ~13 bytes for gcc against 34–62 for lccc, and it is the
   single largest remaining code-size lever on call-heavy code. It also subsumes
   follow-up 4. Cost: a calling-sequence change touching `esp_adjust`
   bookkeeping, CFI, stack alignment, varargs, struct-byval and the `%esp`
   renumbering inside the CFG slot forwarder — it needs its own A/B, not a
   drive-by edit.
2. **Location allocation for i686 values.** The dominant term in the fair
   oracle gap: 37% of lccc's instructions carry a memory operand against gcc's
   1%, because values gcc holds in `%ebx`/`%esi`/`%edi` are homed in slots.
   Upstream's Phase 1 measures at −4 slot references corpus-wide on i686 and
   does not change the kernel at all (549 with the gate on, 549 off), so the
   budget/reach-band calibration (`location_alloc.rs`, 6 GPRs, band 2) is the
   thing to attack, together with the x87 location model at `memory.rs:470` and
   the ~33k remaining GP slot loads.
3. **`emit_call_8byte_stack_arg` copies a slot to the outgoing area through
   `%eax` in four instructions** (`movl sr0,%eax; movl %eax,N(%esp); movl
   sr4,%eax; movl %eax,N+4(%esp)`). Two are achievable: x87 `fldl`/`fstpl`
   (rejected here — not bit-exact for signalling-NaN payloads) or SSE `movsd`
   (exact, needs a target-feature gate).
4. **Zero-valued arguments** (§6.3): 327 census sites where the direct store
   costs 2 bytes more than `xorl`+store. Solved properly by follow-up 1
   (`pushl $0` is 2 bytes), or by a reliable "%eax already holds zero" cache —
   currently blocked by the incomplete i686 accumulator-cache contract that
   `operand_to_eax` itself documents; a stale zero claim would be a miscompile,
   so it must not be attempted before that audit lands.
5. **Raise the CFG pass's fuzz fire rate** above ~8% (§7.10); the reporting is
   in place so the number is now visible per generator.
6. **Per-byte slot model** — would recover the narrow-store→wide-ALU forward
   refused in §4 and turn the 47-shape enumeration into a byte-range
   computation instead of a table.
7. **Phase-ordering invariant** (§7.7) — make "nops reloads, therefore runs
   last" a property the pipeline can check rather than a comment.
8. Re-run the §5.2 interaction A/B after each Location Allocation phase lands;
   the falsifiable prediction in §7.5 makes the result interpretable either way.

## 9. Harnesses added this session (kept outside the repo tree)

| path | purpose |
|---|---|
| `tools/slot_fwd_fuzz.py` | 12-generator differential fuzzer: lccc-on vs lccc-off vs gcc, with instruction counts and a fire-rate metric |
| `tools/minimize.py` | line-level ddmin for a gcc-vs-lccc divergence, **pinning initialising stores** so the minimal case stays defined |
| `tools/fwd_ab.py` | 4-arm static A/B (base/off/prec/both) with byte-identity and per-TU regression detection; now accepts extra env merged into every arm (used for the Location Allocation interaction) |
| `tools/slot_fwd_runtime.py` | 3-arm runtime A/B, hash-first, only times benchmarks whose assembly actually changed; `RT_PAIRED=1` interleaves the arms within every repetition and `RT_FLOOR_MS` (default 20) labels and excludes sub-floor benchmarks from the geomean |
| `tools/dead_materialize_census.py` | counts materialize-then-store pairs whose register is dead afterwards (imm / zero / copy, with live-vs-dead split) — the census that preceded `87ebbef7` and predicted 471 sites |
| `tools/text_size_ab.py` | `.text` **bytes** per TU for two arms via `size -A`, because instruction count and code size can move in opposite directions (§6.3) |
| `tools/zero_arm_census.py` | counts materialize-then-store sequences in *final* assembly by which zero form the peephole left behind (`xorl` vs `movl $0`) — the measurement that mislead §6.4 until it was read as final-output data |
| `tools/asm_identity.py` | per-TU emitted-assembly sha256 comparison between two compiler binaries; proves a compiler refactor is output-neutral (795/795 identical) or finds exactly which TUs it changed (8, in §6.4) |

All three differential harnesses take `AB_OFF_VARS=K=V[,K=V]` so the OFF arm can
disable any gated transformation, not just the slot forwarders; `fwd_ab.py`
takes extra `K=V` arguments merged into every arm (used for the Location
Allocation interaction in §5.2).
| `tools/exec_diff.py` | corpus execution differential against the gcc `-m32` oracle |
