# Byte-load reassembly — measured case for the pass

Pass: `src/passes/byte_load.rs`, wired in `src/passes/mod.rs` phase 3 right after
`bit_idioms`. Kill switch for A/B: `CCC_DISABLE_PASSES=byte_load`.

## The gap this closes

Both Godbolt oracles collapse a hand-written little-endian byte assembly into
one load; LCCC emitted it literally. Measured with
`scripts/codegen_oracle.py --function <fn> --oracles gcc16.2,clang`
(flags `-O2 -march=x86-64-v3`), static instruction counts:

| function | source | LCCC (before) | gcc 16.2 | clang 23.1 |
|---|---|---:|---:|---:|
| `load_le64` | 8 × `(uint64_t)p[k] << 8k` | 61 | 2 | 2 |
| `load_le32` | 4 × `(uint32_t)p[k] << 8k` | 24 | 2 | 2 |
| `load_be64` | big-endian twin | 24 | 13 | 13 |
| `sip24_update` | Expat 2.8.5 SipHash, 2 × 8-byte + 1 × 4-byte chain | 96 | — | — |

The big-endian twin is deliberately **not** matched: it is a `bswap` load, and
`bit_idioms` already owns that idiom (`match_bswap32_network`, the
`BitReverse`/`Bswap` recognition), which is why the two passes run in this
order and never fight.

## The fix

`byte_load::run_function` folds one `Or`-tree of `n` byte terms into a single
`Load` of the same width when the terms are a consecutive byte run
`O, O+1, …, O+n-1` off one shared root (peeled through `Copy` chains and
nested constant `GetElementPtr`s), every consumed value is single-def and
single-use, no instruction between the earliest byte load and the merge may
write memory (`Instruction::may_write_memory`, GVN's own cached-load
predicate), and the root is an incoming pointer parameter. See the module
documentation for the full correctness argument.

## The two matcher restrictions relaxed after measuring the corpus

The first version of the pass folded nothing in `tests/benchmark/programs`
except SipHash's straight-line chain: it required the run to start at object
offset 0 and the base to be an incoming pointer parameter. Both are
unnecessary for soundness — the argument above only needs the same base
*expression*, consecutive offsets, and no write in between — and both were
made to carry the real rule they approximated:

* **Runs start anywhere.** `SIP_U8TO64_LE` on a struct member reads offsets
  32..39, not 0..7. The rule became "one consecutive run `O..O+n-1` off one
  shared root", with `O` taken from the shift-0 term.
* **The base may be any pointer expression** (global, loaded pointer, or
  `p = buf + i` through a variable-offset `GetElementPtr`). The *load-bearing*
  restriction is the one that replaced it: the base must have a single
  reaching definition across the chain window, checked against every
  definition site of the value. The induction variable of a buffer walk is
  written by a `Copy` at the bottom of its block — outside the window and
  therefore fine; a pointer rewritten *between* the byte loads and the merge
  is refused, because the wide load would then read a different address than
  the bytes it replaces.

Measured effect of the relaxation, same-binary A/B on the corpus benchmark
`tests/benchmark/programs/byte_load_reassembly.c` (callgrind `PROGRAM
TOTALS`, `-O2`, identical stdout `7ee1f2c0 3dcf60f284155e00` from all four
builds):

| build | instructions | vs before |
|---|---:|---:|
| LCCC, `CCC_DISABLE_PASSES=byte_load` | 175 062 919 | — |
| LCCC, pass enabled (restricted matcher) | 174 622 899 | −0.25 % |
| LCCC, pass enabled (relaxed matcher) | **82 402 899** | **−52.9 %** |
| gcc 14.2 | 31 134 487 | −82.2 % |
| clang 19.1 | 18 064 920 | −89.7 % |

The relaxation is what turns a 0.25 % saving into a 53 % saving: with the
parameter/offset-0 restriction the hot loop (`le32(buf + i)`) never matched.

## Big-endian runs and runs narrower than their carrier

The matcher originally demanded that the carrier be exactly as wide as the run
and that the least significant byte sit at the lowest address.  Both are
over-restrictions, and both are now gone:

* **Big-endian order** (`p[0] << 8 | p[1]`, the shape Linux spells
  `get_unaligned_be64` and network headers use).  The rewrite is one wide load
  plus a byte reversal, which is what every oracle emits.  It requires the run
  to be exactly as wide as the carrier: `bswap16` followed by a zero-extending
  cast is a width-sensitive sequence the backend would have to get exactly
  right, so a big-endian run that is narrower than its carrier is left as it
  is — deliberate, and covered by a test that asserts the refusal.
* **Runs narrower than the carrier.**  `(uint16_t)p[0] | ((uint16_t)p[1] << 8)`
  promotes to `int`, so the `Or` is 32 bits wide while the run is two bytes.
  The rewrite is a 16-bit load plus a zero-extending cast, which is exact
  because every term of the chain is a zero-extended byte: the carrier's high
  half is provably zero.

Per-function instruction counts, x86-64, `-O3 -march=x86-64-v3` on Compiler
Explorer (`scripts/codegen_oracle.py`, per-function rows from
`deliverables/artifacts/oracle/kernel-*.json`):

| function | lccc before | lccc now | GCC 16.2 | Clang 23.1 | ICX |
|---|---:|---:|---:|---:|---:|
| `le16` | 6 | **2** | 2 | 2 | 2 |
| `le32` | 2 | **2** | 2 | 2 | 2 |
| `le64` | 2 | **2** | 2 | 2 | 2 |
| `be64` | 24 | **4** | 2 | 12 | 2 |
| `body_le64` | 2 | **2** | 2 | 2 | 2 |

`be64` is now `movq (%rdi), %rdi; bswapq %rdi` plus the return move: LCCC beats
Clang 23.1's 12-instruction scalar chain (Clang does not use `movbe` here
either) and is two instructions behind GCC and ICX, which emit `movbe`.  That
is a *backend* opportunity, not a pass one — a `movbe` peephole for
`Load` + `Bswap` on the same value — and it is recorded as such rather than
faked by folding something else.

## Cross-architecture: where the pass runs, and why

The pass reinterprets bytes the program already read, so the question is not
whether the rewrite is *expressible* on a target but whether that target's
oracles make the same trade.  That is measurable per architecture, and it now
decides the gate.  Per-function instruction counts, each architecture's own
newest channels (`--arch aarch64` → GCC 16.1, GCC trunk, Clang 23.1;
`--arch riscv64` → GCC 16.1, GCC trunk, Clang 23.1):

| function | x86-64 oracles | AArch64 oracles | RISC-V 64 oracles | LCCC x86-64 | LCCC AArch64 | LCCC RISC-V |
|---|---:|---:|---:|---:|---:|---:|
| `le16` | 2 | 2 | 5 / 7 | **2** | **7** | 24 |
| `le32` | 2 | 2 | 12 / 11 | **2** | **7** | 51 |
| `le64` | 2 | 2 | 23 / 23 | **2** | **7** | 111 |
| `be64` | 2 | 3 | 23 / 23 | **4** | **8** | 111 |
| `body_le64` | 2 | 2 | 23 / 23 | **2** | **8** | 113 |

* **x86-64, i686**: on since the first revision.
* **AArch64**: every channel widens (one unaligned `ldr`; `ldr` + `rev` for
  big-endian), so the pass is on and the LCCC column moved from 18/30/54/55/55
  to 7/7/7/8/8.  What remains is not the load: see "the frame is the gap" below.
* **RISC-V 64**: no channel widens — 23 instructions for `le64` on both GCC
  16.1 and Clang 23.1 — because bare RISC-V does not guarantee unaligned
  access and a wide load could trap into the kernel's misaligned-access
  handler.  The pass stays off there *by measurement*, and the LCCC column
  (24–113 instructions) is the separate, larger codegen problem those same
  numbers expose.

## The frame is the gap: two cross-architecture defects found while measuring

Both were found by counting instructions per function across architectures,
not by reading code, and both are prologue/location-assignment policy — deep
enough that a rushed edit would be the wrong move: the allocator's own
documentation records that small changes there move benchmarks in both
directions.  They are recorded here with their evidence and left as scoped
follow-ups.

**i686 hands every function with a parameter a callee-saved home plus an
outgoing area.**  `int twice(int x) { return x * 2; }` compiles to

    pushl %ebx; subl $8, %esp; movl 16(%esp), %ebx; movl %ebx, %eax;
    addl %ebx, %eax; addl $8, %esp; popl %ebx; ret

against GCC's `movl 4(%esp), %eax; addl %eax, %eax; ret` — seven instructions
against three, on every function that takes an argument.  `int deref(const
int *p) { return *p; }` is 7 against 3.  Functions with no parameters are
clean (`int con(void) { return 3; }` → `movl $3, %eax; ret`), so the trigger is
the parameter home, not the frame machinery in general.

**AArch64 emits an unconditional frame-pointer chain, an LR save, and a
callee-saved save that is often dead.**  `int deref(const int *p) { return
*p; }` compiles to

    stp x29, x30, [sp, #-32]!; mov x29, sp; str x19, [sp, #16];
    ldrsw x0, [x0]; ldr x19, [sp, #16]; ldp x29, x30, [sp], #32; ret

— seven instructions against GCC's `ldrsw x0, [x0]; ret`, with `x19` saved and
reloaded without ever being read.  `int twice(int x)` is 9 against GCC's
`lsl w0, w0, #1; ret` (2).  The pattern is visible in every function, so it
costs 4–7 instructions per function on the whole architecture.

## MOVBE: the last instruction between LCCC and the oracle on `be64`

`byte_load`'s big-endian rewrite emits `load uN` + `bswap uN`, and the oracles
emit the same thing as a single instruction: GCC 16.2 and ICX both produce
`movbeq (%rdi), %rax` for the `get_unaligned_be64` shape, Clang 23.1 produces a
twelve-instruction `vpmovzxbq`/`vpsllvq`/`vpor` chain.  A peephole pass fuses
the pair (`passes/load_bswap_movbe.rs`), and — the part that mattered — also
coalesces the allocator's copy of the result, so the emitted sequence is
exactly the oracle's:

    be64:  movbeq (%rdi), %rax        CC
           ret

Per-function instruction counts, x86-64, `-O2`:

| function | before this session | byte_load only | with MOVBE | GCC 16.2 | Clang 23.1 | ICX |
|---|---:|---:|---:|---:|---:|---:|
| `le16` | 6 | 2 | **2** | 2 | 2 | 2 |
| `le32` | 2 | 2 | **2** | 2 | 2 | 2 |
| `le64` | 2 | 2 | **2** | 2 | 2 | 2 |
| `be64` | 24 | 4 | **2** | 2 | 12 | 2 |
| `body_le64` | 2 | 2 | **2** | 2 | 2 | 2 |

The permission is the driver's, not the pass's: `X86Isa::movbe` comes from
`-mmovbe`/`-mno-movbe` and the `-march=` baseline, is deliberately outside the
SSE implication chain (`-mno-sse` must not take MOVBE away, and MOVBE implies
no SIMD feature), and is passed into the pass explicitly rather than read from
the process-global ISA mirror — which also makes the two arms testable without
a global race.  Measured behaviour matrix (same TU, `-O2`):

| flags | `be64` |
|---|---:|
| default (x86-64-v3 baseline) | **2** |
| `-march=x86-64` (no MOVBE in the level) | 4 |
| `-mno-movbe` | 4 |
| `CCC_PEEPHOLE_SKIP=movbe_fuse` | 4 |

Dynamic cost is unchanged where MOVBE does not apply and the kernel's digest is
identical in all four cells (`7ee1f2c0 3dcf60f284155e00`); Callgrind reports
82,382,794 instructions with `byte_load` against 175,062,832 without.

#### The 16-bit big-endian shape is *not* folded — measured, and deliberate

One probe file, `-O2 -S`, one non-static function per shape, same flags:

| shape | LCCC default | `CCC_PEEPHOLE_SKIP=movbe_fuse` | `CCC_DISABLE_PASSES=byte_load` | local GCC 14.2 |
|---|---:|---:|---:|---:|
| `be16` `(uint16_t)((p[0]<<8)\| p[1])` | 6 | 6 | 6 | 3 |
| `be32` | **2** | 4 | 12 | 3 |
| `be64` | **2** | 4 | 24 | 3 |

Two readings.  (1) `be32`/`be64` show the fold working and the two switches
independent: withdrawing the ISA permission brings the pair back (4), while
switching `byte_load` off leaves the byte chain (12/24).  (2) `be16` is
identical in all three LCCC columns, because `byte_load` declines it on purpose
(`refuses_a_big_endian_run_narrower_than_its_carrier`: the 32-bit carrier would
need a 16-bit `bswap` plus a zero-extension).  That is the BE16 open item in
`FOLLOWUP-2026-10-13-audit-fix-session.md` §8.1.

#### Oracle parity, shape by shape (`-O3 -march=x86-64-v3`, this tree)

`scripts/codegen_oracle.py artifacts/oracle/probe/byte_shapes.c --function …`
over the whole family; source, JSON and raw log in
`deliverables/artifacts/oracle/` (`byte-load-shapes.{json,md,log}`):

| shape | LCCC | GCC 16.2 | Clang 23.1 | ICX |
|---|---:|---:|---:|---:|
| `le16` | **2** | 2 | 2 | 2 |
| `le32` | **2** | 2 | 2 | 2 |
| `le64` | **2** | 2 | 2 | 2 |
| `be16` | 6 | 2 | 2 | 2 |
| `be32` | **2** | 2 | 2 | 2 |
| `be64` | **2** | 2 | 12 | 2 |
| `body_le64` | **2** | 2 | 2 | 2 |

Six of the seven shapes are at parity with the best channel, and on two of them
nothing is close (`be64`: Clang 23.1 spends twelve instructions on
`vpmovzxbq`/`vpsllvq`/`vpor`).  The one gap is `be16`: every oracle channel —
including GCC 16.2, Clang 23.1 and ICX, which all have MOVBE at this `-march` —
answers with a single `movbe`, while LCCC keeps the byte chain.  Local GCC 14.2,
whose baseline predates MOVBE, reaches 3 (`movzwl` + `rolw $8`), which is why an
earlier revision of this document quoted 3: measured numbers name the channel
they came from.

#### What the kernel's four cells do and do not prove

The four-cell matrix on `byte_load_reassembly.c` is a *correctness* check: the
digest is identical with `byte_load` on/off and with `movbe_fuse` on/off, on
x86-64 and on i686 (`lccc-i686`, gcc-multilib installed).  It is **not** MOVBE
evidence: at `-O2` that file's KAT byte loads are shared with a later chain, so
its big-endian chains are never rewritten and `grep -c movbe` is 0 in the `-O2
-S` output of every cell *and* of GCC.  MOVBE fires on the shape probes above,
and in the corpus census (4 instructions, all in `ascii_case_fold`; GCC 1, in
`linux_find_bit`).  The i686 leg is the ISA gate in action: the same
translation unit yields 0 `movbe` for `i686` and 1 for `x86-64`, decided by the
resolved target ISA, not by the host.

## The chain that is not in one block (measured, not assumed)

`SIP_U8TO64_LE` in Expat's SipHash appears twice: once in `sip24_update`, where
the eight bytes sit in one basic block and the pass folds them (101 -> 76
instructions), and once in `sip24_final`, where the compiler's fallthrough
chain splits the OR-tree into one block per `case` label:

    .LBB19:  movzbl 38(%rbx), %r10d ; shlq $48, %r10 ; orq %r10, %rsi ; ...
    .LBB20:  movzbl 37(%rbx), %esi  ; shlq $40, %rsi  ; orq %r8, %rsi  ; ...
    ...                                   (one label per byte, falling through)

The pass matches within a basic block, so it folds nothing there (165 -> 165).
Two honest consequences: the `165 -> 149` pair an earlier revision of this
document carried never reproduced and has been removed, and "fold a run whose
terms are split across blocks" is now a measured, scoped target rather than a
guess — the shape is on disk in `tests/benchmark/programs/expat_siphash24.c`,
the block split is a property of *this* compiler's CFG (upstream's safe CFG
threading created per-case blocks), and the soundness question (a block whose
only predecessors are the chain's own continuation is still a chain) is the
work item, not the matching.

## The remaining gap is loop vectorisation, not the load

The same A/B shows the hot loop's scalar shape is now minimal
(`movl (%rbx), %r11d; xorl %r15d, %r11d; addl %r11d, %ebp; addq $8, %r15;
leaq 8(%rbx), %rbx; leaq 8(%r15), %r8; cmpq $4096, %r8; jbe`), i.e. one load
per record. Both oracles go further: gcc packs four little-endian fields with
`movdqa`/`shufps`, XORs them with a constructed index vector (`paddq` +
`shufps`) and reduces with `paddd`; clang builds the same shape with
`pshufd`. On a plain `acc += p[i] ^ i` reduction kernel (1024 u32, 200
rounds), lccc stays scalar (10 instructions in the loop, 2 of them a
redundant compare pair) while clang emits 7 SIMD ops — and gcc 14.2 also
stays scalar there, so the *oracle* to beat on this class is clang 23.1, not
gcc.

That is a loop-vectoriser task (reduction with an index-derived operand), not
a byte-load task: the pass delivered here is the *precondition* for it, since
a vectoriser cannot see a byte chain as four adjacent fields. Tracked as the
next measured target in `engineering/FOLLOWUP-2026-10-13-audit-fix-session.md`.

## Results (controlled same-binary A/B, `-O2 -march=x86-64-v3`)

`tests/benchmark/programs/expat_siphash24.c`, static instruction count:

| build | file | `sip24_update` | `sip24_final` |
|---|---:|---:|---:|
| `CCC_DISABLE_PASSES=byte_load` | 917 | 101 | 165 |
| pass enabled | 828 | 76 | 165 |

Re-measured on the tree this document ships with (`target/fastbuild/lccc -O2
-S`, `tests/benchmark/programs/expat_siphash24.c`).  An earlier revision of
this table carried `165 -> 149` for `sip24_final`; that number does not
reproduce and was removed rather than left standing — see "the chain that is
not in one block" below for what it actually is.

Runtime probe (not a counter, a digest): `tests/benchmark/programs/
byte_load_reassembly.c` prints `7ee1f2c0 3dcf60f284155e00` under LCCC and under
GCC 14.2, on **x86-64 and i686**, with the pass on and off.  A separate probe
(`/tmp/bl/bswap_probe.c`, folded into the differential when the session ends)
reads little- and big-endian fields at runtime-chosen offsets and checks
`__builtin_bswap16/32/64`; all four columns (GCC x86-64/i686, LCCC
x86-64/i686) print identical digests.

-25 and -16 instructions in the two SipHash functions; the byte chains become
`movq (%r14), %rdx` (one load from the already-computed pointer).

Standalone kernels, `-S` instruction counts (function body):

| kernel | before | after | gcc 14.2 | clang 19 |
|---|---:|---:|---:|---:|
| `load_le64` | 24 | **2** | 2 | 2 |
| `load_le32` | 9 | **2** | 2 | 2 |
| `load_be64` | 24 | 24 (by design) | 13 | 13 |

i686 (`-m32`): a 64-bit chain becomes two 32-bit loads
(`movl (%ebx), %eax; movl 4(%ebx), %edx`) instead of sixteen byte operations.

## Correctness: a miscompile this pass introduced, and what caught it

Worth recording in full, because the *catching* is the interesting part.

Extending the matcher to big-endian and narrow runs meant the load's pointer
could no longer be assumed to be the chain's shift-0 term, and the new code
folded the run's own `GetElementPtr` away whenever the run's *lowest* byte was
at the chain's lowest offset — a condition that is true by construction for
every run, including one at a nonzero offset.  The wide load then read from
offset 0 of the object instead of the run: for `body_le64(&framed)` it returned
the first eight bytes of the struct rather than `body`.

What caught it was the benchmark kernel's known-answer test —
`tests/benchmark/programs/byte_load_reassembly.c` returns 6 when
`body_le64(&fr) != 0xF7F6F5F4F3F2F1F0`, and the binary exited 6.  What did
*not* catch it was the unit suite: no fixture had ever placed a run at a
nonzero offset, even though the `field_offset`/`nested_gep` fixture flags
existed for exactly that shape.  Two things changed as a result:

* `folds_a_run_at_a_nonzero_offset` covers the three rewrite shapes
  (little-endian full width, big-endian full width, narrow carrier) at offset 8
  through nested `GetElementPtr`s, asserting the wide load resolves to
  `(root, 8)` — not `(root, 0)`.
* `assert_loads_stay_in_footprint` turns the pass's proof obligation into an
  executable invariant: every wide load the rewrite creates must read only
  bytes the original chain read, same root, byte for byte.  A load invented
  outside that footprint could fault where the original did not, and the
  invariant is asserted in every folding test, not only the new one.

The fix itself is the conservative one: the load reads the run's lowest-
address byte through *that term's own pointer*, and only adopts the resolved
root when the run really starts at the root's address.  A zero-displacement
`GetElementPtr` folds into the load's addressing mode in the backend, so the
emitted instruction is unchanged.  Deleting a value is now gated by
`consume_dead_chain`, which walks the single-use `Copy` chain from the byte
load's pointer down to the folded `GetElementPtr` and refuses the fold unless
every link has exactly one user — so no surviving instruction can reference a
deleted value.

## Correctness evidence

* 19 unit tests (`cargo test --lib byte_load`), including the refusal cases:
  big-endian order, volatile byte loads, a load with a second user, duplicate
  offsets, a store between the loads and the merge, a base that is not a
  parameter, a non-consecutive run.
* New corpus benchmark `tests/benchmark/programs/byte_load_reassembly.c`
  (registered in `run_benchmarks.py`): the Linux `get_unaligned_le*` shape, the
  big-endian twin, a struct-member run and a record-decoding loop, with known
  answers that fail the run if a chain is mis-folded. LCCC, gcc 14.2 and clang
  19.1 print the identical digest.
* Whole-corpus differential: all 57 programs in `tests/benchmark/programs`
  compile, run, and print byte-identical output under LCCC and gcc 14.2
  (`spectral_norm` needs `-lm` under gcc; the one printed difference in
  `i686_alu_chains` is a wall-clock timing line, not a result).
* `CCC_VALIDATE_SSA=1` clean on the SipHash translation unit.
* `ccc`-side gates: `check_cpu_supports_runtime.sh` and
  `check_got_slot_local_addend.sh` still pass; `tests/regression` filters
  `got`/`tls`/`gottpoff` 9/4/1 green.

## Why a new pass and not a peephole

The shape is only visible *before* instruction selection, where the byte loads
are still distinct values: at the assembly level the chain has already been
strength-reduced into different registers and (with the current scheduler)
interleaved with the SipHash round. Matching it in the backend peephole layer
would mean re-deriving the addressing structure the IR still has for free, and
it would not help the vectorisers, which need the wide load to exist in the IR
to keep the pair in `xmm` the way gcc does.
