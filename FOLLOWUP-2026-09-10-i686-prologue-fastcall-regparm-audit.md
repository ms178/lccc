# FOLLOWUP-2026-09-10-i686-prologue-fastcall-regparm-audit.md

Red-team audit of `src/backend/i686/codegen/prologue.rs` (12 findings) plus the
follow-up review of the merged casts work (PR #480). All findings below were
reproduced or refuted empirically against GCC 14.2 (`-m32`) as the ABI oracle
and against the x86-64 backend for the cross-backend classes.

## Base

- Anchor: origin/main e6c08ab0 (Merge PR #481). The casts audit (PR #480,
  68d57a3a) is merged upstream and is byte-identical to this tree's casts.rs.
- Every change is validated by: fastbuild, `cargo test --lib` (2301 pass),
  the i686 battery (battery1.c), the 4-way callee/caller interop matrix
  (gcc/lccc × gcc/lccc), dedicated PC-matrix / sret / variadic harnesses,
  and `scripts/ci_local.sh --fast`.

## Adjudicated findings

### Confirmed and fixed

**F128 -> U64 depended on the x87 precision control (follow-up review #1).**
Reproduced: `(unsigned long long)18446744073709551615.0L` under 24- and
53-bit precision control produced 0 (the subtraction-based high path rounded
2^64-1 to 2^63, outside signed range, -> integer-indefinite). GCC 14.2
miscompiles identically on i686 AND x86-64. Fix (both backends): the high
path stores the native extended representation with `fstpt` (stores are
exact, no arithmetic) and reads the 64-bit significand directly when the
sign/exponent word is 0x403e; NaN/inf/>=2^64 fall through to `fisttpq`
(GCC-compatible indefinite). The low path keeps a direct `fisttpq`. No
arithmetic on either path => independent of PC at every setting. lccc now
passes the full matrix at PC24/PC53/PC64 where GCC fails 3 cases.
- i686: `casts.rs cast_x87_to_u64`; x86-64: `f128.rs
  emit_f128_st0_to_int` + `emit_f128_to_u64_cast`.
- Runtime regression: `tests/regression/casts_roundtrip.c` gained a
  PC-matrix section (`fnstcw`/`fldcw`, boundary + NaN cases). The GCC
  oracle binary fails those checks, so `casts_roundtrip.c.env` sets
  `LCCC_NO_COMPARE=1` (established pattern for defective oracles).
- Unit tests: integer-arithmetic precision model
  (`round_integer_to_binary_precision`) proving the counterexample, plus
  ties-to-even and native-payload boundary tests in casts.rs.

**F128 direct-copy ECX clobber (follow-up review #2).** Audited the
allocator model: `collect_i686_scratch_hazard_points`'s catch-all already
marked Cast points as ECX/EDX hazards, and `eliminate_dead_reg_moves`'
census fallback consulted only *textual* reads. That census deleted
`movl %eax, %ecx` right before a fastcall call (the call never names %ecx
yet reads it) — reproduced with a fastcall struct-return: the sret pointer
stayed in EAX while the callee read ECX (SIGSEGV). Fixes:
- `census_reg_reads` now counts `Call` as a read of EAX/ECX/EDX and
  `JmpIndirect` as a read of every GPR (matches the GprLiveness oracle's
  Call model already documented in the file).
- An explicit, documented Cast arm in the hazard collector pins the
  F128-cast scratch contract (same-value casts stay clean).
- The fast copy path itself is unchanged (6 movs, no stack engine); the
  allocator contract is now provable, not incidental.

**Fastcall caller marshalling was broken for every non-trivial shape.**
One authoritative layout (`fastcall_layout`) now drives caller, callee
capture, ParamRef and `ret $N`. The caller-side rewrite fixed: constant
F128/D128/i128 arguments silently NOT written to the outgoing area
(reproduced: fastcall(long double, int) read garbage); aggregates
truncated to one word; variadic fastcall now all-stack with caller
cleanup (GCC rule); sret now marshalled ECX=sret, EDX=arg0, stack=rest
with `ret $4` interop-verified both directions; float/_Decimal scalars
skip-not-break (D32/D64/D128 verified against the GCC oracle battery).

**Parallel capture (Finding 1).** Incoming-register captures are a real
parallel move (`resolve_incoming_reg_moves`): ready moves first, 2-cycles
via xchgl, longer cycles via a stack scratch. Register-sharing params
(Finding 2) get 4-byte conflict slots allocated past the packed slot
region; ParamRef reloads from them.

**Fastcall overwrite order / narrow upper bits (Findings 3-5).** Register
captures run before any stack copy; sub-int params are normalized
(sign/zero-extended) into their slots at capture; ParamRef uses the exact
layout offsets instead of a blanket `reg_count * 4` subtraction.

**D64 (Findings 6-7).** Callee-pop accounting and the 8-byte copy paths
now include D64 (BID container copies — no x87 round trip).

**CFI (Finding 8).** Callee-save pushes now emit `.cfi_offset` (and, in
no-frame-pointer mode, the ESP-tracking `.cfi_def_cfa_offset` chain); the
frame subtraction advances the CFA. Frame-pointer mode keeps CFA=%ebp+8
with only offset growth, as the DWARF spec requires.

**`__attribute__((regparm(N)))`.** Implemented end to end (parser with
GCC-diagnostic parity — bare `regparm` is an error, N>3 warns and clamps,
`fastcall`+`regparm` is rejected; decl-side and def-side collection; per-
call `CallInfo.regparm`; per-function effective regparm in the prologue;
caller staging via the per-call effective value — the global-only gate in
`emit_call_reg_args_impl` was the last gap). Precedence: the attribute
overrides the global `-mregparm` flag; fastcall overrides both (verified
against GCC: `-mregparm=3` + fastcall attribute keeps ECX/EDX). The 4-way
matrix (37 cases incl. 11 regparm signatures) is green in all four
callee/caller combinations.

**x86-64 cross-backend (follow-up review #1 applied).** x86-64 had the
same subtraction-based F128->U64 defect (long double is 80-bit there
too). Fixed identically; ARM/RISC-V route F128<->U64 through
`__fixunstfdi` softfloat (IEEE binary128) and are immune by construction.

### Verified non-issues (current code)

- F9 alignment bias: bias 12 (no FP) / 8 (FP) is the correct CFA-derived
  offset for 16-byte local alignment.
- F10 GOT allowlist: deliberately conservative, documented; correctness
  over one saved register.
- F12 fold sets: rebuilt fresh per function (`build_foldable_global_addr_set_for`
  + `build_folded_value_set`); no cross-function residue.
- `fstpt`/`fldt` copies (F11): replaced with bit-exact 12-byte movl copies
  in all param-capture and ParamRef paths; caller-side F128 args copy the
  slot bit-exactly (no x87 round trip that re-canonicalizes exceptional
  encodings).
- Performance claims in casts.rs: replaced with mechanism statements
  (the "measured ~15%" claim had no retained artifact).

### Known gaps (documented, not silently claimed)

- **Over-aligned parameter allocas**: the frontend drops type-level
  `aligned(N)` on parameter types (`param.struct_align` only covers
  structs), so `&x` for an `aligned(32)` parameter is misaligned vs GCC
  (which dynamically realigns). Repro: ovalign3.c (scratch). Fix plan:
  propagate the parameter type's alignment into the IR param alloca, and
  make the ParamRef read the effective (aligned) address — the capture
  side already targets the effective address.
- `emit_f128_to_u64_cast`'s rax-loading head on x86-64 is legacy-shaped;
  the live st0 path is fixed and tested.

## Test inventory added

- `tests/regression/casts_roundtrip.c`: x87 PC matrix (24/53/64) over
  F128->U64 boundaries + indefinite fallback; fastcall/regparm F128 copy
  pressure test; `casts_roundtrip.c.env` marks the GCC oracle defective.
- casts.rs unit tests: precision model, counterexample, payload
  boundaries, ties-to-even.
