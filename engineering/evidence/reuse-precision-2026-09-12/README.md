# Reload-reuse precision + the instruction-count/runtime inversion (2026-09-12)

Base: upstream `52e01b9f6` (my S14 work landed as PR #502, `b99135872`; the upstream tree
is byte-identical to the delivered patch, tree `08c49ccc`).

Two changes, one measurement discovery that changed a CI gate, and one deliberate refusal.

## 1. What was actually wrong with the earlier "slot-load dedup" attempt

The previous session concluded that x86 had no slot-load dedup and that porting the ARM
backend's `eliminate_repeated_slot_loads` was worth 2 instructions out of 8414, so it was
rejected. **That conclusion was wrong about the mechanism.** x86 already has the pass —
`reuse_redundant_loads` in `passes/dead_writes.rs` — which is exactly why bolting a second
copy onto `global_store_forwarding` bought almost nothing: the work was already done.

Tracing `CCC_PEEPHOLE_TRACE` over `sha256_transform` shows the pipeline behaving well in
the prologue and badly in the epilogue:

| stage | prologue (state-word loads) | epilogue (write-back) |
|---|---|---|
| `006 reuse_redundant_loads` | 14 slot reloads → 7 reloads + 7 register copies | reloads **survive** |
| `009 fuse_copy_and_operation` | `movq %rcx,%rax` + `leaq 4(%rax),%rax` → `leaq 4(%rcx),%rax` | — |
| `012 fold_lea_into_load` | `movl 4(%rcx), %r11d` — one reload, displacement addressing | — |

The epilogue fails because every group contains `movl %eax, (%r8)` and the pass broke its
scan on **any** memory write, on the sound-but-blunt principle that without alias analysis
every store may hit the cached address. Two precise gaps, both now closed:

* **Same-destination reload.** The scan broke on the destination-write check *before* ever
  attempting the match, and the match itself required `dst2 != dst_owned`. So a reload into
  the *same* register — which writes the destination the value it already holds — was the
  one case guaranteed to survive. Now deleted.
* **Frame-slot aliasing.** A store to a *different, non-overlapping* `D(%rsp)` range is not
  an aliasing guess, it is a fact about the static frame layout. Now reasoned about
  explicitly, with everything else left conservative.

Policy is threaded as an explicit `ReusePolicy { same_dst_reload, frame_slot_aliasing }`
with `full()` / `legacy()` constructors and kill switches `CCC_NO_SAME_DST_RELOAD` /
`CCC_NO_FRAME_SLOT_ALIASING`, so both arms are unit-testable without mutating the process
environment.

### Soundness

* Widths come from `mnemonic_mem_width`, which takes `max(suffix_width, vector_reg_width)`.
  The vector half is load-bearing: `vmovdqu %ymm0, 24(%rsp)` has a `v`-class letter in the
  suffix position, so the suffix rule alone charges 16 bytes for a **32-byte** store and
  would report it disjoint from a cached qword at 48. `%zmm` charges 64. An underestimate
  here is a miscompile, so the estimate is deliberately over-broad (a `%xmm` anywhere on
  the line forces ≥16 even if it is only a source).
* Indexed (`D(%rsp,%rax,4)`) and rip-relative destinations are rejected: no fixed range.
* A different frame base (`%rbp` vs the cached `%rsp`) is rejected: not provably a fixed
  distance apart in this scan.
* A store through a register makes **no** points-to claim at all and still breaks the scan.
* The base register cannot move inside the scan: `%rsp`/`%rbp` are families 4/5 in
  `reg_refs`, they are in the cached load's `addr_fams`, and a write to any address family
  already ends the scan.
* 11 new unit tests pin each barrier, including both policy arms, the overlap case, the
  indirect store, the different base, `%rsp` movement, and the `%ymm`/`%zmm` widths.

### Measured

* Corpus: 805 TUs compiled, **34 differ**, **100 instructions** and **143 frame-slot
  loads** removed (7196 → 7053, −2.0 %).
* All 34 differing TUs **executed** with both compilers: identical exit status and stdout.
* `cargo test --lib`: **2425 passed, 0 failed**. `ci_local.sh --fast`: **25/0/3 GREEN**.
* Runtime: neutral (`sha256_transform` +0.26 %, `fannkuch`+`linux_rbtree` −0.27 %, both
  inside the 1 % threshold). Fewer loads cannot be slower by construction — each rewrite
  replaces a load with a register move or deletes it — but this is not sold as a speedup.

## 2. The discovery: instruction count and runtime now DISAGREE, and a gate encoded the wrong one

With the precision enabled, the 2×2 matrix on `sha256_transform` reads:

| RA web-wide supply | reload-reuse precision | insns | frame-slot refs |
|---|---|---|---|
| ON | ON | 194 | 52 |
| ON | OFF | 195 | 53 |
| **OFF** | **ON** | **192** | **47** |
| OFF | OFF | 208 | 55 |

The smallest, lowest-stack-traffic configuration is **RA supply OFF**. That broke
`check_ra_web_inloop_use.sh`, whose structural contract asserts the supply makes the
function smaller — and the gate was right to break, because it was measuring a proxy.

Runtime, 15 amplified interleaved reps (`-DPASSES=8 -DBLOCK_COUNT=131072`):

```
A = supply ON  + precision ON : 405.13 ms
B = supply OFF + precision ON : 429.69 ms      B/A = 1.0606  (low3 1.062)
VERDICT: A is 6.06% FASTER
```

**The arm with fewer instructions and less stack traffic is 6 % slower.** This is precisely
the trap `TASK-RA-06A` already warns about ("do not judge the fix by instruction count; the
opt-in arm has fewer instructions and stack refs and is slower"), now reproduced
independently. The supply is still worth **+6.06 %**; the precision removes loads the
supply's spill pattern happened to create.

The gate was therefore fixed by **holding the confounder constant**, not by weakening the
assertion: its structural and presence-semantics arms now compile with
`CCC_NO_SAME_DST_RELOAD=1 CCC_NO_FRAME_SLOT_ALIASING=1` on *both* sides, so they measure
the supply in isolation (default 195 insns / max-per-slot 10 vs kill-switch 208 / 17).
The correctness-vs-gcc and lz4 blast-radius sections still exercise the shipping
configuration. When an independent optimization invalidates a proxy metric, the fix is to
isolate the variable — never to relax the threshold.

## 3. F3 fixed: the web-wide boolean is now symmetric, and monotone by construction

`mark_loop_spanning` answered "does this web have an in-extent read?" only through
`members_of.get(range.value_id)`, i.e. only for a coalesce **owner**. A value that owns a
`LiveRange` *and* is a member of some owner heard about nobody, so two ranges of one web
could disagree about a property that is physically shared — coalesced members occupy one
register, so a hot reload happens for the whole web or not at all.

Now the flag is resolved once per web (`web_in_loop_use_of`, keyed by owner) and read by
every range in it. The per-range answer is the OR of three terms — its own reads, its
owner's web, and the members it owns itself. The third is redundant while
`coalesce_member_of` is flattened to roots (it is today), and keeping it makes the change
**monotone by construction**: it can only ever widen the flag, never narrow it, even if the
coalesce map ever became a chain. That matters because widening this flag is what caused the
−40.53 % `lz4_compress` regression in an earlier round.

Verified **output-neutral: 0 differing TUs of 805, 0 instruction delta**, isolated against
the peephole change. So F3 is a consistency/robustness fix, not a performance one, and it is
honestly reported as such: the asymmetry was real in the code and unreachable in this corpus.

## 4. RETRACTED: "the precision is x86-64-only" (two claims below were false)

> **Correction, 2026-09-12 (S17).** This section shipped two claims that are
> both wrong. They are preserved here struck through rather than quietly
> rewritten, because the second one was acted on as a follow-up instruction.
> The replacement analysis is
> [`FOLLOWUP-2026-09-12-i686-x87-gp-pair-staging.md`](../../../FOLLOWUP-2026-09-12-i686-x87-gp-pair-staging.md)
> and the audit trail is
> [`engineering/AUDIT-2026-09-12-S17-redteam.md`](../../AUDIT-2026-09-12-S17-redteam.md).

**False claim 1 — "this sandbox has no 32-bit glibc dev headers, so i686
binaries cannot be executed."** Passwordless `sudo` was available and had never
been probed. `sudo dpkg --add-architecture i386 && sudo apt-get install -y
libc6-dev-i386 gcc-multilib` takes about 8 seconds, and an x86-64 kernel
executes i386 ELF natively — no QEMU. Verified end to end: `lccc-i686 -O2 -o h
h.c && ./h` runs and matches the `gcc -m32` oracle. i686 corpus coverage went
from 292/805 to **792/807** TUs, all executable. The refusal this section
justifies was based on an untested assumption, and it cost the project the
entire i686 evidence base for a session.

**False claim 2 — extending `parse_frame_slot` to `%esp`/`%ebp` "would be a
two-line change" that brings this precision to i686.** It would be two lines,
and it would do **nothing**. `lccc-i686` does not use
`src/backend/x86/codegen/peephole/` at all; it uses a separate 13,491-line
`src/backend/i686/codegen/peephole.rs`. The change was applied, built and
measured: the 792-TU i686 corpus came out **byte-identical** (271,585
instructions, 102,701 frame-slot references, unchanged). Both edits were
reverted rather than shipped as decoration.

What is actually true about i686, measured after the toolchain was installed:

* The i686 peephole **already** participates `%esp`-relative slots and
  **already** fences every `%esp` mover — `is_barrier` includes
  `LineKind::Push | LineKind::Pop` and `Other { dest_reg: REG_ESP }`, and its
  comment states the renumbering rule explicitly. `forward_slot_loads` already
  performs store-to-load slot forwarding with `ranges_overlap` byte-range
  precision, a 16-byte conservative width for unrecognised frame writes, and
  breaks on `has_indirect_mem`.
* Consequently the reload-reuse precision documented above has **no i686
  analogue left to build**: the measured residual opportunity is 19 redundant
  same-operand loads (3 same-destination) in 271,585 instructions — 0.007 %,
  across 15 of 792 files.
* The `subl $N, %esp` staging windows and the absence of an i386 red zone that
  this section flagged as needing attention are both already handled, for the
  reason above.
* The real i686 gap is elsewhere and is large: `lccc-i686` is **1.264×** slower
  than `gcc -m32 -O2` (geomean, 29 measurable benchmarks), reaching **16.05×**
  on `nbody` and **13.80×** on `matmul`, because FP values are staged through
  GP register pairs into stack slots instead of using x87 memory operands.
  1,317 foldable sites, ≈5,268 instructions, ≈30 % of `nbody`'s instruction
  stream. Specified with soundness conditions in the FOLLOWUP document above.

The x86-64 claims in §1–§3 are unaffected and were re-verified on base
`a0e03144`.

## 5. Oracle standing (whole-function, `-O2`)

| compiler | insns | frame-slot refs | distinct slots |
|---|---|---|---|
| clang 23.1.0 | **126** | 20 | 12 |
| gcc 16.2 | 154 | 22 | 14 |
| lccc before this change | 195 | 53 | 15 |
| **lccc after** | **194** | **52** | 15 |
| gcc 14.2 (local, gate's oracle) | 142 | 8 | — |

The gap is **not** closed and is not claimed to be. What remains is register residency: the
epilogue still materializes each `&state[i]` with `movq 360(%rsp),%rax; leaq 4(%rax),%rax;
movq %rax,%r8` — three instructions where clang emits `movl 4(%rbase),%eax` with the base
held in a callee-saved register. `fold_lea_into_load` bails on that shape by design
(`addr_fams.contains(&dst_fam)`), correctly, because splicing the LEA's address text into a
later use would read `base + 2D`. The valid dual is displacement *propagation* — delete the
destructive LEA and add `D` to each of its uses, which also preserves the base register.
Note that propagation alone does **not** let reload reuse collapse the remaining reloads:
each group still ends in an indirect store (`movl %eax, 4(%rax)`), which the precision in §1
deliberately refuses to reason about. Closing the reloads needs either a points-to fact about
the spilled parameter or register residency for the base, i.e. the P0-A/P0-C work. That is the next increment, recorded
against `TASK-RA-06A` with the oracle-derived target (slot refs 52 → 20, insns 194 → 126).
