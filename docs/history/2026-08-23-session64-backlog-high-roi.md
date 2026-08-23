# Session 64 — Ten high-ROI backlog items: codegen, DSE, GVN forwarding, `_Pragma`

Date: 2026-08-23

Upstream base: `846f17d5` (latest `ms178/lccc` main at session start; rebased
mid-session from `9d2aba1c` after PR #212/#213 landed). Compiler builds used
`cargo build --profile fastbuild --locked -j 2` (Rust 1.98.0). `swapon` is
not permitted in this sandbox; `scripts/ensure_swap.sh` now warns and
continues (upstreamed in this patch) so the fastbuild wrapper works
unprivileged.

## Mandate

The ten native x86 torture fixes of session 63 were merged upstream (PR
#210), so this session pivoted to the **BACKLOG**:
pick the ten highest-ROI open items and fix them properly.

## Delivered (8 complete items + 2 probe-only, see Follow-up)

| Item | P | Change | Evidence |
| --- | --- | --- | --- |
| IS-24 | P1 | Immediate stores through register-homed (`SlotAddr::Reg`) and over-aligned bases: `movl $3, 4(%rdi)` instead of `movq $3,%rax; movl %eax,4(%rdi)` | probe: `g()` 2 stores both immediate-form; structural gate `check_imm_store_movl_sib.sh` |
| IS-15 | P1 | Indexed I32 loads honor the function-wide sext analysis: `movl (%rcx,%rdi,4), %r9d` instead of `movslq` when the value has no 64-bit consumer (mirrors the non-indexed `mov_load_for_value` path) | probe `load()` + structural gate |
| OP-13 | P1 | New `src/passes/dse.rs`: same-block dead-store elimination with closed-alloca escape analysis, byte-range cell kills, global roots keyed by canonical symbol, volatile/segment-overrides fail closed. Wired at -O1 and in the main loop after DCE (with a follow-up DCE round) | `x=1;x=2;x=3;` now keeps 1 store (GCC parity); 5 unit tests + `dse_overwritten_stores.c` |
| OP-12/OP-32 | P1 | GVN: (a) **canonical address keys** — site-local `GlobalAddr` duplicates (OP-34 policy) get their GEPs folded onto one address VN for load-CSE/store-forwarding keys, without touching the SIB placement policy; (b) F32/F64 stores are forwardable (same session-25 safety class as FP load CSE); (c) surviving `Copy` instructions are value-numbering transparent (dest inherits src VN) | `g1[i]=5; g2[i]=6; return g1[i];` forwards to `movl $5,%eax` (GCC parity); `gvn_site_local_store_forward.c` pins the may-alias counter-case |
| IS-02/AB-01 | P0 | All-SSE 16-byte struct returns (`struct{double,double}` etc.) return in xmm0/xmm1 via `SetReturnF64Second` instead of the I128 Cast/Shl/Or "rax shuttle"; the [Sse,Sse] primary return no longer clobbers xmm1 after an explicit second-half set; inline candidates keep the I128 form (the inliner merges only the Return operand); vector types keep the single-XMM vector ABI | `mk` 24→11 instructions, 88 B→24 B frame; cross-ABI interop both directions (gcc-caller/lccc-caller); `sse_struct_two_reg_return.c` |
| IS-20 | P1 | `vzeroupper` before `ret` for any function that dirtied upper YMM halves (256/512-bit intrinsics, YMM loads/stores); explicit-cleanup sites (64 B memcpy path, AVX FMA reduction) disarm the flag so no duplicate is emitted | intrinsic copy64 emits `vzeroupper` before ret (was missing); `vzeroupper_after_ymm.c` + `.flags` |
| FE-21 | P1 | `_Pragma("...")` operator implemented (C99 6.10.9): de-stringizing (`\"`→`"`, `\\`→`\`), operator queued during macro expansion, pipeline drains and executes each as `#pragma <content>` in source order (per-line + EOF) | `_Pragma("push_macro/pop_macro")` round-trip through macros matches GCC exactly; `pragma_operator.c` |
| MS-08 (partial) | P1 | `ensure_swap.sh`: unprivileged sandboxes now warn-and-continue instead of hard-failing every build wrapper | fastbuild runs in this sandbox |
| — | — | `cargo test --lib` compile fix: 7 LICM unit tests still called the 4-arg `hoist_loop_invariants` (the `idom` parameter was added upstream but the tests were never updated — `cargo test --lib` did not compile on main) | 1101/1101 unit tests run and pass |
| — | — | `check_gvn_disjoint_epochs.sh` load-mnemonic pattern extended to `movl` (IS-15 changed the emitted mnemonic, not the load count; restrict=1 load, plain=2 loads still hold) | gate passes |

## Fixes in detail

### DSE (OP-13) — soundness model

A **cell** is `(root, byte offset, size)`; roots are allocas, canonical
global symbols, or opaque pointer SSA values. Backward scan keeps "pending"
later stores; an earlier store dies only when a pending store fully covers
its cell with no intervening read. Reads kill pending entries they may
alias: exact cells kill overlapping same-root entries; unresolvable
pointers and opaque events (calls, atomics, asm, memcpy, varargs) kill
everything except **closed allocas** — allocas whose derived pointers are
only ever used as `Load.ptr`/`Store.ptr`/const-GEP bases. Globals are
always open (other TUs may hold their address). Volatile stores never die
but can overwrite; segment-overridden accesses are isolated from
default-space cells. `CCC_NO_DSE=1` disables; `CCC_DISABLE_PASSES=dse` in
the main loop.

### GVN canonical address keys (OP-12)

OP-34 deliberately keeps `GlobalAddr` duplicates feeding variable-index GEPs
site-local (each site becomes its own SIB fold with an independent base
register). That policy used to fork the *value numbers*, so
`g1[i]=5 ... load g1[i]` through two sites produced unequal GEP keys and the
store-forwarding candidate lookup missed. The fix records, per numbered
GEP, a canonical address VN keyed by `(canonical symbol VN, offset VN,
type)`; memory-key construction (`ExprKey::Load`, `StoreFwdKey`) folds
through `gep_addr_canonical`. The GEP instructions themselves are untouched
— SIB placement, rematerialization, and the IS-01 regression stay exactly
as they were. Rollback safety: the maps are rebuilt per function
(`GvnState::new`), and the folded keys never feed `expr_to_value` for the
GEPs themselves, so dominator-scope restore semantics are unchanged.

### SSE struct returns (IS-02) — the three traps

1. **Caller/callee class agreement**: the caller side is already class-driven
   (`emit_call_store_result_impl` reads `call_ret_classes`), so both sides
   stay consistent when the callee switches to the F64+Second form.
2. **xmm1 clobber**: `emit_return_i128_to_regs_impl` for [Sse,Sse] moved
   `%rdx → %xmm1` after the primary value; with an explicit
   `SetReturnF64Second` in flight that would destroy the high eightbyte. A
   per-function `func_set_second_ret` flag (reset in the prologue, set by
   the SetReturn emitter) suppresses the `%rdx` move.
3. **Inlining**: the inliner merges ONLY the `Return` operand into its phi;
   a value split across Return + SetReturnF64Second cannot be reassembled at
   an inline site. `is_inline_candidate` functions (inline / always_inline /
   static — the `_mm_*` wrappers are `static __inline__`) therefore keep the
   I128 packing. This was caught by the `agg_fwd_param_home` /
   `simd_sse_float` / `simd_movnt` regressions: `__m128` is a *struct* in
   lccc's headers, so the wrapper returns initially took the new path and
   lost lanes 2-3 at every inline site.

### `_Pragma` (FE-21)

`MacroTable::handle_pragma_operator` parses `_Pragma ( L? "..." )` with
optional encoding prefix, de-stringizes `\"` and `\\` (other escapes pass
through verbatim), removes the operator from the token stream, and queues
the content. The pipeline drains the queue at the top of every source line
and at EOF, executing each via the existing `handle_pragma` — so
`push_macro`/`pop_macro`/`weak`/`visibility` inside macros now apply in
source order. Malformed operators keep the identifier for a downstream
diagnostic. The old `skip_pragma` (silently dropped, C99 §6.10.9 violation)
is deleted.

## Validation

```
cargo test --profile fastbuild --lib        # 1101 passed, 0 failed
tests/regression/run_regression.sh          # 445 passed (+7 new), 5 failed
                                            # (i686 rc=159 x3, arm_csinc,
                                            #  fma_dest_coalesce — all 5
                                            #  pre-existing on main, verified
                                            #  by baseline re-run)
tests/correctness/run_correctness.py        # 50/50 vs GCC
x86_gcc_torture_slice.sh (session-62 fail list re-run):
  20080604-1 20051215-1 20041019-1 20070212-2 20060929-1 20041214-1
  20050121-1 20070614-1 20070919-1 20071210-1 20071220-1 20020412-1
  20230630-2 20230630-4 920302-1 20040709-{1,2,3}   # all PASS (session-63
                                                    # upstream fixes hold)
```

Cross-ABI interop (SSE struct returns): gcc-compiled caller + lccc callee,
and lccc caller + gcc callee, both produce exact results.

## Follow-up (next session)

1. **IS-02b / multi-value YMM intrinsics** (the remaining IS-02 half): two
   live `__m256` values spill through a 72 B stack frame vs GCC's 5-register
   form. Root: vector intrinsic results have no YMM-family register homes
   when more than one is live; needs the RA-21 vector-home machinery
   extended to 256-bit, NOT a peephole.
2. **AB-03**: 4-byte-slot zero-extension audit (probed, no defect found in
   the scalar path; the `movslq`-vs-`movl` half was fixed as IS-15).
3. **MS-06/RA-19**: PhysReg map single source of truth (`emit.rs` vs
   comments; r10/r11 encoding is documented in two places).
4. **DSE cross-block**: the same cell machinery with post-dominance +
   read-between proofs would catch stores dead across blocks (xmltok
   store-heavy TUs). Keep fail-closed.
5. **GVN OP-12 extension**: apply `forms_disjoint` (alias.rs) to load-CSE
   across distinct restrict-param bases in loops — the restrict epoch path
   exists; the *symbolic* disambiguation of two `Other` roots is missing.
6. The 5 pre-existing regression failures (i686 rc=159 trio needs a 32-bit
   loader on the host; `arm_csinc_select` has 16 preprocessor errors —
   likely a header path issue in this sandbox, NOT a code defect; the
   fma_dest_coalesce guard test needs its fixture refreshed).

## Do not

- Do NOT route inline-candidate functions through the split
  Return+SetReturnF64Second form (inliner drops the second half).
- Do NOT forward stores through `Other`-root pointers that may alias
  (the `no_fwd` counter-case in `gvn_site_local_store_forward.c` is the
  pin).
- Do NOT emit vzeroupper unconditionally (only on dirty_upper_ymm; the
  32/48-byte XMM policy measured slower with an unconditional emit).
- Do NOT CSE loads from param allocas — unchanged rule; the new canonical
  address keys only fold *symbol-based* GEPs.
