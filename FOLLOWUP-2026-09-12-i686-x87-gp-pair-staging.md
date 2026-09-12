# FOLLOWUP-2026-09-12-i686-x87-gp-pair-staging.md

The dominant cause of the i686 runtime gap against GCC, measured — plus the
bounded load-side fold that is now **landed and validated** (§4.4, §4.5), and
the root fix that is **not** done.

Date: 2026-09-12 · Base: `ms178/lccc` @ `a0e03144` (Merge PR #505)

---

## 1. Why this document exists

The directive was to enable i686 testing and extend the reload-reuse work to
i686. Enabling the testing worked, and it immediately produced a result that
redirects the whole effort:

**i686 could not be tested at all before, and once it could, the reload-reuse
extension turned out to be worthless while a much larger defect appeared.**

Both halves are recorded here, because the second half is the highest-value
i686 performance work currently identified. Its load-side *mitigation* is
landed, measured and adversarially tested (§4.4–§4.5); the *root fix* — an x87
location model — is not, and the mitigation does not substitute for it.

### 1.1 What unblocked i686 testing

The sandbox had no 32-bit libc headers, which capped the i686 corpus at 292
TUs and made execution impossible. Passwordless `sudo` was available all along
and had never been probed:

```sh
sudo dpkg --add-architecture i386
sudo apt-get install -y libc6-dev-i386 gcc-multilib lib32gcc-14-dev
```

| | before | after |
|---|---|---|
| i686 corpus TUs that compile | 292 / 805 | **792 / 807** |
| i686 binaries executable | no | **yes, natively** |

No QEMU is needed: an x86-64 kernel executes i386 ELF directly. Verified
end-to-end — `lccc-i686 -O2 -o h h.c && ./h` runs and matches the `gcc -m32`
oracle. The 15 non-compiling TUs are correct refusals (`__int128` is not
supported on i686; one `#error` on `__STDC_VERSION__`).

**Prior sessions that declined i686 work on sandbox grounds were wrong to.**
The capability was one `apt-get` away.

---

## 2. The measurement: i686 is 26% slower than GCC, and up to 16×

`lccc-i686 -O2` vs `gcc -m32 -O2`, 7 runs each, min and median agreeing, on
`tests/benchmark/programs`. Harness: `tools/i686_runtime.py`.

geomean(lccc/gcc) = **1.2642** — 25 of 29 measurable benchmarks slower, 4
faster, none at parity.

| benchmark | lccc min | gcc min | ratio |
|---|---|---|---|
| **nbody** | 7734.0 ms | 481.9 ms | **16.05×** |
| **matmul** | 313.1 ms | 22.7 ms | **13.80×** |
| reduction_vecreg | 3578.3 ms | 597.5 ms | 5.99× |
| mandelbrot | 8465.9 ms | 1452.8 ms | 5.83× |
| zlib_ng_adler32 | 210.3 ms | 42.8 ms | 4.92× |
| struct_copy | 131.6 ms | 34.0 ms | 3.86× |
| chacha20_block | 889.7 ms | 288.0 ms | 3.09× |
| sha256_transform | 924.0 ms | 334.4 ms | 2.76× |
| loop_patterns | 203.6 ms | 84.4 ms | 2.41× |
| fp_memfold_stencil5 | 284.9 ms | 141.2 ms | 2.02× |
| bitops | 319.3 ms | 383.0 ms | **0.83×** (lccc wins) |
| fib / constant_recursion | 1.3 ms | 168.6 / 151.0 ms | **0.008×** (lccc folds the workload to a constant) |

Static counts agree in direction: 15,321 instructions for lccc vs 10,006 for
GCC over 50 benchmark TUs (**1.53×**), with 45 of 50 more than 5% worse.

The `fib`/`constant_recursion` rows are not a code-quality win and must not be
quoted as one — lccc constant-folds the entire workload away. Any i686
benchmark table that includes them is measuring the folder, not the backend.

---

## 3. Mechanism: FP values are staged through GP register pairs

Both compilers use x87 (`gcc -m32` defaults to `-march=i686`, SSE2 disabled),
so this is not an ISA-selection difference. It is how the value reaches the
x87 unit.

`matmul` inner loop, GCC (7 instructions, `A[k]` hoisted into `st(1)`, `C`
addressed directly as an x87 memory operand):

```asm
.L3:
    fldl   (%ebx,%edi,8)
    fmul   %st(1), %st
    faddl  (%edx,%edi,8)
    fstpl  (%edx,%edi,8)
    addl   $1, %edi
    cmpl   $256, %edi
    jne    .L3
```

Same loop, `lccc-i686` (~24 instructions):

```asm
.LBB6:
    movl (%esi), %eax          # double A -> two GP registers
    movl 4(%esi), %edx
    movl %eax, 20(%esp)        # ... -> a stack slot
    movl %edx, 24(%esp)
    movl (%edi), %eax          # double B -> two GP registers
    movl 4(%edi), %edx
    movl %eax, 12(%esp)        # ... -> a stack slot
    movl %edx, 16(%esp)
    fldl 20(%esp)              # finally reach the x87 unit
    fldl 12(%esp)
    fmulp %st, %st(1)
    fstpl 4(%esp)              # result -> slot
    movl 36(%esp), %ecx
    movl (%ecx), %eax          # double C -> two GP registers
    movl 4(%ecx), %edx
    movl %eax, 12(%esp)
    movl %edx, 16(%esp)
    fldl 12(%esp)
    fldl 4(%esp)               # result reloaded from the slot
    faddp %st, %st(1)
    fstpl 20(%esp)
    movl 20(%esp), %eax        # result unstaged through GP registers again
    movl 24(%esp), %edx
    pushl %edx
    pushl %eax
```

Three distinct causes, in decreasing order of payoff:

1. **No x87 memory operands.** A `double` is materialised as two `movl`s into
   GP registers, spilled to a slot, then `fldl`-ed from the slot — 5
   instructions where `fldl (%esi)` is 1. Symmetrically on the store side.
2. **No x87 stack allocation.** Every intermediate goes to a slot and comes
   back (`fstpl 4(%esp)` … `fldl 4(%esp)`), so nothing stays on the x87 stack.
3. **No hoisting of loop-invariant x87 operands.** GCC keeps `A[k]` in `st(1)`
   across the whole inner loop; lccc reloads it per iteration.

Cause 1 is the bounded one and is what §4 specifies. Causes 2 and 3 are the
i686 analogue of P0-A (Global Location Allocation) and are a backend project,
not a peephole.

### 3.1 Exact backend site of cause 1

`emit_f64_load_to_x87` (`src/backend/i686/codegen/emit.rs:1106`) is **not** the
culprit and should not be touched: for `Operand::Value` with a home slot it
already emits `fldl SLOT` directly, which is correct.

The staging happens one level earlier, on the load from a *computed address*.
`src/backend/i686/codegen/memory.rs` treats `F64` as just another 8-byte
integer:

```rust
// memory.rs:470 (load path); same lumping at 339, 580, 600, 826, 846
if ty == IrType::I64 || ty == IrType::U64 || ty == IrType::F64 || ty == IrType::D64 {
    // ... movl (ptr), %eax ; movl 4(ptr), %edx ; home both halves to a slot
}
```

So a `double` read through a pointer becomes a GP register pair homed to a
stack slot, and only *then* reaches the x87 unit via `fldl SLOT` — five
instructions where `fldl (ptr)` is one. The store side is symmetric.

Note what this implies for the fix: emitting `fldl (ptr)` directly requires the
value to *live on the x87 stack* rather than in a slot, i.e. the backend needs
an x87 location model (cause 2). Cause 1 and cause 2 are therefore not
independent — the bounded version of cause 1 is the peephole fold in §4, which
recovers the instruction count without changing where values live, and the
root-cause version is x87 location allocation at `memory.rs:470` and its store
counterparts, which is what actually closes the 16× on `nbody`.

---

## 4. The bounded fix: specification, and what landed

**Framing, stated plainly so it is not mistaken for the root fix:** this is a
peephole that undoes staging the backend created. Per §3.1 the root-cause fix
is an x87 location model at `memory.rs:470`. The fold below is justified as an
interim measure only because it is bounded, provable per site with analyses
that already exist, and recovers ≈5,268 instructions concentrated in the four
worst runtime outliers — and because it has a reproducer, a causal mechanism,
and an A/B plan, which is the bar the project sets for a peephole. If the x87
location model is built first, this fold becomes unnecessary and should be
dropped rather than landed. The order actually taken was the reverse — the fold
is bounded and provable per site today, the location model is not — so the fold
carries a kill switch (`CCC_NO_X87_PAIR_FOLD`) that makes its removal a
one-line, fully measurable change once the location model exists.

### 4.1 Opportunity, measured

Pattern counts over the 792-TU i686 corpus (`tools/i686_staging_count.py`
logic, reproduced inline in §6):

| | sites | ≈ instructions saved |
|---|---|---|
| load side: `movl (M),%r1; movl 4(M),%r2; movl %r1,S; movl %r2,S+4; fldl S` → `fldl (M)` | **775** | 4 each |
| store side: `fstpl S; movl S,%r1; movl S+4,%r2; movl %r1,(M); movl %r2,4(M)` → `fstpl (M)` | **247** (was claimed 542 — see the correction below) | 4 each |
| **total** | **1,022** | **≈4,088 (1.50% of the corpus)** |

**Correction, S18.** The store row above was 542 in the S17 census and that
figure is retracted: it counted a *prefix* (`fstp S; movl S,%r1; movl S+4,%r2`)
without requiring the two writes to form a 4-apart destination pair, i.e. it
counted shapes that are not foldable. Re-measured on the baseline arm with the
strict five-line pattern, strictly adjacent, `%esp` slot, displacements exactly
4 apart on both sides: **247 sites**. The loose prefix, whatever consumes it,
measures 408 — still not 542, so the original figure was also taken on a
different binary (before the S17 `global_store_forwarding` deletion). The load
row (775) has **not** been re-measured at the same strictness and should be
treated as an upper bound; what the load side actually realises is 237 sites
(§4.4).

Concentration is exactly where the runtime gap is worst:

| TU | load | store | ≈ insns |
|---|---|---|---|
| `vectorize_matmul_n16` | 83 | 62 | 580 |
| **`nbody`** | 87 | 26 | **452** (30% of nbody's 1,485 instructions) |
| `fp_domain_crossing` | 45 | 28 | 292 |
| `vectorize_matmul_param_alias` | 36 | 18 | 216 |
| `fp_scalar_webs` | 39 | 9 | 192 |

`nbody` — the 16.05× outlier — spends roughly a third of its instruction
stream on this staging. That is the causal link between the mechanism and the
measurement, not a correlation.

### 4.2 Transformation

```
movl (M), %r1        fldl (M)          # r1, r2 dead; S dead
movl 4(M), %r2   ->
movl %r1, S
movl %r2, 4(S)
fldl S
```

### 4.3 Soundness conditions — all four are required

1. **`S` and `S+4` are never read again**, anywhere in the function, and their
   address is never taken. The existing never-read-store machinery
   (`eliminate_never_read_stores` / `…_range`) already answers this; reuse it
   rather than writing a second liveness scan.
2. **`%r1`/`%r2` are dead after the staging stores.** They are the backend's
   scratch pair, so this usually holds, but it must be proven per site —
   `census_reg_reads` is the existing tool.
3. **`M` does not alias `S`.** If `M` is itself a frame slot, the store to `S`
   could clobber the source before the `fldl` reads it. Refuse when `M` is
   frame-relative and the byte ranges overlap (`ranges_overlap` already
   exists); refuse indexed/indirect `M` outright.
4. **No barrier between the staging and the `fldl`** — `is_barrier()` covers
   labels that are branch targets, calls, jumps, push/pop and any write to
   `%esp`, all of which renumber `S` or may clobber `M`.

Width must match exactly: this is a `double` (8-byte) pattern. The `float`
(4-byte, single `movl`) and `f128`/x87-extended (10-byte, `fstpt`/`fldt`)
variants are *different* transformations and must not share this code path —
in particular `fstt` does not exist in GAS, so the 10-bit form has no
non-popping store and cannot be folded the same way.

### 4.4 Validation performed — all gates green

The load-side fold is **landed** (commit `269f72ef`, branch `s18`), behind the
kill switch `CCC_NO_X87_PAIR_FOLD`. Every number below was measured with the
*same binary*, the kill switch being the only variable, on native i386.

**Static, 792-TU i686 corpus** (`tools/i686_corpus.py asmdiff`):

| | instructions | `(%esp)`/`(%ebp)` slot refs |
|---|---|---|
| fold off (kill switch) | 271,585 | 102,701 |
| fold on | **270,637** | **101,990** |
| delta | **−948 (−0.3491%)** | **−711** |

38 TUs change, 754 are byte-identical, **0 regressions**. The kill-switch arm
reproduces the recorded pre-fold baseline *exactly* (271,585 / 102,701), which
is the proof that the pass is inert when disabled rather than merely quiet.
948 ÷ 4 = **237 sites folded**. Largest per-TU effects: `nbody` −128,
`vectorize_matmul_n16` −128, `fp_domain_crossing` −116,
`vectorize_trip_count_gate` −68, `loop_promote_affine_alias` −60,
`fp_scalar_webs` −52, `simd_fp_oracle` −48.

**Byte-identity scan** over all 807 `.c/.cpp/.cc` under `tests/`
(`tools/x87_byte_ident.py`): 38 TUs differ, 0 assembler failures. Instruction
counts alone would have missed count-neutral text changes; the changed set is
defined by SHA-256 of the emitted `.s`, not by counts.

**Execution differential** (`tools/x87_fold_diff.py byte-list`, three arms per
TU — fold off, fold on, `gcc -m32 -O2` — assembled with `gcc -m32 -c`, linked
and run natively):

* 33 TUs assembled, linked, ran, and matched **off == on**; 5 are compile-only
  (no `main`).
* **ASSEMBLY FAILURES: 0** — GAS accepts every rewritten operand, including the
  SIB forms (`fldl (%ebx,%edi,8)`).
* **EXECUTION DIFFERENCES (off vs on): 0.**
* 29 of the 33 produce output **byte-identical to the gcc oracle**. The 4 that
  differ do so in *both* lccc arms identically, i.e. pre-existing: two are
  last-digit `%g` printing (`fp_domain_crossing`, `vex_promote_semantic`), one
  is a truncated-prefix display (`vec_alias_versioning`), and in
  `vectorize_map_expr_tree` lccc prints `OK` where gcc prints `FAIL`.

**Differential fuzz** (`tools/x87_fold_fuzz.py 250`): 250 generated programs in
the fold's domain, 11 generators, 250/250 built and ran, fold fired on 103.
**Hard failures (build, or off != on): 0.** Per generator (runs, fold fired):

| generator | runs | fired | |
|---|---|---|---|
| `g_plain_chain` | 23 | 23 | straight-line staging |
| `g_loop_indexed` | 23 | **23** | loop-carried SIB source — the shape §4.5 enables |
| `g_matmul_small` | 22 | 22 | |
| `g_mixed_precision` | 22 | 22 | |
| `g_struct_field` | 23 | 13 | |
| `g_addr_taken` | 23 | **0** | adversary: frame address formed → condition (4) |
| `g_call_in_window` | 23 | **0** | adversary: call barrier |
| `g_index_changes` | 23 | **0** | adversary: SIB index rewritten in the window |
| `g_redux_two` | 22 | **0** | adversary: slot overwritten before consumption |
| `g_union_pun` | 23 | **0** | adversary: type punning through the storage |
| `g_volatile` | 23 | **0** | adversary: volatile access pattern |

Every adversary refuses to fold, and every generator that does fold stays
correct. 6 programs differ from gcc in the last printed digit; in all 6 both
lccc arms agree byte-for-byte, so the fold is not implicated (pre-existing x87
excess-precision behaviour of the backend).

**Unit tests, in tree** (`peephole.rs`, driving the pass directly — the same
idiom `forward_slot_loads` uses, because later pipeline passes rewrite the same
text and would mask which pass produced the shape under test):

| test | what it pins |
|---|---|
| `…folds_to_a_direct_memory_operand` | the basic 5→1 rewrite |
| `…folds_when_one_wide_store_retires_the_pair` | nbody's `fstl` retirement (a `movl`-pair-only scan lost all 37 of its sites) |
| `…folds_across_a_loop_back_edge` | the post-condition follows the CFG; a linear scan refuses every hot loop |
| `…folds_across_an_esp_shift` | slot renumbering under `subl/addl $imm, %esp` |
| `…not_folded_when_the_slot_is_read_again` | condition (5) |
| `…not_folded_when_the_base_is_rewritten` | condition (1) |
| `…not_folded_when_the_index_is_rewritten` | condition (1) on the *index*, with an indexed-source control |
| `…not_folded_when_a_frame_address_is_formed` | condition (4) |
| `…not_folded_across_a_non_slot_write` | condition (2) |
| `…not_folded_across_a_call` | condition (2) |
| `…not_folded_when_esp_is_exchanged_in_the_window` | `xchg %esp, %ebx` (§4.5, finding 3) |
| `…not_folded_when_esp_is_exchanged_after_the_load` | idem, post-condition side |

Each negative test carries its own **control**: the same function minus the
offending line, asserted to fold. Without the control a negative test can pass
for the wrong reason — that is how the SIB bug in §4.5 was found.

`cargo test --lib`: **2,497 passed, 0 failed**. `ci_local.sh --fast`: **25
passed, 0 failed, 3 skipped — ALL GATES GREEN**. `cargo clippy --all-targets
-- -D warnings`: clean. `rustfmt`: clean.

**Runtime** — the number that decides whether it was worth landing
(`tools/i686_fold_runtime.py`, three arms, min of N runs, native i386):

| benchmark | off min | on min | ON/OFF | gcc min | OFF/GCC | ON/GCC |
|---|---|---|---|---|---|---|
| `nbody` (N=9) | 7665.6 ms | 7052.8 ms | **0.920** | 477.1 ms | 16.068 | **14.784** |
| `nbody` (N=7, repeat) | 7587.4 ms | 7019.5 ms | **0.925** | 484.4 ms | 15.662 | **14.490** |
| `matmul` (N=9) | 313.7 ms | 309.1 ms | 0.985 | 22.6 ms | 13.864 | 13.663 |
| `vector_remainder` (N=9) | 1.3 ms | 1.3 ms | 0.979 | 1.3 ms | 1.021 | 0.999 |

`nbody` improves **7.5–8.0%**, reproduced by two independent runs, with the
same binary and the kill switch as the only difference; the static hot-loop
delta is −128 instructions (1,485 → 1,357). `matmul` (−1.5%) and
`vector_remainder` (1.3 ms total) are **inside the noise floor and are reported
as uninformative, not as wins** — per §22/§23 an arm that cannot resolve the
effect is not evidence. `fp_scalar_webs`, `simd_fp_oracle` and `k06_dot` are
not standalone-linkable (no `main`); their change is verified statically and by
the differential/fuzz gates, not by timing.

### 4.5 Landed: realised share, one deliberate deviation, and three findings

**This is an interim mitigation, not the root fix, and it does not close the
gap.** Per §3.1 the root cause is the x87 location model at `memory.rs:470`,
which materialises every F64 in a GP register pair before it can reach the x87
stack. A peephole can only clean up afterwards, at sites whose entire
surrounding window and successor CFG happen to be provable. After the fold,
`nbody` is still **14.5×** gcc and `matmul` **13.7×**. The measured ceiling of
this direction is §4.1's ≈5,268 instructions (1.94%); the load side realises
**948 (18% of the identified opportunity, 31% of the 775 load-side sites)**.
The store-side mirror **is now implemented too** (`fold_x87_gp_pair_unstaging`,
its own kill switch `CCC_NO_X87_PAIR_UNSTAGE`), and §4.6 records what it buys.

**Deviation from §4.3 condition (3), deliberate and sound.** The spec said
"refuse indexed/indirect `M` outright". The implementation instead accepts an
indexed source when (i) the addressing text is identical between the two loads
and the displacements differ by exactly 4, (ii) *every* register the address is
computed from — base **and** index — is unchanged across the window
(`mem_addr_regs`, not a base-only check), (iii) no unmodelled memory write,
call or barrier occurs in the window, and (iv) the source is not frame-relative
in any component (`%esp` or `%ebp` appearing as base *or index*), which is what
makes "M does not alias S" true by construction rather than by proof. Refusing
all indexed forms would have rejected `(%esi,%ecx,8)` — the loop-carried shape,
i.e. the hot one: `g_loop_indexed` fires 23/23 and stays correct, and
`g_index_changes` (index rewritten in the window) refuses 23/23.

**Finding 1 — the SIB operand split was silently rejecting every indexed
site.** `movl_operands` split on the *first* comma, so `movl (%ebx,%edi,8),
%eax` parsed as `("(%ebx", "%edi,8), %eax")` and failed the "stores consume the
loaded registers" check. The corpus measurement could not see this: 0 of the
792 TUs changed either way, because every corpus site that survives the other
conditions happens to be base-only. A **unit test with a control** found it in
one run. Lesson recorded: corpus deltas measure what the pass does, never what
it silently declines to do.

**Finding 2 — `ret` is modelled as reading `%eax`, and that is correct, not
conservative.** Condition (6) refused a hand-written test function whose staged
pair was `%eax`/`%edx`. It is not a liveness bug: nothing redefined `%eax`
between the staging stores and the `ret`, so the loaded value *was* the returned
one and deleting the load would have been a genuine miscompile. Real staging
uses scratch registers that are redefined before the return, which is why 237
sites fold in the corpus. Consequence worth stating: the fold's coverage
depends on register allocation leaving the pair dead, so it is a *mitigation
whose reach the allocator controls* — one more reason the location model is the
real fix.

**Finding 3 — an unsuffixed `xchg` was invisible to the shared register
oracle.** `line_reg_use_def` special-cased `"xchgl" | "xchgw" | "xchgb"`, but
GAS accepts the bare mnemonic: `xchg %esp, %ebx` measured `uses=0x18,
defs=0x000` — a plain read, as far as every guard built on that oracle is
concerned. The fold survived it anyway, for a principled reason:
`function_forms_frame_addr` runs a taint fixpoint seeded at `%esp` over operand
text, mnemonic-agnostically, so the `xchg` taints `%ebx` and condition (4)
refuses the whole function. Two permanent tests now pin that. The oracle itself
is fixed separately (`mnemonic.starts_with("xchg")`, both operands read *and*
written, plus `reg_use_def_models_the_unsuffixed_xchg`), because DCE and copy
propagation consume the same oracle and the comment above it already warns that
a missing half can delete a live value. Measured effect on the 792-TU corpus:
**byte-identical** (kill-switch arm still exactly 271,585 / 102,701) — the fix
adds soundness without changing a single emitted instruction, since the emitter
always suffixes and the bare form can only arrive via inline asm or
hand-written `.s`.

**Inline asm was adversarially checked, not assumed.** The peephole sees asm
text verbatim, so a user can write the staging shape inside `__asm__`. Three
adversaries were compiled and run fold-off vs fold-on vs gcc: the slot read by
C code afterwards, the GP pair still live inside the asm, and a numeric local
label with a backward branch straddling the staging. In all three the emitted
asm is **byte-identical** between arms and all three match gcc's output — the
asm lines are classified as ordinary instructions, so the same conditions
refuse them.

### 4.6 The store-side mirror, landed

```text
    fstpl S                  fstpl M
    movl S,   %r1       ->
    movl S+4, %r2
    movl %r1, M
    movl %r2, M+4
```

The x87 unit has already produced the value; the backend pops it into a frame
slot, reads it straight back through a GP pair, and writes it out four bytes at a
time. `fstpl` already stores all eight bytes in exactly the layout the two
`movl`s produce, so the rewrite retargets the x87 store and deletes the four GP
lines. The mnemonic is kept verbatim, which preserves the pop (and lets the
non-popping `fstl` ride the same path with no extra argument).

Eight conditions, the load side's six plus two that only exist here: `M` must not
be able to name the slot bytes (a `%ebp` component is refused; an `%esp`
destination is accepted only when the two 8-byte ranges are disjoint — partial
overlap is the miscompile, because with `M = S+2` the original leaves the pair's
bytes at `S+2..S+10` while `fstpl S+2` writes the same double in a different
layout), and no register in `M`'s addressing expression may be `%r1`/`%r2`,
because the write to `M` moves *ahead* of the loads that define them.

**Measured, same binary, three arms** (`CCC_NO_X87_PAIR_FOLD` and
`CCC_NO_X87_PAIR_UNSTAGE` independently):

| arm | instructions | slot refs |
|---|---|---|
| both folds off | 271,585 | 102,701 |
| load side only | 270,637 | 101,990 |
| **both on** | **270,349** | **101,774** |

Store side alone: **−288 instructions (−0.1064%), −216 slot refs, 15 TUs
changed, 0 worse**. Combined: **−1,236 instructions (−0.4551%), −927 slot
refs**. The load-only arm reproduces the recorded v7 numbers exactly, which is
the proof that the two passes compose without interfering. Per TU
(base → load → both): `fp_domain_crossing` 1077 → 961 → **861**, `nbody`
1485 → 1357 → **1349**, `vectorize_matmul_n16` 2611 → 2483 → **2475**.

**Where the 247 candidates go**, measured by re-implementing the cheap textual
conditions over the baseline arm:

| refusal | sites |
|---|---|
| (4) destination has a `%ebp` component | 31 |
| (4) destination overlaps the slot / (5) destination addressed by the pair | **0** |
| (6) function forms a frame address | **0** |
| reach (7)/(8): post-condition or pair liveness | 216, of which **72 fold** and **144 are refused** |

So the frame-address taint — the coarsest condition, and the one §6.3 of the
audit criticises — costs the store side **nothing**; the whole residual is the
dead-store post-condition and register liveness. The post-condition gives up at
`Call`, `InlineAsm` and `JmpIndirect`, and that is the single largest lever
left: under condition (6) no register holds a frame address, so no argument can
point into the frame and a callee — whose own frame is *below* `%esp` — cannot
reach the caller's slots. Relaxing it is a sound-looking argument that needs its
own adversarial validation (a callee that receives a frame pointer derived
somewhere the taint fixpoint cannot see), so it is **filed, not done**.

**Validation.** `cargo test --lib`: 2,508 passed, 0 failed (11 new tests: five
positive — basic, SIB destination, disjoint frame destination, the non-popping
`fstl` form, across a loop back edge — and six negative, each with its own
control: overlapping destination, destination addressed by the pair, `%ebp`
destination, slot read again, pair still live, sequence not adjacent).
Byte-identity scan: 40 TUs changed corpus-wide (was 38). Execution differential
over those 40: **0 assembly failures, 0 off-vs-on differences**, 35 ran and
matched, 31 of the 35 byte-identical to `gcc -m32 -O2`, the same 4 pre-existing
divergences as before. `ci_local.sh --fast`: 25/0/3 green; `clippy
-D warnings`: clean.

**Fuzzing, and a coverage hole found in the harness.** The first 340-program run
after the store side landed reported *exactly* the same census as before — 103
programs fired — which is how it became clear that **none of the 11 existing
generators can produce the store-side shape**: a new pass with zero fuzz
coverage. Six generators were added (three that produce it — store through a
pointer in a loop, into struct fields, at an indexed stride — and three
adversaries: destination whose address is taken, volatile destination, a call
between the store and the slot's retirement). Re-run at 340 programs: 340/340
built and ran, **134 fired**, `g_store_indexed_stride` 20/20 and
`g_store_struct_out` 20/20, all three store adversaries 0/20, **0 hard
failures**. 34 programs differ from gcc in the last printed digit; both lccc
arms agree byte-for-byte on every one (so `off == on` holds for all 340), and 10
of the 12 listed are programs where no fold fired at all — pre-existing x87
excess-precision behaviour, not this change.

**Runtime, honestly.** The store side produces **no measurable runtime win**.
The three timed benchmarks move within noise: `nbody` 0.925 (combined) against
0.920/0.925 for the load side alone, `matmul` 0.988, `vector_remainder` 1.013.
Its 15 changed TUs are mostly regression tests whose runtimes are far below the
measurement floor, and the benchmark programs gained 8 instructions each. It is
landed because it is 0-regression across every gate, completes the
transformation family, and removes 216 more frame-slot round-trips — **not**
because it is a runtime win, and it should not be cited as one.

## 5. Negative results from this session (do not retry)

Recorded so the directions are closed rather than rediscovered.

1. **Porting `reuse_redundant_loads` to i686 is worthless.** Measured on the
   792-TU corpus with a destination-accurate model: **19** redundant
   same-operand loads, of which **3** same-destination, out of 271,585
   instructions (0.007%), across 15 files. Reason: the i686 peephole already
   participates `%esp` slots and already fences every `%esp` mover (see
   `is_barrier`, whose comment states the rule explicitly), and
   `forward_slot_loads` already does store-to-load forwarding with
   `ranges_overlap` precision. The work is done; it was done in a different
   file than the x86-64 one.

2. **The "2-line i686 extension" recorded in
   `engineering/evidence/reuse-precision-2026-09-12/README.md` §4 is wrong.**
   Adding `"%esp" => 4, "%ebp" => 5` to `parse_frame_slot` in
   `src/backend/x86/codegen/peephole/passes/dead_writes.rs` has **zero**
   effect on i686: `lccc-i686` uses `src/backend/i686/codegen/peephole.rs`, a
   separate 13.5k-line implementation. Verified empirically — the change was
   applied, built, and the 792-TU corpus came out byte-identical (271,585
   insns / 102,701 slot refs, unchanged). That README section should be
   treated as retracted.

3. **Width-agnostic `push`/`pop` classification in the x86-64 peephole is
   unreachable.** The x86-64 corpus contains **0** `N(%esp)`, **0** `N(%ebp)`
   and **0** `pushl`/`popl`/`pushw` operands, so both that change and the
   `parse_frame_slot` extension are dead code on the only backend that uses
   them. Implemented, measured, and **reverted** rather than shipped as
   defensive decoration.

4. **Collapsing `fstpX S; fldX S` pairs is not worth it.** 346 adjacent pairs
   corpus-wide (372 `fstpl→fldl`, 78 `fstpt→fldt`, 22 `fstps→flds`), but
   **zero** of them are in any benchmark program — they are all in regression
   tests. No realistic benchmark win, so per the project's own bar the
   direction is stopped. (The `fstpt`/`fldt` subset is additionally not
   foldable to a non-popping store: GAS has no `fstt`.)

5. **A claimed "guaranteed miscompile" from untracked implicit `%esp`
   movement was wrong.** `reuse_redundant_loads` breaks its scan on
   `LineKind::Push | LineKind::Pop` (dead_writes.rs:288) as well as on calls,
   returns and jumps, so it never scans across an SP move. A whole-corpus
   detector confirms 0 reachable instances on both x86-64 (805 TUs) and i686
   (792 TUs).


6. **A linear post-condition is the bottleneck, not the pattern match.** The
   first implementation scanned forward from the `fldl` and gave up at the first
   back edge. That is exactly the shape of a hot FP loop — the slot is only
   rewritten by the *next* iteration's staging — so the linear version refused
   every loop-carried site and folded 0 of nbody's 37. Replaced by a
   CFG-following all-paths scan (`slot_pair_dead_on_all_paths`), which is what
   makes nbody −128 instructions and −8% runtime.

7. **A rewrite-only fold has zero effect.** Rewriting `fldl S` to `fldl (M)`
   without nop-ing the four staging lines measured exactly 0 corpus change: the
   offsets are reused for different values, so
   `eliminate_never_read_stores` cannot clean them up afterwards. The fold must
   delete the staging itself, under its own post-condition.

8. **Corpus deltas cannot detect a silently declined shape.** See §4.5 finding
   1: the SIB split bug changed nothing measurable across 792 TUs and was found
   only by a unit test with a control. Every guard in this fold now has both a
   positive and a negative test, and every negative test carries its control.



9. **The S17 store-side census figure (542 sites) is retracted.** Re-measured on
   the baseline arm with the strict five-line pattern it is **247**, and the
   loose prefix that the original figure appears to have counted is 408 — on a
   binary that also predates the S17 `global_store_forwarding` deletion. A
   census that is not tied to a foldable shape overstates the opportunity by
   2.2×; every figure in §4.1 is now either reproduced by the strict scan or
   explicitly marked as an upper bound.


---

## 6. Harnesses added (kept outside the repo tree)

| path | purpose |
|---|---|
| `tools/recover_env.sh` | idempotent restore of rust, git (from `artifacts/lccc.bundle`), 32-bit toolchain, 8G swap, exec bits, upstream sync |
| `tools/i686_corpus.py` | i686 coverage + per-TU insn/slot counts |
| `tools/corpus_counts.py` | insn/frame-slot counts for any lccc binary, either width |
| `tools/sp_hazard.py` | whole-corpus detector for same-slot load pairs straddling an implicit SP move |
| `tools/i686_esp_opportunity.py` | redundant-reload / dead-store / store-forward opportunity model |
| `tools/oracle_cmp.py` | static insn-count comparison vs `gcc -m32 -O2` |
| `tools/i686_runtime.py` | runtime A/B (min + median) vs `gcc -m32 -O2` |
| `tools/x87_byte_ident.py` | full-corpus byte-identity scan (SHA-256 of the `.s`), fold off vs on — defines the authoritative changed set |
| `tools/x87_fold_diff.py` | 3-arm correctness gate (off / on / `gcc -m32 -O2`): assemble every `.s`, link, run natively, compare stdout+stderr+exit |
| `tools/x87_fold_fuzz.py` | differential fuzzer, 11 generators incl. 6 adversaries, per-generator fold-fired census |
| `tools/i686_fold_runtime.py` | 3-arm runtime A/B (min + median of N), reports `IDENTICAL-ASM` for uninformative arms |
| `tools/i686_corpus.py asmdiff` | the authoritative instruction / slot-reference counter (`^\s+[a-z][a-z0-9]*\b`) — do not hand-roll one |

These live in `/home/user/tools`, not in the repository: they are session
instrumentation, and the patch must not carry them.
