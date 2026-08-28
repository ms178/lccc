# LCCC Follow-up / Kontinuität — Session 90 (2026-08-28, cross-backend ABI round)

**Scope:** work through the `current_tasks/` queue across all four supported
architectures (x86-64, i686, AArch64, RISC-V), transfer recent code-quality
improvements between backends, and add regression tests for everything.

**Base:** `origin/main @ 2b53a6ec` (PR #274 = merged ms178-1.patch)

---

## 1. Queue disposition (8 of 9 closed)

| Task | Verdict | Evidence |
|---|---|---|
| fix_arm_asm_caspal_instruction | **fixed** | `encode_casp` per LLVM `AArch64InstrFormats.td`; all 4 orderings × X/W pairs + SP base byte-exact round-tripped through Capstone (0x4860FD62 et al.); `LineKind`-propagation skip extended to the whole casp/cas family |
| fix_arm_asm_global_branch_relocs | **fixed** | `resolve_local_branches` now checks `global_symbols`/`weak_symbols` before in-place baking (GAS preemption parity); `readelf -r` shows CALL26/JUMP26 against same-section globals, locals stay baked |
| fix_arm_asm_org_directive | **fixed** | `.org new-lc [, fill]` implemented with token-boundary-aware `.`/label substitution, immediate evaluation (GAS parity), backward-move and forward-ref errors. Vector-table shape (3×128 B entries, fill bytes) verified |
| fix_arm_asm_quad_prel64_relocation | **already fixed** (stale file) | parser `sym+offset` decomposition, size-based PREL32/PREL64 selection, `RelocType::Prel64` (260) all present; `.quad sym+48 - .` emits `R_AARCH64_PREL64` with addend 0x30 |
| fix_arm_movw_symbolic_relocations | **fixed** | new `AbsGSpec` classification, write-time alias/label resolution with overflow checks, MOVW relocations (ABI 263–272) for globals/externals; kernel `tramp_alias` shape encodes inline byte-exact (G2_s=0xFFFF, G1_nc=0xFF5F, G0_nc=0x5FFC) |
| fix_i686_double_param_high_word_store | **already fixed** (stale file) | `f01` asm carries both stores; all trigger shapes pass at -O0/-O1/-O2; locked with `tests/regression/i686_double_param_high_word.{c,flags}` |
| fix_pcre2_stack_frame_bloat | **already fixed** (stale file) | width-partitioned small slots landed earlier; A/B differential lives in the suite (`CCC_NO_SMALL_SLOTS`) |
| fix_riscv_va_arg_long_double_struct | **fixed (end-to-end)** | see §2 |
| fix_dash | **deferred** | needs a RISC-V execution environment (qemu-user + riscv sysroot) absent from this sandbox; compile-side attempt not meaningful without the target libc |

## 2. RISC-V va_arg struct{long double} — the full repair

The earlier IR-lowering fix aligned the *callee's* va_arg read pointer, but
the *caller* still placed over-aligned structs on 8-byte boundaries and the
callee's named-parameter layout matched that old shape. Three coordinated
fixes make both sides agree with the psABI:

1. **Caller forward layout** (`riscv/codegen/calls.rs`): per-arg alignment
   padding from the previously-ignored `struct_arg_aligns` parameter, via
   the shared `compute_stack_arg_padding`. `struct { long double }` after
   an odd 8-byte overflow arg now lands 16-byte aligned.
2. **Space accounting** (`call_abi.rs`): `compute_stack_arg_space` is now
   padding-aware (it previously rounded the unpadded total, which can
   under-allocate when two over-aligned structs interleave with scalars —
   8+16+8+16 needs 64 B, the old math reserved 48).
3. **Named-parameter layout** (`call_abi.rs` shared callee loop) and
   **va_start base** (`named_params_stack_bytes` now derived from the
   ParamClass offsets themselves, so it can never drift from the layout).

`misalign(1..9, {42.0L})` assembly now shows: caller reserves 32 B, places
a9@0 and the struct@16; callee va_list=sp0+8, align-up → sp0+16, reads
exactly the struct. Before, the caller wrote @8 while the callee read @16.

**Cross-backend transfer:** ARM's caller got the identical padding, and ARM
`emit_va_arg_struct` now honors its previously-ignored `align` parameter
(register-path `__gr_offs` rounding-down, stack-path 16-byte align-up —
AAPCS64 parity). The shared named-layout loop benefits x86-64 too (its
caller already padded; the callee now matches).

## 3. MOVW relocation types — a latent linker bug found and fixed

The AArch64 linker's MOVW type numbers assumed a consecutive
G0/G0_NC/G1_NC/G2_NC/G3 layout that matches nothing: per IHI0056B /
LLVM `AArch64.def` the real numbers interleave the non-NC forms
(G0=263, G0_NC=264, G1=265, G1_NC=266, G2=267, G2_NC=268, G3=269,
SABS_G0=270, SABS_G1=271, SABS_G2=272). G1_NC was resolved with G1's
shift, G3 with G2's — dead-wrong code that could never fire before the
assembler gained MOVW emission. Fixed constants + handlers for all ten
forms. There is no G3_NC / SABS_G3 in the ABI; `:abs_g3_nc:`/`:abs_g3_s:`
are rejected with precise messages.

## 4. Other quality items

- **Zero-warning contract repaired for the test profile**: duplicated
  `#[test]` attribute on `else_hoist_keeps_original_polarity` (shipped in
  ms178-1.patch; `cargo test` refused to compile with `-D warnings`).
- `encode_cas` suffix parsing switched from `contains` probes to an exact
  grammar shared with `encode_casp` — `casq`/`casx` now error instead of
  silently assembling as relaxed CAS. All twelve valid CAS forms pinned
  byte-exact (Capstone cross-check).
- CASP even-start/consecutive/uniform-width validation is architectural,
  not cosmetic: the encoding has no fields for Xs+1/Xt+1, so accepting
  `caspal x0, x2, x4, x6` would silently change which registers
  participate in the atomic operation.

## 5. Verification state

- Unit suite: **1262 passed / 0 failed / 6 ignored** (12 new: CASP exact
  words ×2 + suffix rejection, CAS family drift pins, `.org` padding/
  errors/absolute-arith, global-branch triage, MOVW tramp_alias/external/
  overflow×2, CASP pipeline guard).
- Regression suite: **476 PASS / 0 FAIL / 0 AB-diff** (was 474/0/0; two new
  tests: `i686_double_param_high_word`, `ld_struct_va_arg_align`; the i686
  test's GCC oracle comparison skips in this sandbox for lack of a static
  32-bit libm — GCC builds it on any multilib host).
- fastbuild build: zero warnings (`-D warnings` enforced).
- Boot gate: SKIP (no kernel tree in this sandbox).
- toolchain: rustup 1.98.0 per `rust-toolchain.toml`; sandbox constraints:
  2 vCPU / 4 GB, no qemu-aarch64/riscv64, no 32-bit syscalls under seccomp
  (i686 freestanding tests verified via int3/ud2 death-signal verdicts).

## 6. Next-session entry points (priority order)

1. **fix_dash (riscv)**: reproduce under qemu-user with a riscv64 sysroot;
   the dash sources are downloaded/configured in the session notes — the
   failure mode (compile vs runtime) is still unknown.
2. **P17 second harvest** (recursive body walk, ~10 nested-diamond sites)
   and **P18 generalisation** (session-89 §6 items 3–5 unchanged).
3. **CCC_PEEPHOLE_SKIP plumbing for the i686 text pipeline** (A/B harness
   parity with x86-64).
4. **x86-64 callee-side named struct{long double} test**: the shared-loop
   padding now aligns named over-aligned stack params on the callee to
   match the caller; a dedicated hosted probe with GCC cross-check would
   pin it independently of `ld_struct_va_arg_align` (which covers the
   variadic path).
5. Pre-existing clippy lints across `elf_writer_common.rs`,
   `simplify.rs`, arm encoder files (cosmetic; build is warning-free).

## 7. Snapshot ledger

`S04-crossbackend-abi` — base `2b53a6ec`, deliverable ms178-2.patch
APPLIES-CLEAN (validated with `git apply --check` on a pristine base
worktree), zero garbage (no eprintln!/dbg!/TRACE/env-gated probes added;
the pre-existing `resolve_pending_instructions` warning path untouched).
