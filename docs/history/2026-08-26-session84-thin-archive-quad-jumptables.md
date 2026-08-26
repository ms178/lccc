# Session 84 — Thin-Archive `/NNN` Refs, `.quad` Jump Tables, and Harness-Wipe Recovery

Date: 2026-08-26
Base: upstream `ms178/lccc` main @ `c113069e` (PR #244 merged during the session)
Head: `deb8b895` (S11)
Snapshots: **S09** (ar strtol refs), **S10** (identical-blocks jump-table protection), **S11** (identical-blocks function boundaries)

---

## 1. Accomplished

### 1.1 Kernel failure #5 — thin-archive `/NNN` extended-name refs (S09)

**Symptom.** The `vmlinux.o` link died with
`failed to read member '/4611          /'` from `vmlinux.a`
(a `!<thin>` archive of 1007 members).

**Root cause.** GNU ar writes long-name references as `/offset` (decimal
offset into the `//` string table), normally `/4635       ` (space padded).
The thin-archive rewrite (`ar mPi --thin`, driven by the kernel's
`scripts/Makefile.vmlinux_a`) can leave the name field space-padded *after*
the terminator slash: `/4611          /`. BFD reads all of these via plain
`strtol` semantics (`get_extended_arelt_filename`): leading decimal digits,
stop at the first non-digit. lccc's `parse_long_name_offset` required digits
to the end of the field (modulo one trailing `/`), so exactly the 52
slash-suffixed refs in the kernel archive failed.

**Fix.** `src/backend/elf/archive.rs::parse_long_name_offset` now parses the
leading decimal prefix (checked arithmetic, `None` when no digit leads).
This also covers `/NNN:origin` compounds from binutils thin rewrites.
Regression tests: `parse_long_name_offset_strtol_prefix_semantics` and
`parse_vmlinux_a_and_binutils_thin_name_ref` (exact 16-byte field shapes,
full thin-archive round trip). Validated against a patched control corpus
(all five member name fields slash-suffixed → link rc=0) and, at the unit
level, 1181 passing tests.

### 1.2 Kernel failure #6 — `.quad` jump-table targets merged away (S10)

**Symptom.** After #5, the vmlinux link died with
`undefined symbols: <section 0 '' in vmlinux.a(arch/x86/entry/syscall_64.o)>`
plus 10 more objects.

**Root cause (two real bugs in `identical_blocks.rs`).**

1. **Jump-table targets unprotected on x86-64.** The pass excludes
   jump-table target blocks from merging, but only collected labels from
   `.long .LBBn - .Ljt_n` entries (the i686 REL form). x86-64 absolute
   tables emit `.quad .LBBn`, which were not scanned. Linux 6.18
   `syscall_64.c` has 104 byte-identical
   `case nr: return __x64_sys_ni_syscall(regs);` blocks; the pass merged
   98 of them, leaving 98 `.quad` entries pointing at removed labels. The
   ELF writer resolved the dangling relocations to **symbol index 0**
   (empty name), which lccc-ld reports as `<section 0 ''>` — an
   unreadable error for a codegen defect.
2. **Short blocks never registered.** The `block_hashes` push was nested
   inside the `instr_count >= 4` hash refinement, so blocks with fewer
   than 4 instructions could never be merge candidates at all (the
   refinement's only purpose is collision resistance). Pre-existing bug,
   found while writing the control test for fix #1.

**Fix.** Scan both `.long ` and `.quad ` lines for `.LBB` jump-table
targets; move the hash-table push out of the refinement branch.
Regression tests: `quad_jump_table_targets_are_not_merged` (kernel shape)
and `non_jump_table_blocks_still_merge` (control that the pass still
fires).

**Validation.** `syscall_64.o` rebuilt with the fix: relocs 1350 → 1449,
**0 sym0 relocations**, objtool rc=0. Full suite: **1183 passed, 0 failed**.

### 1.3 Kernel failure #7 — cross-function block merging (S11)

**Symptom.** The vmlinux link succeeded, then modpost died:
`update_srbds_msr+0x2d (section: .text) -> mds_apply_mitigation
(section: .init.text)` — a reference the source does not contain.

**Root cause.** lccc emits no `.cfi_startproc` under
`-fno-asynchronous-unwind-tables` (kernel default), and the
`identical_blocks` func_id grouping was keyed on CFI directives. Every
block in an object therefore shared func_id 0, and byte-identical
epilogue blocks merged **across functions**: update_srbds_msr's
early-return block was redirected into mds_apply_mitigation's epilogue
in .init.text — a wrong cross-section jump at runtime and a modpost
mismatch at build time. Any same-section cross-function merge would
have been silent; this one happened to cross an init boundary.

**Fix.** Function identity now comes from global (non-`.L`) labels in
the block-collection scan (CFI bumps remain as an additional signal).
Regression test `identical_blocks_never_merge_across_functions_without_cfi`
verified red-green (old grouping: 1 survivor instead of 2; new: 2).

**Validation.** bugs.o recompiled: branch now local (`je 47b`), 0
relocations against `.init.text`/`.text` sections, objtool rc=0. Full
suite: 1184 passed. **All 1027 kernel objects were recompiled from
scratch afterwards** — the old binary could have left silent
same-section cross-function merges in every object.

### 1.4 Harness-wipe recovery

Mid-session the harness wiped the sandbox: `.git` metadata, most of the
kernel tree (all source dirs), `kpkg/archpkgbuilds` content, the Rust
toolchain, clang/mold and several apt packages were lost. Recovered:

- **lccc:** fresh clone of `ms178/lccc`, rebased onto the *new* upstream
  main `c113069e` (PR #244 merged meanwhile), `ms178-1.patch` (S08)
  re-applied with `git apply` (the deliverable is a plain diff, not a
  mailbox — `git am` fails on it by design), bug-#5 fix re-implemented
  from the session notes and committed.
- **Base-ref discipline:** the snapshot script reuses a recorded base if
  the commit still exists in the object store, which silently produced a
  patch containing *upstream's* intervening commits after the rebase.
  The base must be moved on every rebase (`echo <new-main> >
  artifacts/.base_ref`) and the ledger re-annotated; S09 was regenerated
  cleanly this way.
- **Kernel:** re-downloaded `linux-6.18.44.tar.xz` (cdn.kernel.org),
  re-applied the 26 PKGBUILD `source[]` patches in order, restored the
  surviving VM-minimal `.config` (714 symbols, CachyMod features on) and
  the `.lccc-vm-config` marker. `localversion.10-pkgrel = -2.1`,
  `localversion.20-pkgname = -cachymod`.
- **Toolchain:** rustup 1.98.0 (minimal + rustfmt/clippy), apt reinstall
  of qemu-system-x86, cpio, bison, flex, bc, dwarves, libelf-dev.
- **Cargo linker trap:** `.cargo/config.toml` pins `clang
  -fuse-ld=mold`; when clang/mold are absent, builds must pass
  `--config 'target.x86_64-unknown-linux-gnu.linker="gcc"'` **and**
  `RUSTFLAGS="-D warnings"` in the environment. `--config
  ...rustflags=[]` does NOT override the file's rustflags (cargo
  concatenates them), so without the env var `-fuse-ld=mold` leaks into
  the gcc driver and the link fails with `cannot find 'ld'`.
  `build_lccc_fast.sh` handles this internally; `cargo test` invocations
  must replicate it.

---

## 2. State of the kernel build

- Tree: `/home/user/kernel-work/linux-6.18.44`, VM-minimal config
  (allnoconfig + `scripts/kernel-vm.fragment`), CachyMod patches applied
  and functional (CACHY, SCHED_BORE, SCHED_CACHE, HZ_800, TCP_CONG_BBR,
  PREEMPT verified in `.config`).
- Milestone 1 build in progress at session end: 1081 objects compile with
  lccc (`CC=lccc LD=lccc-ld HOSTCC=gcc -j2`), objtool validates every
  object, then the `vmlinux` link and `bzImage`.
- Boot validation: `scripts/qemu_boot_test.sh` (busybox initramfs,
  serial-console verdict; TCG, no KVM). Expected checks: lccc banner,
  config symbols, bbr in congestion algos, CACHE_HOT_BUDDY sched feature,
  BORE in sched_debug, 2 CPUs, clean poweroff.

---

## 3. To-do for future agents

### 3.1 Kernel (immediate)

1. Finish the bzImage build (S10 binary). Any new link error is bug #7.
2. Run `qemu_boot_test.sh` → first VM boot of an lccc+lccc-ld kernel.
3. Milestone 2: the package **default config** (PKGBUILD `config` +
   `config.sh`), with `SCHED_CLASS_EXT` (needs `DEBUG_INFO_BTF` → pahole
   path) and module linking; the fragment tree must remain untouched.
4. objtool emitted "empty alternative entry" *warnings* on
   syscall_64.o — cosmetic today, but investigate whether lccc emits
   zero-size ALTERNATIVE triples anywhere (warning is objtool-level
   noise or a real quirk; do not silence).

### 3.2 lccc structural debt surfaced this session

- **Unresolved-reloc diagnostics.** The ELF writer silently writes symbol
  index 0 for any relocation whose name it cannot find; the linker then
  prints `<section 0 ''>` or an empty name. Emit a *diagnostic at write
  time* (object name + section + offset + original name) — this bug cost
  hours of archaeology. A debug-only `LCCC_DEBUG_SYM` print proved the
  write path is not even the one that produced the sym0 (the empty name
  came from a `.LBB` label never registered), but a permanent, cheap
  warning for truly-empty names is still worth it.
- **`identical_blocks` robustness.** Jump-table target collection scans
  raw text for `.LBB` tokens in `.long`/`.quad` lines. Fragile by
  construction (any other absolute data directive referencing a block
  label — e.g. `.xword` on other arches — repeats the bug). Consider
  protecting *every* label referenced from a data section, or have the
  ELF writer refuse (error, not index 0) to write relocations against
  unknown labels, turning future instances into compile-time errors.
- **Oracle/codegen quest.** `scripts/codegen_oracle.py` defaults are
  correct (gcc16.2/clang/icc/icx); the quest to beat all four on
  representative tests (benchmark corpus, zlib-ng, gzip, expat) continues.
  The `identical_blocks` short-block registration fix slightly changes
  codegen everywhere — re-run the scoreboard to confirm no regressions
  and look for newly merged blocks in the benchmark corpus.

### 3.3 Process

- **Snapshot immediately** after any validation, never defer (this
  session lost the bug-#5 fix to a wipe and had to re-implement it).
- **Move the snapshot base on every rebase** (see §1.3) and verify the
  deliverable applies against a pristine `origin/main` worktree, not just
  against the current tree.
- `/tmp` is wiped without warning; keep only reproducibles there. The
  kernel tarball is a 2-second re-download — do not cache it in the
  workspace (snapshot cap ~128 MB).
- The session doc convention (`docs/history/YYYY-MM-DD-sessionNN-*.md`)
  holds; this is session 84.

---

## 4. Errors and dead ends (do not repeat)

- Do not "normalize" the kernel archive (`ar t` → `ar rc`) instead of
  parsing it; lccc-ld must read GNU thin archives natively — it now does.
- Do not chase "member stride" theories for archive refs; the format is
  defined by BFD's `strtol` prefix semantics, full stop.
- `git am` on the deliverable fails ("Patch format detection failed") —
  it is a plain `git diff`; use `git apply`.
- In cargo, `--config target.<triple>.rustflags=[]` does not override
  `.cargo/config.toml` rustflags (concatenation); use the `RUSTFLAGS`
  env var.
- Do not run `cargo test` without the linker overrides when clang/mold
  are absent (see §1.3).
