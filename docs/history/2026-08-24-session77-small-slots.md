# Session follow-up: width-partitioned 4-byte spill slots (x86-64)

Session base: `7bd693060a50c1d28eb268fcf2a6f6e3c3529277` (latest main, 2026-08-24)
Session head: `78a8d9167c90f0d9ed2f84ac20d8128269f1430e`

## Accomplished

1. **pcre2 stack-overflow fixed (the headline).** Values whose IR type is
   ≤ 4 bytes now get 4-byte spill slots on x86-64. pcre2's `compile_branch`
   (1290 simultaneously-live 32-bit values across a call) shrinks from a
   10320-byte frame to 5128; the 792-level recursion of test 8 no longer
   overflows the 8 MiB stack — an 850-level recursion completes. GCC frame
   for the same shape: 616 bytes (remaining gap = 5128 − 616 is dominated by
   the spill-everything codegen model, not slot width).

2. **Three soundness classes found and fixed while enabling small slots.**
   Each is pinned by a regression test (`small_slot_width_partition.c`)
   and by comments at the hazard site:
   - MachInst slot-to-slot Copy relays were unconditional 64-bit
     (`lower_copy`), reading stale neighbour bytes out of a small source
     slot and clobbering the neighbour of a small destination slot
     (−O0 loop-phi miscompile, `o0_phi_multidef`).
   - Tier-3 pool offsets could sit at 4-mod-8 while the deferred-region
     base was not forced 8-aligned; finalize-time alignment rounding then
     *shifted an 8-byte slot onto the bytes of a following 4-byte slot*
     (rot() v11/v14 [40,48) vs [44,48) overlap).
   - Tier-3 greedy expiry is definition-order-relative: a size-descending
     reorder (tried for packing) handed a still-live occupant's slot to a
     later value (zlib-ng `build_tree` v89/v319 → SEGV). Processing order
     is now documented as a soundness invariant.

3. **Latent F32 spill bug fixed for its own sake:** XMM→slot Copy always
   used `movsd` (8 bytes); F32 now uses `movss`. With 8-byte slots this was
   unobservable; with small slots it would corrupt the neighbour.

4. **Validation battery (all green):**
   - 431/431 regression suite — GCC oracle comparison *plus* a new A/B
     differential (every test re-run with `CCC_NO_SMALL_SLOTS=1` and
     required to produce identical output; zero diffs).
   - 50/50 correctness suite, 1167/1167 `cargo test --lib`.
   - zlib-ng 2.3.3 pinned gate: minigzip roundtrips at levels 1/2/6/9
     (self-roundtrip + system-`gzip -t` validation) all pass.

5. **Support tooling landed:** `scripts/run_regression_suite.sh` (oracle +
   A/B differential runner), `scripts/mixed_width_slots.py` (per-function
   stack-slot width-consistency auditor), `scripts/memwatch.sh` (userspace
   OOM watchdog for swapless containers), zlib-ng gate `LCCC_SWAP_WAIVER`
   for no-CAP_SYS_ADMIN sandboxes, and the mold-through-gcc-driver fallback
   (`build_lccc_fast.sh`) for boxes without clang.

## Discovered, NOT yet fixed — highest-priority next work

**zlib-ng inflate miscompile/hang (PRE-EXISTING UPSTREAM).**
`CVE-2018-25032-fixed-level-1/-6` ctest cases hang (50 s timeouts) and
`CVE-2018-25032-default-level-6` fails fast. Proven pre-existing: reproduced
identically with the unmodified upstream compiler (changes stashed), and
with `CCC_NO_SMALL_SLOTS=1`. Isolated reproduction (zlib-ng 2.3.3 build dir):

- `./minideflate -c -k -m 1 -w -15 -s 4 -F -1 <fixed.txt> > out.gz` —
  compress exits 0 (16943-byte stream).
- `./minideflate -c -k -d -m 1 -w -15 -1 out.gz` — **infinite loop**.
- `./minideflate -c -k -d -m 1 -w -15 -1 <system-gzip stream>` — **exit 1**,
  so the lccc-compiled *decompressor* is broken even on a valid external
  stream: the defect is in inflate, not in deflate's output.

Suggested attack: gdb the hang (attach, `bt`, find the spinning loop in
inffast/inflate), diff the assembly against GCC for that function, then
bisect passes with the existing env knobs. The gzip-stream failure (exit 1)
is the easier entry point: a wrong early error return, likely in the header
or `inflate()` state machine around `state->mode` transitions.

## Open bug queue (from current_tasks/, unchanged this session)

AArch64 assembler: caspal instruction, global branch relocs, `.org`
directive, PREL64 relocation, movw `:abs_g*:` relocations. RISC-V: dash
failure, va_arg `struct{long double}`. i686: double-param high-word store.
The pcre2 stack-frame-bloat item is now FIXED (this session).

## Technical debt / observations

- The 5128-vs-616 frame gap vs GCC is the spill-everything model: lccc
  materializes ~all SSA values to slots at −O1. Register allocation
  coverage (fewer slot-homed values) is the lever, not further slot
  shrinking.
- Small slots are x86-64 only for now (`set_target_small_slots`); i686,
  AArch64 and RISC-V keep 8-byte slots until their store/load paths get the
  same width audit the x86-64 ones received here. `mixed_width_slots.py`
  is the tool for that audit.
- The harness wiped the workspace between sessions (as documented); the
  full toolchain + repo restore from scratch takes ~10 min. Everything
  needed is in this patch plus `rustup` + micromamba + qemu-user-static.
