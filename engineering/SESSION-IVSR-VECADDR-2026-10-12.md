# Session: IVSR address-use completion (GVN glue + vec-memory triples + affine chaining)

Date: 2026-10-12. Follow-up to `AUDIT-pr793-followup-2026-10-11.md` §4 (worst-15)
and the IVOPTS-1 work order in `backlog.md` (PERF-1..6 ranking).

## Problem

nbody's pair loop ran **1.721x** GCC (worst native benchmark). Dissection showed
the inner pair walk compiles to 5 instructions of *address arithmetic per outer
iteration* (`movslq`, `imulq $56`, `leaq bodies(%rip)`, `addq`, `movq`) plus
2 `imulq $56` per outer iteration that survived into the inner loop's operand
stream, while GCC keeps one marching pointer per array walk (zero multiplies,
one `leaq` materialisation).

Root cause (three cooperating gaps in `src/passes/iv_strength_reduce.rs`):

1. **GVN Copy glue.** The pass's use-scan matched GEP offsets *literally*
   against the surviving multiply's result. GVN replaces redundant muls with
   `Copy` glue to the old value ids first, so the scan never saw the multiply
   as an offset operand. (The `-O2` IR always looks like this.)
2. **Vector-memory triples were invisible.** The pair loop's `j * 56` feeds
   `VecLoad*`/`VecStore*` argument triples `(base, index, disp)` — the same
   `base + derived [+ disp]` address a GEP offset computes — but the scan
   collected GEP offsets only. The multiply stayed alive for the vector
   accesses alone and no recurrence ever formed for `j`.
3. **Affine siblings rebuilt the same pointer.** The `j = i + 1` init is
   `base + (i+1)*56`, collected (after (1)/(2) are fixed) as a second,
   separately-recurred pointer provably equal to `ptr_i + 56` at every
   iteration.

## Fix (all in `src/passes/iv_strength_reduce.rs`)

* `DerivedExpr` now carries both `gep_uses` and `vec_uses` (the latter with the
  store/load polarity from `alias::vec_intrinsic_access`, which is the single
  modelled contract for those argument shapes and fails closed on value
  displacements, odd arity, and unmodelled ops).
* `BaseKey` (`Value(root)` / `Symbol(name)`) + `alias_map` (Copy, same-size
  int/ptr casts, zero-offset GEPs; bounded walk, cycle-safe). GEP offsets,
  vec triple indexes, and `ptr_add` offsets all resolve to the multiply
  root through the map.
* **One pointer recurrence per (IV, stride, base-key, mul type).** GEP uses
  rewrite to `Copy(ptr_iv)`; vec uses rewrite in place to
  `(ptr_iv, Const(0), disp)` with arity and the store's value preserved
  (every consumer — `vec_intrinsic_access`, liveness, `emit_vec_store_addr` —
  indexes slots positionally).
* **Symbol-keyed GlobalAddr bases** are invariant by construction (a link-time
  object); the recurrence init materialises its own fresh `GlobalAddr` in the
  preheader, so a base *defined inside the loop body* still reduces.
* **Affine sibling chaining:** a group whose uses are all GEPs and whose
  `(iv, stride, mul-type, base)` matches an already-created recurrence at a
  different constant offset `k` rewrites to `GEP(parent_phi, (k-k0)*stride)` —
  no phi, no init, no latch step of its own. Groups process in sorted
  `(iv, stride, k)` order so the parent materialises first.
* **Affine form through widening casts:** `find_affine` now walks Copy glue and
  `cast_preserves_offset_value` casts to the `Add(iv, k)`, so
  `mul(sext(i + 1), stride)` collects as the affine `(iv + 1) * stride`.
  WRAP LICENSE (documented at the site, same doctrine as `quadratic_sr.rs`):
  a wrapping narrow `Add` would diverge; narrow signed overflow is undefined
  in the C semantics LCCC compiles (the driver has no `-fwrapv` contract).

Untouched on purpose: the overlap guard ("keep inner, skip outer" — OP-08
nested-matmul address-doubling), `offset_product_cannot_overflow`,
`unsigned_iv_bound`, `MAX_CAST_CHAIN_LENGTH`, the max-1-pointer-phi policy
(chaining is additive to it), and the scalar derived-IV default-off knob.

## Measured results (nbody, `-O2 -march=x86-64-v3`, 50M steps, min of 5 interleaved)

| metric | before | after | GCC |
|---|---|---|---|
| `imulq $56` in advance | 2 | **0** | 0 |
| `leaq bodies(%rip)` materialisations | 3 | **1** | 1 |
| inner-loop address arithmetic | 5 insn/outer | **0** | 0 |
| runtime ratio vs GCC | 1.721x | **~1.09x** | 1.00x |
| checksum (50M steps) | — | bit-exact vs GCC | — |

The remaining ~9% is FP register pressure in the inner body (`movapd`/`pshufd`
scalar move chains and 2 stack spills that GCC keeps in registers) — the
"coalesce scalar FP moves" leg of IVOPTS-1 and RA work, NOT address
arithmetic.

## Tests and gates

* 6 new unit tests in `iv_strength_reduce.rs` (33 total in the module,
  4476 lib tests green): `gvn_copy_glue_gep_offset_still_reduces`,
  `vec_addr_index_triple_becomes_pointer_recurrence`,
  `vec_addr_use_alone_forms_recurrence`, `vec_addr_value_disp_is_left_alone`
  (fail-closed negative), `global_addr_base_defined_in_loop_body_still_reduces_via_symbol`,
  `affine_sibling_chains_to_parent_recurrence`.
* New regression gate `tests/regression/check_ivsr_vecaddr_struct_walk.sh`
  (registered in `scripts/ci_local.sh` as `ivsr-vecaddr-struct-walk`):
  self-checking integer pair-walk (bit-exact reference leg) at -O0..-O3 x
  {64-bit, i386, `CCC_NO_IVSR_PTR_ADD=1`} plus the SHAPE pin
  (0 x `imulq $56`, exactly 1 `leaq bodies` materialisation).

## To-do (ranked, next session)

1. **Coalesce scalar FP moves (IVOPTS-1 leg 3).** nbody's remaining gap is
   `movapd %xmm15,%xmm0; pshufd $0x0E,%xmm15,%xmm0` plus
   `movsd %xmm15, 24(%rsp)` spill/reload of a value that lives in `xmm15`.
   That is x86 ISEL/RA territory (`split_m64_form`'s "FE-collects-scalar-forms"
   comment is the entry point).
2. **Pointer-recurrence chaining across reduce calls.** The sibling chain
   only fires within one `reduce_loop` call. A P2 created by the *inner*
   loop's reduce that equals `outer_ptr + const` is not yet folded (needs a
   post-pass over recurrence phis).
3. Apply the same triple collection to the `ptr_add` (address-Add) scan —
   `Add`-derived addresses feeding vec triples are still invisible there.
4. Re-run the worst-15 table (`engineering/AUDIT-pr793-followup-2026-10-11.md`
   §4) with the new binary; nbody, matmul, and the expat pair-loop family
   should move. Next targets per the ranked list: PERF-6 > PERF-5 >
   loop-rotation > PERF-4.
5. `expat_siphash24` (worst-15 #2 at 1.585x): GCC wins via inlining threshold
   differences — separate workstream, see backlog PERF-5/PERF-6.
