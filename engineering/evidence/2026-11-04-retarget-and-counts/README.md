# Round 11 — accumulator retarget, i686 defined-zero counts, and the defect log

Everything below is measured, not estimated. Method: one probe corpus (`probe.c`,
15 functions over `clz`/`ctz`/`popcount`/`bswap`/`ffs`/`rotate`/variable shifts at
32 and 64 bits), compiled at `-O2 -S` by identical invocations, and counted per
function body from the label to the `ret` (directives, comments and blank lines
dropped). The baseline is `main` at `5a3b208` built from the same checkout into a
separate binary set, so a row difference is caused by the patch and nothing else.

## Result

| target | before | after | delta |
|---|---|---|---|
| x86-64 | 98 | 98 | — |
| i686 | 265 | **252** | −4.9 % |
| AArch64 | 185 | 185 | — |
| RISC-V 64 | 491 | **472** | −3.9 % |

No function on any target got longer.

Per function (before → after):

| fn | i686 | riscv64 |
|---|---|---|
| np2 | 16 = | 48 → 44 |
| np2ll | 45 = | 46 → 42 |
| highbit | 11 = | 41 → 39 |
| fls | 17 = | 49 → 45 |
| clz32 | 8 = | 37 = |
| ctz32 | 12 → 8 | 29 = |
| clz64 | 22 = | 38 = |
| ctz64 | 25 → 20 | 28 = |
| pc32 | 19 = | 24 = |
| pc64 | 26 = | 25 = |
| bsw64 | 12 = | 37 = |
| bsw32 | 5 = | 20 = |
| ffs32 | 16 → 12 | 47 → 44 |
| rol32 | 6 = | 14 = |
| shl64v | 25 = | 8 → 6 |

## Cross-compiler oracle (same corpus, `-O2 -S`)

`gcc-16` 16.2.0 and `clang-23` 23.1.4 from the `harness-cache-7` toolchain cache;
`clang --target=aarch64-linux-gnu` / `--target=riscv64-linux-gnu` for the cross
columns. ICX 2026.2 is not reachable from this sandbox (the Intel PPA is outside
the allowed egress list), so it is absent rather than guessed.

| target | lccc | gcc 16.2 | clang 23.1.4 |
|---|---|---|---|
| x86-64 | 98 | 60 | 86 |
| i686 | 252 | 92 | 131 |
| AArch64 | 185 | — | 54 |
| RISC-V 64 | 472 | — | 356 |

## What the patch does

1. **i686 `ctz32`**: the defined-zero diamond becomes the sentinel pair the
   x86-64 backend already emits (`movl $32, %dst` ; `bsfl %src, %dst`), because
   `bsf` leaves its destination untouched on a zero source. The staging register
   is an explicit parameter whose call sites name a register they have proven
   dead: gpr32 `Clz`/`Ctz` are `UnaryOp` scratch hazard points
   (`collect_i686_scratch_hazard_points` marks them `(true, true)`), and the two
   64-bit sites run in a branch that overwrites `%edx` three instructions later.
2. **RISC-V `retarget_dying_accumulator_moves`**: the emitter computes every
   result in `t0` and copies it to the value's home; `t0`..`t6` are scratches, so
   the producer writes the home directly and the copy disappears. Guards:
   single-destination rewritable producer into an accumulator; the consumer is the
   next live line and is `mv D, tK`; `tK` is dead afterwards (forward scan,
   overwrite before read, `ret` ends it, control flow refuses).

## Defect log — every item found by the sweep, and its disposition

1. **Miscompile in the first version of the retarget pass (fixed).**
   `dest_only_write` treated `sext.w t0, t0` as a pure overwrite of `t0`; it
   reads `t0`, so retargeting the producer away from `t0` left the pre-shift
   value there and `(i32)(u64 >> 3)` sign-extended the wrong operand. Caught by
   `check_cross_alu_abs_runtime` (digest mismatch at `-O1`, `-O2`, `-Os` on
   RISC-V). Only definitions that read nothing are pure overwrites now; the shape
   is pinned by a unit test.
2. **The `clz` half of the i686 change was a pessimisation (reverted).** With the
   i686 relay peephole from #1095 in the pipeline, staging `clz32` through a dead
   register instead of the bracketed `pushl %ecx`/`popl %ecx` pair blocks the
   relay from folding the incoming frame load into the accumulator, so the
   parameter takes a callee-saved home and the function pays
   `pushl %ebx`/`popl %ebx` plus a frame slot: `clz32` 8 → 9, `highbit` 11 → 12.
   Keeping the bracket (and letting the dead-scratch peephole remove it when
   `%ecx` is provably dead) is the measured better shape.
3. **Three gates pinned register names rather than the idiom they certify
   (fixed).** `check_clz_zero_sentinel` required `bsrl %ecx, %eax`; the count may
   fold onto the operand's own home. `check_atomic_backends` required
   `srli t0, t0, 32`; the retarget folds the trailing copy into the home.
   `check_bit_select_fold` compares the same corpus with and without the
   bit-select fold, and the unfolded fallback has more copy pairs to fold, so the
   OFF build could end up shorter — the cross-length row now holds the retarget
   constant, as it already did for the i686 select-cmov and load→copy relay.
4. **Seven gates fail locally in this sandbox only** (`cannot find -lgcc`, missing
   32-bit glibc headers). Verified to fail identically against the `main` binaries,
   so they are environment-limited (no i686 multilib sysroot here) and not a
   property of this patch.
5. **Three unit tests failed in CI, all test-side (fixed in `c9cf579`).** The
   3.9 GB sandbox cannot build the lib test target (rustc peaks at 3.7 GB), so
   `cargo test` is not runnable here and CI was the first oracle for them.
   `sext_self_form_kept_when_not_sext_wide` and
   `a_short_srai_is_not_a_sext32_producer` asserted the literal text
   `sext.w t0, t0`; the retarget legitimately rewrites that pair into the home it
   is copied to (`sext.w t1, t0`, with the trailing `mv` deleted), so they now
   assert that a `sext.w <dst>, t0` line survives — the idiom they mean, not a
   register spelling (the same over-fitting as item 3). My own
   `retarget_refuses_a_live_scratch` used `addw t0, t6, t6` as its "live read",
   which only *writes* `t0`; the reader must name it in an operand slot
   (`addw t6, t0, t0`) for the refusal to be the behaviour under test.
6. **`CCC_NO_RV_ACCUMULATOR_RETARGET` tripped the census ratchet (re-baselined).**
   `scripts/audit_census.py --check` counts distinct knob literals; the new kill
   switch is now documented in `scripts/README.md` (so the undocumented count
   stays at 262) and `knob_literals` is re-baselined 560 → 561 in the same commit
   that adds it. `panic_sites`/`panic_sites_hot` were deliberately left at their
   old baseline instead of being tightened opportunistically.

**CI status:** run 41827411507 at `c9cf579` — 23/23 checks green, including
`Authenticated Intel ICX 2026.2 integration`, both unit-test lanes, both
clippy/MSRV lanes and every contract shard. This README edit is
documentation-only; the numbers above were measured at `5117774`.

## Ranked backlog, with the measurement that motivates each item

1. **AArch64 argument registers as value homes.** `collect_abi_reg_hints` maps
   only params 4..7 (x4..x7) and declares x0..x3 scratch, so every parameter is
   copied into a callee-saved register and back:

   ```
   clz32 (lccc, 9):              clz32 (clang, 2):
     stp x29, x30, [sp, #-32]!     clz w0, w0
     mov x29, sp                   ret
     str x19, [sp, #16]
     mov x19, x0                 <- copy in
     mov x0, x19                 <- copy back
     clz w0, w0
     ldr x19, [sp, #16]
     ldp x29, x30, [sp], #32
     ret
   ```

   The frame exists only because of that copy, so a correct x0..x3 hint removes 7
   of those 9 instructions across the whole AArch64 corpus (185 → ~90 on this
   probe). It needs the 58 hard-coded `x0` uses in `arm/codegen` audited first:
   each one is an implicit assumption that x0 is free.
2. **i686 64-bit parameter/result staging** (`np2ll` 45 vs gcc 17, `shl64v` 25 vs
   10, `clz64` 22 vs 6): wide values are staged through frame slots even when the
   input pair arrives in `%eax:%edx` and leaves in `%eax`.
3. **x86-64 `popcount`** (17/22 vs gcc 5/4, clang 16/20): lccc and clang both
   expand the SWAR sequence; gcc calls `__popcountdi2`, which is shorter but is a
   call. A size/speed trade worth deciding deliberately, not by accident.
4. **RISC-V `ctz`/`ffs`** (29/44 vs clang 11/15) and `bsw64` (37 vs 26): the base
   ISA has no `ctz`/`rev8`, but clang's expansions are much shorter than the
   current SWAR forms.
5. **i686 leaf frames**: a leaf whose parameter is used once still pays
   `pushl %ebx` + `subl $8, %esp` + `movl 16(%esp), %ebx` + `addl` + `popl`
   because the parameter is homed in a callee-saved register. Leaf functions
   cannot be clobbered by a call, so caller-saved homes are free there.
