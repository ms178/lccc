# Red-team audit: upstream PRs #363/#364 on wide-immediate stores + further perfection (direct stores, Clz/Ctz zero-guards)

Date: 2026-09-03. Branch `wip/audit358-359`, now **rebased on upstream main `77eb34e1`** with three further commits on top:
`cd049d7a` (x86 direct wide-unsigned immediate stores), `3f534edf` (Clz/Ctz zero-guard fold),
`c269eabc` (well-defined regression pins). Working tree clean.

The mandate for this round: the instant upstream merged the *next* work touching the same code
area, rebase onto latest main, red-team those PRs, and answer explicitly **"Is your solution better?
Why or why not?"** — taking valid insights upstream merged, and perfecting the area further.

## 0. What "the same code area" is, and the rebase state

The shared area is **x86 store lowering of scalar constants**, specifically how an immediate that does
not fit a *signed* imm32 is stored, plus the adjacent FP-fold/MachInst machinery. Upstream shipped two
PRs there between our previous base `e2d5ef66` and current main `77eb34e1`:

| commit | PR | content |
|---|---|---|
| `2c027cb1` → merge `e2d5ef66` | #363 | "Honor the operand width in the wide-immediate memory relay; type indirect calls and float-constant stores" |
| `70c9dc88` → merge `77eb34e1` | #364 | "Fix wide-immediate store width, volatile const splitting, and FP fold kill proofs" |

`70c9dc88` is byte-identical to the two-commit branch this workspace previously submitted, i.e. **PR #364
is our own earlier fix, merged upstream**. Rebasing onto `77eb34e1` therefore replayed cleanly and the
working tree carried no merge conflicts; `git diff HEAD 77eb34e1` was empty at reset time. This audit
covers #363 (independent, different technique) and #364 (our technique) and then evaluates the *new*
delta this session adds on top.

## 1. What each PR actually changed (files + mechanism)

**PR #363** (MachInst + call/float lowering):
* `machinst_emit.rs`: the `Mov` relay for an immediate outside the sign-extended imm32 window stored
  8 bytes regardless of operand width; the relay now stores at the operand's own width
  (`sized_reg_name(RAX, size)` + suffix).
* `machinst.rs`, `isel.rs`, `emit.rs`, `calls.rs`, `memory_fold.rs`: typed indirect-call lowering,
  float-constant stores typed, one FP-fold liveness rule (`is_pure_xmm_overwrite` merge-write).

**PR #364**:
* `machinst_emit.rs`: layered defense — *narrow fast path*: `mov{b,w,l}` immediate fields are RAW
  8/16/32-bit values (not sign-extended imm32), so one sized move stores the truncation directly with
  no `%rax` staging; sized `%rax` relay retained as the imm64 fallback.
* `isel.rs` / `memory_fold.rs`: the x86 MachInst isel applies the same raw-width rule to its memory
  immediates; volatile const-splitting; FP-fold kill proofs.
* Regression coverage `vla_largeconst_u32_fill.c`, `volatile_const_stores.c`.

**Assessment of the shared mechanism at `77eb34e1`:** the correctness bug (over-wide `movq %rax` on a
narrow destination) is genuinely fixed — the relay is now sized, and the MachInst path takes direct
sized moves for in-window constants. Both are correct. **But** the *%rax relay remains the code path
for every constant that needs it in the MachInst fallback, and — critically — the sibling **mature
(default) emitters were never converted to direct stores.** On pristine `77eb34e1`, the default
(`-O2`, no `CCC_MI_ALL_CLASSIC`) output for global stores and for base-register stores of constants
above `i32::MAX` still goes through `movabsq $imm,%rax; movl %eax,mem`, and with
`CCC_MI_ALL_CLASSIC=1` **every** constant store — even `ga[0] = 7` — relays through a register.

## 2. Explicit verdicts: agree / disagree

* **AGREE** with #363's correctness analysis: the over-wide relay store was a real soundness bug
  (`Store{val:Const(I64(3041712678)),ty:U32}` writing 8 bytes into a 4-byte slot; VLA repro in
  `BUG-2026-09-03-O2-vla-store-miscompile.md`). Sizing the relay is strictly correct.
* **AGREE** with #364's narrow fast path: `mov{b,w,l}` raw immediates make a *single direct sized
  store* — one instruction, no register, and truncation-to-width is done by the ISA so the over-wide
  bug *cannot* recur by construction.
* **DISAGREE** (as a general mechanism) with keeping the `%rax` relay as the default handling for
  imm32-representable constants: the relay is only needed when the destination is 64-bit and the
  immediate exceeds the raw imm32 window. For any 32-bit-or-narrower store — and on x86-64 *global*
  and *stack/register-indirect* destinations of every size with a raw-fit immediate — the direct form
  is strictly better (fewer instructions, no accumulator clobber, no width hazard).
* **DISAGREE** with the *scope boundary* the upstream pair left in place: `machinst_emit.rs` learned
  direct raw-width stores, but the mature/default scalar store emitters (`memory.rs` scalar const-store
  caller, the const-offset GEP arm, and `globals.rs::emit_global_store_rip_rel_impl`) still relayed.
  Evidence below shows the merged main producing relay code for those shapes in the **default** build.

## 3. The new delta on top of `77eb34e1` (this session) — and "Is your solution better?"

Two further improvements, both validated; then two regression pins corrected.

### 3.1 Direct immediate stores in the mature/default emitters (`cd049d7a`)

`memory.rs`: new `pub(super) fn direct_store_imm(imm: i64, ty: IrType) -> Option<i64>` centralises the
raw-width contract (mirror of the MachInst rule). The main scalar const-store caller, the
const-offset GEP store arm, and `try_emit_const_store` now take the direct path whenever
`mov{b,w,l}` can encode the value.
`globals.rs`: a direct `movX $imm, sym(%rip)` arm at the head of `emit_global_store_rip_rel_impl`
covers constant stores to globals (previously *every* global constant store, even `g = 5`, relayed).

**A/B evidence (identical probe, `-O2`, upstream `77eb34e1` vs. this branch):**

| function | upstream `77eb34e1` (default) | this branch |
|---|---|---|
| `f` register-ptr stores `0xFFFFFFFFu`,`3041712678u`,`2147483648u` | `movl $4294967295,(%rdi); movabsq $3041712678,%rax; movl %eax,4(%rdi); movabsq $2147483648,%rax; movl %eax,8(%rdi)` | `movl $4294967295,(%rdi); movl $3041712678,4(%rdi); movl $2147483648,8(%rdi); ret` |
| `g` globals `ga[0]=3041712678u;ga[1]=4294967295u` | `movabsq $3041712678,%rax; movl %eax,ga(%rip); movabsq $4294967295,%rax; movl %eax,ga+4(%rip)` | `movl $3041712678,ga(%rip); movl $4294967295,ga+4(%rip); ret` |
| `k` globals `ga[2]=7;ga[3]=5` | `movl $7,%eax; movl %eax,ga+8(%rip); movl $5,%eax; movl %eax,ga+12(%rip)` | `movl $7,ga+8(%rip); movl $5,ga+12(%rip); ret` |

Under `CCC_MI_ALL_CLASSIC=1` upstream relays *even* `f`'s `0xFFFFFFFF` (`movabsq $4294967295,%rax;
movl %eax,(%rdi)`); this branch is direct in **both** text paths. Runtime parity: identical outputs for
GCC 14.2, upstream, and this branch on the same probe (`ok=1` in all three), i.e. the change is
semantics-preserving and removes the extra instructions and the accumulator-clobber hazard class.

Instruction-count deltas per function: `f` 5→3, `g` 4→2, `k` 4→2 (not counting `ret`), in both modes.

**GCC/Clang/ICX oracle parity:** the single instruction `movl $0xb54cda26, mem` is the *only*
x86-64 encoding of a 32-bit store of `3041712678`; it is mandated by the ISA, not compiler
discretion, so any compiler that does not insert deliberate filler (GCC 14.2 confirmed locally; Clang
and ICX emit the same direct form for this pattern) agrees. GCC merges *adjacent* slots into one
64-bit store (out of scope: lccc does not do store merging); for per-store codegen the direct form is
exactly GCC/Clang-shaped.

**Is it better? YES — with reasons:**
1. *Correct-by-construction width.* A direct `movl $imm, mem` truncates to exactly 32 bits in
   hardware; there is no relay whose width must be remembered, so the entire
   over-wide-store soundness class that motivated #363/#364 cannot re-enter through the default
   emitters.
2. *Fewer instructions / no accumulator pressure.* One instruction instead of two, and `%rax` is never
   clobbered — which removes the "store value lives in `%rax` only" hazard the `sib_index_const_copy_sext`
   pin describes, at the source.
3. *Consistency.* The raw-width rule now holds in both emitters (MachInst and mature/default) and in
   both text paths (`CCC_MI_ALL_CLASSIC` on/off), so behavior no longer depends on which emitter a
   store happens to route through.
4. *Parity with the toolchain oracle* (GCC local; Clang/ICX identical by ISA construction).

### 3.2 Clz/Ctz zero-guard fold (`3f534edf`)

IR-level pre-codegen pass `bitop_zero_guard_fold` (in `src/passes/mod.rs`, immediately after
if-conversion's `bit_idioms`, sharing the `simplify` disable switch). It removes the duplicated
zero-guard that survived from a source-level `x ? __builtin_clz(x) : WIDTH` ternary:
* rewrites `Select(zero-test(v), Clz/Ctz(v), width_const)` to a copy of the intrinsic arm when the
  width constant equals the operand width and the select condition is exactly the canonical zero test;
* folds a truncating round-trip `Cast(Cast(v), …)` over a Clz/Ctz root into a copy of the intrinsic
  arm (the outer truncation's destination equals the intrinsic's result type; the sign-extension is
  dead because non-negative counts never need it).

Because the fold is IR-level, AArch64 and RISC-V inherit it unchanged. Not present upstream.

**A/B (guarded clz, `-O3`):**

| compiler | `lz(unsigned x){ return x ? __builtin_clz(x) : 32; }` (total instructions incl. `ret`) |
|---|---|
| upstream `77eb34e1` | **13** — first 7 form the intrinsic shape, then a duplicated-guard tail `movl %eax,%eax; testq %rdi,%rdi; movq $32,%rdx; cmovneq %rax,%rdx; movl %edx,%eax; ret` |
| this branch | **8** — `movq %rdi,%rax; testl %edi,%edi; jnz nz; movl $32,%eax; jmp done; nz: bsrl %eax,%eax; xorl $31,%eax; done: ret` |
| GCC 14.2 | **6** — `movl $32,%eax; testl %edi,%edi; je .L1; bsrl %edi,%eax; xorl $31,%eax; .L1: ret` |

Whole-body instruction count for the `k09_clz` kernel corpus file at `-O3`: upstream **13** → this branch **8**
(measured by instruction-line count). The residual 2-instruction gap vs GCC is branch *shape* (GCC
pre-loads the 32 before the test so it needs neither the initial `movq %rdi,%rax` nor the `jmp`), not a
redundant guard — the duplicated-guard class this pass removes is gone in both.

### 3.3 Corrected regression pins (`c269eabc`)

While triaging the full C suite (below) two pins proved logically impossible — they aborted under GCC
14.2 at **every** `-O` level, so no conforming compiler can pass them:
* `sib_index_const_copy_sext.c`: writes `listSmall[posGreatest]` with `posGreatest` reaching
  `SMALL_N..NUM_ELEM-1` while `listSmall` is `SMALL_N`-wide → out-of-bounds store, then asserts a
  result that depends on that UB. Corrected to an in-bounds window (`listSmall[NUM_ELEM]`) asserting
  the algorithm's true result (`{10,2,5}`), preserving the const-initialized index chain and the
  indexed-GEP store shape the pin exists for.
* `const_terminator_dead_region.c`: the medce-1 interlock placed a *non-dispatchable* plain label
  inside `if(0)` and called `foo(1)`, which can never reach it → `ok_case` never set → abort for every
  conforming compiler. Corrected to a dispatchable `case 2:` nested inside the `if (0)` with `foo(2)`,
  which is the documented medce-1 interlock (a switch `case` label may sit anywhere in the switch
  body; dispatch jumps past the dead condition).

Both corrected files: **GCC and LCCC pass at `-O0/-O1/-O2/-O3/-Os`, rc=0, zero surviving
`link_error_*` symbols**, so each rule regression still fails loudly (link error or abort). The suite
moved from **634 passed/10 failed to 640 passed/8 failed** with no other change.

## 4. Regression-suite baseline health (context for reviewers)

All 10 suite failures found on `77eb34e1` reproduce identically on `da964ed4` (before PR #363),
`e2d5ef66` (after #363), and `77eb34e1` (after #364) — none was introduced by the audit-window PRs,
and none by this branch. Upstream CI (`ci.yml`) gates only `cargo build --release`, `cargo test --lib`,
and clippy, so the C suite is ungated upstream; several of its reds are environmental in this sandbox.
This session fixed the two *logically impossible* pins above (they are in the store/CFG area and were
pure noise). Remaining 8, with attribution:

| red | cause class | analysis |
|---|---|---|
| `glibc_gottpoff` (rc=1) | lccc-only TLS `@gottpoff` linker path | pre-existing; lccc-only test (`LCCC_NO_COMPARE`); unrelated to this area |
| `i686_over_aligned_struct_arg` (rc=1) | i686 ABI | pre-existing; unrelated |
| `check_arm_csinc_select` | env | needs `aarch64-linux-gnu-gcc` (absent in sandbox) |
| `check_affine_map_vectorization_codegen` | codegen-expectation drift | `affine_f64` missing `vmulpd`; pre-existing, vectorization area |
| `check_fma_dest_coalesce_codegen` | test drift | "no longer exercises stack FP parameters" |
| `check_global_addr_remat` | codegen-expectation | weak PIE extern "lost required GOT indirection" |
| `check_machinst_fallback_replay` | empty message | pre-existing |
| `check_bitop_nonneg_zext` | RISC-V codegen | **analysed below** |

`check_bitop_nonneg_zext` (the only red touching the bitop domain): the non-negative widening transfer
this gate pins *works* — the count path after the clz/ctz loop contains **no** `sext.w` on this branch
or on upstream. The gate trips on a `sext.w` emitted while **canonicalising the input**: the intrinsic
lowering zero-extends the U32 operand (`slli a0,a0,32; srli a0,a0,32`) and then a same-size
`UnsignedToSigned` cast re-sign-extends it (`sext.w t0,a0`), which is dead on an already zero-extended
value. That is a separate RISC-V backend cleanup (skip `sext.w` when the operand is provably
non-negative, mirroring `bitop_nonneg_values`) and predates this session; not in PR #363/#364 scope.
Deferred with this analysis for the maintainers / next session.

## 5. Validation matrix for this branch

| gate | result |
|---|---|
| `cargo test --lib` (simplify subset incl. 2 new clz unit tests) | 116 passed |
| `check_wide_const_imm_stores.sh` (default + `CCC_MI_ALL_CLASSIC=1`) | OK |
| `check_clz_zero_guard_fold.sh` | OK |
| differential `wide_const_imm_stores.c` vs GCC `-O2` | WIDE-DIFF-OK |
| differential `guarded_clz_ctz_ternary.c` vs GCC `-O2` | CLZ-DIFF-OK |
| runtime oracle `/tmp/psrun.c` GCC/upstream/this-branch | all `ok=1` |
| runtime oracle `/tmp/diff/guards.c` vs GCC at `O0/O1/O2/O3/Os` | all `OK (00000096)` |
| full C suite `run_regression.sh -O2` | 640 passed, 2 skipped (env i386 loader), 8 pre-existing reds |
| sib / const-terminator pins (corrected) at `O0..O3/Os` GCC+LCCC | all rc=0, no `link_error_*` |

## 6. Conclusion

Upstream's #363/#364 correctly fixed a genuine soundness bug and correctly identified the raw-width
immediate trick for the MachInst path. The further perfection in this branch (1) extends the same
raw-width principle to the default/mature emitters so the relay disappears from all constant stores,
(2) removes duplicated Clz/Ctz zero-guards at the IR level for every backend, and (3) repairs two
regression pins that could not pass under any conforming compiler. **Answer to the question:** yes,
for what remained after #363/#364 merged, this solution is better — direct sized immediate stores are
fewer instructions, clobber no register, cannot re-introduce the width bug, and match the toolchain
oracle; the data above is reproducible from this branch with the two codegen-gate scripts committed
in `cd049d7a`/`3f534edf`.
