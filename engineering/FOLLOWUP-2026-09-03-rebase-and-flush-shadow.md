# 2026-09-03 — Rebase onto upstream `6fcbb1d2` + `flush_machinst` shadow fix

Session goal (user): deliverable `ms178-1.patch` re-based onto the latest
`ms178/lccc` main (it had moved), full battery of tests green, workspace
cleaned, and all remaining issues either fixed or explicitly deferred.

## 1. Environment was restored mid-stream (harness wipe)

Between turns the harness restored `/home/user` from the S02 snapshot:
- `lccc/.git`, `lccc/target/`, `/opt` rust toolchain, `/swapfile`, `/tmp`
  (QEMU harness, initramfs, zstd oracle inputs), and the ~full 6.18.47 kernel
  tree were all gone (kernel-work now holds only a ~1.7 MB stub).
- What survived: `lccc/` source (partial — `src/` held only 3 files),
  `artifacts/lccc.bundle` (full git history, `main` = `df5b9134`),
  `artifacts/*.patch`, `archpkgbuilds/packages/linux-cachymod-6.18/`
  (config + config.sh + all PKGBUILD patches), `ms178-1.patch` (S02).

Recovery performed and now repeatable:
1. `git clone /home/user/artifacts/lccc.bundle /home/user/lccc`, checkout
   `df5b9134`, `remote add origin https://github.com/ms178/lccc.git`.
2. 12 GiB `/swapfile` created and activated (user requirement supersedes the
   repo script's 8 GiB default).
3. `scripts/arena_session_restore.sh` reinstalled rustup/Cargo 1.98.0 into
   the persisted `/home/user/.cargo`, apt deps, exec bits, and built a warm
   `fastbuild` baseline. **This script is the one-command recovery entry
   point for every future session after a wipe.**

## 2. Compiler fix carried into the rebased tree (NEW this session)

Bug: `X86Codegen::flush_machinst()` early-returns when `machinst_buf` is
empty but leaves the IR shadow `machinst_buf_ir` populated by no-op
lowerings (a coalesced `Copy` whose src and dest resolve to the same home
emits zero `MachInst`s yet is recorded). A later flush — possibly in a
*different function* after a block/function boundary — then replays stale IR
whose values have no register homes there, tripping the `operand_to_rax`
hard gate ICE.

Observed in the lccc kernel build at `drivers/tty/tty_io.c`:
`tty_get_tiocm`'s tail `Copy v19 = v12` (a coalesced no-op) leaked across the
function boundary and was replayed inside `tty_tiocmget`, where `v12` has no
home → `ccc: internal error: x86 codegen: operand_to_rax: value 12 ... has no
register home, no stack slot and no acc-cache entry`.

Fix (commit `81b31419`): clear `machinst_buf_ir` in the no-op early return.
A no-op window needs no replay — the values already sit at their
destinations.

Verification: full `cargo test --profile fastbuild -j2` on the rebased tree +
fix → **1597 passed / 0 failed / 6 ignored** (upstream #356–#358 added 42
tests vs the previous 1555).

## 3. Rebase onto upstream `6fcbb1d2` (was `4280c6c6`)

Upstream main moved through three PR merges:
- #356 `4ef1be6c` x86 machinst: typed direct calls (CallTyped) + xmm0/xmm1
  scratch relays for slot-homed floats.
- #357 `933ba320` x86/i686: FMA memory-operand folding, i686 encoder
  gas-parity fixes, unroll overflow bail.
- #358 `bc72818a` stack_layout: size slots from emitter width; disable
  unsound Tier-2 sharing. **This is upstream's own fix for the same preboot
  zstd "corrupt" symptom we fixed in `ac28351e`** — at the allocator layer
  (never give a 4-byte slot to a value whose materialisation width is >4 B),
  whereas our fix is a defensive MachInst Copy-width gate (only emit a narrow
  relay when both src and dest are certified small). They are complementary
  and coexist; no semantic conflict.

Rebase result (`git rebase --onto 6fcbb1d2 4280c6c6 main`):
- commit 1/3 (`d3bd587e` blocklayout): only conflict was
  `scripts/bisect_boot_size.sh` (add/add) — upstream's newer richer version
  (real `_end <= 0x8000` gate) superseded our primitive variant; kept
  upstream's, dropped ours. `src/passes/block_layout.rs` +
  `src/driver/pipeline.rs` merged cleanly.
- commits 2/3 (mi-diag-knobs, `prologue.rs`) and 3/3 (copy-width,
  `isel.rs`+`emit.rs`) applied cleanly against upstream's own changes to the
  same files.

New base recorded in `/home/user/artifacts/.base_ref` and tracked
`lccc/.base_ref`: `6fcbb1d2`.

## 4. Deliverable regenerated (S03)

`scripts/lccc-snapshot.sh "machinst-flush-shadow" "…"` →
- `/home/user/ms178-1.patch` (15 186 B, **APPLIES-CLEAN** vs `6fcbb1d2`)
- `/home/user/artifacts/ms178-1.S03-machinst-flush-shadow.patch`, refreshed
  `lccc-src.tar.gz`, `lccc.bundle`, `SNAPSHOT_LEDGER.md`, per-commit
  `artifacts/series/`.
- Commit chain: `6fcbb1d2` (upstream) → `ef1105c5` blocklayout → `fffe674a`
  mi-diag-knobs → `ac28351e` copy-width → `81b31419` flush-shadow →
  `6b5c0983` base_ref.

## 5. Open items — DEFERRED to next session (kernel compile+boot acceptance)

Not finished this session; a full lccc kernel boot under QEMU has still never
been observed. Handoff for the next agent:

1. **Kernel source is gone** (only a ~1.7 MB stub remains). Re-download
   `https://cdn.kernel.org/pub/linux/kernel/v6.x/linux-6.18.47.tar.xz`,
   extract to `/home/user/kernel-work/linux-6.18.47`, then re-apply the
   kernel-cachymod patch set exactly per the PKGBUILD order (source array in
   `archpkgbuilds/packages/linux-cachymod-6.18/PKGBUILD`; the package default
   flow is `eevdf`/`gcc`/native/800 Hz/full tickless+preempt/THP
   always/BBR3; the canonical `config` in the package dir is clang-LTO, so a
   GCC build must run the `scripts/config` sequence + `config.sh` as recorded
   in the previous session — resulting .config must carry `CC_VERSION_TEXT`
   = lccc, `LTO_NONE`, `CC_OPTIMIZE_FOR_PERFORMANCE` (O2), VMAP_STACK=y,
   INIT_STACK_NONE=y, HZ=800, SCHED_BORE=y, CACHY=y).
2. **Assembler gap**: lccc's integrated assembler rejects
   `lib/crypto/x86/chacha-avx512vl-x86_64.S` with `ccc: error: bad dst
   register` (AVX-512EVL file compiled when `CONFIG_CRYPTO_CHACHA20_X86_64`
   is set; the canonical config enables it). Diagnose the exact EVEX/mask
   operand that trips `src/backend/…` assembly encoder or route such `.S`
   files through an external assembler. Likely more AVX-512 `.S` files in
   `arch/x86/crypto` and `lib/crypto/x86` will fail the same way — worth a
   sweep of every `.S` compiled by the config.
3. **Real-mode `-m16` boot hang**: a fully-lccc bzImage produced zero output
   after "Booting from ROM…" while the same payload with a gcc-compiled
   real-mode setup booted to `Run /init`. Hypothesis is an lccc `-m16`
   codegen bug (frame layout uses `push %ebp` + `subl $244,%esp`, call-arg
   windows on `%esp`, etc.). Decisive experiment for next time: after a full
   clean lccc build boots-or-hangs, rebuild ONLY `arch/x86/boot/*` with gcc
   and re-concatenate `setup.bin` + the lccc `vmlinux.bin` (the bzImage rule
   is a plain concat of 4K-padded `setup.bin` and `vmlinux.bin`), then boot.
4. **Zstd oracle inputs are gone** (`/tmp/zstd-oracle`, 8 MB slice). Regenerate
   via `scripts/zstd_preboot_oracle.sh` (needs the kernel tree) and re-run
   `scripts/zstd_oracle_cc.sh` FULLMI/CLASSIC/FORCE on the final binary.
5. **Regression test**: add a codegen-level test reproducing the
   cross-function stale MachInst-IR-shadow replay (coalesced copy at a
   function tail followed by an unresolvable copy in the next function).
   `machinst_tests.rs` unit tests cannot reach `flush_machinst` (stateful);
   needs the full-TU C harness used by `tests/regression/*.c`.
6. `.config` stability: the previous default-config `.config` kept drifting
   to kbuild defaults (`(NEW)` symbols auto-resolved). Use the canonical
   config regeneration flow above and save the resulting `.config` into the
   package dir before building.

## 6. Workspace hygiene performed

- Deleted pre-wipe `/home/user/lccc-old` (933 MB) and `/tmp` scratch
  (arena-workspace, IR dumps, disassemblies) — 2.1 GB freed, then the wipe
  reset the disk entirely; currently ~20 GB free after the recovery rebuild.
- Kept required files: `archpkgbuilds`, `artifacts`, `lccc`, kernel stub,
  `ms178-1.patch`.
