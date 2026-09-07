# `-O2` miscompile: rotate/ARX chains under scalar register pressure

**Severity:** P0 — silently wrong code at `-O2`, `-O3`, `-O4`, `-Os`. Correct at `-O0`/`-O1`.
**Status:** **fixed.** Pre-existing upstream defect (reproduced at `main` = `adecdaed` and again at
`2d1db599` with zero local changes). Root cause is the x86-64 asm peephole
`fold_accumulator_alu_store`, not the register allocator.
**Found while:** landing the `aggregate_sroa` constant-offset split (form 4). The split is *not*
the cause; it only removes frame slots, which pushes hot scalar code onto the buggy path.

## Symptom

```
$ ./target/fastbuild/lccc -O2 artifacts/repros/rot16_arx_scalar_-O2.c -o /tmp/m && /tmp/m
MISCOMPILED out[5]=1667510585 (want 1667511609)
$ gcc -O2 artifacts/repros/rot16_arx_scalar_-O2.c -o /tmp/g && /tmp/g
ok
```

Exactly one word of the output is wrong, always low by **1024 (bit 10)** — one operand of one
`xor` is a *different live value*, differing in one bit.

| build | `out[5]` |
|---|---|
| `lccc -O0` / `-O1` | 1667511609 |
| `lccc -O2` / `-O3` / `-O4` / `-Os` | **1667510585** |
| `gcc -O2` | 1667511609 |

## Evidence that it is codegen, not an IR pass

1. **Aggregates are irrelevant.** `rot16_arx_scalar_-O2.c` (nine or sixteen plain `u32` locals,
   straight-line adds/xors/rotates, no array, no local address taken) miscompiles identically.
   Nothing in `aggregate_sroa` can see such a function.
2. **The split merely exposes it.** `rot16_arx_array_-O2.c` (same arithmetic through `u32 x[16]`)
   miscompiles *with* the split and is correct *without* it: when the fields keep their own frame
   slots the two values do not compete for one location.
3. **The final IR is right.** For the miscompiling array build, `CCC_DUMP_IR_AFTER=1` shows the
   `x[5]` chain as `v284 = Or(Shl(v270,7), LShr(v270,25))`, `out[5] = Add(v284, in[5])`, with
   `v270 = Xor(v228, v263)` and every operand traced back to the source. Evaluating that chain in
   isolation yields **1667511609** — gcc's answer, so the value was corrupted between IR and
   assembly.
4. **The assembly names the fault.** In `core` of the minimized scalar repro (`-O2 -S`):

   ```
   addl 28(%rsp), %r8d      ; x5 + x1 -> x1, result LEFT IN %r8d, which held x5
   movl %r8d, 16(%rsp)      ; store x1 to its slot
   xorl 16(%rsp), %r14d     ; x13 ^= x1        <- reads the slot: correct
   xorl %r11d, %r8d         ; x5 = x5 ^ x9     <- reads %r8d, which now holds x1!
   ```

   `rotl7(24584 ^ 10) = 3145984` is computed instead of `rotl7(24576 ^ 10) = 3147008`
   (`24584` is `x1`), and `3147008 - 3145984 = 1024`.

## Root cause

The register allocator's assignment is **legal**. Dumping the emitted asm before the peepholes
(`LCCC_NO_PEEPHOLE=1`) against the final asm (`diff`) shows the rewrite:

```
  pre-peephole (correct)                     post-peephole (wrong)
  movl 24(%rsp), %r8d
  orl  20(%rsp), %r8d        ; %r8 = x5 (rotl12 result)
  movl %r8d, %eax            ; stage into the accumulator
  addl 28(%rsp), %eax        ; eax = x5 + x1
  movl %eax, 16(%rsp)        ; store x1
  xorl 16(%rsp), %r14d       ; x13 ^= x1
  xorl %r11d, %r8d           ; x5 ^= x9        <- reads %r8 == x5
```

`fold_accumulator_alu_store` (src/backend/x86/codegen/peephole/passes/local_patterns.rs) folds the
three-line staging sequence into `addl 28(%rsp), %r8d ; movl %r8d, 16(%rsp)`, which is legal only
if nothing reads `%r8` afterwards — and the very next-but-one line does.

The pass *has* a kill scan for exactly this ("Step 4: verify %SRC_REG is dead …"), and its arm

```rust
LineKind::Other { dest_reg } if dest_reg == src_family => break,   // "overwritten → safe"
```

accepts any instruction whose classified *destination* is the source register. On x86 a two-address
ALU op is also a **read** of that destination: `xorl %r11d, %r8d` writes `%r8` and consumes its old
value in the same instruction. So the last consumer of the value was mistaken for a redefinition.
Phase 4 of the same directory carries a note about "a register-renaming bug" in a different pass;
this is the same failure shape in a pass that is on by default.

**Fix** (same file): the kill arm now requires a *full, value-independent* redefinition, via a new
predicate `pure_family_write(td, fam)` that accepts only `mov*`/`lea` into the bare 32-/64-bit
register name whose sources do not mention the family (so `movq 8(%r13), %r13`, `movzbl %al, %eax`,
`movl 4(%r8), %r8d` are reads), plus the `xorl/xorq %R, %R` zeroing idiom; partial writes
(`movb $3, %r8b`), `cmov*` (old value survives the not-taken edge), `pop`, and every RMW ALU op are
uses. The same all-width source test was applied to `fam_read_after`, whose `mov_store` arm had the
matching latent hole (it substring-matched only the 64-/32-bit spellings, so `movzbl %al, %eax`
counted as a pure write).

`CCC_NO_PEEPHOLE_PHASE1=1` (the phase that runs this pass) was the confirming toggle.

## The sweep that lied

`killswitch_bisect.sh` in this directory was used earlier to conclude "no in-pass kill switch hides
it", which sent the investigation to the allocator for a session. That conclusion was an artifact
of the tool, not of the compiler: the script prefixed every harvested name with `CCC_` while the
names already began with `CCC_NO_`, so it exported `CCC_CCC_NO_X` — variables no code reads — and it
read its list from `/tmp/killsw.txt`, which the sandbox wipes between turns. Either failure mode
prints "no FIXED-BY". The rewritten script harvests the list itself and treats an empty list as
fatal; run against a compiler with the old predicate it reports immediately:

```
switches tried: 140
FIXED-BY  CCC_NO_PEEPHOLE
FIXED-BY  CCC_NO_PEEPHOLE_PHASE1     <- the culprit
FIXED-BY  CCC_NO_REGALLOC            <- reshapes allocation so the pattern never
FIXED-BY  CCC_NO_SMALL_SLOTS            arises; a direction, not a proof
```

**Methodological note.** A `[RA-W4] homes` dump (`CCC_DEBUG_RA_INTERVALS=1`) showing a *legal*
assignment was read as "the allocator is fine, so it must be a use-count miscount". It was legal and
the second inference was wrong: a text-level peephole runs *after* allocation and can manufacture an
illegal reuse that no IR- or RA-level accounting will ever show. For any live-range symptom, diff the
pre-peephole asm against the final asm **first** — `LCCC_NO_PEEPHOLE=1` makes it a one-line check.

## Verification of the fix

* The three repros and `tests/regression/peephole_acc_fold_arx_src_kill.c` are now correct at
  `-O2`/`-O3`/`-Os`, with the split both off and forced on (`CCC_AGGREGATE_SPLIT=1`); output is
  identical to `gcc -O2`.
* Falsifiability: with `pure_family_write` short-circuited to the old semantics,
  `rmw_of_source_register_blocks_the_fold` and
  `partial_or_indexed_writes_of_source_block_the_fold` (in `acc_fold_src_kill_tests`) fail, the C
  regression test returns 1 (`out[5]=3145990`), and the repros miscompile again. The other three
  tests guard against over-refusing.
* Codegen cost: 0. Instruction counts over 20 corpus programs are identical between the fixed and
  old-semantics compilers (4694 vs 4694, per-program delta 0), including `arith_loop` (211), the
  program the pass was written for, and `chacha20_block` (709). The `i686_alu_chains` mismatch in
  that census is a harness artifact — it is an `-m32` benchmark (see its header) and was compiled
  for x86-64 by both sides.

## Why this unblocks the ChaCha win

`chacha20_block`'s state array `x[16]` is only 40 % of the story; the rest is that the 16 ARX
variables must be *register resident*. Form 4 of `aggregate_sroa` (constant-offset splitting) makes
them register-resident; while this defect was live the split produced wrong numbers, which is why it
was landed opt-in. With the peephole fixed the split's default can be flipped, and P0-2 (unroll the
small constant-trip copy loops) becomes the next codegen gate.

## Reproducing

```
# 1. build
cargo build --profile fastbuild --locked
# 2. the self-checking repros (exit 1 = miscompiled; all 0 after the fix)
for f in rot16_arx_scalar_-O2 rot16_arx_scalar_-O2_min rot16_arx_array_-O2; do
  ./target/fastbuild/lccc -O2 artifacts/repros/$f.c -o /tmp/x && /tmp/x; echo "$f rc=$?"
done
# 3. the culprit phase, on a compiler that still has the old predicate
LCCC_BIN=/path/to/unfixed/lccc bash artifacts/repros/killswitch_bisect.sh \
    artifacts/repros/rot16_arx_scalar_-O2_min.c
# 4. control: the same sources under gcc, and lccc at -O1
gcc -O2 artifacts/repros/rot16_arx_scalar_-O2.c -o /tmp/g && /tmp/g
./target/fastbuild/lccc -O1 artifacts/repros/rot16_arx_scalar_-O2.c -o /tmp/o1 && /tmp/o1
```

## Tooling in this directory

* `killswitch_bisect.sh <src.c>` — for every `CCC_NO_*` switch (harvested from `src/` at run time),
  report which ones restore agreement with gcc. `SPLIT=0` to leave the aggregate split at its
  default, `LCCC_BIN=` to point at another compiler. A hit names a pass that either did it or hides
  it; see "The sweep that lied".
* `delta_shrink.py <file.c>` — line-level delta debugger with an "output differs from gcc" oracle;
  reverts a deletion when the file stops compiling or the mismatch disappears. Produced the
  minimized `rot16_arx_scalar_-O2_min.c` from the original 16-variable kernel.
