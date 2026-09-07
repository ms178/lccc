# FOLLOWUP — 2026-09-07: native rotates, imm32 bit-pattern hoisting, duplicate GlobalAddr merge; and the stack-slot-traffic wall behind them

## TL;DR

**Landed here (3 fixes, all measured, all in `ms178-1.patch`):**

| # | fix | headline effect |
|---|---|---|
| A | native rotate idiom (`RotateLeft`/`RotateRight`) | `chacha20_block` 9.86× → **4.41×** slower (2.24× faster); `sha256_transform` 2.76× → **2.04×** (1.35× faster). Also fixed a latent `unreachable!()` **crash** in the MachInst window-regalloc path. |
| B | imm32 **bit-pattern** constant hoisting | lz4 1.01× faster [0.984, 0.994], no regression anywhere. Removes a per-iteration load for every 32-bit hash multiplier (`0x9E3779B1`, xxHash, MurmurHash3). **Also un-dead-locked the pass on AArch64**, where it was a no-op for its own motivating case. |
| C | duplicate variable-index `GlobalAddr` merge | **LANDED.** Worth `expat_xml_scan` **−23.3%** (0.767) and `lz4_compress` **−15.3%** (0.847), geomean **0.943** over 7 benchmarks. It initially miscompiled `tests/regression/pic_indexed_store_static_global.c` (`-O2 -fPIC`, RC4-style indexed swap on a static struct) — **but the defect was not in C.** Root cause: the x86 peephole predicate `line_writes_memory` located an instruction's destination with `rfind(',')`, which for `movb %al, (%r10,%r12)` lands on the SIB comma *inside* the address, so every store through an indexed address was invisible to the guard that stops `reuse_redundant_loads` from reusing a pre-store load. C merely hoisted the base into a register, making the address indexed and eligible. Fixed at the source (canonical `last_top_level_comma`), whole bug class audited, 6 tests added. Full diagnosis below. |

**Still needed upstream, ranked** (detail in Part 2):

1. **The duplicate `GlobalAddr` merge (fix C) is a large, already-measured win
   that currently miscompiles** — `expat_xml_scan` 1.29x faster, `lz4_compress`
   1.21x faster. Reproducer, asm-level diagnosis and the opt-in patch are in
   Part 2 section C. Highest value-per-effort item here *if* the underlying
   emitter/RA bug is found, because the measurement and the patch already exist.
2. **mem2reg / SROA for constant-indexed scalar arrays** — the single biggest
   remaining win. There is no general mem2reg and `aggregate_sroa.rs` only handles
   Memcpy'd structs, so ChaCha20's local `u32 x[16]` stays in memory for all 20
   rounds (371 stack movs vs GCC's 52). This is what the remaining 4.40x *is*.
3. **Linear-scan RA is homing long-lived loop-carried pointers to stack slots** —
   lz4 does 49-51 frame movs in a 184-byte frame where GCC does 10 in 56 bytes,
   with both using all 15 GPRs. Includes pure slot-to-slot copies. This is very
   likely also the root of item 1's miscompile: "value N has no register home, no
   stack slot and no acc-cache entry" is precisely the failure mode the
   `pic_indexed_store_static_global` regression test was originally written for.
4. **PF-07 global-address promotion misses lz4's `hash_table`** — the mechanism
   exists and works for `gzip_crc32`, but lz4's bases never enter the candidate
   `set`; about 3 instructions per iteration of the match-search loop.
5. **`int_const_hoist` has no RISC-V immediate model** (it is treated as x86, so
   RISC-V under-hoists exactly the way AArch64 did before fix B).
6. **The AArch64 half of fix B is unmeasured** — no AArch64 execution capability
   in this sandbox (no qemu-aarch64, no cross GCC); validated by unit tests and
   asm inspection only. Needs a real AArch64 benchmark run.
7. **The 39-benchmark corpus is not a correctness gate.** It reported 39/39
   `Correct: pass` on a build that miscompiled a `-fPIC` regression kernel;
   `scripts/run_regression_suite.sh` caught it. Performance work must pass the
   regression suite, not only the benchmark output check.
8. **Infra:** `.gitignore`'s `bench_*` swallows harness sources (fixed here for
   `scripts/bench_rank.py`); `isel.rs`'s `unreachable!()` catch-alls are invisible
   to `cargo check`; `arena_session_restore.sh` did not handle a missing `.git`
   (fixed in this series, see Part 3).
---

Base ref: `8ef2c1d67a40fa7e83d7f02f5839438b3cc85988` — current `ms178/lccc` `main`,
so this series is rebased on latest upstream (`git merge-base HEAD origin/main`
== `origin/main`).

The rebase picked up upstream `42537a9` *"Gate x86 vectorization on ISA flags;
veto aggregate-frame inlining at -Os"*, which introduces the `X86Isa`
code-generation permission struct in `src/passes/mod.rs`. That resolves the
"x86-64-v3 vector width / FMA gating" item carried by earlier sessions' follow-up
notes — it is no longer open work. The only rebase conflict was purely additive
(both sides inserted a new item at the same point in `src/passes/mod.rs`:
upstream's `X86Isa` struct and this series' `target_rotate_bits`); both are kept.

Host: 2-core Xeon @ 2.60 GHz, 1.9 GB RAM + 8G `/swapfile`, AVX2/FMA/BMI1/BMI2/POPCNT.
No hardware PMU (`perf` absent), so every performance claim below is paired,
randomized-order, CPU-pinned wall clock from `tests/benchmark/run_benchmarks.py`,
with bootstrap CIs. Measurement class is *VM screening*: it reliably ranks
directions and large effects, and it is not a substitute for PMU attribution.

LCCC itself is built only with the `fastbuild` cargo profile (opt-level 1, no LTO,
incremental) and `CARGO_BUILD_JOBS=2`, per the standing research policy.

---

## Part 1 — what landed, and the evidence for it

### A. Native rotate idiom (`RotateLeft` / `RotateRight`)

The ARX rotate idiom `(x << n) | (x >> (W - n))` was being emitted as a shift /
shift / or triple through a temporary on every target. Every hash and cipher
spells it out portably (`ROTL32`/`ROTR` in ChaCha20, SHA-256, MD5, BLAKE2,
siphash; the kernel's `rol32`/`ror32`), and x86 `rol`/`ror`, AArch64 `ror`/`extr`
and RISC-V Zbb `rol`/`ror` each cost one instruction.

Added a canonical IR pair plus:

* width-aware constant folding — **rotates must not be folded through
  `eval_i64`/`eval_i128` for typed I32/U32**, which would rotate at the
  container width (`0x80000001 rol 1` → `0x2`, not `0x3`);
* `match_rotate` in `src/passes/bit_idioms.rs`, wired into `recognize_function`
  behind a `max_rotate_bits` capability (`target_rotate_bits()` in
  `src/passes/mod.rs`: X86_64/Aarch64/Riscv64 = 64, I686 = 32 — I686 routes I64
  through the paired-register `emit_i128_binop` path, not the 32-bit ALU);
* native lowering on x86-64 / i686 / AArch64, RISC-V funnel shift, MachInst
  `ShiftOp::Rol`/`Ror`;
* simplify identity folds and `loop_unroll` classification.

Two recognition rules that were needed to reproduce GCC exactly and to stay correct:

* `match_rotate` **must run strictly after** `bit_reverse32` and
  `bswap32_network`. bswap's final stage `(y >> 16) | (y << 16)` *is* a
  rotate-by-16; recognising it first destroys `bswapl`. After each replacement the
  walker does `index += consumed; continue;` so stale def-lists cannot re-match.
* Rotate may match `LShr` on the right half, **never `AShr`** — folding an
  arithmetic-shift pseudo-rotate is a miscompile. Rotate-by-zero / by-W identity
  folding *is* safe (unlike `BitTest`), because the count is masked mod W.
* Direction rule: with a run-time amount, the direction whose count is the direct
  value wins; with two constants, the smaller amount wins
  (`(x>>7)|(x<<25)` → `ror $7`, `(x<<12)|(x>>20)` → `rol $12`).

**Compiler crash found and fixed on the way.** `src/backend/x86/codegen/isel.rs:619`
(MachInst window-regalloc path) had a `_ => unreachable!("unhandled binop")`
catch-all. `cargo check -D warnings` cannot flag it: exhaustiveness is satisfied
by the wildcard. It only fires at runtime, on the window-regalloc path. Added
`Rol`/`Ror` to `machinst.rs`, `shift_mnemonic` (8 arms), the `ShiftX` unreachable
guard, `binop_to_shift`, and an early return in `try_lower_shiftx` (BMI2 has no
register-count rotate; `rorx` is immediate-only and right-only).

**Lesson for any future IR-op addition:** adding an `IrBinOp`/`ShiftOp` variant
ripples to ~17 files / ~65 references. Exhaustiveness catches most of them under
`-D warnings`, but `isel.rs`'s catch-alls are invisible to the type checker.
Every backend path — including MachInst — must be runtime-tested.

Measured (x86-64, `-O2 -march=x86-64-v3`, vs GCC 14.2, reps 9):

| workload | before | after | change |
|---|---|---|---|
| `chacha20_block` | 9.863× slower | **4.408× slower** | 2.24× faster |
| `sha256_transform` | 2.760× slower | **2.044× slower** | 1.35× faster |
| everything else | — | — | unchanged (1.223–1.713) |

Correctness: 9-case probe (`rl_const`→`roll $16`, `rr_const`→`rorl $7`,
`rl_var`→`roll %cl`, `rr_var`→`rorl %cl`, `rl64`→`rolq $23`; AShr- and
bad-sum-based pseudo-rotates correctly *not* folded; `bswap32` and the bswap
network still → `bswapl`). Output matches GCC across `-O0 -O1 -O2 -O3 -Os` and
`-m32`. Regression suite 646 PASS / 0 FAIL / 0 AB-diff failures; `cargo test`
2030 passed / 0 failed.

### B. imm32 **bit-pattern** constant hoisting (`src/passes/int_const_hoist.rs`)

Symptom, in lz4's match-search loop:

```asm
    movl  $0x9e3779b1, %eax        ; preheader
    movq  %rax, 0x28(%rsp)         ; ...constant parked in a stack slot
.LBB14:
    imul  0x28(%rsp), %eax         ; memory-operand multiply, every iteration
```

GCC emits `imul $0x9e3779b1, %edi, %eax` — a self-contained 3-operand immediate.

Root cause: `large_int_const()` decided "is this constant free as an immediate?"
with a **signed** i32 range test. That is right for 64-bit instructions
(`imulq $imm32` sign-extends) and wrong for 32-bit ones: `imull`/`andl`/`cmpl`
take the imm32 **bit pattern verbatim**, so all of `[0, u32::MAX]` is free. The
backend already knew this — `const_as_imm32_typed(op, is_32bit_op)` in
`src/backend/x86/codegen/emit.rs` documents exactly that rule — but the pass did
not, so it hoisted the constant into a slot and turned a free immediate into a
load per iteration plus a live register across the loop.

This is not an lz4 curiosity. It hits essentially every 32-bit hash multiplier in
existence, all of which live in the unsigned half of `u32` by design:
`0x9E3779B1` (lz4, zstd, xxHash, the Fibonacci hash), `0x85EBCA77` and
`0xC2B2AE3D` (xxHash32 primes 2 and 5), `0xCC9E2D51` and `0x1B873593`
(MurmurHash3 c1/c2).

Fix: `for_each_int_operand` now reports an `imm32_exact` flag derived from the
instruction's own `ty`, mirroring the backend predicates (`matches!(ty, I32|U32)`
for `BinOp`, as in `alu.rs`; the same for `Cmp`, as in `comparison.rs`). Shift
counts are explicitly excluded — they encode as imm8/`%cl`, never imm32. The
div/rem `needs_reg` branch is untouched: `div_by_const` expands those into a
**64-bit** magic multiply, where sign extension genuinely applies.

**A second, larger bug fell out of the same function.** The signed-i32 shortcut
was not gated to x86:

```rust
if (-4095..=4095).contains(&v) && AARCH64 { return None; }   // imm12 free
if v >= i32::MIN && v <= i32::MAX       { return None; }     // <- applied to AArch64 too
```

On AArch64 the second line suppressed hoisting for *everything* in `[-2^31, 2^31)`
— which is where every interesting AArch64 constant lives, since imm12 tops out at
4095. The pass's own motivating example, sieve's marking bound
`cmp j, #10000000`, fits in i32 and was therefore **never hoisted**: on AArch64
the pass was a no-op for the exact case it was written for. Now the two immediate
models are properly separated (AArch64: imm12/cmn only; x86-64: bit-pattern u32
for 32-bit ops, signed i32 for 64-bit ops).

Measured, paired against the pre-fix binary in a single harness run
(`--lccc`/`--ccc`, reps 11, CPU-pinned, seed 20260907):

| workload | LCCC(new)/LCCC(old) | CI |
|---|---|---|
| `lz4_compress` | **0.989 (1.01× faster)** | [0.984, 0.994] |
| `chacha20_block` | 1.001 | [0.999, 1.002] |
| `sha256_transform` | 1.000 | [1.000, 1.004] |
| `zstd_count` | 0.998 | [0.983, 1.010] |
| `expat_xml_scan` | 0.998 | [0.996, 1.009] |
| `gzip_crc32`, `hash_table`, `sqlite_varint`, `zlib_ng_adler32` | ≈1.000 | — |

So: a small but CI-significant win on lz4, and no regression anywhere. The value
of this fix is mostly *class* removal — a whole family of hash kernels no longer
pays a load per iteration — plus restoring a dead pass on AArch64.

11 new unit tests, including the boundary values (`0x7FFFFFFF`, `0x80000000`,
`0xFFFFFFFF`, `0x1_0000_0000`), the shift-count exclusion, the `Cmp` path, and
AArch64 regression guards at `4095`/`4096`/`10_000_000`.

> **Unmeasured:** there is no AArch64 execution capability in this sandbox (no
> `qemu-aarch64`, no cross GCC), so the AArch64 half of fix B is validated by unit
> tests and asm inspection only. It needs a real AArch64 benchmark run upstream.

### C. Duplicate variable-index `GlobalAddr` merge (`src/passes/global_addr_cse.rs`)

`classify_site_local_indexed()` keeps a global's address at each of its use sites
(refusing CSE/hoist) when it feeds a variable-index GEP whose stride is
1/2/4/8 — the strides an x86 SIB scale can encode. The stated rationale is that
hoisting lengthens the base's live range and can evict the natural index,
"destroying x86 `sym(,%idx,scale)` selection".

That rationale no longer holds, because **the backend never selects that form**.
Counting absolute `sym(,%reg,scale)` memory operands in generated assembly:

| flags | absolute `sym(,%reg,scale)` operands |
|---|---|
| `-O2 -march=x86-64-v3` | **0** |
| `-O2 -march=x86-64-v3 -fno-pic -fno-pie` | **0** |
| `-O2 -march=x86-64-v3 -fno-pie` | **0** |

A RIP-relative base cannot simultaneously carry an index register, so a
variable-index global access pays a base register (or a rematerialised
`leaq sym(%rip)`) *no matter where the base lives*. The exemption was protecting
a phantom, while the cost was concrete: the register allocator rematerialises each
**unmerged** IR value separately at each of its own uses.

The final IR for lz4's `main` had all ten relevant `GlobalAddr`s already in the
entry block (so LICM had done its job), but **three separate values for
`hash_table`** and four for `src_data`:

```
block 0: dst_data, dst_data, src_data, src_data, src_data,
         hash_table, hash_table, hash_table, dst_data, src_data
```

and the match-search loop consequently re-derived the base three times per
iteration:

```asm
.LBB14:
    leaq  hash_table(%rip), %rcx      ; (1)
    movl  (%rcx, %r14, 4), %r9d
    leaq  src_data(%rip), %rcx        ; (2)
    leaq  (%rcx, %r9), %rax
    ...
    leaq  hash_table(%rip), %rcx      ; (3)
    movl  %esi, (%rcx, %r14, 4)
```

`gzip_crc32` had the same shape in cold code: `leaq gzip_crc_data(%rip)` three
times in three consecutive instructions.

Fix: withdraw site-locality when the **same symbol is materialised two or more
times** in the function. The live-range trade only exists for a single occurrence;
with two there is nothing left to protect, only a duplicate `leaq` to delete.
Single-occurrence bases keep the old contract (new test pins it).
Kill switch: `CCC_NO_GADDR_MERGE_DUP_INDEXED`.

Measured, paired ON-vs-OFF across the **full 39-benchmark corpus** in one harness
run (reps 7, warmup 1, CPU-pinned, seed 20260907). Every benchmark also reported
`Correct: pass` against GCC.

| workload | LCCC(new)/LCCC(old) | CI | vs GCC before → after |
|---|---|---|---|
| `expat_xml_scan` | **0.774 (1.29× faster)** | [0.770, 0.775] | 1.713 → **1.317** |
| `lz4_compress` | **0.828 (1.21× faster)** | [0.803, 0.846] | 3.012 → **2.505** |
| `sqlite_varint` | 0.984 (1.02× faster) | [0.983, 0.985] | 1.205 |
| `zstd_count` | **1.025 (1.02× slower)** | [1.012, 1.037] | 1.419 |
| other 35 | 0.995 – 1.007 (neutral) | — | — |

Aggregate vs GCC: geometric mean **0.7885**, arithmetic mean 1.1124, n=39.
Static effect on lz4: 283 → 278 instructions, 184 → 168-byte frame, 13 → 11
`leaq sym(%rip)`.

`zstd_count` is the one real regression and it is the honest cost of the change:
a merged base now occupies a register across a loop. 2.5% against 29% and 21% is a
clear net win, but if it matters the gate can be tightened to additionally require
that at least one occurrence is loop-contained.

Two pre-existing tests encoded the old behaviour with the same symbol twice
(`variable_index_global_addrs_remain_site_local`,
`derived_variable_index_bases_remain_site_local`). The first was rewritten to
assert the merge; the second was repointed at two *distinct* symbols so its actual
coverage — that the parent-chain walk sees through `Copy` and same-size `Cast` to
the `GlobalAddr` root — survives intact. 17 tests in the pass, all green.

#### C miscompiled — and the defect was NOT in C

`scripts/run_regression_suite.sh` on the rebased tree caught it:

```
FAIL  pic_indexed_store_static_global (GCC mismatch)
      gcc : 72ade9eeea1898d9d30c975a36eb3b271d8b382381c91f36|0
      lccc: 72747640007c7e1808c886888a8c4623250b969849299ea0|0
```

The first instinct — "the merge is unsound, revert it" — was **wrong**, and
reverting would have buried a real backend miscompile that any future
base-register hoist could re-trigger. What follows is the actual root cause.

##### Localization: the merged IR is correct

A 4-way bisection knob (`CCC_GADDR_MERGE_DUP_INDEXED_SCOPE=none|cse|gvn|both`,
since deleted) split the transform across its two consumers. `none` was correct;
`cse`, `gvn` and `both` all produced the **identical** wrong hash. Two passes
that merely emit the same IR cannot independently be unsound — so the IR was
exonerated and the backend indicted.

Confirmed directly. `CCC_DUMP_EACH_PASS=1 CCC_DUMP_FUNC=HashFinalX` dumps the
function after all 70-odd passes; the swap loop's reload is present in every
one of them, including the last:

```
GetElementPtr { dest: Value(23), base: Value(22), offset: Value(20) }   ; &gh.s[i]
Load  { dest: Value(24), ptr: Value(23) }                              ; t = gh.s[i]
GetElementPtr { dest: Value(40), base: Value(22), offset: Value(37) }   ; &gh.s[j]
Load  { dest: Value(41), ptr: Value(40) }                              ; v = gh.s[j]
Store { val: Value(41), ptr: Value(23) }                               ; gh.s[i] = v
Store { val: Value(24), ptr: Value(40) }                               ; gh.s[j] = t
Load  { dest: Value(53), ptr: Value(23) }                              ; RELOAD gh.s[i]
```

`CCC_VERIFY_IR=1` is silent on both variants. Disabling `gvn` **or** `expr_sink`
"fixed" it — and the dumped IR of the failing block is **byte-identical** with
`expr_sink` on and off. Identical IR, different asm: the defect is in codegen,
and it is sensitive only to register allocation, which is what those two passes
perturb.

##### Root cause: `line_writes_memory` mis-parses indexed addresses

The emitted loop, wrong on the left, correct on the right:

```asm
    movzbl (%r10,%r13), %eax        movzbl (%r9,%r13), %eax     ; v = s[j]
    movb   %al, (%r10,%r12)         movb   %al, (%r9,%r12)      ; s[i] = v
    movb   %dl, (%r10,%r13)         movb   %r8b, (%r9,%r13)     ; s[j] = t
    addl   %edx, %ebp        <--    movzbl (%r9,%r12), %r8d     ; RELOAD s[i]
    movzbl %bpl, %r9d               addl   %ebp, %r8d           ; t += s[i]
```

The left side never reloads: it adds the **stale** pre-store `t` (`%edx`) to
itself. `reuse_redundant_loads` (x86 peephole, `dead_writes.rs`) replaced the
reload with a register copy of the earlier load of the same address, and
copy-propagation then folded that copy into the `addl`.

Its guard against exactly that is `line_writes_memory`, which locates the
destination operand with `rfind(',')`:

```rust
let Some(comma) = t.rfind(',') else { ... };
let dst = t[comma + 1..].trim();
dst.contains('(') && !t.starts_with("lea")
```

An AT&T memory operand carries its own commas inside balanced parentheses. For
`movb %al, (%r10,%r12)` the **last** comma is the SIB separator between base and
index, so `dst` is `"%r12)"` — which contains no `(`. The predicate therefore
returned `false`, and *every store through an indexed address was invisible to
the guard*. The scan walked straight past both stores and reused the stale value.

The codebase already had the correct primitive — `last_top_level_comma`, whose
own doc comment spells out the hazard ("a conventional `memrchr(',')` cannot be
used to determine an instruction's destination operand"). `line_writes_memory`
simply did not use it.

##### Why C exposed it, and why it hid for so long

`load_operand`, which decides whether a load is even eligible for reuse, rejects
`%rip`-relative operands. Without the merge, the two `gh.s[i]` addresses came
from distinct `GlobalAddr` bases and materialized as `movzbl gh+2(%rip), %eax` —
never eligible, so the buggy guard was never consulted. Merging the duplicate
base hoisted it into a register (`%r10`), which made the address
`(%r10,%r12)`: eligible, indexed, and guarded by a predicate that was blind to
indexed stores. C did not create the bug; it removed the accident that was
masking it.

##### The fix

`line_writes_memory` now uses the canonical splitter, and the splitter itself was
relocated from `x86/codegen/peephole/types.rs` to `backend/peephole_common.rs`
(`pub(crate)`, re-exported at the old path so no call site changed) so every AT&T
backend shares one implementation instead of re-deriving the split. This is a
correctness fix, not a workaround: no transform was disabled, no IR was
re-split, and the optimization is fully retained.

##### Red-team audit of the whole bug class

All 20 `rfind(',')` operand splits in `src/backend/` were audited against the
one criterion that matters — *is the last operand ever an indexed memory
operand?*

| Site | Verdict |
| --- | --- |
| `dead_writes.rs` `line_writes_memory` | **BUG** — fixed (the miscompile) |
| `peephole_common.rs` `replace_source_reg_att` | Latent — hardened; `expect(dead_code)`-enforced unused in shipped builds, but it would have spliced into the middle of a memory operand |
| `helpers.rs:547` (LEA read check) | Safe — a LEA destination is always a register |
| `memory_fold.rs:1036` | Safe — fires only on `LineKind::StoreRbp`, a displacement-only stack slot |
| `liveness.rs:566`, `local_patterns.rs:1531`, `copy_propagation.rs:135`, `dead_code.rs:193` | Safe — each is gated on a *register* destination via `parse_dest_reg_fast`/`get_dest_reg`, which already used `last_top_level_comma` |
| `redundant_ext.rs:343,376` | Safe — gated on `movslq`/`movsxd`, register destination |
| `i686/codegen/peephole.rs` (4 sites) | Not reachable from this defect; i686 has no RIP-relative form and the sites parse register-register lines. Left as-is to keep the fix minimal and reviewable |

So the central classifier was already correct, and the one predicate that
inspects an arbitrary line's destination *specifically to detect memory writes*
was the single reachable failure — which is exactly the shape of a bug that
survives review.

##### Tests added (not weakened)

* `line_writes_memory_detects_indexed_stores` — 16 assertions: indexed stores
  with and without displacement and scale, indexed RMW, `lock`, single-operand
  memory writes, and the negative cases (loads, LEA, reg-reg, `cmp`) that must
  keep returning `false`.
* `reload_after_indexed_store_is_not_replaced_by_the_stale_load` — end-to-end
  through `peephole_optimize` on the exact s-box swap shape; asserts the reload
  still reads memory and that no stale-value copy appears.
* `reuse_still_fires_when_no_store_intervenes` — proves the fix did not simply
  disable the optimization: with no intervening store the redundant load is
  still eliminated.
* `last_top_level_comma_skips_sib_address_fields` — the splitter contract,
  including single-operand forms (where `rfind` invents a destination) and
  unbalanced parentheses (depth must not underflow).
* `att_source_rewrite_keeps_indexed_memory_destination_intact`.
* GVN: `gvn_keeps_variable_index_global_addrs_site_local` asserted the
  pre-merge behaviour (two `GlobalAddr`s survive). It was **replaced by two
  tests covering both sides of the policy** — `gvn_merges_duplicate_variable_
  index_global_addrs` (merge fires; both GEPs resolve to the one surviving base,
  following the `Copy` GVN leaves for copy-propagation) and
  `gvn_keeps_single_variable_index_global_addr_site_local` (a single-occurrence
  base stays site-local, unmoved and unrenumbered). Net coverage increased.

##### Validation and measured effect

```
regression suite : PASS=652 FAIL=0 SKIP=15 (AB-diff failures: 0)
cargo test       : 2045 passed; 0 failed; 6 ignored
cargo fmt        : clean      cargo clippy --all-targets: clean
pic_indexed_store_static_global: PASS at -O1/-O2/-O3, ±-fPIC, -fno-plt, -march=x86-64-v3
```

The 15 skips are environmental and honest: no 32-bit multilib (`Scrt1.o`
missing) and no `qemu-i386`, so the ELF32 subset skips rather than passing
vacuously — which is what the suite's own contract requires.

Paired 3-way A/B in a single harness invocation (merged vs. the same tree with
only the duplicate gate removed vs. GCC), `--reps 9 --warmup 2 --cpu auto`,
randomized compiler order, `artifacts/bench/fixC-final.{json,md}`:

| Benchmark | merged | no-merge | merged/no-merge |
| --- | ---: | ---: | ---: |
| `expat_xml_scan` | 53.36 ms | 69.60 ms | **0.767** (−23.3%) |
| `lz4_compress` | 9.77 ms | 11.53 ms | **0.847** (−15.3%) |
| `sha256_transform` | 587.58 ms | 595.44 ms | 0.987 (−1.3%) |
| `gzip_crc32` | 151.94 ms | 152.24 ms | 0.998 |
| `sqlite_varint` | 32.52 ms | 32.60 ms | 0.998 |
| `chacha20_block` | 1.2252 s | 1.2234 s | 1.001 |
| `zstd_count` | 16.25 ms | 15.68 ms | 1.036 (+3.6%) |
| **geomean** | | | **0.943 (−5.7%)** |

Static confirmation of the mechanism: on the reproducer, the merge cuts the
emitted instruction count from 108 to 94.

`expat` reproduces the pre-rebase measurement (0.767 vs 0.774) across a change
of upstream base, which is the reproducibility evidence a single-run number
cannot give.

**Honest caveat — `zstd_count` regresses 3.6%.** The bootstrap CI for that pair
is [1.020, 1.048], which excludes 1.0, so it is resolved rather than pure noise;
but the harness itself flags the benchmark as below its 20 ms reliability floor
(median 16 ms, CV 6.7–10%, "process-launch and scheduler noise may dominate").
Against a 23% and a 15% win and a 5.7% geomean improvement this is a clear net
positive, and it is recorded here as open work rather than explained away: the
follow-up is to diff `zstd_count`'s inner loop between the two binaries and
determine whether the merged base costs a rematerialization there.

##### What the benchmark suite got wrong

The perf harness ranked C as a win and said nothing about correctness; the
regression suite caught it. That asymmetry is the real process finding, and it
is unchanged by this fix: **the benchmark corpus is not a correctness gate.**
The RC4-swap shape (load–store–store–reload of the same indexed address) is now
covered at three levels — C regression test, peephole unit test, and splitter
unit test — but the corpus still cannot fail a build for miscompiling. See the
ranked list.

---

## Part 2 — the wall behind all three: stack-slot traffic

Fixes A–C moved lz4 from 2.96× to 2.56× and expat from 1.71× to 1.32×, but the
ranking is unchanged in shape: `chacha20_block` 4.40×, `lz4_compress` 2.56×,
`sha256_transform` 1.99×. All three have the same underlying defect, and it is
not an instruction-selection problem. (Ratios are LCCC/GCC from
`artifacts/bench/fixC-final.md` on base `f4875b4b`; `gzip_crc32` is the one
benchmark LCCC now wins outright at 0.90×.)

Counting frame traffic in the generated assembly for lz4's `main`
(`lz4_compress_block` inlines into it):

| | LCCC | GCC |
|---|---|---|
| frame size | **184 B** | 56 B |
| `mov` to a frame slot | 22 | 5 |
| `mov` from a frame slot | 27 | 5 |
| total frame `mov`s | **49–51** | **10** |
| memory operands total | 64 | 14 |
| distinct GPRs used | all 15 | all 15 |

Both compilers use every general-purpose register; LCCC simply also keeps ~5× more
values in memory. `chacha20_block` is the extreme case: **371 stack movs vs GCC's
52**.

Representative shapes from lz4's hot loop:

```asm
    movl  (%rsi), %eax
    movl  %eax, 168(%rsp)        ; store a value that was just computed
    ...
    movq  %rax, 24(%rsp)         ; ref -> slot
    ...
    movq  24(%rsp), %rax         ; reload
    cmpq  %r15, %rax
    ...
    movq  24(%rsp), %rax         ; reload again
    cmpq  96(%rsp), %rax         ; src also lives in a slot
```

and, in the loop preheader, a pure slot-to-slot copy:

```asm
    movq  104(%rsp), %rax
    movq  %rax, 128(%rsp)
```

The loop-carried pointers `ip`, `iend`, `mflimit`, `src`, `anchor`, `op` are all
slot-homed and re-loaded per iteration. GCC keeps them in registers.

### Ranked open work

**1. No mem2reg / no SROA for constant-indexed scalar arrays. (Biggest single win.)**

`src/passes/` has `aggregate_sroa.rs` (handles Memcpy'd **structs** only),
`loop_memory_promote.rs` (loop-specific) and `vector_temp_promotion.rs` (vector
temps). There is **no general mem2reg and no SROA for scalar arrays**.

ChaCha20's state is a local `u32 x[16]` addressed entirely by constants. It never
becomes 16 SSA values, so all 20 rounds do load / ARX / store against the stack
instead of living in registers — that is the remaining 4.40×, and it is why the
rotate win, large as it was, could not close the gap. Same shape in SHA-256's
`W[64]` schedule and in any kernel that uses a small local array as a register
file (very common in crypto and DSP).

Proposal: a dominance-frontier mem2reg over `Alloca` + constant-index `GEP`,
reusing the existing `CfgAnalysis`/dominator machinery and the IR's `Phi`.
SROA the array into per-element SSA values when every index is a compile-time
constant and the address never escapes; otherwise fall back to today's behaviour.
Expected effect: `chacha20_block` 4.40× → well under 2×, `sha256_transform`
2.04× → near parity.

**2. Linear-scan RA homing long-lived loop-carried scalars to slots.**

`src/backend/regalloc.rs` (~10k lines) is a linear-scan allocator with a rich
policy surface (`CCC_NO_HOT_LOOP`, `CCC_NO_COALESCE`, `CCC_NO_PHI_COALESCE`,
`CCC_NO_INDEX_HOME`, `CCC_CALLER_SAVE_SPANNING`, …). In lz4 the values that lose
are precisely the ones that should win: pointers live across the whole loop,
evicted in favour of shorter-lived temporaries.

Also worth a pass: a post-RA **slot-to-slot copy eliminator** (`mov slot1,%r; mov
%r,slot2` where the two slots hold the same value) — those are pure waste and
there are several in lz4's preheader.

**3. PF-07 global-address promotion misses lz4's `hash_table`.**

`src/backend/x86/codegen/prologue.rs` already contains the right mechanism (PF-07):
when a foldable `GlobalAddr` is defined at a strictly *shallower* loop depth than a
variable-index GEP that consumes it, drop it from `never_materialized` so it gets a
register home and the SIB access reads that home. It demonstrably works —
`gzip_crc32_update` hoists `leaq gzip_crc32_table(%rip), %r9` above the loop and
runs `xorl (%r9, %rdi, 4), %r8d` inside it, matching GCC.

It does not fire for lz4. A temporary diagnostic (since removed) printed, for
lz4's `main`:

```
[PF07] pic_mode=true set=[59, 60, 481] gmap_keys=[52,59,60,61,62,133,136,162,481]
[PF07]   gep base=60  blk=5  consumer_depth=1 def_blk=Some(0) def_depth=0 -> PROMOTE
[PF07]   gep base=481 blk=12 consumer_depth=2 def_blk=Some(0) def_depth=0 -> PROMOTE
[PF07]   gep base=481 blk=14 consumer_depth=2 def_blk=Some(0) def_depth=0 -> PROMOTE
```

`pic_mode` is true, depths are correct, promotion fires — but lz4's `hash_table`
bases (IR values **147 / 172 / 180**, all defined in block 0, all consumed by
variable-offset GEPs in the loop at block 14) are **not in `set`**, so PF-07 never
considers them and the emitter keeps re-deriving the base per use.

Corroborating: `CCC_NO_GLOBAL_ADDR_REMAT=1` changes lz4's output *not at all*
(byte-identical), so these bases are not arriving via
`build_rematerializable_global_addr_set_for`; they are on the **foldable** path.
The next step is to trace `build_foldable_global_addr_set_for` /
`build_global_addr_map_for` in `src/backend/generation.rs` and find why
147/172/180 are excluded while 59/60/481 are included. Worth ~3 instructions per
iteration of lz4's match-search loop (out of ~20), and the same shape recurs in
every table-driven kernel.

Note the interaction with fix C: merging the three `hash_table` values into one
web is what produced the 1.21× lz4 win, and it also means PF-07 now only has to
promote *one* value instead of three.

**4. `int_const_hoist` has no RISC-V immediate model.** The pass knows two models
(AArch64 imm12, x86 imm32) and treats everything else as x86. RISC-V `addi`/branch
immediates are 12-bit and need `lui`+`addi` above that, so RISC-V currently
under-hoists exactly the way AArch64 did before fix B. Add a third model.

**5. `.gitignore:40` `bench_*` swallows benchmark harness sources.** Fixed this
session for `scripts/bench_rank.py` with a `!` negation (the file already had two
such negations for the same reason — `bench_kernels.py`, `bench_iv_widen_ab.sh`).
Any future `scripts/bench_*` needs the same. Better: anchor the pattern to actual
build output rather than prefix-matching source names.

**6. `zstd_count` regresses 3.6% under the duplicate merge — unexplained.**

The paired CI [1.020, 1.048] excludes 1.0, so it is a resolved effect, not pure
noise, but the benchmark sits below the harness's own 20 ms reliability floor
(median 16 ms, CV 6.7–10%). Next step: diff `ZSTD_count`'s inner loop between
`/tmp/lccc-merged` and `/tmp/lccc-nomerge` and determine whether the single
merged base costs a rematerialization or an extra address computation per
iteration that the three separate RIP-relative bases did not. If it is a
rematerialization, this is the same mechanism as item 3 (PF-07) and the two
should be fixed together. Do not close this by disabling the merge: the geomean
is 0.943 and expat/lz4 are −23%/−15%.

**7. Residual `rfind(',')` operand splits on the i686 backend.**

The x86 audit (table above) cleared every reachable site, but
`src/backend/i686/codegen/peephole.rs` still has four `rfind(',')` splits
(~lines 469, 1471, 1603, 2730). They were left alone deliberately — they parse
register-register lines and i686 has no RIP-relative form, so none is reachable
from this defect, and touching them without an i686 execution oracle (no
multilib, no `qemu-i386` in this environment — the ELF32 regression subset
skips) would be unverifiable churn. They should be converted to the shared
`last_top_level_comma` when an i686 runner is available to prove the result.
`replace_source_reg_att` was hardened now precisely because leaving a known
mis-parse in a `pub(crate)` helper is a trap for the next caller.

---

## Part 3 — infrastructure hazards worth upstreaming

**Arena harness wipes drop more than `.git/config`.** Observed this session: the
entire `.git` directory, `/opt` (rustup), `/swapfile`, `/tmp`, `target/`, and the
+x bits on tracked worktree files. `artifacts/lccc.bundle` is **thin** (built from
a `--depth 200` clone) and cannot restore history by itself:

```
error: Could not read 4589b5b9b1c68ca55d358955d595bd26e7c3d603
fatal: Failed to traverse parents of commit b7f1819427818e2abb5dd9fdf615b53cd993a061
error: /home/user/artifacts/lccc.bundle did not send all necessary objects
```

What did work, and what `scripts/arena_session_restore.sh` should learn (it
currently only handles a missing `.git/config`, and its `git remote get-url`
check dies with `fatal: not a git repository` when `.git` is absent entirely):

```sh
git clone https://github.com/ms178/lccc.git /tmp/t               # latest upstream
mv /tmp/t/.git /home/user/lccc/.git                              # keep the restored worktree
cd /home/user/lccc && git reset -q --mixed <my-true-base>        # never `checkout -- .`
git diff --binary > /tmp/mywork.patch                            # capture the work
git reset -q --hard origin/main && git apply --3way /tmp/mywork.patch   # re-base
while read -r f; do [ -f "$f" ] && chmod +x "$f"; \
  done < <(git ls-files -s | awk '$1 == "100755" {print $4}')
```

**Confirmed again this session, with two additions.** The wipe also removed
`~/.rustup` (the toolchains) while leaving `~/.cargo/bin/rustup` in place *and
stripped of its +x bit* — so the failure mode is `Permission denied` on a binary
that exists, not `command not found`. Recovery is `chmod +x ~/.cargo/bin/*` then
`rustup toolchain install stable --profile minimal` (~10 s). `/swapfile` also
disappears; `scripts/ensure_swap.sh` recreates it and fails harmlessly on the
`/etc/fstab` write (no permission), which does not matter because `swapon`
already succeeded. And background process launches do **not** inherit the shell's
`PATH`, so every long-running command needs its own `export PATH="$HOME/.cargo/bin:$PATH"`.

The mixed reset is essential: it rebuilds the index from HEAD without touching the
worktree, so uncommitted work survives and shows up as ordinary modifications.
When the base has moved upstream, capture the work as a patch against the *true*
base first and re-apply it onto the new `origin/main` — this session that
re-application was conflict-free across 22 files, but `--3way` is what makes a
conflict survivable rather than fatal.
`ms178-1.patch` + `artifacts/` survived the wipe intact, which is the whole point
of snapshotting after every validated fix. Making the bundle **self-contained**
(`git bundle create ... --all` from an unshallow clone, or `--depth` deep enough to
reach a real root) would remove the network dependency from recovery.

**Harness/CLI gotchas (all cost time this session):**

* `run_benchmarks.py --opt -O2` fails argparse; use `--opt=-O2`.
* `--skip-perf` means *correctness only, no timing claim* — it does not mean
  "skip PMU probing". Use `--no-pmu-probe` for that.
* IR dump env vars are `CCC_DUMP_IR` / `CCC_DUMP_IR_FUNC` (not `LCCC_*`). The dump
  is a Rust `Debug` rendering, so opcode histograms need indent-aware parsing
  (instruction variants sit at 16 spaces, block labels at 12).
* `sed -n "<line>,+Np"` with a computed range breaks on empty or multi-match
  `grep`; extract the line with `grep -n | cut -d: -f1` first.
* `awk` instruction counting is fragile here — the system `mawk` does not
  understand `\s`. Use a Python analyser or `scripts/codegen_scoreboard.py`.
* Useful runtime bisection switches discovered: `CCC_NO_GADDR_CSE`,
  `CCC_NO_GLOBAL_ADDR_REMAT`, `CCC_NO_GEP_FOLD`, `CCC_DEBUG_GEPFOLD`,
  `CCC_NO_HOT_LOOP`, `CCC_NO_COALESCE`, `CCC_NO_PHI_COALESCE`,
  `CCC_NO_INDEX_HOME`, `CCC_CALLER_SAVE_SPANNING`. These allow A/B of backend
  policy with no rebuild — a wrapper script that exports the flag and `exec`s the
  binary can be passed to `run_benchmarks.py --ccc` for a properly paired
  same-run comparison against `--lccc`.

---

## Reproducing

```sh
scripts/build_lccc_fast.sh                     # fastbuild profile, -j2
scripts/run_regression_suite.sh                # 652 PASS / 0 FAIL / 15 SKIP expected
cargo test --profile fastbuild                 # 2045 passed / 0 failed expected
cargo fmt --check && cargo clippy --profile fastbuild --all-targets   # both clean

# The miscompile gate, across the flag matrix:
for f in "-O1 -fPIC" "-O2" "-O2 -fPIC" "-O3 -fPIC" "-O2 -fPIC -fno-plt" "-O2 -fPIC -march=x86-64-v3"; do
  target/fastbuild/lccc $f -o /tmp/g tests/regression/pic_indexed_store_static_global.c \
    && /tmp/g   # must print 72ade9eeea1898d9d30c975a36eb3b271d8b382381c91f36|0
done

# Backend-only localization, no rebuild needed:
CCC_DUMP_EACH_PASS=1 CCC_DUMP_FUNC=HashFinalX \
  target/fastbuild/lccc -O2 -fPIC -S -o /dev/null <repro>.c 2> each_pass.txt
CCC_DISABLE_PASSES=gvn,expr_sink,slforward,redundantloads,dse \
  target/fastbuild/lccc -O2 -fPIC -o /tmp/x <repro>.c     # exact-token, comma-separated

# Paired 3-way A/B in ONE harness invocation. `--lev`/`--ccc` are spare compiler
# slots: point them at variant builds of your own tree and every compiler is
# interleaved in the same randomized paired round, which is the only way to get a
# trustworthy delta on a 2-core VM. Here: merged vs. same-tree-without-the-merge
# vs. GCC. Build the variant by editing the source, `cp` the binary aside, then
# restore the source and rebuild — verify the rebuild is byte-identical to the
# saved binary so the tree you ship is the tree you measured.
python3 tests/benchmark/run_benchmarks.py \
  --only expat_xml_scan,lz4_compress,sqlite_varint,zstd_count,chacha20_block,sha256_transform,gzip_crc32 \
  --compilers lccc,lev,gcc --lccc /tmp/lccc-merged --lev /tmp/lccc-nomerge \
  --reps 9 --warmup 2 --cpu auto \
  --json artifacts/bench/fixC-final.json --markdown artifacts/bench/fixC-final.md
```

Evidence retained in `artifacts/bench/`: `baseline.{json,md}`,
`correctness.{json,md}`, `rotate-after.{json,md}`, `imm32-ab.{json,md}`,
`gaddr-ab.{json,md}`, `fixC-final.{json,md}`.
