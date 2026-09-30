# The two remaining gaps: BB-SLP on nbody, and a spilled induction variable in lz4

**Status:** root-caused, oracle-validated, NOT implemented. The candidate that
was pursued first (a bounded-trip guarded peel of nbody's triangular inner `j`
loop) was built, measured, found performance-neutral, and reverted. What
follows is the measurement that refuted it and the specification for what
replaces it, with the target instruction sequences taken from the Compiler
Explorer oracles rather than reasoned about.

A full 41-program paired benchmark run against GCC puts the geomean at
**0.7296** — lccc is ahead overall — and isolates the two places it is behind:

| benchmark           | lccc/gcc | what the gap actually is                   |
|---------------------|---------:|--------------------------------------------|
| `lz4_match_extend`  | **1.898** | loop-carried IV spilled to the stack       |
| `nbody`             | 1.251    | no BB-SLP over adjacent F64 pairs          |
| `expat_xml_scan`    | 1.253    | not yet root-caused                        |
| `hash_table`        | 1.211    | pointer chasing                            |

The largest gap in the entire suite is **not** the one the peephole work was
aimed at, and that fact is the main lesson of this document. Part 2 root-causes
`lz4_match_extend`, which is 1.9x — larger than nbody — and is a register
allocation defect, not a code-generation pattern defect.

## Part 1 — what was tried, and why it was thrown away

`nbody`'s inner loop is triangular:

```c
for (i = 0; i < NBODIES; i++)          /* NBODIES == 5, constant trip */
    for (j = i + 1; j < NBODIES; j++)   /* trip NBODIES-1-i: DATA-DEPENDENT */
```

A data-dependent trip defeats `resolve_const_operand`, so the loop stays rolled
and its exit test sits in the critical path. Every oracle peels it, which made
"peel the triangular loop" the obvious candidate. It was implemented — the
original header kept as the first guard, each later body copy ending with its
own increment and exit test — verified correct against gcc, and then measured
against its own kill switch with a paired, interleaved, order-alternating
harness:

```
nbody          31 rounds
  peel ON  median 333.63 ms
  peel OFF median 331.61 ms
  median paired ratio OFF/ON = 0.9989  (-0.11%)
  interquartile range of the ratio: [0.9671, 1.0168]
  rounds where peel ON was faster: 13/31
```

**−0.11%, faster in 13 of 31 rounds: noise.** The harness's own noise floor is
calibrated by `struct_copy`, where the peel declines and both binaries are the
same code: ±3% there. Across all 39 benchmark programs the peel changed exactly
one binary, at +40% instructions (306 → 427) for no runtime gain. Reverted.

A second, independent measurement now corroborates the refutation, from work
that predates it: unrolling nbody's OUTER `i` loop by 5 grows the body 293 →
1048 instructions and costs **48% wall time** (390 ms → 578 ms) purely in fetch
footprint, with no spill. Two separate experiments, same conclusion — nbody's
inner loop is not where its time goes.

The "1.252x → 1.076x" figure that motivated the whole line of work compared
against a baseline recorded on a *different tree state*, not against the
change. The honest A/B of the change itself is the −0.11% above. Treat any
before/after number that does not come from a kill-switch differential on one
tree as unverified.

### A regression the measurement caught that review would not have

Running the peel from inside Pass A's fixpoint preempted Pass A's outer-first
cascade — the mechanism that exists precisely to flatten triangular nests, where
an outer constant-trip loop is unrolled so each clone's inner init becomes a
constant expression the complete unroller then flattens. `struct_copy` went
136 instructions / 22.8 ms → 182 / 33.4 ms, a **47% loss**, entirely from
scheduling the fallback before the primary transform. Reordering it as a last
resort made it decline there (136 = 136) while still reaching nbody, whose
outer body is far too large for Pass A's budget.

Lesson worth keeping: for any fallback transform, *when it runs* is a question
about optimization quality, not about correctness. And a null result on the
target can be concealing a large loss elsewhere — which is exactly what a
per-target A/B hides.

## Part 2 — lz4's 1.9x: the loop's induction variable lives in the stack
### The 1.898x gap is a store-to-load-forwarding chain, not an instruction count

The LZ4 match-extension loop, `-DMATCH_RICH=1 -DPASSES=2048`. Both
compilers emit **7 instructions per iteration**. They are not the same
7:

```asm
; GCC — every live value in a register, zero stack traffic
.L12:
        addq $1, %rbx            ; anchor  (callee-saved)
        addq $1, %r9             ; p       (callee-saved)
        cmpq %r13, %rbx          ; limit   (callee-saved)
        je   .L43
.L10:
        movzbl (%r9), %edi
        cmpb  %dil, (%rbx)
        je    .L12

; lccc — p and the limit round-trip through the stack EVERY iteration
.LBB14:
        movq  112(%rsp), %rcx    ; <-- reload p
        movzbl (%rcx), %r8d
        cmpb  (%rbx), %r8b
        jne   .LBB16
.LBB15:
        addq  $1, %rcx
        movq  %rcx, 112(%rsp)   ; <-- store p
        addq  $1, %rbx
        cmpq  56(%rsp), %rcx     ; <-- reload limit
        jb    .LBB14
```

Identical instruction count, 7 versus 7. What differs is that lccc's
`p` is **loop-carried through memory**: the store at the bottom of the
iteration must complete before the load at the top of the next one can
be serviced by store-to-load forwarding, ~4-5 cycles of serialisation
per byte compared. In a loop whose entire body is two loads, a compare
and two increments, that latency — not the instruction count — is the
runtime. It is why 7 instructions cost 1.898x.

### The registers were available

lccc's prologue pushes **all six** callee-saved registers:

```
        pushq %rbx / %r12 / %r13 / %r14 / %r15 / %rbp ; subq $152, %rsp
```

and `main` references them 8 to 34 times each. Yet the extension loop
uses only `%rbx`; `p` is allocated to `%rcx`, a **caller-saved**
register, and given a stack home. Five callee-saved registers are
already saved in the enclosing frame and are idle across this loop.
Wider, `main` spends 70 of 304 instructions (23%) touching `(%rsp)`.

So this is a callee-saved register **allocation** failure, not a lack of
registers: the allocator prefers a caller-saved register plus a spill
slot over a callee-saved register that is already paid for.

### What to build, and why it is a better target than a peephole

In an inner loop, when a loop-carried value must be spilled and a
callee-saved register is live in the enclosing function's frame, keep
it in the callee-saved register. The saving is already amortised by the
prologue, and it deletes a store-to-load-forwarding cycle from the
loop's critical path. Register-pressure heuristics that decide before
callee-saved availability is known are the likely root; the fix is a
spill-cost model that charges a stack slot in a hot loop at its real
price (forwarding latency, not one store plus one load).

**Acceptance test:** `lz4_match_extend` at 1.898x becomes at or below
GCC's ratio, with the extension loop at 0 stack operations and byte
identical stdout. Note the existing `struct_copy` A/B, where a
scheduling change cost 47% — pressure is global, so this must be
measured across the suite, not just on lz4.

## Part 3 — what the measurement identified about nbody

GCC's inner loop packs the component pairs; lccc does not. From `-O2 -S` on
`tests/benchmark/programs/nbody.c`: **187 instructions for GCC, 412 for lccc.**
The op mix names the shape — GCC emits `mulpd:6`, `movupd:8`, `unpcklpd:6`,
lccc emits `vmovddup:17` and scalar `vmulsd`/`vaddsd` throughout. The body
stride is `$56` (`struct body { double x,y,z,vx,vy,vz,m; }`), so the velocity
fields GCC updates with a single `movups` are **adjacent F64 pairs**.

### The specification, taken from the oracles

Reproducer — the nbody shape with nbody's register pressure removed:

```c
typedef struct { double a, b; } P;
void upd(P *p, const P *q, double s) {
    p->a = p->a + q->a * s;
    p->b = p->b + q->b * s;
}
double dot(const P *p, const P *q) {
    double da = p->a - q->a, db = p->b - q->b;
    return da * da + db * db;
}
```

`scripts/godbolt.py compare --oracles gcc162,cclang2310,cicxlatest`:

| compiler   | instructions |
|------------|-------------:|
| **lccc**   | **15**       |
| clang 23.1 | 12           |
| ICX latest | 11           |

**ICX, the best of the three, in full — this is the target:**

```asm
; upd: 4 instructions. One 128-bit load, one broadcast, ONE packed FMA that
; reads its second operand straight from memory, one 128-bit store.
upd:
        vmovupd         (%rsi), %xmm1
        vmovddup        %xmm0, %xmm0
        vfmadd213pd     (%rdi), %xmm1, %xmm0
        vmovupd         %xmm0, (%rdi)
        retq

; dot: 5 instructions. Packed subtract and multiply over BOTH lanes, then a
; shuffle-based horizontal reduce -- not two scalar subtracts, two scalar
; multiplies and an add.
dot:
        vmovupd         (%rdi), %xmm0
        vsubpd          (%rsi), %xmm0, %xmm0
        vmulpd          %xmm0, %xmm0, %xmm0
        vshufpd         $1, %xmm0, %xmm0, %xmm1
        vaddsd          %xmm0, %xmm1, %xmm0
        retq
```

lccc, for the same two functions:

```asm
upd:
        movsd  (%rdi), %xmm3
        vfmadd231sd (%rsi), %xmm0, %xmm3
        movsd  %xmm3, (%rdi)
        movsd  8(%rdi), %xmm5          ; second lane, separately
        vfmadd231sd 8(%rsi), %xmm0, %xmm5
        movsd  %xmm5, 8(%rdi)
dot:
        movsd  (%rdi), %xmm2
        vsubsd (%rsi), %xmm2, %xmm2    ; scalar
        vmulsd %xmm2, %xmm2, %xmm2
        movsd  8(%rdi), %xmm4          ; second lane, separately
        vsubsd 8(%rsi), %xmm4, %xmm4
        vfmadd231sd %xmm4, %xmm4, %xmm2
```

lccc scores 15 against ICX's 11 on identical source: **4 wasted instructions
and, more to the point, 4 FP ops on two pipes where ICX needs 2 on one.**

### It is a missing capability, not a closed gate

Neither SLP kill switch changes the output:

```sh
$ CCC_NO_BB_SLP=1        lccc -O2 -S adj.c   # 0 packed ops, 15 insns
$ CCC_NO_FIXED_SLP=1     lccc -O2 -S adj.c   # 0 packed ops, 15 insns
```

So BB-SLP never considers the shape. The grouping rule for *two adjacent scalar
F64 loads* feeding a common store — as opposed to a `VecLoad`/`VecStore` pair —
does not exist. That is a feature, not a one-line gate.

## Part 4 — how to build the SLP, and what will bite

1. **Grouping.** Recognise two scalar F64 `Load`s at `base` and `base+8` whose
   consuming arithmetic is lane-identical, and fuse into `vmovupd` +
   `vfmadd213pd (mem),reg,reg` + `vmovupd` store. `adj.c` is the acceptance
   test: ≤11 instructions, byte-identical stdout against gcc and against
   `CCC_NO_BB_SLP=1`.

2. **Aliasing is the hard part, and it is where a wrong version miscompiles.**
   Packing two 64-bit field accesses into one 128-bit access means a single
   store where there were two, and a single 16-byte load where there were two
   8-byte loads. Three distinct hazards, all of which a "looks obviously safe"
   implementation gets wrong:
   - a 16-byte load at `base` can fault on a page boundary that the original
     8-byte loads straddled safely — the same hazard the zero-ext fold's
     equal-width rule exists to prevent;
   - a 128-bit store to `p->a` clobbers `p->b`, so anything that can alias
     `base+8` across the pair must keep the scalar form;
   - a 128-bit *load* is a wider read than the program performs, so it is only
     sound when the 16 bytes are provably the same object — nbody's bodies are
     a `bodies[]` array of one struct type, so the stride-period
     field-disjointness machinery the pair-loop work already built is where the
     proof belongs, and reusing it is the difference between a day and a week.

3. **`dot` is a second, separable feature.** The horizontal reduce
   (`vsubpd`+`vmulpd` then `vshufpd $1` + `vaddsd`) is a different pattern —
   packed tree then lane-extract — and should not be bundled with the store
   fusion. Land the store fusion first; it is the one nbody's `advance` needs.

4. **Do not re-derive the trip.** nbody's inner loop is fine rolled. Two
   independent measurements say so.

## Part 5 — the zero-extended compare fold, measured honestly

While re-establishing this work on the current main, the S10 zero-extended
compare fold (`fuse_zero_ext_cmp`) was re-applied and then measured for the
first time against its own kill switch:

```
lz4_compress   fold on  4.094 ms   fold off 4.092 ms   -0.96%  (IQR spread 10.7%)
zstd_count     fold on 22.180 ms   fold off 21.279 ms  -2.80%  (IQR spread 23.2%)
```

The fold removes **one** instruction from a 285- and a 117-instruction
function, in workloads of 4 ms and 22 ms where process launch dominates. Both
deltas are inside the noise floor and one points the wrong way. **It is a
codegen-fidelity and I-cache improvement, not a speed claim**, and it should
never have been recorded as an optimization without this measurement. It is
kept because it is correct, it matches what GCC/Clang/ICX emit, it never grows
code, and it fires on 18 of 882 programs under `tests/` (including
`lz4_compress`, `zstd_count`, `sqlite_vdbe_peephole`).

Two things about it were wrong and are now fixed:

* **Its kill switch was shared.** `fuse_zero_ext_cmp` sat under the same
  `CCC_PEEPHOLE_SKIP=load_alu_fuse` guard as its sibling
  `fuse_load_into_alu`. Any A/B with that variable measured the *pair* — a
  first measurement attributed 21 firing programs to this fold, and on
  splitting the switch the figure was 18, with `linux_find_bit` (the largest
  single diff, 4 instructions → 2) belonging to the sibling all along. One
  transform, one switch, now `CCC_PEEPHOLE_SKIP=zero_ext_cmp`.
* **It had no gate.** `check_narrow_cmp_flag_law.sh` pins the *sibling*
  `fold_narrow_load_imm_compare`, sharing the `flag_consumers_are_zf_cf_only`
  safety law, so the law was covered and the shape was not.
  `check_zero_ext_cmp_fold.sh` now pins the shape on the real corpus programs
  where it measurably fires, pins the switch's own effect, asserts the reject
  side on SF/OF readers and past a safe reader, and runs a 65536-pair
  execution differential against gcc. Its non-vacuity is verified: disabling
  the fold makes it report 4 failures.

A methodological note for whoever picks up the SLP work: the kernel that most
obviously "should" fold — `*a < *b` on two `u8` in a small function — is
if-converted to a `cmov` or phi-copied by register allocation **before the
peephole or SLP pass ever runs**. A gate written against that kernel tests the
allocator and reports the feature as dead. Pin shapes on programs where the
transform measurably fires, and prove the pin by disabling the transform and
watching the gate go red.
