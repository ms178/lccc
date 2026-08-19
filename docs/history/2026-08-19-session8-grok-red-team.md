# 2026-08-19 (session 8) — Grok regalloc/liveness/generation red-team + fixes

**Base:** `origin/main` @ `31b8e404` (user's "Update generation.rs" v2; the three
Grok-4.6 commits `d58c4de1` regalloc, `8cc40a6c` liveness, `cc834af1`/`31b8e404` generation)
**Snapshot:** `/home/user/ms178-1.patch`

## The three Grok commits did not build and miscompiled

Every defect below is fixed, committed, and validated:
908 unit / 50 correctness / 336 regression (6 pre-existing GCC-oracle mismatches
unchanged) / 18 i686 ABI / warning-free `-D warnings` build.

### Compile errors fixed
1. `generation.rs`: `AtomicInc { ptr.0 }` — ptr is `Operand`, not `Value` (2 sites).
2. `regalloc.rs`: `evict_group` E0716 (`&[vid]` temp) + E0308 (`Vec<u32>` deref).
3. `regalloc.rs`: `base_ok` closure E0502 (captured `assignments` across a later
   mutation) — now takes `&assignments` as a parameter.
4. `liveness.rs` tests: `CallInfo` struct-literal used removed fields
   (`param_types`/`calling_conv`/`attributes`).
5. `generation.rs`: `Cast { .. }` match arm missing `..` (E0027).

### Miscompiles fixed (correctness is the hard constraint)
1. **Call-argument home clobber** (`regalloc.rs` + `RegAllocConfig::call_arg_regs`):
   call results were homed in ABI arg registers overwritten by the next call's
   argument staging. `printf("%d %d", add(3,4), mul(3,4))` → `7 4203888`.
   Fix: Phase 2 splits into four waves by argument position (indirect>=1,
   indirect arg0, direct>=1, rest), most-constrained first, occupancy seeded
   forward (`LinearScanAllocator::run_with_seed`).
2. **Indirect-call target register** (`indirect_target_regs`): `%r10` holds the
   callee address until `call *%r10`; a value homed in `%r10` was passed the
   callee address as its own argument (pgo_sections devirt). NOTE: the x86-64
   regalloc numbering is historically swapped in `phys_reg_name()` —
   `PhysReg(10)` is *named* "r11" and `PhysReg(11)` is "r10".
3. **Intrinsic vector-peephole clobber** (`generation.rs`): `clobber_after_call_like()`
   after every Intrinsic cleared `direct_fp_result` right after
   `FixedDistance*`/`VecHorizontalAdd*` set it → SLP distance kernels returned 0.
   Intrinsics are *producers* of that state; now only `reg_cache.invalidate_all()`.
4. **Stack-alignment segfault (latent)** (`x86 prologue/emit`): a frame-less
   function with calls and `-fomit-frame-pointer` emitted no 8-byte pad, so
   `%rsp ≡ 8 (mod 16)` at the call → `movaps` in printf faults
   (float_special/main). `aligned_frame_size_impl` now returns 8 for
   `raw_space <= 0 && func_has_calls`; `func_has_calls` is computed in
   `calculate_stack_space_impl`. ARM/RISC-V are unaffected (no return-address
   misalignment); i686 always accounts for the return address.
5. **Immediately-consumed real_use** (`regalloc.rs`): the rework replaced the
   Copy-web backward fixpoint with a single-source `copy_src_of: dest → src` map
   that strands the pre-loop `VecZero*` producer of a loop-carried accumulator
   (p17/p18 dot products spilled the zero to the stack and reloaded it each
   iteration). Restored the fixpoint over ALL Copy instructions.
6. **`cheap_remat` GlobalAddr exclusion** (`regalloc.rs`): removed. Codegen has
   no rematerialisation path, so excluding call-spanning `GlobalAddr`/`LabelAddr`
   from callee-saved allocation just spilled them (nbody `bodies` +258 bytes).

## Code size (tests/benchmark/programs, -O2 text, 28 files, vs pre-Grok base)

net **+130 bytes (+0.53%)**. Wins: matmul −54, sqlite_varint −78, fannkuch −66,
expat −28, binary_trees −24, spectral_norm −22. Losses: nbody +258,
glibc_memcmp +83, zlib_ng_adler32 +72 (Grok register-choice deltas on
call/FP-heavy multi-block functions; below the 1% investigate threshold).

## RESEARCH SPIKE — the Grok "segments" claim (measured, NOT landed)

Claim: scan `liveness.segments` instead of `liveness.intervals` in
`collect_gpr_scan_intervals` / Phase 2/2c; leave `iv_map` on fat intervals.

Measurement (gzip-1.14 deflate.c, `longest_match`):
- rsp-refs: pre-Grok 120, current 138; `deflate.o` text 6443 vs 6359.
- `longest_match`: 162 values, **48 multi-segment** (holes) — the packing
  opportunity is real.

Prototype result: a hole-aware `call_spanning` set (a call in a *gap* does not
force a callee-saved register) gave **−188 bytes net** and passed unit +
correctness, but produced **two real miscompiles** (`sqlite_yy_shift`
nondeterministic garbage, `os_postinline_size_policy`) and was REVERTED.

Why the naive version is unsound (write-up for the next attempt):
1. **Segment-boundary calls**: a loop re-entry segment starts exactly at the
   call that re-clobbers the register (the `.Lstr0` format string used every
   iteration). The check must be `seg.start <= cp < seg.end` (inclusive left),
   NOT `seg.start < cp`.
2. **Merged (coalesced) intervals**: `collect_gpr_scan_intervals` returns the
   *merged* leader interval for a copy-coalesce group; a `value_id`-keyed set
   built from the original segments under-approximates call-spanning for the
   leader (the union may contain a call the leader's own segments do not).
   Any segment-based check must key on the merged union, not the raw segments.
3. **The full packing win** (register reuse *inside* a hole) requires the
   linear scan to force same-value segments onto one register (LLVM live-range
   splitting); the current `LinearScanAllocator` allocates one register per
   value_id and has no move insertion. This is a larger, separate change.

Recommended next attempt order: (a) hole-aware call-spanning keyed on merged
intervals with inclusive-left boundaries, gated by `CCC_SEGMENT_SCAN`, measured
against gzip/zlib-ng/expat + the full suites; (b) only then the multi-segment
packing allocator.

## Remaining levers (unchanged, see docs/history/2026-08-18-session7…)

- Realmode/kernel 32 KiB boot gate: still ~4 KB over (text budget).
- %eax-reserved i686 accumulator + param capture to ABI homes.
- nbody/memcmp/adler32 register-choice tuning (Grok Phase 2c/priority deltas).

## Environment protocol (harness wipes /opt, swap, and /tmp between turns)

1. Rust + swap vanish on reset: `curl sh.rustup.rs | sh -s -- -y --profile minimal`
   into /opt; `fallocate -l 8G /swapfile && mkswap && swapon`.
2. `apt-get install -y libc6-dev-i386 gcc-multilib` (32-bit headers vanish too).
3. Baseline pre-Grok worktree for code-size diffs: `git worktree add ws-baseline d36ad727`.
4. Code-size oracle: `tests/benchmark/programs/*.c` at -O2 (28 files).
