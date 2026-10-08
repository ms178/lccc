# S21: bottom-testing loops that walk memory, and the dynamic-instruction harness

The Godbolt oracle compares *static* instruction counts at `-O2` and reported
lccc ahead of GCC 16, Clang 23 and ICX on every corpus program
(`tools/oracle/godbolt_oracle.py`, 2026-10-05: totals 1328 vs 1723 / 1913 /
1970). Static counts say nothing about how often an instruction runs, and the
first thing this stretch did was measure the runtime: **the static lead did not
survive contact with Callgrind** on five of the eight oracle programs.

| program | lccc Ir | GCC 14 Ir | lccc/GCC | previous static count (lccc/GCC) |
|---|---|---|---|---|
| bitops | 143,644 | 106,884 | 1.344 | 122 / 128 |
| control | 1,180,662 | 1,077,512 | 1.096 | 168 / 116 |
| float | 7,630 | 7,181 | 1.063 | 155 / 123 |
| int_alu | 5,905 | 4,548 | 1.298 | 92 / 68 |
| loops | 36,319 | 23,657 | 1.535 | 128 / 229 |
| memory | 29,896 | 243,815 | **0.123** | 242 / 579 |
| pixmap | 1,729,175 | 1,520,345 | 1.137 | 256 / 283 |
| strings | 1,121,903 | 880,840 | 1.274 | 165 / 197 |

Counts are executed instructions in the program's own object, libc and the
dynamic loader excluded, with stdout verified byte-identical across all
compilers before any ratio is reported. That table is the honest starting
point: lccc wins `memory` by 8x and loses the byte-walking programs badly.

> **Re-implemented 2026-10-05 after a harness wipe.** The code described in
> §2.1 was lost with the execution environment (the worktree survived, the
> pass did not); it was rebuilt from this document's specification and its
> numbers reproduced exactly on the merged tree: `k_strcmp_signed`
> 844,951,167 → 751,844,222 Ir (−11.02%), wall 52.5 ms → 35.9 ms, `k_memchr`
> and `k_adler32` bit-identical, same six tests plus the removed negative
> control. `docs/SESSION_FOLLOWUP_2026-10-05_MERGE_765.md` §3 has the current
> form of the argument and the fresh measurements; the numbers in §3 below are
> the ones this document recorded when the pass was first written.
>
> **Not in this tree (2026-10-06).**  That rebuild is not part of the
> encoder/matrix delta that ships this document: `src/passes/loop_invert.rs` is
> untouched by it, `a_verbatim_copy_lands_after_the_edge_copies` does not
> exist, and no file under `src/` differs for it.  §2.1 and §3 below are
> therefore a specification to re-apply, not a description of this revision;
> treat their numbers as unreproduced here.  §1's instruments
> (`tools/oracle/hot_instr.py`, `callgrind_compare.sh`, `callgrind_own_ir.py`)
> ARE present and runnable, and §5's harness defects are fixed in them.

## 1. The instruments (new, in `tools/oracle/`)

* `callgrind_own_ir.py` — sums Callgrind Ir for one object. `callgrind_annotate`
  alone mixes in `libc` and `ld.so`, which for these kernels is 30-50% of the
  run and hides exactly the differences being chased (its own table attributes
  a program's cost to `__libc_start_main` unless the callee records are walked
  in the right order, so this wrapper filters by object instead).
* `hot_instr.py` — per-instruction attribution. lccc emits no DWARF line
  tables, so `callgrind_annotate --auto=yes` cannot attribute to source; run
  Callgrind with `--dump-instr=yes` and this joins the profile's instruction
  addresses with `objdump -d`. It also summarises by instruction class
  (load / store / lea / control / alu / regmov), which is what turned the
  `strings` investigation from guesswork into arithmetic.
* `callgrind_compare.sh` — builds each oracle program with lccc and the local
  references, runs all of them under Callgrind, refuses to print a ratio when
  stdouts differ, and writes a JSON record (`/home/user/callgrind_s21.json`,
  `/home/user/callgrind_s21_after.json`).

## 2. Defect: the loop test stayed at the top for headers that load

`strings.c` walks three byte strings per outer iteration (`hash`,
`count_byte`, `str_cmp_words`). lccc ran **9 instructions per byte** against
GCC's 6:

```text
lccc                                      GCC
.L7c0: movsbl (%rbx),%r9d                 .L11a0: add    $1,%rcx
       test   %r9b,%r9b                          xor    %rsi,%rax
       je     .L7e0                              movzbl (%rcx),%esi
       add    $1,%rbx                            imul   %rbp,%rax
       movzbl %r9b,%eax                          test   %sil,%sil
       mov    %r15,%rsi                          jne    .L11a0
       xor    %rax,%rsi
       mov    %rsi,%r15
       imul   %r12,%r15
       jmp    .L7c0                   <-- taken, every iteration
```

`loop_invert` exists to fix exactly this and its header documents a measured
~2x on `k_memchr`, but it declined these loops:

```text
[INV] header 10 has a non-duplicable instruction
[INV] header 13 has a non-duplicable instruction
[INV] header 16 has a non-duplicable instruction
```

The refusal is rule 2 ("loads are refused outright") plus rule 3 ("every value
defined in `H` is used only inside `H`"): the header's `Load %b = *%p` *is* the
loop's payload, and the body consumes it, so a **renamed** copy in the latch
would leave the body reading the guard's byte forever. The renaming was never
the point, though — it is only what made rule 3 necessary.

### 2.1 Fix: verbatim duplication when the header walks memory

`src/passes/loop_invert.rs` now has two cloning modes. A header that only
computes keeps the existing fresh-name path (separate live ranges, rule 3
enforced). A header containing non-volatile loads/stores is duplicated
**verbatim** — same destinations, same operands — and rule 3 is skipped
because the copy re-defines the values the body reads.

The correctness argument is that the rotation is not a motion at all, it is a
re-labelling: the header's instruction sequence executes at the same points in
the same order, `n+1` times per entry, whether it sits at the top of the
iteration or at the bottom of it.

```text
original :  H1 B1 H2 B2 ... Hn Bn H(n+1)   exit
rotated  :  H1 B1 H2 B2 ... Hn Bn H(n+1)   exit
            ^guard                 ^latch
```

Every load and store in `H` therefore reads and writes the same addresses, in
the same order, the same number of times; nothing is hoisted, sunk or
speculated. The duplicates are appended *after* phi elimination's edge copies,
so they observe the next iteration's induction variable, and the header's own
terminator becomes the once-executed guard. Volatile accesses, calls, atomics,
inline asm and stack allocations are still refused — the argument above does
not extend to them and no measured loop needs them.

## 3. Measurements after the fix

Oracle programs, Callgrind, same harness (`callgrind_s21_after.json`):

| program | before | after | Δ | vs GCC |
|---|---|---|---|---|
| **strings** | 1,121,903 | **1,012,019** | **−9.79%** | 1.274 → 1.149 |
| bitops, control, float, int_alu, loops, memory, pixmap | — | unchanged | 0% | — |
| total (8 programs) | 4,255,134 | 4,145,250 | −2.58% | |

Real bench harness (`tests/bench/driver.c` + kernel, best-of-5):

| kernel | base Ir | new Ir | Δ | base time | new time |
|---|---|---|---|---|---|
| `k_strcmp_signed` | 87,534,567 | 77,890,022 | **−11.0%** | 5.16 ms | 3.49 ms |
| `k_memchr` | 8,556,061 | 8,556,061 | 0 | 0.580 ms | 0.575 ms |
| `k_adler32` | 5,151,148 | 5,151,148 | 0 | 0.306 ms | 0.296 ms |

Blast radius: of the first 80 TUs under `tests/bench`, `tests/corpus`,
`tests/benchmark`, exactly 5 change assembly (`k_strcmp_signed`,
`k04_strlen`, `k13_strcmp`, `fannkuch`, `linux_rbtree`) — all byte/pointer
walks. The whole change in `k04_strlen` is one line:

```diff
-    jmp .LBB1
+    cmpb $0, (%rsi)
+ jne .LBB2
```

## 4. Verification evidence

* `cargo test --profile fastbuild --lib`: **4135 passed / 0 failed / 7 ignored**
  (4131 before; +5 new pass tests, −1 superseded negative test).
* New/updated pass tests: `a_header_load_is_inverted_verbatim`,
  `a_verbatim_copy_lands_after_the_edge_copies`,
  `a_header_load_escaping_into_the_body_is_inverted_for_memory_but_not_for_pure_values`,
  `a_volatile_header_load_is_not_inverted`,
  `a_header_with_a_call_is_not_inverted`,
  `a_header_with_a_stack_allocation_is_not_inverted`; the pure-header path's
  tests are unchanged and still pass.
* Differential fuzzing against GCC/Clang
  (`scripts/fuzz_diff.py`, all output-verified): `differential` 40/40,
  `phi_cfg` 40/40, `synthetic` 30/30, `stress_suite` 91 cases;
  `unroll_stress.py` 12 configurations, 0 failures.
* `cargo fmt --all -- --check` clean; `scripts/ci_local.sh --fast` — see the
  snapshot commit message for the gate summary.
* Pristine-HEAD A/B: `/tmp/lccc_base` (built from a stashed tree) was used to
  attribute every difference above to this change and nothing else.

## 5. Harness defects fixed on the way (both produced false compiler reports)

1. `scripts/fuzz_diff.py` `stress_suite`: `gen_fp_stress` cases were reported
   as **miscompiles**. They are not: lccc's documented default code-generation
   baseline is x86-64-v3, which includes FMA3, so `u*0.999 + 0.001` is
   contracted into `vfmadd231sd` (even at `-O0`); the reference `/usr/bin/gcc`
   targets baseline x86-64, has no FMA, and cannot contract. Both answers are
   conforming C (`-ffp-contract=fast` is the C default) and differ by 1 ulp.
   Reproducer: `double step(double u){ return u*0.999 + 0.001; }` from
   `u = 0x1.af9e818f741bfp+37` — lccc `0x1.af3002f7808b8p+37`, GCC/Clang
   `0x1.af3002f7808b9p+37`, and `-ffp-contract=off` makes lccc bit-identical.
   The fp_stress generator now pins `-ffp-contract=off` for both compilers so
   the harness measures arithmetic instead of contraction policy.
2. `scripts/unroll_stress.py` reported a **reference** arm that could not run
   (`[gcc-O0] RUN TIMEOUT`) as an lccc crash — `scripts/fuzz_diff.py` surfaced
   it as `[LCCC_CRASH] unroll_stress`. A reference arm that produces no output
   says nothing about the compiler under test; those are now printed as
   `[SKIP] ... reference arm inconclusive` and do not fail the batch. A driver
   that exits 0 without producing an executable (`NO EXECUTABLE`) used to
   abort with a traceback; it is now an unusable arm. `--run-timeout` is
   configurable.

## 6. Follow-ups, with the data that motivates each

1. **Count-bits loop idiom** (`bitops`, lccc/GCC = 1.344). The loop
   `while (x) { x &= x-1; c++; }` is kept as a five-instruction loop
   (`lea / and / add / test / jne`, 16,196 iterations = 56% of that program's
   instructions) although lccc can already lower `IrUnaryOp::Popcount` to the
   `popcnt` instruction on its default target (`__builtin_popcount` compiles to
   a single `popcntl`) and `bit_idioms` recognizes the straight-line SWAR
   form. Folding the loop into `Popcount` would take that program from 1.344 to
   ≈ 0.7 against GCC. Needs a new loop-level folder; `loop_idiom` is
   copy-specific (`loop_invert` is the wrong home: this one needs SSA phis).
2. **Register-allocator copies and separate zero-extension in byte loops**
   (`strings`, now 1.149). The remaining gap to GCC is three instructions per
   byte: `mov %r15,%rsi` + `mov %rsi,%r15` around the xor/multiply chain, and
   `Cast(I8→U8)` materialised as `movzbl %r9b,%eax` next to a `movsbl` load
   instead of one zero-extending `movzbl (%rbx),%eax`. Both are instruction
   selection / coalescing, not loop structure, and would show up in every
   byte- or char-oriented workload.
3. **`-O0` and FMA contraction** (see §5.1): legal, but it makes `-O0` output
   differ from GCC's `-O0` and from lccc's own `-O2` reference flags in
   differential harnesses. Decide explicitly: either document it next to the
   x86-64-v3 default policy, or gate contraction on `-O1`+.

## 5. S24: memory-header rotation — implemented, measured, **rejected**

> **Superseded by §6 (S26).** The 15.9% regression this section is built on does
> not reproduce: re-measured on the same host with the same licence, as a
> 61-sample interleaved A/B of one binary (`CCC_LOOP_INVERT_MEMORY=1` vs `0`),
> the byte walk moves **+0.57%** (41-61 wins) — noise. §5's number was a
> `min`-of-31 statistic, and §6.4 measures that statistic inventing 5-25%
> effects on four kernels of this corpus when the two arms are the *same code*.
> §5's mechanical record (what was built, the hand-revert experiment, the
> alignment and generality probes) stands as the history of the decision; its
> magnitude does not, and the licence is off by default for a different reason
> than the one recorded here (it is neutral, not harmful).

### 5.1 What was built

Rule 2 used to refuse any header containing a memory access. The S24 change
replaced it with a two-licence rule (`CloneMode::{Pure,Memory}`): a non-volatile
`Load`/`Store` in the header could be duplicated into the latch, clones taking
fresh value ids from `sound_next_value_id()`, with `set_dest` extended to know
about `Load` (without that, a cloned load re-defined the guard's `Value` — two
non-`Copy` defs of one id, which is exactly what `verify.rs` check 7 /
`validate_unique_defs` rejects after phi elimination).

The soundness argument is short and it holds: the rotated loop executes the
header `N + 1` times at the same program positions as the top-test form (the
guard once, the latch copy once per iteration after the induction-variable
update), nothing is inserted between a body and the copy, and rule 3 keeps a
header-defined value from reaching the body. A duplicated non-volatile access
therefore reads and writes exactly the state the next iteration's top test would
have. Volatile accesses, atomics, calls, inline asm, `alloca`, `memcpy` and
`PgoCounterInc` stay refused.

### 5.2 What it measured, and why it is refused

Corpus sweep (`tests/bench/k_*.c` + every `tests/benchmark/programs/*.c`,
`CCC_DEBUG_LOOP_INVERT=1`, `-O2`). With the memory mode in: 242 rotations
`mode=Pure`, **3 `mode=Memory`** — all three in
`tests/benchmark/programs/strlen_bench.c` — and 25 rule-3 refusals, 0 for any
other reason. So the memory mode's entire blast radius in the corpus was one
kernel (1 of 64 TUs; `strlen_bench` `.text` 832 → 847 bytes, since reverted).
On the shipped (pure-only) tree the same sweep over 65 TUs reports 242 pure
rotations and 15 rule-3 refusals — 11 where the value escapes the loop
entirely, 4 where it is used by the body — the count differing because a
rotation changes the CFG the next attempt sees.

On a purpose-built kernel that isolates the paying shape (`while (*p) p++;` over
4095 bytes with no interior zero, `/tmp/harness/tests/bench/k_strlen_scan.c`),
interleaved samples, checksums identical:

| arm | min | median |
|---|---|---|
| S23 (top-tested loop; memory headers refused) | **49.28 ms** | 49.53 ms |
| S24 (rotated by the memory mode) | 59.24 ms | 59.50 ms |
| S24 with `CCC_NO_LOOP_INVERT=1` | 50.15 ms | 50.80 ms |

**+15.9% for the rotation**, paired win rate 2/99 — i.e. not noise, and not the
rest of the S24 delta: disabling the pass on the S24 binary lands within 1.4% of
S23.

Three experiments localize it to the loop *shape*, not to the change's
incidental effects:

1. **Alignment** — hand-stripping both `.p2align` directives from the S24
   assembly changed nothing (59.80 ms vs 59.60 ms).
2. **Hand-revert** — editing only the rotated loop in the S24 assembly
   (`.LBB8: addq $1,%r8; cmpb $0,(%r8); jne .LBB8`) back into the top-tested
   form (`cmpb; je; add; jmp`), leaving every other byte of the function
   identical, recovered **100%** of the loss: 49.46 ms (+0.26% vs S23).
3. **Generality** — a *word*-granularity scan (`while (*p) p++;` over `int*`,
   which also rotates in `mode=Memory`) is neutral: 10.204 ms vs 10.211 ms
   (+0.06%, 16/31 wins). The loss is specific to the byte-granularity walk,
   where lccc's top-tested loop runs at ~1.0 cycle/byte and the rotated one at
   ~1.18 — a 0.18-cycle/iteration constant, which is 18% of a loop that tight.

### 5.3 The competitive reading, which is why this matters

The same kernel, three builds, 31 interleaved samples, `-O2`:

| build | min | vs GCC |
|---|---|---|
| **lccc S23 (top-tested)** | **49.28 ms** | **−15.92%** |
| lccc S24 (rotated) | 59.24 ms | +1.07% |
| GCC 14.2 `-O2` | 58.61 ms | — |

GCC 14.2 emits exactly the shape the rotation would have produced —
`.L7: addq $1, %rax; cmpb $0, (%rax); jne .L7`, guard in front — so lccc's
un-rotated byte walk is **15.9% faster than GCC's** on this host, and rotating
it would have traded that lead for a dead heat. That is the whole decision: the
rotation is *sound* and on this hardware it is *slower*, and the project's rule
is that a >5% regression needs a justification, not a rationale.

So rule 2 is back to pure-only, the memory mode is **removed** (not gated),
`set_dest` no longer needs `Load`, and the shape is pinned by tests that carry
the measurement in their docstrings (`a_header_containing_a_load_is_not_inverted`,
`a_header_containing_a_store_is_not_inverted`). What stays from S24 is the
*diagnostic*: a rule-3 refusal now names the value, the block and whether that
block is inside the loop, which is what turned "25 refusals" into a scoped list
(16 of them are a test chain that starts in the header and decides the exit in
the *next* block — a region-based rotation, not a block-based one, is the
follow-up that would reach the byte loops).

**Follow-up, in priority order.** (1) Rotate the *test region*, not the header
block, so a chain like `B1: v5 = load [v27] -> v5 ? B4 : exit` /
`B4: v11 = load [v28]; v13 = v5 == v11 -> v13 ? B2 : exit` becomes one
duplicable unit; the shape question above says the duplicate must keep the
*load* where the top test had it, so this needs the measurement repeated per
shape, not assumed. (2) Unroll the byte walk (4 bytes per iteration, one branch
per 4) before considering rotation again — that is what would move the 1.0
cycle/byte floor, and it is a peephole on the same loop, not a CFG transform.


## 6. S26: the census instrument, the refusal landscape, and a re-measurement

`loop_invert` had no instrument. `CCC_DEBUG_LOOP_INVERT=1` printed a rotation
line and one refusal reason, and every census in §§1-5 was hand-rolled shell.
This stretch added the missing half -- a reason for *every* decision the pass
makes -- and used it to price the pass per kernel, which is what turned the
memory-licence question from a verdict into a number.

### 6.1 What was added

* `scripts/loop_invert_census.py` -- whole-corpus census. Compiles every TU in
  the tree's own corpus (`tests/bench/k_*.c`, `tests/benchmark/programs/*.c`,
  `tests/benchmark/kernel_corpus/*.c`, 90 TUs) with the debug flag and folds the
  pass's stderr into per-TU counters: rotations (split by licence), and refusals
  by rule -- non-duplicable, value escapes (in-loop / outside-loop), multi-latch,
  oversized header, header that does not decide the exit.
* The pass now reports all of those reasons, and every rotation carries its
  **shape** (`header=N insn/L ld/S st, loop=M insn/K mem`), so a decision can be
  correlated with the loop it was made on.
* The memory licence (§5) is wired to `CCC_LOOP_INVERT_MEMORY=1`, off by
  default, so the §5 experiment is reproducible on a built binary. With the
  switch off the pass's codegen is **byte-identical** to the pre-S26 tree:
  `scripts/census_full_delta.sh` (5 optimisation levels, whole corpus, two
  binaries) reports zero differing TUs.
* The pass's three switches (this one, `CCC_NO_LOOP_INVERT`, and
  `CCC_DEBUG_LOOP_INVERT`) are now resolved **once, in the driver's pipeline**,
  and handed down as a value, instead of being read out of the environment
  inside the pass -- where the read happened **once per function**. The
  env-read ratchet in `tests/regression/check_env_test_hygiene.sh` moved down
  with them, 155 -> 153, which is the only direction its policy allows; adding
  a switch at the call site instead would have failed that gate, which is how
  the per-function reads were found.

### 6.2 The refusal landscape (90 TUs, -O2, licence on)

| decision | shipped (licence off) | licence on |
|---|---|---|
| rotations | 278 | 283 (5 licensed) |
| refused: a header value escapes `H` | **16** (5 in-loop, 11 outside) | **27** (16 in-loop, 11 outside) |
| refused: header walks memory, licence off | 22 | -- |
| refused: non-duplicable instruction | 0 | 0 |
| refused: multi-latch loop | 0 | 0 |
| refused: header over `MAX_DUP_INSTS` | 2 | 2 |
| refused: header does not decide the exit | 1 | 1 |

(The 17-loop difference in the escape row is the licensed-but-escaping memory
headers: turning the licence on does not rotate them, it moves the reason they
are refused from "memory" to "the loaded value is used outside `H`".)

Two things follow. First, the pass's reach is not limited by size or by latches
in this corpus -- it is limited by **rule 3** (27 loops) and secondarily by
memory (22, of which the licence reaches 5 and rule 3 then refuses the other
17). Second, the memory licence's entire reach is *five* loops in 90 TUs, and
all five are the same 3-4 instruction `while (*p) p++;` walk:

```text
[INV] rotating header=3 latch=4  (memory: header=1 insn/1 ld/0 st, loop=3 insn/0 mem)   k_strlen_scan
[INV] rotating header=20 latch=22 (memory: header=1 insn/1 ld/0 st, loop=3 insn/0 mem)  strlen_bench
[INV] rotating header=23 latch=32 (memory: header=1 insn/1 ld/0 st, loop=14 insn/4 mem) strlen_bench
[INV] rotating header=26 latch=29 (memory: header=1 insn/1 ld/0 st, loop=8 insn/2 mem)  strlen_bench
[INV] rotating header=3 latch=4  (memory: header=1 insn/1 ld/0 st, loop=3 insn/0 mem)   k04_strlen
```

### 6.3 Red-team finding: a header store is not licensed by "the header runs n+1 times"

§5 licensed stores on the argument that the header executes `n + 1` times at the
same program positions in both forms. The count is right only while every
iteration reaches the latch. With a side exit -- `for (i = 0; (a[i] = b[i]) != 0;
i++) { if (x) break; }` -- the top-tested form executes the header once more than
the rotated form (at the top of the iteration that breaks, before the body
decides to leave), so the two forms leave **different bytes** at the loop exit.
A *load* is different: the dropped execution's value is used only inside `H`
(rule 3), so it is unobservable. Stores are therefore gated on
[`loop_exits_only_through_header`] -- "no block of the loop except the header and
the latch branches out of it" -- which is exactly the condition under which the
count is equal. Two tests pin it: `a_header_store_is_rotated_when_the_loop_exits_only_through_the_header`
and `a_header_store_is_refused_while_the_loop_has_a_side_exit` (which also
asserts the asymmetry: the same side exit does not stop a load).

### 6.4 The statistics, which is where §5 went wrong

An A/B on this host has to survive a sample distribution with a heavy *fast*
tail: the same binary, sampled 41 times a few minutes apart, reports minima of
1.38 ms and 2.29 ms for the same kernel. `min` picks whichever arm happened to
catch a fast tail sample, so on this box it manufactures effects. Measured
directly -- both arms the *same* machine code, 101 samples, `min` next to the two
statistics that do not read the tail:

| kernel | delta on min | delta on median | paired wins (B faster) |
|---|---|---|---|
| k_strcmp_signed | **+25.72%** | +0.06% | 55/101 |
| k_adler32 | **+7.18%** | -32.37% | 72/101 |
| k_varint | -1.14% | -2.31% | 57/101 |
| k_hashmix | +0.09% | -1.70% | 61/101 |

The `min` column is a phantom at *every* sample count: here it turns a 0.06%
non-effect into a 25.72% regression and a 32% improvement into a 7% regression,
while the median and the paired win rate agree with each other on all four. It
also moves with the sample count in both directions -- the first sweep of these
kernels at 31 samples reported `min` deltas of +7.29% (hashmix), +3.52%
(varint), +1.52% (strcmp_signed) and +1.42% (adler32), i.e. four phantoms whose
signs are uncorrelated with the 101-sample ones. §5's 15.9% was a `min`-of-31 comparison, and its hand-revert
"recovery" reproduced on the same statistic; re-measured as an interleaved A/B of
one binary with the licence on and off, 61 samples:

| kernel | licence off | licence on | delta on min | wins (on faster) |
|---|---|---|---|---|
| k_strlen_scan | 44.0 ms | 44.5 ms | +0.57% | 41/61 |
| k_strlen_scan (repeat, 121 samples) | -- | -- | within +/-1% | ~50/50 |

A shape-isolating 2x2 on `k_hashmix` (hand-built arms: {top-tested, rotated} x
{header aligned, body aligned}, identical otherwise, `driver.o` linked from GCC
so only the kernel TU differs) puts the rotation at +4.16% on min and +0.11% on
min for the alignment factor, with the win rates 26/41 and 21/41 -- i.e. the same
"effect" and the same statistic, moving in the opposite direction the moment the
two arms are prepared by hand instead of by a rebuild. That is the whole
argument for the median/win-rate pair in this document from here on.

### 6.5 What the pass is actually worth, per kernel

Interleaved A/B of one binary: `CCC_NO_LOOP_INVERT=1` (base) against the shipped
configuration (`CCC_LOOP_INVERT_MEMORY=0`, i.e. pure rotations only), 101
samples, `-O3 -march=x86-64-v3`:

| kernel | median | paired wins |
|---|---|---|
| k_matchlen | **-44.21%** | 101/101 |
| k_memchr | **-32.78%** | 100/101 |
| k_adler32 | -32.37% | 72/101 |
| k_namechars | -8.29% | 71/101 |
| k_adler32_do8 | -4.28% | 92/101 |
| k_classify | -3.37% | 83/101 |
| k_map64_sub | -3.07% | 56/101 |
| k_varint | -2.31% | 57/101 |
| k_hashmix | -1.70% | 61/101 |
| k_strlen_scan | +0.13% | 44/101 |
| k_strcmp_signed | +0.06% | 55/101 |

Two kernels carry the pass (a 2-3x on scan loops); the rest are single digits,
and the two non-wins are +0.13% and +0.06% -- inside the noise band this host
cannot resolve. (An earlier version of this table carried 31-sample `min`
numbers for the middle rows, e.g. -11.17% for classify; the medians above are
from the same 101-sample runs as the rest.)

### 6.6 The escape pool, measured before it is built

27 loops are refused only because a value defined in the header is used
elsewhere. Rotating them requires the sound form of §5's "verbatim" idea: the
copy may not re-define the guard's value (`verify.rs` check 7 rejects a second
definition of a value id -- consumers index def sites by id), so each escaping
use needs a **merge value** fed by edge copies: one on the entry edge (cold,
splitting `H -> B`), one at the end of the latch (hot, one `mov` per iteration
that the allocator may coalesce), plus splits on the `-> X` edges for values the
exit block reads. The one instance measured by hand first -- the shape §2.2
records as S21's -32% wall win -- is `k_strcmp_signed`'s loop, whose header loads
`*a` and whose next block tests it: rotating that loop's latch in the emitted
assembly (61 interleaved samples, checksums equal) gives **+0.79%**, 9/61 wins.
Neutral. So the machinery is not built: the pool is real, the prize is not, and
the measurements that would have to come first are the two shapes above, both
already neutral.

### 6.7 What the numbers say is left

* **The byte walk is level with GCC, not 16% ahead.** Re-measured with the same
  driver (`-O3 -march=x86-64-v3` for lccc against `-O2` and `-O3` for GCC 14.2,
  61 interleaved samples): lccc 45.765 ms vs GCC -O2 46.122 ms (+0.78% for GCC)
  and lccc 45.399 ms vs GCC -O3 45.402 ms (+0.01%). The 1.18x recorded in
  `docs/benchmarks.md` was a `min`-of-5 sample. The loop shapes differ (lccc
  top-tests, GCC rotates) and the shapes are worth ~0 in either direction.
* **The lever that is left is the loop's trip structure, not its shape.** The
  byte walk runs at ~1.0 cycle/byte because the address recurrence is one
  `add`; nothing done to the test moves that. Unrolling the walk 4 bytes per
  iteration with the SWAR zero test (`(w - 0x01010101) & ~w & 0x80808080`) is
  the transform that moves the floor, and it needs an alignment prologue (a
  4-byte load from an unaligned tail can cross into an unmapped page, so the
  wide loads must start at an aligned address and the tail be finished
  byte-wise) -- that is a codegen transform with a correctness surface, not a
  peephole, and it is the next session's piece of work.
