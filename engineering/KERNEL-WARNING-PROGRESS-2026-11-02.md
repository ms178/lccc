# Kernel compiler work — intermediate evidence, NOT readiness

This work continues PR #1065. The requested acceptance criteria remain all 13
inventoried warning families/options, full native kernel link and real objtool
validation, a kernel QEMU boot, green hosted CI, and measured correctness-backed
runtime/code-size gains. Those criteria are **not yet met**.

## Executed checks (before latest-main integration)

* Rebuilt compiler/native linker with the pinned Rust 1.99.1 fastbuild toolchain.
* Layout/unused differential suite: **2,940** non-kernel GCC 16.2 comparisons,
  covering code-generation and syntax-only behavior, headers, numeric levels,
  severity precedence, GNU statement-expression value uses and pragma scopes.
* Enum-conversion suite: **500** non-kernel GCC comparisons, including implicit
  arguments/returns/assignments, aggregate initialization, explicit casts,
  promoted arithmetic, statement expressions and ordinary-namespace shadowing.
  Enumerator diagnostic provenance is separate from its language-mandated C type
  and follows the existing scoped constant undo log. Completed-enum provenance
  uses shared ownership, not one copy of all variants per enumerator.
* Verbose assembly: **12** target/optimization combinations, positive/negative
  option ordering, function commentary and native ELF byte equality. Comments
  are added after optimization/validation, not used to alter code generation.
* Integrated existing warning gate: passes its original precedence suite,
  **60** pragma checks, **697** include-path checks, **636** date/time checks,
  plus the newly integrated suites above.
* Integrated linker gate: ELF32/64 RWX/fatal/suppression contracts, AArch64 and
  RISC-V shared plumbing and QEMU-user freestanding execution, and **16** strict
  discard-none ELF32/64 comparisons with GNU ld. Local absolute and FILE symbols
  are retained; stripping still wins regardless of discard-none ordering.
* Census: **262**, baseline **262**; no metric grew.
* Actual CachyMod Linux **6.18.58**, package commit
  `df9c917272d816361e0da61bb951b5732cb1c662`, stable source commit
  `2926d88bb7fc4608fd942453d929c39e4e5ec006`, **29** package patches:
  boot-only `_end=31088`, limit `32768`, headroom **1680** bytes; GNU-linker
  byte parity and all **24** boot ISA scans passed. Setup image SHA-256:
  `93bc0dc7e1ca24f59905028e74d21a64008d9d2a50cfd72b2dbc4db9b88caeff`.
  Kernel target translation units used LCCC; only host tools used GCC.
  Tag signature authentication was unavailable (`gpg-verified=no`), explicitly
  recorded in the preparation stamp. No signature-verification claim is made.

These results are fixture/gate evidence, not universal diagnostic parity. A
follow-up malformed negative numeric-option rejection is included; the final
binary rebuild and complete rerun must validate that follow-up too.

## Host environment repair (outside the repository)

Installed reviewed wheel-backed Bison/Flex/pkgconf, host bc 7.0.3, m4 and zstd
1.5.7. m4's unchanged suite has **109 passes, 21 legitimate platform skips, zero
failures** after fixing its glibc portability assumptions and escaping literal
braces in its old Perl copyright helper. Patches and source/hash receipts live
under `/home/user/kernel-host`. Matched elfutils 0.188 and OpenSSL 3.0.20 public
headers to the installed runtimes; a warning-clean host probe verified both and
checked the OpenSSL version number at runtime. zstd compression roundtrip passed.
`/home/user/toolchains/current-env.sh` supplies this host-only prefix separately
from the reviewed compiler cache. No fake libraries or validator shims were used.

## Open gates and scope

Still outstanding: designated-init, frame-larger-than, vla-larger-than,
implicit-fallthrough, format-overflow, format-truncation, missing-format-attribute,
restrict and stringop-truncation; the remaining inventoried compiler options;
latest-main integration and post-merge revalidation; complete unit/ABI gates;
full kernel link/objtool, kernel QEMU boot, refreshed hosted CI and measured gains.
Kernel preparation continues to report the unimplemented options; those warnings
have not been suppressed. The boot size above is not a performance improvement
claim or a replacement for the full default-config kernel acceptance gate.

## Post-merge follow-up (working progress)

Integrated main `abb7708da`. The frontend CI lane merge was checked against the
exact named-step union from both parents, with all 145 gates preserved; recorded
costs rebalance to 291.1 / 291.2 / 291.2 seconds. Workflow shell and lane-plan
structural checks pass. Post-merge compiler build (`--bins`) passed in 3m09s.
The complete integrated warning gate, all-target linker gate and Microsoft
scalar/wide, record and variadic ABI scripts pass after integration. Malformed
negative numeric-warning rejection also passes six strict/non-strict GCC
comparisons. Post-merge actual boot-only gate remains 31,088 bytes with the same
setup image hash, GNU-linker byte parity and 24-object ISA pass.

The focused unit gate found two stale expected-registry assertions introduced
when adding warning families (Wextra membership, then trailing `=` labels).
Both expectations are now explicit and corrected, without removing assertions.
Final diagnostic units: **17 passed / 0 failed**; symbol-table units: **2 passed /
0 failed**. These final focused units used fastbuild with opt-level 0 and debug 0
to avoid repeatedly spending 6+ minutes optimizing the unit-test harness under
swap pressure. The actual compiler binaries and kernel gates use the normal
optimized fastbuild binaries, unchanged by the lib-only unit command.

The real full kernel build is now running serially (`LCCC_JOBS=1`), with real
host-built objtool and the package configuration. At the latest observation it
was compiling `init/do_mounts.o`; no full-link result exists yet. Its log still
contains unimplemented-option warnings, so this is **not** a zero-warning pass.
Latest source fixes are pushed as `bf51dd9f2` to draft PR1065. Hosted checks for
that revision are still running; the previous revision's assertion-unit check
failed on the stale suffix expectation, so hosted-green readiness is not claimed.

## Current sandbox follow-up: nominal enum identity

The preceding local-process observations belong to the previous sandbox; that
process and its log did not survive the wipe, so its completion is unknown.
Hosted kernel run 41382248854 did complete with a verified enum-conversion
failure in kernel/irq/manage.c. This is repaired in f21c59e9c with declaration
identity rather than snapshot/name equality. See
`engineering/ENUM-IDENTITY-REDTEAM-2026-11-02.md` for the root cause, alternatives,
720 differential checks, 54 native/cross-target callback executions, focused unit
results, and the separate prototype-scope defect found by the audit. Final Clippy
also passed with `-D warnings` (52.36 seconds). Refreshed hosted kernel and ICX
checks are pending; the PR remains draft and not ready for delivery.

## Fresh local recovery and firmware dependency repair

Recovered the actual Debian bookworm host-tool export from hosted check
125244349590. All six Git blobs were verified against their Git object IDs and
per-part SHA256s, the reconstructed archive against SHA256
`316dc021c5cbf04d94534ce82bc784b6271000a68ac8b742fd9265bb110dbcff`,
and all 93 Debian packages against the archive's SHA256SUMS before extraction.
The private installation is in the excluded cache, not vendored into Git.
Bison 3.8.2, flex 2.6.4, m4 1.4.19 and genuine QEMU system emulator 7.2.22
run from that prefix. Host tools use GCC 16.2; kernel objects use only LCCC.
The 4 GiB swap is active. Private system headers must use `-isystem`, not `-I`:
the latter incorrectly shadows kernel tools' own linux/types.h and classifies
glibc headers as project headers under GCC's redundant-declaration warnings.
No warning flag or real objtool validation was disabled to correct that setup.

**Corrected diagnosis after inspecting the full package manifest:** the original
export already included `ipxe-qemu` transitively (its SHA256SUMS entry appears in
the first restore log). The initial claim that `--no-install-recommends` omitted
that package was wrong. The real defect was firmware discovery: Debian installs
`efi-e1000.rom` under `/usr/lib/ipxe/qemu`, while the smoke script searched only
`/usr/share/qemu` and `/usr/share/seabios`. The shared staging helper now searches
all three locations as well as the explicit QEMU_DATA_DIR. The workflow also
names `ipxe-qemu` explicitly to document the requirement; that alone is **not**
the root-cause fix. Workflow path filters include the smoke script, helper and
regression so future changes trigger actual kernel validation.

The regression retains its original verdict and symlink-security checks, tests
both explicit package requests, and now exercises the real helper against a
hermetic Debian split-directory layout: BIOS under share/seabios, QEMU ROMs under
share/qemu and iPXE under lib/ipxe/qemu. It checks the selected bytes and removes
the EFI ROM to verify a hard failure without machine-installed firmware masking
the absence. These fixtures pass. A separate temporary fixture with the explicit
package request removed also fails as expected. Workflow shell validation
(13 files, 2 rules) and `git diff --check` pass. None of these fixtures claims a
guest boot.

For this already-exported local host environment, the missing EFI ROM was
retrieved from upstream `qemu/qemu`, tag `v7.2.0`, path
`pc-bios/efi-e1000.rom`, Git blob
`6312b11c2f5868bb6e5f0f15dfa31ad7dbda0a02` (159232 bytes), verified against
its Git object ID and SHA256
`f37222fcdf0481bfbaf7a16d9aaf740aeb41d3d24c187801eb6e9a628cec0fc8`.
This upstream copy was an initial local transport workaround, **not** evidence
that the Debian package lacked the file. The refreshed export from check
125253134759 was recovered and verified (archive SHA256
`80dc7eb2323c48bddfd6f7edcb49f69caba4570affd4016049a513ac70544bac`, still
93 packages). The local prefix now uses the actual packaged Debian iPXE ROM,
249856 bytes, SHA256
`f034ae9a3fef092f2d55a7a46cfe2c1cc81469ee1166878e6c6ce70d12ebaa74`.
Real QEMU 7.2.22 starts with default PC devices and this private firmware prefix.
This tests real firmware loading, not a kernel boot, and uses no fake validator.

Fresh preparation applied all 29 package patches to Linux tag v6.18.58,
commit `2926d88bb7fc4608fd942453d929c39e4e5ec006`, package commit
`df9c917272d816361e0da61bb951b5732cb1c662`. Tag signature verification remains
**not established** and is recorded as `gpg-verified=no` by the preparation
script. Preparation and x86-64 defconfig/prepare passed using LCCC.

Fresh boot-only gate: **31024 / 32768 bytes, 1744 bytes headroom**;
GNU ld flat-image parity PASS; ISA scan PASS on 24 objects. Image SHA256:
`aba4f14ae04d5f9bd5f4acdb70a19d21dc19a05a2fc426124be550b4deeb7391`.
This is not a controlled optimization comparison against the previous differently
configured sandbox and is not a runtime/performance claim. The boot link still
reports its existing RWX segment warning; zero-warning acceptance is unmet.

The real full local defconfig build is running with actual objtool,
CONFIG_STACKPROTECTOR=y, CONFIG_UNWINDER_ORC=y, CONFIG_OBJTOOL=y and
CONFIG_MITIGATION_RETPOLINE=y. `kernel/irq/manage.o` has compiled without the
previous enum errors. Unsupported-option warnings are still emitted, not hidden.
Full link and system-QEMU boot have **no result yet**. Logs are
`/home/user/kernel-current-{prepare2,defconfig,boot,full}.log`.

Separately, hosted check **125247359597** for 4a1fc85a0 completed successfully:
the authenticated ICX 2026.2 lane includes the mandatory enum callback ABI step
(`--require-icx`) against exact GCC 16.2 / Clang 23.1.4 and the other vendor ABI,
SIMD and qualifier gates. This closes the previously unmeasured *hosted* ICX
lane for that revision, not the local ICX provisioning limitation. The hosted
kernel job is now building full bzImage; no hosted kernel boot result exists.
No PR readiness or all-CI-green claim is made.

### Hosted parity failure repaired, not allowlisted

The 4a1fc85a0 fast-contract shards failed when the reverse hosted/local audit
found that the dedicated hosted `check_enum_forward_abi.py` invocation had no
**direct** `ci_local.sh` entry. Although the warning runner already invokes the
same matrix transitively, the parity guard deliberately does not interpret
Python imports. Added an actual fast local gate with `--require-cross`, not a
comment, a hosted-only allowlist exception, or a disabled validator. This costs
one additional small ABI matrix locally; it preserves the warning integration
and the mandatory authenticated hosted ICX comparison unchanged.

Reproduction: `check_ci_gate_parity.py` failed explicitly with the missing script
before the repair. Afterward: parity PASS (503 commands, 27 invocation contracts),
**83 parity regression tests PASS**, including a new negative mutation which
removes the direct enum gate and must fail. The exact added ABI command also
passed: **54 measured executions, zero unmeasured cross executions**. Local ICX
remains unavailable and is reported explicitly; the earlier hosted ICX check is
successful. `bash -n scripts/ci_local.sh` and `git diff --check` pass. No compiler
source changed during this host/CI repair; the local full kernel process remains
on the previously validated compiler binaries. The new revision still needs
fresh hosted CI and the outstanding acceptance gates listed above.

## PID-1 console and stronger guest-runtime audit

Inspection of Linux 6.18.58 `init/main.c:console_on_rootfs()` found another
concrete smoke-harness defect: it opens `/dev/console` and initializes PID 1's
three standard descriptors **before** executing `/init`. The old unprivileged
`find | cpio` archive contained an empty /dev directory and no console node.
Creating device nodes only later inside /init cannot prevent the initial console
warning or retroactively initialize those descriptors.

The harness now uses the kernel's genuine **host-built** `usr/gen_init_cpio`
(default inferred from the bzImage tree; explicit GEN_INIT_CPIO supported).
Its manifest provides console character device 5:1 (0600), null 1:3 (0666),
root-owned directory/file metadata, /bin/sh -> busybox and executable /init.
This needs no host root privileges, fakeroot, custom archive writer or validator
shim. A missing/failing generator is a hard failure. Relative input filenames
handle whitespace in TMPDIR. Redundant staging-directory/symlink/chmod operations
were removed because the manifest supplies that metadata directly.

Alternatives rejected: privileged mknod is unavailable to the sandbox and
unnecessary; reopening descriptors in /init leaves the kernel's initial-console
warning unfixed; a new custom CPIO writer would duplicate a standard Linux host
tool and add archive-format/security maintenance. No compiler optimization or
whole-program speedup is claimed for this host-harness repair.

The real guest script now also mounts devtmpfs and a bounded 32 MiB tmpfs, runs
two concurrent workers writing/checksumming 8 MiB of zeros apiece, checks each
worker's exit status separately, compares both files and syncs before emitting
`LCCC_DEFCFG_MEMORY_IO_PASS`. The SHA256 was independently computed as
`2daeb1f36095b44b318410b3f4e8b5d989dcc7bb023d1426c492dab0a3053e74`.
This exercises process scheduling, demand-zero pages, tmpfs allocation and VFS
I/O/data integrity; it is not exhaustive hardware or kernel correctness proof.
The verdict requires this new marker and rejects kernel WARNING/BUG output and
the initial-console warning as well as the existing panic/Oops/failure markers.

Executed validation:

* Expanded hermetic harness regression PASS: original success/security verdicts,
  missing runtime marker, kernel warning, console failure, missing/failing archive
  tool, whitespace TMPDIR, and split Debian firmware discovery/refusal.
* The **actual generated** worker/join shell logic passes with both command-stub
  workers succeeding and correctly rejects either first-worker or last-worker
  failure. These are explicitly status-propagation unit tests, not guest boots.
* Embedded init passes both Bash syntax validation and real BusyBox ash `sh -n`.
* Real Linux gen_init_cpio built by GCC 16.2 generated the archive, including in a
  whitespace-containing TMPDIR. Independent newc decoding verified all 9 entries,
  device major/minor/modes, root ownership, /bin/sh target, executable /init,
  embedded workload, and byte-for-byte equality of the actual static BusyBox.
  Archive SHA256:
  `ef8cc0e8031100eab4c52a0c7ca9ecf8398beeb418cb2f6c93cd5fa7cf43bda3`.
  Only QEMU was a fixture for this archive test; **no guest boot is claimed**.
* Private Debian `file` needed its actual private magic database selected in its
  wrapper. Corrected host-only MAGIC to the verified package's
  `usr/lib/file/magic.mgc`; real file now recognizes the static BusyBox. This is
  an environment repair, not weakening the static-binary preflight.
* Workflow shell validator PASS (13 files, 2 rules), hosted/local gate parity
  PASS (503 commands, 27 invocation contracts), and git diff --check PASS.

At this observation the original local full build is still running (over 1700
C translation units compiled, no compile error observed), with genuine objtool,
4 GiB swap and the existing optimized fastbuild compiler/linker. Every hosted
check for 3bc6e92b1 except the still-running full kernel job has completed
successfully, including Test Suite and authenticated ICX. These are **not**
results for the new harness revision. Full link/guest boot and all previously
listed compiler warning/option/semantic/performance findings remain open.

## Orphan/CET/prototype-section blocker repair checkpoint (still NOT ready)

Implemented and locally exercised with the current optimized fastbuild binaries:

* The explicit-script x86-64/i386 `lccc-ld` path accepts all four
  `--orphan-handling` policies, in joined and separated spelling. Empty named
  inputs count as orphans; GC-dead inputs, ELF bookkeeping and SHF_EXCLUDE do
  not. Explicit wildcard and `/DISCARD/` matches are not orphans. Fatal warnings
  are enforced. Default placement avoids the new diagnostic classification pass.
  **Scope limitation:** non-default policies without an explicit script are
  rejected, not implemented; shared compiler-driver option plumbing is unfinished.
* Discard chooses strong/weak winners before deleting definitions: GNU does not
  resurrect a losing weak symbol after discarding the strong winner. Dead input
  definitions are excluded before dynamic-export planning, and their FDEs are
  pruned before layout. GNU `--defsym` expressions deliberately see a discarded
  input's section-relative offset with section base zero. Preserve this value
  for aliases and deferred expressions without allowing ordinary relocations to
  silently bind to it. This ordering follows measured GNU behavior, not an
  assumption about weak fallback or alias failure.
* A single observable-label map distinguishes actual indirect targets (global
  label initializers and `LabelAddr`) from direct asm-goto-only destinations.
  A default no-op architecture hook emits ENDBR64/ENDBR32 when branch protection
  is enabled. i686 now propagates that option and protects function entries too.
  Direct asm-goto destinations receive no unnecessary landing pad. Other
  architectures retain the no-op behavior and execute the common goto fixture.
* Function definitions inherit explicit sections from function prototypes in the
  existing signature prepass, including function-type typedef declarations.
  Function-pointer objects are not function entities. Explicit inherited sections
  take precedence over cold-section fallback. The actual hugetlb header annotates
  both the target and weak alias; alias-only propagation was not its root cause.
  GCC 16.2 also applies a section declared after a definition; Clang 23.1.4 warns
  and ignores it. The GNU-specific case is tested separately, not presented as
  agreement between the oracles. Conflicting redeclaration diagnostics have not
  been comprehensively audited.

Validation receipts in the user workspace:

* `kernel-orphan-ibt-build2.log`: optimized fastbuild binaries rebuilt successfully.
* `kernel-blocker-clippy.log`: `cargo clippy --profile fastbuild --bins -- -D warnings`
  PASS, 54.07 seconds; formatting and diff whitespace checks PASS.
* `orphan-policy-rwx-regression.log`: existing ELF32/64 RWX/flat/driver matrix,
  AArch64/RV64 shared and QEMU tests, 16 discard-none links, and **128** new
  GNU-oracle orphan-policy checks PASS. Includes zero-size inputs, GC, exclusions,
  last-option-wins, invalid/missing values, ordinary discarded references, weak
  resolution, absolute/deferred defsym expressions and FDE pruning. 2.368 seconds.
* `kernel-codegen-contracts.log`: **83** object/runtime contracts PASS at O0/O2/Os.
  LCCC x86-64/i686/AArch64/RV64, native GCC **16.2.0**, Clang **23.1.4**; branch,
  full and disabled CET modes on x86; direct asm-goto disassembly; actual bytes at
  computed-goto and function-entry targets; section/weak-alias checks and runtime.
  Cross runtimes use QEMU. 8.918 seconds with the kernel build running concurrently.
  **ICX is explicitly UNMEASURED locally**, not counted as passing.
* Hosted/local parity PASS: 504 commands, 27 invocation contracts. The additional
  gate and measured costs are recorded in the shard plan; lane balance PASS.
* `kernel-blocker-boot-gate.log`: fresh actual Linux 6.18.58 boot-only gate PASS,
  `_end=31056`, limit 32768, headroom 1712 bytes; GNU flat bytes identical; all
  24 boot objects pass the baseline ISA scan. setup.bin SHA256
  `97aea81fdca529293419185d39b57434b54c11dd55eb1356ac0be025ccf73cae`.
  This is **32 bytes larger** than the previous 31024-byte observation, not a
  performance/code-size improvement.
* `kernel-blocker-actual-objects.log`: actual patched Linux `kernel/bpf/core.o`
  and `mm/hugetlb.o` rebuild PASS through the canonical harness, with compiler
  identity invalidation intact. Both `__alloc_bootmem_huge_page` and its weak
  `alloc_bootmem_huge_page` alias have identical addresses in `.init.text`.
* `kernel-blocker-bpf-objtool.log`: genuine kernel objtool
  `--ibt --link --dry-run --Werror kernel/bpf/core.o` returns zero with no
  diagnostics. This focused dry run is NOT validation of final vmlinux.

Latest main was fetched and merged: already up to date at
`d202421fe236290f019969992c1c2a5669ae9615`.

The canonical full build is now running again with unchanged compiler/linker
hashes, preserved configuration, real objtool and 4 GiB swap. Logs:
`kernel-blocker-full-build.log`, then (only after successful build)
`kernel-blocker-qemu-run.log` / `kernel-blocker-qemu.log`.
**No successful final vmlinux link or guest boot is established at this checkpoint.**
The build still prints unsupported option/warning families. All-13-family
completion, zero kernel warnings, all-architecture/default-link orphan support,
local ICX, measured runtime/code-size gains and green CI for this revision remain
open. This is a pending checkpoint, not the requested production-ready delivery.


## Latest-main integration and traced-boot investigation

Integrated upstream `bfdcd13e0` by merge `96e41d467`. An attempted historical
rebase was aborted: replaying the old CI reorganizations repeatedly created
misassigned/duplicated gates. This is a merge, not a completed rebase. The final
workflow retains the full named-gate union from both parents without duplicates.
The repository splitter predicts backend shard costs of 520.9 / 521.2 seconds
(84 / 118 gates); these are estimates, not measured CI speedups. Lane-plan and
CI/local parity checks pass (506 commands, 27 contracts).

Post-integration validation with the reviewed toolchain:

* Normal optimized fastbuild compiler/linker: successful build confirmed.
* Clippy, fastbuild binaries, debug info off, `-D warnings`: PASS, 43.15 seconds.
* Full linker RWX suite: PASS, including 80 script-permission, 128 orphan-policy
  and 16 discard-none GNU-oracle cases, and cross-target runtime checks.
* Kernel codegen contracts: PASS, 83 object/runtime contracts.
* DWARF 2/3/4/5 suite: PASS against GCC 16.2 across C89 through C23, all four
  targets, consumer round trips and multi-object relocation.
* Fresh actual patched Linux 6.18.58 defconfig/prepare: PASS. Boot-first gate:
  **31024 / 32768 bytes**, 1744 bytes headroom, GNU flat-image parity and all
  24 ISA scans PASS. SHA256:
  `aba4f14ae04d5f9bd5f4acdb70a19d21dc19a05a2fc426124be550b4deeb7391`.
  This is not a controlled performance or code-size improvement measurement.

The previous hosted kernel run timed out in QEMU without the guest smoke
marker. `BOOT_TRACE_INITCALLS=1` now adds only `initcall_debug ignore_loglevel`
to the existing command line, and the kernel workflow enables it. Default mode
preserves the exact previous command line. Invalid values fail before boot.
Hermetic regression tests check both exact command lines and verify that trace
mode still rejects a wrong guest CPU count; the existing workload, log/security,
verdict and firmware-dependency tests remain enabled and pass. Workflow shell
checks also pass. These fake-QEMU tests are not an actual guest boot.

A fresh full local build followed by traced system-QEMU is running with the
private verified host tools, two build jobs and active 4 GiB swap. Kernel target
code uses LCCC exclusively. Logs are `/home/user/kernel-full-current.log` and,
only after a successful build, `/home/user/kernel-qemu-current{,-result}.log`.
There is no successful full-link/guest result at this checkpoint. Unsupported
option warnings remain visible; all thirteen families/options, zero warnings,
full objtool/guest validation, ICX 2026.2, measured gains and green hosted CI
remain unresolved. This autosave is PENDING, not a readiness declaration.

## Recovered guest faults and compiler repairs (pending full rebuild)

The diagnostic exporter is now hosted-green (run 41472547163, job
125370433113). An explicit job permission map had omitted Actions read access;
that scope is now present without Actions write. The subsequently surfaced
failure was gh's terminal-escape guard rejecting raw evidence. Only captured
log/ZIP downloads opt in to binary bytes; normal JSON requests retain the guard.
Bytes are compressed/blob-exported, never displayed on a terminal. Eight tests
cover minimal permissions, exact binary preservation, wrong blob identities,
input validation, credential/URL redaction and error propagation.

Recovered the old 902789 kernel logs and artifact via the published Git blobs,
verifying Git object IDs, per-part and complete-file SHA256s and lengths. The
artifact SHA256 is
`ad34de8d7698ea665013d9bee0e1e524a21666e1736ac845346908ebb5e59d2d`.
Unlike the previously accessible log tail, the complete serial log establishes:

* At 0.035s, `apply_returns` reports a conditional branch at a return-thunk
  patch site; at 0.241s it reports an unpatched return thunk.
* At 1.337s, the async storage-probe worker faults in
  `update_alloc_hint_after_get`: a volatile GS store uses the stored value as
  its address. The worker exits with interrupts disabled and boot never reaches
  the smoke marker. Thus describing the old result only as an unexplained
  timeout was incomplete.

Two independently reduced, executed compiler defects explain those observations:

1. `eliminate_binop_push_pop_pattern` redirected a load before calling
   `mark_nop` on three pinned instructions. Pinning correctly refused those
   deletions, leaving the old move to overwrite the new address register.
   Ordered peephole dumps reproduce the first bad transformation precisely;
   disabling phase 1 for diagnosis makes the reduced fixture pass. The fix
   checks all four edited lines before performing ANY change. Production
   peepholes and volatile pinning remain enabled. Unit tests pin each of the
   four lines independently and retain the unpinned optimization positive.
2. Basic branch inversion changed `Jcc local; JMP __x86_return_thunk` into a
   conditional external thunk transfer. Linux's runtime return-site patcher
   requires an unconditional JMP. Both x86 backends now limit this basic rule
   to assembler-local targets; proven marked external sibling-call folding
   remains separate. Existing local-label and 256-edge reference-index tests
   remain unchanged and pass.

Cross-target testing also exposed i686 silently ignoring the return-thunk
option. It now emits the external-thunk ABI, including GCC 16.2's ECX-carried
return-PC form for callee-cleaned sret/fastcall arguments. The extension whose
third vector return lane occupies ECX instead preserves all return registers
and relocates the PC before using the ordinary thunk. Standalone ABI tests
execute caller-cleaned, record-return, wide fastcall and three-lane vector
returns under qemu-i386. They are not Linux kernel oracle builds.

Final fix validation:

* Optimized fastbuild PASS; Clippy `-D warnings` PASS (18.70s).
* x86-64 peephole units: 1803 PASS, zero failures, five existing ignored tests.
* i686 peephole units: 401 PASS, zero failures, one existing ignored test.
  Unit harness uses fastbuild opt-level 0/debug 0; compiler binaries use normal
  optimized fastbuild. No ignored tests were added or validators disabled.
* Kernel-pattern suite: 107 object/runtime contracts PASS across four LCCC
  targets, GCC 16.2 and Clang 23.1.4; ICX explicitly unmeasured.
* Entire GS runtime suite PASS, including the new standalone ABI fixture at
  O0/O1/O2/O3 and full-buffer byte checks for signed/unsigned 8/16/32/64-bit
  volatile stores. Existing indexed-fold shape checks remain enabled.
* Actual patched Linux `kernel/cpu.o` and `lib/sbitmap.o` rebuild PASS through
  the canonical harness. Genuine objtool `--rethunk --ibt --link --dry-run
  --Werror` passes both objects without diagnostics. These are focused object
  checks, NOT final vmlinux validation.
* Boot-first gate remains 31024/32768, GNU flat bytes identical, 24 ISA scans
  PASS, SHA256 unchanged from the preceding checkpoint.

Stopped the older full local build after reproducing its compiler defect; it
was NOT a completed link. A new full build with compiler identity invalidation
and the repaired compiler is running, followed by strict traced QEMU on success.
Logs: `/home/user/kernel-full-fixed.log`, then
`/home/user/kernel-qemu-fixed{,-result}.log`. No new full-link/guest result is
claimed yet. All-thirteen warning/option completion, zero warnings, measured
runtime/code-size gains and current-head green CI remain outstanding.

## Fresh latest-main full build and guest evidence (7548b21e3)

The compiler source at 1c738ef92, with the workflow-only 7548b21e3 follow-up,
completed the actual patched 6.18.58 defconfig **native full link and bzImage**.
The build log contains 2,939 `CC` progress lines. Image SHA256:
`d53801e0e5aebffdbabd23c2f1fb65f0e2764e707b614e133f2ce3ecddf591d5`.
The fresh boot-first setup remains 31,024 / 32,768 bytes, with GNU flat-image
parity and 24 ISA scans passing. No GCC kernel-target build was performed.

The first QEMU launch stopped before guest execution because the restored host
prefix's QEMU_DATA_DIR covered QEMU but not SeaBIOS. Staging the real packaged
QEMU/SeaBIOS/iPXE files into a complete private directory corrected that local
environment; no validator or virtual device was removed. The unchanged image
then passed the strict traced smoke on **both qemu64 and max**, two CPUs each:
version/serial markers, both concurrent 8 MiB demand-zero/tmpfs/hash workers,
comparison, sync and clean power-off. Real system QEMU is 7.2.22. These are
bounded smoke results, not a claim that every kernel subsystem works flawlessly.
The guest still reports missing optional wireless regulatory.db firmware.

**Full objtool is NOT clean.** The final vmlinux.o reports 416 warnings:
271 indirect-call mitigation warnings, 86 returns with UACCESS enabled,
56 redundant UACCESS disables, and three calls with UACCESS enabled.
The build accepted these warnings; successful link/guest smoke is not a strict
objtool acceptance pass. The warnings remain visible and require fixes.

The follow-up callback audit repairs the global-cell mitigation/snapshot defect
and the GOT-call mitigation bypass; see
`engineering/CALLBACK-MITIGATION-AUDIT-2026-11-02.md`. Its compiler has passed
215 object/runtime contracts against GCC 16.2.0 and Clang 23.1.4, all three
Microsoft ABI suites, the symbol-version regression and all-target Clippy with
-D warnings. Actual arch/x86/events/core.c rebuilt with this compiler passes
real objtool --retpoline --Werror. This is focused evidence only: this follow-up
compiler has **not** yet completed a fresh full kernel/QEMU pipeline. ICX 2026.2
is not installed locally; the authenticated vendor CI lane now explicitly
requires it for these same native contracts. Hosted results are pending.

All remaining warning families/options, zero build warnings, complete strict
objtool validation and measured kernel performance gains remain open. No
production-ready or universal-performance claim is made.

## Boolean merge follow-up

The casted 0/1 merge in actual check_zeroed_user now has a narrowly proved
canonicalization rather than a validator annotation. Final 13 IR tests, 233
four-target/oracle object/runtime contracts and all-target Clippy pass. Its
kernel function shrinks 611->591 bytes; a bounded native userspace replay shows
about 1.57% lower median workload time, not a whole-kernel speedup. The actual
object still fails strict objtool on two dead stac/clac helper findings.
See `BOOLEAN-MERGE-UACCESS-AUDIT-2026-11-02.md` and its raw measurement JSON.
Newly confirmed target-specific hardening-option omissions are explicitly
listed there and remain unfinished. All-13/zero-warning readiness is not claimed.
