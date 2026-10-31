# Linux 6.18.58 defconfig: compiler/linker investigation

## Scope and provenance

This is a separate run from `2026-10-30-linux-6.18.58-rebased.md`.
The starting upstream LCCC main was `9bc081312ec63a2c3896604f0e4039a00140f7df`;
`ms178/archpkgbuilds` main was `8000af9d8b8b5f6c6ebbe4a03f6d7fde5f2f9451`.
Its linux-cachymod PKGBUILD specifies 6.18.58 (already the LCCC scripts'
central fallback), and its archive's SHA-256 was verified as
`069b19f9032c4dac8149ae4b6520c35debca0f42a9f6d52ee55ba6ca9b818013`.
The preparation script applied 29 listed patches in package order; generated
kernel headers were built with LCCC. Host tools alone used system GCC 14.2.

Host: virtualized Intel Xeon model 106, 2 vCPUs, 1.9 GiB RAM, kernel
`6.1.158+`, QEMU 10.0.13 (TCG). Two 4-GiB swapfiles were enabled on the
largest available disk. Rust 1.99.1, cargo, rustfmt and clippy were installed;
all LCCC rebuilds used `scripts/build_lccc_fast.sh` (`-O1`, `-j2`). Restored
pinned local GCC 16.2, Clang 23.1.4 and binutils 2.47 oracle tools; no kernel
translation unit or boot code was compiled with an oracle compiler. GNU ld was
used only as a **link** oracle for the same LCCC-produced boot objects.

## Boot-size gate

The package-config setup-only build succeeded before the defconfig run:
`_end=31088` bytes, 1680 bytes below 32768, 24 object ISA audit passed and
GNU ld's flat setup image matched `lccc-ld` byte-for-byte. The setup harness
uses generated offset *stubs*: this result neither proves the final bzImage
uses real compressed-image offsets nor that it boots. Re-run on the final
compiler and target config. The boot log SHA-256 from this run is
`57aa6348b74e46752a46d815154d83dd7cfa638126e595783ef568da1035430c`.

## Fixes and evidence

1. **Setup-only offset isolation.** Two boot probes replaced a real Kbuild
   `zoffset.h` with stubs. A later incremental build could reuse the newer
   fake offsets. Copy `header.S` and deterministic stubs into a private
   temporary directory instead; never mutate Kbuild offsets. A regression
   verifies file hashes and actual quoted-include lookup; it is called from
   the existing local/hosted boot-size gate. This must remain safe on kill:
   the temporary directory may be left behind, but real offsets are untouched.
2. **`--no-warn-rwx-segments`.** LCCC currently emits no RWX LOAD-segment
   warnings, so an explicit request to disable them is already honored. Its
   previous "not implemented" warning became fatal under the `defconfig`
   Kbuild link's `--fatal-warnings`, blocking `realmode.elf`. Test both option
   orders, output identity, and rejection of an explicit unsupported warning
   enable request; the real `arch/x86/realmode/` Kbuild target passed with
   LCCC/lccc-ld. The test is registered in the linker suite. **Remaining:**
   implement genuine RWX warning support and ordered enable/disable behavior;
   this fix must not be represented as implementing those diagnostics.
3. **Callback return type under shadowing.** The `up_f up` parameter in
   `kernel/time/timer_migration.c` shadows global `void up(...)`; sema instead
   selected the global return type and falsely rejected `if (up(...))`.
   Resolve the visible lexical symbol before the translation-unit function
   map. The new regression exercises boolean, floating, local and dereferenced
   callback calls; GCC differential and existing shadowing/builtin/noreturn
   tests passed. The real kernel `timer_migration.o` then built with LCCC.
4. **x86 `. = expression` and final `.org` validity.**
   `arch/x86/kernel/relocate_kernel_64.S` uses this assignment for 16 fixed
   six-byte exception-vector entries. It now reaches existing `.org` layout
   machinery; absolute as well as label targets are checked after branch
   relaxation, so a genuinely backwards seek fails rather than silently
   corrupting the table. Dot-relative negative targets and out-of-range
   offsets fail closed. The actual LCCC-generated `relocate_kernel_64.o`
   built; `nm`/`objdump` showed `kexec_debug_exc_vectors` size 96 and
   six-byte slots. Synthetic 32/64-bit vectors and invalid targets matched
   GAS 2.47; the expanded cross-target NOBITS/`.org` gate passed 149/149,
   including its eight documented pre-existing divergences. The synthetic
   oracle test is invoked by that existing CI gate.

## Oracle screening (not runtime performance)

`engineering/evidence/kernel-2026-10-30/codegen-workloads.{json,md}` records
an `-O2` Compiler Explorer survey of existing gzip, zlib-ng and Expat
benchmark files using LCCC, GCC 16.2, Clang 23.1.0, ICC 2021.10 and ICX
latest. These are whole-translation-unit static instruction counts, not
cycles, hot-loop counts, benchmark correctness, hardware counter data or
Raptor Lake performance. Gzip's LCCC count was 410 vs ICC's 293; zlib-ng
235 vs ICX 294; Expat 206 vs GCC 175. They are leads, not proof of wins.
The initial `-march=x86-64` run correctly rejected ICC's unsupported option;
`-O2` was used for all compilers in the recorded run instead.

## Open items and continuation

- A pristine x86-64 `defconfig` (not the minimal VM fragment) was selected
  and full `bzImage` attempted. Rebuild from clean objects after any compiler
  change: Kbuild fingerprints command text, not binaries. Successful focused
  objects are not evidence of a completed image or successful QEMU boot.
  Use `CC=lccc LD=lccc-ld AS=lccc HOSTCC=gcc`, `-j2`, real objtool, and
  preserve each failure log. Do not compile kernel code with GCC.
- `defconfig` enables IBT, stack protection and FTRACE. LCCC reports numerous
  accepted-but-unimplemented kernel options including
  `-fno-delete-null-pointer-checks` and `-fno-strict-overflow`; audit their
  actual code-generation semantics, not just whether the build finishes.
  In particular signed/pointer wrapping and null-related transforms are
  correctness-sensitive; do not silently claim compatibility.
- A QEMU result must use the **actual** final offsets and a suitable initramfs,
  verify console and end-of-test/poweroff, and state which config and features
  were checked. The earlier VM-fragment success is not a defconfig success.
- `ci_local.sh --fast` must be rerun to completion on the final tree; a
  focused pass or an ungated snapshot is not a green CI result. Confirm disk
  headroom before Clippy. The artifacts ledger explicitly labels ungated saves.
- The original task also asks for ICX installation, mold preference checks,
  benchmark expansion, QEMU boot and real-hardware performance. No completion
  or performance supremacy claim for those is made by this session record.
- The tracked source archive, cumulative patch and dependency bundle are in
  `/home/user/artifacts`, with canonical `/home/user/ms178-1.patch`. The
  patch is relative to the stated upstream base; rebase to latest main and
  verify before subsequent delivery. Large kernel and toolchain trees under
  excluded cache/target paths are disposable and must be reconstructed.

## Continued: SELinux and signed-overflow policy

The next clean defconfig attempt reached 771 CC commands, then rejected
`security/selinux/ss/sidtab.c:66`: the last expression of an unexecuted GNU
statement expression contained `sizeof(struct { _Static_assert(...); })`.
The parser queued the member assertion after the expression as a synthetic
block item; consumers then mistook it for the source-level tail and inferred
`int` instead of a pointer. Member assertions now carry an origin bit, and
statement-expression result/type/qualifier consumers skip only synthetic
member assertions at the tail. They remain semantically checked. A reduced
fixture failed on the preceding compiler, now agrees with GCC/Clang in runtime
checks at O0/O2 in both ABIs (ICX m64 also passed); a false nested member
assertion is rejected. Seven adjacent statement-expression regressions pass.
The real `sidtab.o` compiled with LCCC, real objtool and default config.
The normal and explicit source-level assertion distinctions were investigated:
GCC and Clang differ on a standalone assertion at a GNU expression's tail;
this fix deliberately changes only parser-generated member assertions.

The kernel's `-fno-strict-overflow` had been accepted as an unimplemented
option, despite overflow-dependent folds in `simplify.rs`. The `abs` comparison
regression's own LCCC wrapping contract exposes a difference between that
option and implemented `-fwrapv` at INT_MIN. This does not demonstrate a kernel
runtime failure, and the C library's `abs(INT_MIN)` itself has a separate
standards caveat; this is an existing documented LCCC regression contract,
not a claim that GCC must agree on that library edge. Implemented the umbrella
flag's signed-overflow portion using the existing wrapping policy and GCC 16.2
last-option and -ftrapv precedence. Added runtime, optimizer-control and strict
probe checks to the existing iabs fast gate. `-fstrict-overflow` now resets the
signed-wrap state too. Both umbrella flags warn that pointer-overflow semantics
are not established, and `LCCC_STRICT_OPTIONS=1` rejects them; **full GCC
semantics are not claimed**. The old UB-source-scan prose was corrected: a
negative string scan is not proof that UB-based transforms do not exist.

Installed valgrind 3.24 after fast CI's oracle preflight reported it missing;
the verifier then passed with GCC 16.2, Clang 23.1.4 and multilib. Installed
local ICX 2026.2.0 (not used for kernel). Earlier CI runs were intentionally
stopped after the prerequisite/final-tree changes; none is a completed green
run. The cumulative patch remains ungated until the full final-tree CI passes.
An exploratory pinned-VM runtime screen (three gzip/zlib-ng/Expat derived
programs, GCC/Clang/ICX) and raw matched samples are saved under
`engineering/evidence/kernel-2026-10-30/native-batched-runtime.*`.

A new clean defconfig bzImage build is running after the compiler change.
The existing `kernel_boot_isa.sh` applied its two explicit baseline ISA
lines to the kernel decompressor/EFI-stub build files before the build; this
is needed because LCCC's default ISA otherwise exceeds QEMU qemu64. This is
in addition to the 29 package patches, not a change to defconfig. Outcome,
final setup size and boot remain unknown at this point. Keep checking the
build and CI logs; do not present a partially compiled tree or setup stubs as
a booted kernel. Pointer-overflow and null-check options remain open risks.

## Arena reset and upstream refresh

An Arena workspace/environment reset interrupted the pending full kernel and
fast CI runs. The persisted logs end during compilation (560 kernel CC lines
and 93 completed CI gates); neither run reached an exit verdict, produced a
complete kernel image, or passed CI. Their generated kernel tree, binaries,
toolchains and background processes were not retained. `LCCC-RECOVER.py`
restored the exact session Git history from the authenticated bundle and
upstream base. The persisted checkout was incomplete and lost executable bits;
all differences were deletions or mode-only, with no modified tracked file
content and no untracked files. The worktree was restored from the recovered
HEAD after verifying that fact. The cumulative patch's reverse apply check and
`git diff --check` passed on the repaired tree.

Fetched the live `ms178/lccc` `main`: it was still
`9bc081312ec63a2c3896604f0e4039a00140f7df`. The session branch already had
that commit as its merge base; `git rebase origin/main` reported up to date.
The 8 GiB of swap remained active. Reconstruct the excluded dependencies and
rerun from clean objects; no intermediate setup-only result substitutes for a
real `bzImage` and QEMU boot. The later defconfig setup-only probe was
`_end=26928` (5840 bytes spare, 24-object ISA scan passed, ld.bfd flat-image
parity), using private offset stubs and section GC. A no-GC link of those same
objects gave `_end=31024`; neither is the actual Kbuild final-offset setup
image. Resume fast CI from the beginning on the exact final tree; do not treat
partial earlier logs as a green stamp.

## New upstream main during the continuing run

A fresh `git ls-remote` detected that `ms178/lccc` main advanced from
`9bc081312ec63a2c3896604f0e4039a00140f7df` to
`2e1d460faf911cf57764ef26462024825ed5466d` while the next defconfig and CI
runs were active. Stopped both before rebuilding the compiler, fetched main,
and rebased all 14 session commits onto the new tip without conflicts. A
backup branch retains the pre-rebase head. Diff whitespace and patch reverse
apply checks passed; the cumulative patch was snapshotted with the new tip as
its explicit base. New upstream changes touch parser declarations/AST, inline
assembly, i686 vector lowering and x86 peepholes: automatic conflict-free
application is not proof of semantic compatibility. Rebuild with fastbuild
`-O1 -j2`, rerun focused parser/assembler/linker/overflow regressions and the
full fast CI, then clean-build the kernel again with the new binary. The
interrupted defconfig run had passed the previously failing SELinux `sidtab.o`
with the old binary; it did not complete a kernel image.

Added `scripts/qemu_defconfig_smoke.sh` for an actual defconfig image, separate
from the custom-VM-fragment harness. Its mocked verdict test checks missing
markers/log, panic, a failing QEMU process and an output-symlink control; that
mock is not a guest boot. The real harness creates a private initramfs workspace,
runs static BusyBox as guest userspace, checks the serial sentinel, two CPUs,
completion and poweroff, and publishes logs by atomic rename. Run it only after
an actual LCCC/lccc-ld bzImage exists. No successful full-image or QEMU boot
is claimed here.

## Upstream division-diagnostic regression and second recovery

After rebasing onto `2e1d460faf911cf57764ef26462024825ed5466d`, the clean
6.18.58 defconfig build failed in `kernel/fork.o` and
`arch/x86/kernel/sys_x86_64.o`: the kernel's `include/linux/` `mman.h`, whose
`_calc_vm_trans` has a constant comparison choosing an arm, and new
`-Wdiv-by-zero` reported
zero division in the *untaken* arm under kernel `-Werror`. GCC 16.2 accepted a
small macro reproduction; the just-rebased LCCC rejected it. The error is a
diagnostic defect, not evidence that the selected expression divides by zero.

Replaced the diagnostic's untyped i64 shortcut with an ICE check and the
existing typed C constant evaluator, bypassing the general const-value cache
and permitting only named constants. This handles comparisons, usual integer
conversions, unsigned wrap, casts, enum/ICE expressions, and the GCC `a ?: b`
short circuit without hiding warnings for runtime conditions or ordinary
`const` objects. It avoids consulting the ICE evaluator for normal non-logical
binary operators and fast-paths integer literals. The compiler's execution
semantics are unchanged: only warning suppression is adjusted. Unproven or
non-ICE conditions remain conservative; this is not a proof of full GCC
warning parity.

The fastbuild (`-O1 -j2`) succeeded, the actual minimized macro now compiles
with `-Werror=div-by-zero`, the option-effect ledger passes, and the new
`check_divzero_constant_arms.sh` matches GCC 16.2 in both 32- and 64-bit modes
on 19 selected/dead/live/unsigned-wrap/narrowed/ordinary-const cases. Added it
to fast CI. Rebuild the actual kernel objects and run all CI on the final tree
before claiming the fix is fully integrated.

A further workspace reset removed `.git`, Rust and generated artifacts. The
exact S17 Git history was restored from the authenticated bundle and upstream;
the uncommitted investigation survived and was preserved before the worktree
was restored. Verified upstream was still at the tip above. Rust 1.99.1,
GCC 16.2, Clang 23.1.4, binutils 2.47, ICX 2026.2, QEMU, static BusyBox and
host prerequisites were reconstructed; 8 GiB swap is active. The first kernel
prepare attempt extracted all 29 patches before the rebuilt compiler was
ready, leaving an unstamped incomplete scratch tree; it was removed only after
confirming the failed prepare log, and preparation was retried from the
SHA256-verified archive. Full kernel build, actual setup-size measurement,
QEMU boot and full fast CI remain pending. Snapshot S18 is **UNGATED**.
