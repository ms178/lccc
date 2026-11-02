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
