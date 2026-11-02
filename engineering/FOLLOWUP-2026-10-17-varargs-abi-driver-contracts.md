# Session follow-up — 2026-10-17: vector varargs ABI, linker sysroot robustness, driver option contracts (#847), census ratchet

Branch: `arena/7071cc95-lccc`, head at writing: `5dfcbd90` (pushed).
Vehicle: cumulative PR against `ms178/lccc` `main` (see "PR topology" below).

## What landed this session

### 1. Vector-shaped variadic arguments per the SysV x86-64 ABI (`2264bc9b`)

Root cause of the `evself` defect: `[Integer]`-carrier vectors (u8x3 ≤ 4 B,
bit-packed `bool`) travel in **GP** registers, but `Instruction::VaArgVector`
always read XMM.

Fix (final design):
`Instruction::VaArgVector { dest_ptr, va_list_ptr, size, align,
eightbyte_classes }`; classes from `CType::sysv_vector_carrier()`
(None → empty). Dispatch in `emit_va_arg_vector_impl`:

- empty → `emit_va_arg_struct_overflow` (no SSE at all: GP overflow fetch)
- GP count > 0 → `emit_va_arg_struct_ex_impl` (per-eightbyte GP fetch)
- else ONE XMM slot: `cmpl $160, fp_offset; ja mem` → `movdqu` (size ≥ 16) /
  **`movq`** (size < 16 — `movdqu` would overrun the 8-byte alloca) store →
  `addl $16, 4(%rcx)`; `no_sse` → GPR overflow fetch with align fixup

Caller side: `StructSseReg` staging passes the value in XMM, the `%al` census
counts it; `infer_struct_eightbyte_classes` consults the carrier; ExtVector
size/align arms gated `Target::X86_64`. Threading through instruction.rs,
generation, liveness, traits, emit, inline, outline_switch, copy_prop,
ip_purity, ipcp, licm, loop_unroll, vector_temp_promotion, expr_access,
expr_calls.

psABI ground truth (measured GCC 16.2): `%al` = hidden upper bound (0–8) of
vector registers for variadic calls; variadic vectors travel in XMM registers
and count into `%al`; the consumer advances `fp_offset` by 16 per vector slot
and fetches ONE `movdqu` per register-class slot. GCC's `movl $N,%eax` is
conformant; lccc's old pointer-spill and the 2-slot `[Sse,Sse]` consumer fetch
were not.

Validation: GCC-cross 4-leg battery (GG/GL/LG/LL, expect 167) PASS at
-O0/-O1/-O2/-O3; self round-trip (`evself.c`, want=1169) PASS at all tiers;
ext_vector direct AND function-pointer round-trips pinned in the new gate
`tests/regression/check_ext_vector_varargs_abi.sh` (12 PASS lines), wired as
ci_local gate `ext-vector-varargs-abi` + GitHub CI step.

Known residual (pre-existing, unchanged): 1-lane f32/f64/ld ext vectors
classify `[Integer]` (tracked gap in `sysv_vector_carrier`); `-mno-sse` +
FP va_arg remains a hard reject (x87 gap, same at the base commit).

### 2. GNU-ld sysroot strip-and-retry (`ddb5b5a2`)

A sysroot tree whose `libc.so` script had been pre-resolved against the very
same `LCCC_SYSROOT` failed every link with the doubled candidate
`$SYSROOT/$SYSROOT/lib32/libc.so.6`. GNU ld strips a baked-in prefix and
probes the remainder; the shared script resolver and the i686 linker's own
copy now do the same (unit test beside the existing prefix test). The m32
debroot tree additionally got the merged-usr compat symlinks
(`lib32→usr/lib32`, `lib→usr/lib32`).

### 3. Census ratchet held by real refactors (`ae5b6a6a`)

The vector-varargs merge pushed three lowering functions over the audit
census thresholds. Lowered the counts instead of re-baselining:
`try_push_complex_param_ir` out of `build_ir_params` (func_lowering),
`classify_abi_return` out of `register_block_func_meta` (stmt_init),
`try_fold_builtin_or_attr_call` out of `lower_function_call` (expr_calls).
Pure seams, no behavior change.

### 4. expat_xml_scan push count: re-baselined with cause (`ddb5b5a2`)

7 pushes vs 6 at e8ae10a — measured cost of `cb066155` (coalesce overlap
tolerance requires single-def endpoints; one callee-saved save for correct
multi-def group handling). The hot loop's instruction ladder is unchanged.
The same refresh folded in previously unrecorded improvements from the fix
chain (gzip_crc32 insns 258→255 / stackmem 15→14, sqlite_varint stackmem
11→7).

### 5. PR #847 landed and completed (`434c2105..` cherry-picks + `5dfcbd90`)

The 9 driver option-contract commits were cherry-picked onto this branch and
completed. The integration resolved the collision between #847's permissive-
base strictness and the A13 builtin policy / tier policy already on this
branch:

- One `BuiltinPolicy` object (blanket + sticky per-name withdrawals) now
  reaches every fold consumer explicitly: sema const-eval, expression type
  checker, lowering const-eval, lowering builtin ladder, simplifier (main +
  low-tier math fold), interprocedural purity, post-inline strlen fold.
  A per-name `-fno-builtin-strlen` genuinely keeps the ordinary `strlen` call
  while `__builtin_strlen` still folds, at every tier.
- Option table synthesis: table no-op rows stay silent, meaning-changing
  requests refuse, and a table MISS gets the generic tier treatment
  (tolerated diagnostic once per spelling; fatal under LCCC_STRICT_OPTIONS=1)
  instead of a hard refusal — a newer GNU's inert preference must not break a
  real build, strict mode keeps capability probes honest.
- Reclassified after re-measurement against the kernel/glibc build surface:
  `-fno-plt`, `-fsemantic-interposition`, `-fno-semantic-interposition` are
  inert no-ops (correct-but-different / conservative code, not miscompiles);
  `-fsanitize-recover*`/`-fsanitize-undefined-*` configure an instrumentation
  that does not exist. LTO stays a refusal (ignoring it would read as
  support).
- Duplicate driver field declarations collapsed; the A13 arms keep
  `freestanding`/`no_builtins` fields in lockstep with the policy.
- `strlen("ab\0cd")` folds to 2 (embedded-NUL), shared const-eval and
  post-inline pass.
- Gate `tests/regression/check_driver_option_contract.sh` extended and PASS;
  65/65 cli unit tests; full suite 5075/0/9; fmt clean; clippy clean.

#848 (`ext_vector_type: model padded layout and the real by-value ABI`) is
fully contained by the merged #849 rework plus this branch's varargs work —
verified by ancestry (`186bd1ba` is an ancestor) and concept spot checks
(`sysv_vector_carrier`, `i686_ext_vector_gp_word_count`, ExtVector call ABI).

## Environment notes

- Swap: 8 G `/swapfile` active throughout (verified via /proc/swaps).
- ICX 2026 + Clang 23 remain unobtainable (blocked hosts) — disclosed, not
  faked; oracle legs SKIP honestly (`LCCC_REQUIRE_*` in CI enforces).
- m32 host-GCC oracle legs remain env-impossible (gcc-multilib absent; run
  legs need a host i386 loader); lccc's own -m32 legs PASS with
  `LCCC_SYSROOT=/tmp/debroot`.

## Open work (next sessions)

1. aarch64 gas re-provision (hc3) + encoder differential; i686 run-level
   probe blocked on 32-bit libc (dpkg step).
2. Kernel task: linux-cachymod 6.18.55 full build+boot with lccc
   (`prepare_kernel_tree.sh` → `build_kernel_full.sh`).
3. 1-lane f32/f64/ld ext-vector carrier classification gap.
4. `-mno-sse` x87 varargs path.
