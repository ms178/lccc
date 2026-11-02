# Follow-up: MachInst sub-word reload fix — kernel boots to init exec (S04)

Session 2026-09-17 (afternoon). Base: upstream `main` `8ad39bfe`
(+ session commit `002b146f`). Deliverable: `/home/user/ms178-1.patch`
(70,149 B, APPLIES-CLEAN vs `8ad39bfe`).

## What landed

1. **MachInst sub-word arriving-reload fix** (`machinst_alloc.rs`) —
   `arriving_reload(slot, reg, size, ty)`; U8→`movzbl`, I8→`movsbq`,
   U16→`movzwl`, I16→`movswq`; S32+ unchanged. Kills the kernel
   `vsnprintf` phantom-NUL defect (root cause analysis in
   `journal/2026-09-W3.md`, 09-17 entry).
2. **Regression tests**: `tests/regression/machinst_subword_reload_sign.c`
   (FAILS old / PASSES fixed, proven both ways by compiler revert);
   unit test `subword_arriving_reload_defines_full_register`.
3. **`prepare_kernel_tree.sh` whole-tree extraction audit** — after a
   silently-short tar extraction (97 files missing, e.g.
   `arch/x86/boot/compressed/vmlinux.lds.S`; surfaced only as a deep
   `No rule to make target` in `make bzImage`), the script now compares
   the extracted file count against the archive listing and re-extracts
   once before failing. Sentinels alone were insufficient (all cluster
   in the archive's alphabetical head).
4. Journal + STATE.md kernel-mode status section.

## Validation matrix (all green)

| gate | result |
|---|---|
| reduced repro (`-O2 -m64 -funsigned-char`, full driver) | byte-exact |
| `cargo test` | 2862 pass |
| linker suite (`run_linker_tests.py`) | 241/241 |
| `ci_local.sh --fast` (re-run after all edits) | 40/40 gates |
| kernel build (fresh `make clean`, final toolchain) | bzImage PASS |
| QEMU boot (`qemu_boot_test.sh`) | **0 NUL bytes** in serial log; boot reaches `run_init_process` at ~11.3 s |

## Milestone impact

- **Defect (a) phantom-NUL: CLOSED.** `SMBIOS 2.8`, `Memory slots
  populated: 1`, `node 0 populated` all clean; `od` scan of the entire
  serial log finds zero `\x00`.
- **Defect (c) acpi/`__device_attach` dev->p crash: CLOSED** — it was
  downstream of (a) (NUL-corrupted kobject names). This boot completes
  the full driver model init and reaches `Run /init as init process`.
- **Defect (b) CPU1 bring-up: still open** — `smp: Brought up 1 node,
  1 CPU` (secondary hotplug times out).
- **Defect (d) NEW: `execve` fails `-E2BIG`** — `Starting init: /bin/sh
  exists but couldn't execute it (error -7)`, preceded by `BUG: Bad
  rss-counter state mm:... type:MM_ANONPAGES val:1` and a
  `kernel_execve`-stack WARN (see log tail). M3's last blocker.

## Next session attack plan

### Defect (d) — exec E2BIG (highest priority; single remaining blocker for userspace)

- `-E2BIG` from `kernel_execve` with a one-page argv is bogus; suspect
  either `bprm->p` stack usage accounting in `fs/exec.c`
  (`copy_strings`/`count` miscompiled under window RA — same shape as
  (a): sub-word locals + 64-bit tests under `-funsigned-char`) or the
  `rss-counter` BUG pointing at `acct_arg_size`/`get_arg_page` mm
  accounting. Both stacks show `kernel_execve+0x11a`.
- Repro vector: build a freestanding `execve` harness exercising
  `count()`/`copy_strings()` logic with `-funsigned-char -O2`, or diff
  `fs/exec.o` disasm lccc vs GCC oracle (gcc allowed as host oracle
  only). `CCC_NO_MACHINST=1`/`CCC_MI_ALL_CLASSIC=1` kernel rebuild of
  `fs/exec.o` (+ relink) is the fast isolator if the harness doesn't
  reproduce.
- The `Bad rss-counter` at `MM_ANONPAGES val:1` right at exec suggests
  a page is faulted/anon-counted that shouldn't be — cross-check
  `__bprm_mm_init` gate logic.
- Serial-log tail (evidence): `/tmp/qemu-boot.log` lines ~600–665
  (kernel_execve+0x11a, run_init_process+0xa7, rss-counter BUG,
  error -7, panic).

### Defect (b) — CPU1 bring-up

- Secondary CPU start uses 16-bit trampoline + INIT-SIPI-SIPI;
  bring-up hang with `do_boot_cpu` timeout. Candidate vectors: trampoline
  header layout (already has linker coverage), `cpu_init` register
  state, or `smp_callin` loop — compare `arch/x86/kernel/cpu/common.o`
  and `smpboot.o` vs oracle. `maxcpus=1` boots fine (this session's
  evidence); try `nosmp`-vs-`maxcpus=2` in qemu_boot_test.sh knobs.

### Hygiene

- The S02/S03 linker fix is still NOT upstream — only in this patch /
  snapshots. Upstream it as a PR (emit_script.rs post-layout phdr
  count + link.rs + lccc_ld.rs + `script_vdso_declared_note_phdrs`)
  so a future `git pull` can't strand it again. Same for the kernel
  harness scripts and the machinst fix (separate PRs).
- Re-run `ci_local.sh` FULL (non-fast) once on a quiet machine.
- Kernel harness scripts carry the `.cmd`-relink guard: after any
  lccc/lccc-ld rebuild, kernel `make` reuses stale link artifacts —
  the guard deletes them; keep it in sync with build_kernel_vm.sh.

## Environment notes

- Swap 8 GiB active (`/swapfile`, UUID 2d491971); fastbuild preset
  (`CARGO_BUILD_JOBS=2`, `-O1`); swap + patience for full kernel
  rebuilds (~35 min from clean, ~45 s resumed).
- QEMU: `QEMU_DATA_DIR=/home/user/kernel-work/qemu-fw`; serial log at
  `/tmp/qemu-boot.log` (harness-writable, /tmp may be wiped).
- Kernel tree integrity is now audited by `prepare_kernel_tree.sh`;
  if `bzImage` ever fails with `No rule to make target
  arch/x86/boot/compressed/vmlinux.lds`, re-run
  `prepare_kernel_tree.sh` (it will restore from the tarball).
