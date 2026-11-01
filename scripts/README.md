# scripts/

Developer and research tooling. None of these are needed to build LCCC.

Compiler-free corpus audit entrypoint: `bash scripts/ci_corpus_tools.sh` (not
`ci_local.sh --fast`). It is a tooling/contract gate, not compiler CI or runtime
performance validation. Compact recovery: `python3 scripts/lccc_recover.py
--repo /opt/lccc-work/lccc --offline --require-exact-history`; the published
standalone `LCCC-RECOVER.py` survives a worktree reset. A session range that
*merged* upstream is proved and restored against BASE **and its ancestry** (the
receipt records which proof ran, `base_fetch`): the merged range's second
prerequisite is the old fork point, which a `--depth=1` BASE cannot satisfy, so
both the snapshot proof and the recovery tool deepen BASE instead of rejecting
a bundle that restores perfectly. Offline recovery, which has BASE alone by
construction, reports the honest source fallback for that shape. See the
[audit and recovery instructions](../engineering/PR724-AUDIT-2026-10-02.md).

| Script | Purpose |
|---|---|
| `audit_levkropp_since.py` | Historical marker scan, plus exact `--commit SHA --against REF` JSON inventory for consolidation audits: compares all changed Git entries without pretending filename/subject matches prove semantic adoption. Offline fixture: `test_audit_levkropp_since.py`. |
| `rust_toolchain.sh` | Shared selector: resolves the moving channel from `rust-toolchain.toml` (stable Rust/Cargo, with matching rustfmt and Clippy), while honoring an explicit bisection/reproduction override. CI records the resolved component versions on every run. Its behavior is covered by `tests/regression/check_rust_toolchain_selector.sh`. |
| `build_lccc_fast.sh` | Incremental `fastbuild` profile (target compiler opt-level 1, no LTO), using the manifest-selected current Rust/Cargo release, exactly two Cargo jobs, and warnings denied by default. Use for RA/codegen loops. |
| `build_lccc_o1_j2.sh` | Ship-quality release: Rust opt-level 1, two Cargo jobs, thin LTO, swap active, manifest-selected current Rust/Cargo release, and warnings denied by default. |
| `ensure_swap.sh` | Idempotently verify swap or recreate/activate the disposable 8 GiB `/swapfile` after a constrained-harness root reset; both compiler build scripts invoke it. |
| `ci_enable_swap.sh` | Create and verify an active 4 GiB swap file in the ephemeral GitHub runner temp directory before memory-heavy CI, Clippy, oracle, and benchmark jobs; `LCCC_CI_SWAP_SIZE` overrides the size. |
| `ci_install_gcc16.sh` | Provision GCC/G++ 16 from the runner distribution or Ubuntu Toolchain PPA when needed; `--multilib` adds both GCC16 and system multilib runtimes for the i686 differential suite and fails closed when `gcc-16 -m32` cannot link. Success is defined by *exact package resolvability* (`apt-cache pkgnames`), not by `apt-get update`'s exit status, because apt exits 0 when a repository index 503s. Keeps apt's index (`Dir::State::lists`) beside the cached debs, so a warm runner resolves the toolchain without touching the PPA CDN, and retries a flapping edge over a bounded six-round window. When the index still cannot be refreshed, it installs the verified debs a previous run cached (dpkg needs no index) and requires the same version contract; a cache that does not cover every requested package fails closed. Publishes one bounded `::error` annotation on failure so every CI lane is diagnosable from the Checks API. |
| `ci_install_mold_oracle.sh` | Provision mold **3.0.0** exactly (the latest public mold release; it superseded the `harness-cache-1` tag's 2.42.1 binary): downloads the official x86-64 release asset, requires the GitHub-published SHA-256, refuses a foreign/stale prefix, and earns exit 0 by linking and running a probe through it (`-fuse-ld=mold`), not by a version string. Building mold from source instead is `tools/linker/setup_oracles.sh` / `tests/linker/setup_oracles.sh` (mold 3.x has no CMake: the x86-only preset is the mold-cli Cargo features `x86_64,i386`). |
| `ensure_qemu_user.sh` | (Re)provision the pinned QEMU user-mode emulators (`qemu-aarch64/arm/i386/riscv64`) at **11.1.2** — replacing the `harness-cache-3` tag's 8.2.2 binaries — from Debian sid's `qemu-user` 11.1.2+ds-3 `.deb`, pinned by the signed Packages-index SHA-256 with a snapshot.debian.org content-hash fallback. A warm prefix is reused only when every binary reports 11.1.2; a stale set is replaced. `--self-test` exercises the version probe offline. |
| `publish_agent_toolchain_cache.sh` | Opt-in hosted publisher that assembles every current oracle/toolchain into a new immutable orphan tag whose parts are reachable over `github.com` only. It verifies the locked Clang/Rust assets, the pinned mold release, Debian sid QEMU packages and pinned binutils 2.47 sources, kernel relocs v7.3 sources, and the existing transport-tag payloads; it records `MANIFEST.sha256`, `CACHE.json`, and `PARTS.tsv` in the tag. |
| `ensure_binutils_247_binary.sh` | Restore the reviewed bookworm-runtime-compatible binutils 2.47 program set from the immutable GitHub-only agent cache instead of compiling it on each runner. It verifies `as`, `ld`, `ld.bfd`, `objdump` and `readelf`, supports `--verify-only`/`--print-bin`/`--self-test`, and returns success in both print modes. `tools/linker/setup_oracles.sh` adds the separately tested native BFD wrapper for sysroot, multiarch libc search and new dtags instead of pretending the generic cached binary has that host layout. |
| `restore_agent_toolchain_cache.py` | Consumer for that orphan tag. It reassembles `<name>.partNN` blobs with `git cat-file`, verifies every canonical file against `MANIFEST.sha256` and the reviewed in-tree lock, rejects unsafe tar paths/links before extraction, and writes wrappers plus `env.sh`. Extraction uses descriptor-relative, no-follow operations and atomic file replacement on supported POSIX hosts (including Python without tar filters); elsewhere it requires Python's native data filter and fails before opening an archive when neither is available. The POSIX path rejects duplicate members, special files, forward hardlinks and any archive path traversing a symlink. Receipts are selection-specific. The extraction records an installed manifest (every member's type, size and execute bit, every symlink target, plus the wrapper and `env.sh` digests), so a warm restore answers from it in **~0.3 s instead of re-opening 517 MiB of xz-compressed archives** (measured 20.9 s → 0.28 s on the 13-tool cache) while still repairing deleted payloads, lost execute bits, corrupt or stale wrappers. After extraction every symlink is re-resolved, so a *chain* of individually legal links that would place a wrapper outside the prefix is rejected before the first write. Supports `--only`, `--verify-only`, `--no-shims`, and `--self-test`. No non-GitHub download host is needed. |
| `offline_deb_extract.sh`, `offline_deb_validate.py` | Unprivileged, staged extraction of **trusted, verified** offline Debian packages into an empty sysroot strictly below `/home/user` or `/tmp`; refuses root and system destinations, preflights archive traversal/symlink hazards, confines absolute symlinks, and prints compiler/linker environment recipes. No package is installed on the host. All packages for one root must be supplied in the same invocation. Contracts: `python3 -m unittest discover -s tests/corpus -p test_offline_deb_extract.py`. |
| `check_toolchain_cache_freshness.py` | Audits every package in the tag hash cache against its latest public release. `--offline` (unit-tested, no network) fails when any two files disagree about the same package's pin; online mode additionally queries upstream (GitHub releases, GNU/sourceware listings, kernel.org, Arm permalinks, Intel's APT index, Debian madison) and fails STALE on any pin that is no longer latest. Deliberate exceptions (the 1.97.0 MSRV floor, the historical 23.1.3 tag tree) are reported as INFO, never skipped silently. |
| `arena_session_restore.sh` | Restores swap and Git metadata after an Arena reset, consumes the separately restored hash-locked toolchain cache, and reports missing multilib support without installing host packages. `--source-only` skips all provisioning; `--swap-only` checks that swap is active. |
| `ensure_i386_multilib_safe.sh` | Safely verifies or provisions 32-bit `-m32` C/C++ multilib support without ever upgrading live `libc6:amd64`, adding foreign `:i386` multiarch, or clobbering usr-merge `/lib -> usr/lib` symlinks; falls back to an isolated rootless sysroot under `/var/tmp/lccc-i686-sysroot` (`LCCC_I686_SYSROOT`) when mirror security updates skew against the running container. Covered offline by `tests/corpus/test_i386_multilib_safety.py`. |
| `kernel_version.sh` | Single source of truth for *which* Linux tree the kernel harness uses (explicit `KERNEL_DIR`/`KVER` > prepared tree on disk > package PKGBUILD > pinned fallback). Sourced by every kernel script; CLI via `--kver`/`--default-dir`. |
| `prepare_kernel_tree.sh` | Recreate the linux-cachymod tree (version from the package PKGBUILD) with its patch series and generated boot headers after a harness wipe. |
| `build_kernel_boot.sh` | Build all x86 real-mode setup objects with LCCC (`-ffunction-sections`), link with `lccc-ld --gc-sections`, preserve non-relocation boot payloads through a build-local `KEEP` script, enforce the authentic 32 KiB ASSERTs, and require flat-image byte identity with available BFD/LLD oracles. |
| `kernel_boot_isa.sh` | Boot-path ISA contract: `patch` rewrites the flag-wiping x86 boot/compressed Makefiles to name the x86-64 baseline idempotently and refuses to guess on refactor; `scan` rejects every BMI/ABM/VEX/SSE4.2-class mnemonic in boot objects. ZSTD `DYNAMIC_BMI2` runtime-dispatched clones are the sole exception: `lzcnt`/`bmi`/`bmi2` mnemonics are stripped only inside the exact ZSTD clone symbol names (POSIX/mawk-safe awk), while AVX/SSE4.2/`movbe` and every other symbol stay fully vetoed. The symbol-name trust boundary and an anti-vacuity control are documented in `tests/regression/check_kernel_boot_isa.sh`. |
| `elf_sections.sh` | Shared ELF section helper: `lccc_elf_code_bytes` sums every `SHF_EXECINSTR` section of an object. Sourced by the size harnesses, because summing sections named `.text*` reports `header.o` and `bioscall.o` as 0 bytes (their code is in `.bstext`/`.entrytext`/`.inittext`) and understates `tty.o`'s gap against GCC from +184 to +57. |
| `realmode_corpus.sh` | Compare LCCC/GCC executable (`SHF_EXECINSTR`) bytes per `arch/x86/boot` C file under the real `-m16 -Os` flags. |
| `asmdiff.py` | Whole-object differential against GNU as: section bytes, relocations, and symbols. See `tests/asm-diff/README.md`. Takes the oracle pair as flags (`--as`, `--objdump`): the `betterok` groups accept a smaller encoding only when the pinned disassembler proves both objects decode identically, so both gates run it against the 2.47 pair. |
| `insndiff.py` | Per-instruction encoding differential against GNU as. Reduces an encoding bug to a single mnemonic in one step; supports `--sweep` over register/immediate matrices. A shorter-than-GAS encoding is reported as `BETTER` only after the tool disassembles both forms and confirms they decode identically. |
| `encdiff.py` | Multi-assembler encoding differential: LCCC against GNU as **and** the Clang, GCC, ICC and ICX integrated assemblers over the Compiler Explorer API. Supports x86-64 and i686 (`--32`, local `lccc-i686`/GAS `--32`, remote compiler `-m32`). A shorter result is a `BEATS` only after objdump round-trips it against every distinct shortest oracle encoding in the selected mode; missing/undecodable disassembly is reported as unverified and fails the gate. Casefile harvesting skips `reject` groups. |
| `gen_encoding_sweep.py` | Generate instructions that have MORE THAN ONE legal encoding (accumulator short forms, imm8 sign-extension, redundant REX, VEX2-vs-VEX3, scale-1 index folds, ...). These are the only places an encoding can be improved, and most are invisible to a structural-coverage corpus. |
| `gen_asmdiff_corpus.py` | Generate the `tests/asm-diff/*.casefile` corpora. Every case is validated against GNU as before being written. |
| `gen_apx_asmdiff.py` | Generate `tests/asm-diff/apx.casefile`: REX2/EGPR/NDD/`{nf}`/`{evex}` plus the unsigned-imm32-into-EGPR size win (tagged `betterok`). |
| `godbolt.py` | Compiler Explorer client for GCC/Clang/ICC/ICX assembly; `compare --local-ref LABEL=EXECUTABLE` adds explicit native local references without network, with launcher-only hashes and fail-closed missing/ignored-option reports. |
| `codegen_oracle.py` | Batch local/Compiler-Explorer comparison with deterministic artifacts and architecture-aware x86 or AArch64 instruction/load/store/stack-traffic statistics; AArch64 defaults to GCC 16.1, GCC trunk, and Clang 22.1. `--rank` is the whole-corpus survey absorbed from the deleted `codegen_scoreboard.py`: every function of every source is ranked by instruction-count gap to the best oracle, worst first, with the scoreboard's historical defaults preserved (`-O2 -march=x86-64-v3`, `--min-insns`, `-v`, `--baseline`, the `gaps` JSON schema, and the always-exit-0 survey contract — compiler failures go to stderr; `--oracles ''` is the scoreboard's local-only same-binary A/B mode). Both report modes share ONE metric implementation; the old oracle-vs-scoreboard divergence (non-VEX packed SSE mnemonics undercounted) is fixed, and remote compiles share the scoreboard's `att-v2` digest cache. |
| `check_codegen_refactor_identity.py` | Byte-for-byte assembly gate for pure codegen refactors. It compiles a corpus with before/after LCCC binaries under a scrubbed default `CCC_*`/`LCCC_*` environment and honours adjacent `.flags` files; the report separately canonicalizes only LCCC PGO-generation process-ID suffixes, which are intentionally nondeterministic. |
| `reproduce_historical_peephole_utf8.py` | Extracts and executes the exact historical UTF-8-corrupting helper. It resolves the current Rust channel from `rust-toolchain.toml` even though the intentionally historical snippet is compiled with its original Rust-2021 semantics. |
| `aarch64_execute_suite.py` | GCC `gcc.c-torture/execute` differential runner: LCCC assembly is validated by a mandatory GAS 2.47, linked for AArch64, and executed against GCC/optional Clang under QEMU. |
| `x86_gcc_torture.py` | Full native x86-64 **and i686** GCC `gcc.c-torture/execute` matrix (`--arch=i686`), plus the `gcc.c-torture/compile` leg with `--mode=compile`. That leg follows compile.exp's contract: every source must compile and assemble to an ELF object. gcc gates eligibility the same way, and there is no link or run. Splits compilation and linking so every PASS covers LCCC object generation **and standalone `lccc-ld`** (`-m elf_x86_64` / `-m elf_i386`, driven through `gcc [-m32] -no-pie -B<shim>`; `--link-mode=driver` keeps the legacy one-step `lccc-i686` compile+link), defaults to the corpus `ensure_gcc_torture.sh` provisions (the JSON records it as `gcc_testsuite`), uses GCC only as eligibility/CRT oracle, understands unconditional `dg-options`, scales timeouts by `dg-timeout-factor`, reports `run_expensive_tests` sources (memclr/memcpy-a*) as `unsupported` unless `--expensive`/`GCC_TEST_RUN_EXPENSIVE` is set (gcc's own harness rule), enforces timeouts, and writes deterministic JSON/failure logs. `--from-list FILE` re-runs only the tests named in a previous report (one JSON `test` value per line, blank/`#` lines ignored, unknown names warn+skip, intersects positional tests and `--filter`); the partial-run JSON gains a `from_list` field so evidence tooling can see the filtering. |
| `lccc-snapshot.sh` | Harness-wipe-resistant autosave: commits then atomically publishes a verified squashed `ms178-1.patch`, series, tarball, bundle, and ledger with SHA-256 records. Set `LCCC_BASE_REF` when re-anchoring a new session after an upstream rebase. Accepts a matching `ci_local.sh --fast` (or `mode=full`) stamp for the same tree. Owner policy: no full local CI run; GitHub CI runs the slow gates on every PR. `LCCC_SNAPSHOT_ALLOW_PARTIAL=1` still labels a `--slow`-only autosave `PARTIAL-NOT-DELIVERABLE`. |
| `ci_ubuntu_chroot.sh` | Runs a command -- normally `ci_local.sh --fast` -- in an Ubuntu 24.04 debootstrap (the GitHub runner's userland: GCC 13.2, glibc, binutils) with `$HOME` bound at the same path. The system GCC is an oracle for several gates; PR #638 passed ci_local on Debian's GCC 14.2 and failed CI on Ubuntu's 13.2. `--setup-only` builds the chroot; `LCCC_CI_CHROOT`/`LCCC_CI_MIRROR` override its path and mirror. |
| `torture_evidence.py` | Summarizes native torture reports without conflating compile and execute legs. Schema-2 incomplete/fatal/cardinality-mismatched reports are refused; filtered selections are labelled explicitly. |
| `check_ci_gate_parity.py` | Both directions of the ci_local.sh / workflow mirror: every local gate runs in hosted CI, every hosted script, cargo subcommand and `--config` build mode runs locally (comments do not count on either side), and no `tests/regression/check_*.sh` is wired into nothing. Exceptions: `ci_gate_allowlist.txt` (orphans) and `ci_hosted_only.txt` (hosted-only measurements); both may only shrink. Self-test: `test_ci_gate_parity.py`. |
| `ci_test_report.py` | Failure observability for the suite GitHub runs on every PR. Runs a command with stdout/stderr merged and streamed byte-for-byte unchanged (exit status preserved, `128+n` when the child dies by signal) and, on failure, republishes the verdict where a triager can read it: one `::error` annotation per failing test placed on the panic's own `file:line`, a summary annotation naming every failure first (GitHub displays only a bounded number of annotations per step, and the list is what must survive the cut), and a Markdown `$GITHUB_STEP_SUMMARY` carrying the complete failure list, the panic messages and the log tail. `--parse-log FILE` runs the same parser over a saved log. It exists because `actions/jobs/{id}/logs` answers HTTP 500 for a failed job and the public job page carries no verdict text, so a red `cargo test` step used to be diagnosable only by coincidence; its `--self-test` covers the escaping rules, both panic spellings, rustc diagnostics, `-D warnings` promotions, ANSI/CRLF/invalid UTF-8 input, the annotation cap and run-mode exit codes. Wired into both mirrors (`ci_local.sh` cargo-test gates and the two `Run tests` steps). |
| `bench_kernels.py` | Separate-TU kernel timing harness. Supports a CPU-pinned, AB/BA-balanced `lccc-alt` compiler arm via `--lccc-alt-env NAME=VALUE`, verifies checksums, and hashes only the timed function. |
| `fuzz_diff.py` | Unified differential-testing harness and single entry point for every differential engine (`synthetic`, `csmith`, `yarpgen`, `stress_suite` + the nine absorbed `tests/fuzz` engines `differential`, `phi_cfg`, `intcmp_thread`, `m32`, `regparm`, `slot_rmw`, `alias_m32`, `alu_torture`, `aarch64`). Compiles every generated program with LCCC **and all selected references** across the requested `-O` levels, compares stdout/stderr/exit status, and saves miscompile reproducers. The `stress_suite` engine sweeps `--count` seeds per generator and evaluates every case under the cartesian product of repeatable `--config-env KEY=VALUE` axes; a case where the reference AND lccc die by the same signal is a GEN-BUG failure (generator bug, never a silent skip). The generator-config engines import the historical `tests/fuzz` generators (historical flags, seeds, and gcc oracle preserved; `Oz` maps to `-Os` on references); the standalone-pipeline engines (nostdlib int80 or cross-compilation) are forwarded to their `tests/fuzz` script and adopt its exit code, SKIPping cleanly when the host lacks ELF32/int80 or the cross toolchain (int80-blocked hosts fall back to compile-only validation). `--check-engines` is the CI wiring guard against stub engines. |
| `fuzz_identities.py` | Manual differential red-team for the scalar ring identities (`bit_idioms_identities`): draws programs from the idiom families (remainder windows at every width, sign-mask shifts in both mask forms with runtime counts, complements of negation, saturated bounds, zero-test selects in all four arm orders, or-chains, nested boolean selects, boolean products in four spellings plus the three-operand `x * (x || (y && z))` form, power-of-two division and remainder), builds each three ways (folds on, `CCC_NO_RING_IDENTITIES=1` + `CCC_DISABLE_PASSES=bit_idioms,divconst`, GCC 16.2) and compares the executed digest. Programs are UB-free by construction — signed negation, signed multiply and a `-1` divisor stay away from `INT_MIN` and shift counts stay below the width — because a differential oracle only decides defined programs. Not a gate: a run is hundreds of compiles. |
| `mutpair.py` | Missed-alarm differential pairs (Regehr/Livinskii transplant, blog.regehr.org/archives/2124): generates a UB-free-by-construction uint32 function, applies one UB-preserving mutation, and keeps only execution-filtered pairs that are *known* inequivalent (or known equivalent) under a bit-exact Python model. Emitted as one TU printing both result columns; standalone mode judges every compiler/opt level against the model itself, and `fuzz_diff.py --engine mutpair` routes the same cases through the reference oracle. `--self-test` includes the negative control — a deliberately broken fake optimiser must be caught — so the harness itself is tested for missed alarms. See `engineering/REGEHR_2124_TRANSPLANT.md`. |
| `slice_superopt.py` | Minotaur-lite enumerative superoptimiser for small integer slices: cost-bounded bottom-up synthesis with observational-equivalence pruning, candidate refinement *proven* by exhaustive 8-bit truth tables (the Alive2 role; no sampling, so no missed alarm at the checked width) and re-gated at 16/32/64 bits before reporting. Constants are solved by pool enumeration; cost is a documented op-weight proxy (a win is a hypothesis until `bench_kernels.py` confirms it). Emits a reviewable candidate table only — nothing is auto-applied to codegen. `--self-test` runs positive strength-reduction controls and inequivalent-pair negative controls. |
| `yarpgen_mutpair.py` | Regehr & Livinskii Method 1 on whole programs: for each seed, generates the original UB-free YARPGen program and its `--mutate=exprs` sibling (same globals/driver shape), keeps only pairs whose final-state hash *provably differs* under the reference compiler (execution filter, GCC -O0/-O2 consensus), then requires LCCC to preserve the inequivalence at every requested `-O` level. A pair that collapses under LCCC is a missed alarm / candidate miscompile. `--self-test` plants a collapsing fake reference and requires the tool to catch it; `test_yarpgen_mutpair.py` unit-tests the pair/filter logic. Needs a provisioned YARPGen (`$YARPGEN` or `--yarpgen`, see `ensure_yarpgen.sh`). See `engineering/REGEHR_2124_TRANSPLANT.md`. |
| `ensure_yarpgen.sh` | Idempotently provisions the pinned intel/yarpgen (`1adb290f453505838f3aa33dc571f292b0c1810f`) into `~/.cache/lccc/yarpgen/<shortsha>/`: shallow clone + CMake `-j2` build, then a probe battery (binary runs, expected `--mutate=exprs` help surface, a generated case compiles and links) before writing the stamp; any probe failure exits non-zero and callers SKIP cleanly. Prints the binary path for `--yarpgen`/`$YARPGEN`. |
| `ensure_gcc_torture.sh` | Idempotently (re)provisions `gcc.c-torture/{execute,compile}` + `gcc.dg` from the GCC release tarball (default **16.2.0**, `GCC_TORTURE_VERSION`) into the snapshot-excluded `~/.cache` zone — the ~22k-file corpus and the 107 MB tarball would each blow the workspace snapshot cap. The tarball must match a sha512 pin (cross-checked against the GNU release signature when added; `GCC_TORTURE_SHA512` for unpinned versions). The stamp is trusted only after the manifest hash, the required directories (`gcc.c-torture/{execute,compile}`, `gcc.dg`), the exact file set and every file's sha256 check out; any failure auto-repairs. Re-provisioning drops the stamp first and writes it only after the new tree validates. `--force` re-extracts; `tests/regression/check_ensure_gcc_torture.sh` is the contract test. |
| `ensure_gcc_torture_git.py` | GCC **17-git** companion to the release provisioner: resolves master once (or accepts `--ref <full SHA>`), sparsely fetches the torture, gcc.dg and harness-support trees, checks the complete selected file set and every Git blob, repairs a damaged cache by atomic directory exchange, and writes a revision-bearing corpus stamp. Offline repair/failure tests: `python3 -m unittest discover -s tests/corpus -p test_gcc_git_provision.py`. Default checkout: `~/.cache/lccc-gcc17`; point the runner's `--suite` at its `gcc/testsuite/gcc.c-torture/{execute,compile}`. |
| `x86_gcc_torture_i686.py` | Compatibility wrapper: forwards to `scripts/x86_gcc_torture.py --arch=i686` (all flags, including `--from-list`, pass through unchanged). |
| `ci_gcc17_torture.sh` | Reproducible full-source GCC 17-git `gcc.c-torture/{execute,compile}` runner for one x86 target. The corpus revision is PINNED by default to `c94b688a…`, the trunk revision the 2026-10-31 campaign was measured against (`GCC17_TORTURE_REF`, or the dispatch `ref` input, overrides it -- including `refs/heads/master` to track trunk); the reference compiler is GCC 16.2 exactly (`GCC_TORTURE_GCC_VERSION`), which the driver asserts before it compiles anything, and the distribution's rolling `gcc-16` is 16.0.1. It compiles/links/runs execute tests through standalone `lccc-ld`, compile-assembles the compile leg, and writes complete schema-2 JSON plus failure logs. Its guards distinguish a MISSING artifact binary from a NON-EXECUTABLE one, because `actions/upload-artifact` stores content only and a bare-binary artifact arrives at 0644. `GCC_TORTURE_ADVISORY_FAILURES=1` (what the hosted lane sets) publishes candidate failures as an inventory and keeps the lane green on runner exit 1, while exit 2 -- all-skipped, reference preflight, mid-flight input drift -- is always fatal; local runs are strict by default. Defaults: `-O2`, `-j2`, 180-second compile timeouts, 10-second run timeouts; override via `GCC_BIN`, `GCC_TORTURE_FLAGS`, `GCC_TORTURE_JOBS`, `GCC_TORTURE_COMPILE_TIMEOUT`, `GCC_TORTURE_RUN_TIMEOUT`, optional regex `GCC_TORTURE_FILTER`, `GCC17_TORTURE_ROOT`, and `GCC_TORTURE_RESULTS_DIR`. `.github/workflows/gcc17-torture.yml` runs it on mainline compiler changes, weekly, or manually; x86-64 and i686 lanes share one hash-verified fastbuild artifact (`lccc-fast-binaries-<sha>`, deliberately NOT the tarball name `lccc-fast-<sha>`) and preserve reports even on failure. |
| `csmith_diff.py`, `yarpgen_diff.py` | Compatibility wrappers: translate the historical flag spellings (`--ccc`, `--clang`, `--gcc`, `--jobs`, `--tests`, `--seed-start`, ...) and forward to `scripts/fuzz_diff.py --engine csmith|yarpgen`. |
| `run_gep_chain_stress.sh` | Compatibility wrapper (bash): forwards to `scripts/fuzz_diff.py --engine stress_suite` over seeds `first..last` (`LCCC_BIN`→`--lccc`, `GCC_BIN`→`--refs`, trailing args→`--opts`), driving `gen_gep_chain_stress.py` for folded-address (chained-GEP) liveness. A seed where GCC and lccc both die by the same signal is reported GEN-BUG and fails the run. |
| `run_slot_stress.sh` | Compatibility wrapper (bash): forwards to `scripts/fuzz_diff.py --engine stress_suite` over seeds `first..last` with the four-way CCC layout matrix declared via `--config-env CCC_NO_TIER2_GRAPH=1`/`CCC_NO_SMALL_SLOTS=1` (default | no-tier2 | no-small-slots | both), driving `gen_slot_stress.py` for stack-slot allocation. |
| `perf_ab.py` | Unified A/B screening engine, default metric = runtime: interleaved AB/BA rounds, empty-process floor exclusion, geomean verdicts, JSON/Markdown. `--metric insns` is the static gate absorbed from the deleted `peephole_ab.py`: every program compiles to assembly in both arms for per-function and total instruction counts AND to binaries whose (exit status, stdout) pairs must match exactly — a pass that changes observable behaviour is a miscompile, exit 1. Presets carry the B-side environment (`--preset peephole_skip` = peephole_ab's default `CCC_PEEPHOLE_SKIP` pass list; `--preset peephole` = whole optimizer off), `--env KEY=VALUE`/`--skip PASS` drive custom arms, `--no-run` disables the execution check, and positional sources extend the corpus. Both arms are scrubbed of ambient `CCC_*`/`LCCC_*` variables. |
| `benchmark_fp_memfold_ab.py` | Thin wrapper forwarding to `scripts/perf_ab.py --preset fp_memfold` (interleaved AB/BA screening with empty-process floor exclusion; see `perf_ab.py`). |
| `benchmark_reduction_vecreg_ab.py` | Thin wrapper forwarding to `scripts/perf_ab.py --preset reduction_vecreg`. |
| `benchmark_vecreg_new_ops_ab.py` | Thin wrapper forwarding to `scripts/perf_ab.py --preset vecreg_ops`. |
| `benchmark_vector_remainder_ab.py` | Thin wrapper forwarding to `scripts/perf_ab.py --preset vector_remainder`. |
| `bench_iv_widen_ab.sh` | Compatibility wrapper (bash): forwards to `scripts/perf_ab.py --preset iv_widen` (same `CCC_NO_IV_WIDEN=1` kill switch, interleaved AB/BA rounds with output equality). Historical interface preserved: positional kernels and `KERNELS`→`--only` (default: the 8-kernel set), `N`→`--reps`, `LCCC_BIN`→`--compiler-a`; `CPU` is accepted but legacy-ignored (perf_ab interleaves the arms instead of pinning separate blocks). |
| `ra_quality_census.py` | Register-allocation quality census: classifies every instruction of every function into RA buckets (insns/rrmov/stkref/push/acc) for LCCC vs GCC (Clang optional), with `.type`-based function boundaries. `--ab-env KEY=VALUE` (repeatable) runs the same-binary A/B experiment absorbed from `ra_ab_census.py`, keeping its exit-1 hot-code regression gate (kernel spill traffic, instruction count tiebreak; `--no-gate` disables); `--kernels` censuses the curated kernel_corpus hot-function map absorbed from `kernel_count.py`, with `--filter` substring narrowing. |
| `ra_ab_census.py` | Compatibility wrapper: forwards to `scripts/ra_quality_census.py --ab-env` (`--env KEY=VALUE` translated; `--opt`/`--cflag`/`--top`/`--json`/positional files pass through). The historical tool never consulted GCC/Clang, so the wrapper turns those optional columns off; the JSON export keeps the historical top-level keys. |
| `kernel_count.py` | Compatibility wrapper: forwards to `scripts/ra_quality_census.py --kernels` — per-function instruction counts of the kernel corpus, LCCC vs GCC, at -O2. Historical positional substring filters become `--filter` values; the `LCCC`/`GCC`/`CFLAGS` environment interface is unchanged; the unified run reports the full RA bucket set plus the LCCC-wins summary. |
| `symstr_migrate.py` | Span-driven `String` → `SymStr` migration helper. Its diagnostic build resolves the manifest-selected Rust channel, locates the persisted Cargo proxy, and denies warnings unless explicitly opted out. |
| `gen_lcccsimd.py`, `strip_scalar_dups.py` | SIMD intrinsic header generation helpers. |

## Codegen kill switches and opt-ins

Environment knobs that gate the SHA/CH-style scalar codegen work; every
one is resolved once per compilation and several gates consult the same
resolution so fold-time prediction and emission cannot drift. Any value
disables (they are presence checks unless noted).

| Variable | Default | Effect |
|---|---|---|
| `CCC_NO_ANDN_FUSION` | unset (fusion on) | Disables the BMI1 3-operand `andn` fusion (backend `detect_and_not_fusions` + `emit_and_not_impl`) AND the bool-mux algebra's defer-to-fusion prediction (`has_scalar_andn`), in one shared resolution. |
| `CCC_NO_BOOL_ALGEBRA` | unset (pass on) | Disables the whole `bit_idioms` bool-mux algebra (CH mux fold, MAJ majority fold, distributive). |
| `CCC_NO_LOWEST_SET_BIT` | unset (fold on) | Disables the `bit_idioms` lowest-set-bit rewrite: `1 << Ctz(y)` becomes `y & -y` (or `(x + 1) & ~x` for a complemented source) plus a `cmov` for the exact `y == 0` arm. A/B and bisection only; the fold is bit-exact, including the zero case. |
| `CCC_NO_RORX` | unset (emission on) | Independent kill switch for BMI2 `rorx` instruction selection (does not affect the ANDN/bool-algebra gates). |
| `CCC_NO_RING_IDENTITIES` | unset (pass on) | Disables the `bit_idioms` ring-identity sub-pass: the `x % C` vs `x` window compare, the sign-mask shift to arithmetic shift, the saturated min/max select, the zero-test select and the complement-of-negation rewrite. A/B and bisection only; every fold is bit-exact in the operand ring. |
| `CCC_IVSR_SCALAR_DERIVED` | **opt-in** (`=1` to enable) | Enables the scalar derived-IV flavor of IV strength reduction (`iv*C` without a GEP). Off by default: measured net runtime loss while latch phi-web parking exists (the PR #602 golden-workload regressions). |

The `-mbmi`/`-mno-bmi`/`-march` front-end flags are the primary control;
these switches exist for bisection and for pinning the pre-BMI shapes in
the golden codegen gates.

## Why more than one oracle

`insndiff.py` and `asmdiff.py` compare against a single local GNU as. That is
the right tool for *correctness* -- "do we agree with binutils" -- but it
cannot answer *quality*. When several encodings of an instruction are legal,
matching GAS only proves we match GAS; it says nothing about whether a shorter
legal encoding exists.

`encdiff.py` exists for that second question. It assembles the same
instruction with every reachable oracle and scores LCCC against the best
result any of them produced. Use `--32 --lccc target/fastbuild/lccc-i686
--casefiles tests/asm-diff/i686/*.casefile` for like-for-like i686 coverage:
local GAS uses `--32`, remote GCC/Clang/ICC/ICX receive `-m32`, and semantic
round trips use objdump's `i386` decoder. The default remains x86-64. That is
how the scale-1 index fold was found:
for `mov -1(,%rdi,1),%rcx`, GAS, clang, gcc and icx all emit 8 bytes while ICC
emits 4, because ICC folds a scale-1 index into the base slot and drops the
SIB byte. Comparing only against GAS would have shown a clean pass forever.

`encdiff.py` batches many instructions into one Compiler Explorer request
(each probe fenced by `ud2` inside its own naked function) and queries the
oracles concurrently, so a 1,900-instruction sweep against five assemblers
takes ~30 s rather than ~2 h.

A shorter encoding is never accepted on size alone. `encdiff.py` disassembles
LCCC's candidate against every distinct shortest oracle encoding and reports
`BEATS` only if all decode to the same canonical instruction. If objdump is
missing, returns an error, or cannot decode either stream, the result is
`UNVERIFIED-BEATS`/`UNVERIFIED-BYTES` and the command exits nonzero; it never
counts as a win. Byte-different equal-length and longer candidates are also
round-trip checked before being called correct or merely longer. `insndiff.py`
does the same for per-instruction comparisons, and `asmdiff.py` checks whole
objects when a case is marked `betterok`. The classifier, fail-closed
behavior, VEX disassembly marker normalization, and reject-group filtering are
pinned by `python3 scripts/test_encdiff.py` in both local and hosted CI. This
is not ceremony -- the first version of the scale-1 fold was 4 bytes shorter
*and wrong* for `%r12`/`%r13`, because those register numbers mean something
different in the base slot than in the index slot, and only the round-trip
check caught it.

Two verdicts record deliberate refusals, so they are not mistaken for work
left undone:

* `DECLINED-FP` -- clang and icx exchange the sources of `vaddps`/`vmulps`
  (and the pd/scalar forms) to reach the 2-byte VEX prefix. x86 FP add and
  multiply are not bit-commutative: with two NaN inputs the result carries
  SRC1's payload. Measured here, `vaddps` over (0x7fc00001, 0x7fc00002) gives
  0x7fc00001 one way and 0x7fc00002 the other, so the byte is not taken.
  Bitwise FP ops (`vandps`/`vorps`/`vxorps`) have no NaN rule and were
  measured bit-identical under exchange, so those ARE shortened.
* `DECLINED-WRONG` -- ICC encodes `xchg %eax, %eax` as the one-byte `0x90`.
  That is not equivalent: the xchg is a 32-bit register write and zeroes the
  upper half of RAX, while NOP does nothing. Measured from
  RAX=0x1122334455667788, `87 c0` leaves 0x0000000055667788.

## Optimization environment knobs (registry)

Codegen kill switches read from the environment, kept here so a bisection
preset or a gate can never rely on an undocumented one.  Passes resolve
them once per translation unit (never per function).

| Knob | Default | Effect |
| ---- | ------- | ------ |
| `CCC_NO_ANDN_FUSION` | unset (fusion ON) | Disables the scalar `Not`+`And` → 3-operand `andn` fusion.  Gates BOTH halves of the contract: the backend emission (`supports_and_not` / `emit_and_not_impl`) AND the middle end's CH-mux fold defer (`scalar_andn_available` in passes/mod.rs) — with fusion off, the mux fold takes the shape back instead of deferring to a fusion that will never fire. |
| `CCC_IVSR_SCALAR_DERIVED` | unset (OFF) | Opt-in: arms the scalar derived-IV strength-reduction flavor (induction `i * stride` → a dedicated `+stride` recurrence).  Default OFF because it measured as a net loss on the current allocator (the extra recurrence raises register pressure past the win); revisit after the Unit-4 allocator work.  Pinned by `test_production_default_leaves_scalar_derived_ivs_off` (default) vs `test_scalar_derived_iv_hash_shape` (armed). |
| `CCC_PEEPHOLE_SKIP` | unset (nothing skipped) | Comma-separated phase-2 peephole sub-pass names to disable for bisection (e.g. `store_alu_fold`).  All peephole passes are ENABLED by default. |
| `CCC_NO_BOOL_ALGEBRA` | unset | Disables the boolean mux/majority algebra rewrites (bit_idioms Patterns B/C). |
| `CCC_NO_LOWEST_SET_BIT` | unset | Disables the `1 << Ctz(y)` -> `y & -y` lowest-set-bit rewrite (with its exact `y == 0` arm). |
| `CCC_NO_RING_IDENTITIES` | unset | Disables the `bit_idioms` ring-identity sub-pass (remainder window compares, sign-mask shift, saturated bounds, zero-test select, complement of negation). |
| `CCC_NO_TWO_BLOCK_UNROLL` | unset | Disables the two-block guard-free unroll (loop_unroll pass B). |
| `CCC_UNROLL_LEGACY_PERSIST_GATE` | unset (fail-closed gate) | Restores the lenient persisting-inner-loop gate: unrecognized nested inner loops (multi-latch, non-Branch latch, no Add/Sub-IV, no exit on the found IV, no init) are skipped instead of vetoing the outer complete unroll. Bisection/A-B and emergency revert only — pinned by `persist_gate_legacy_restores_lenient` and `e2e_persist_gate_csv_nest_rolled_unless_legacy` in `src/passes/loop_unroll.rs`. Note: recognized-but-dynamic bounds still veto in both modes (no legacy skip on that arm). |
| `CCC_UNROLL_GATE_TRACE` | unset | Prints `[UNROLL-GATE] veto fn=… outer=… inner=… arm=…` to stderr on every persist-gate veto (veto paths only; no cost when unset). |

## Oracles

The differential tools need a reference assembler:

```bash
export LCCC_GAS=/path/to/as
export LCCC_OBJCOPY=/path/to/objcopy
export LCCC_OBJDUMP=/path/to/objdump   # used to verify shorter encodings
```

`encdiff.py` accepts each of these as a CLI flag too (`--as`, `--objcopy`,
`--objdump`); the corpus gates use the flags so the parity checker sees the
pinned paths (as + objdump come from one binutils 2.47 build via
`scripts/ensure_gas_247.sh` — the disassembler decides the BEATS/ok
verdicts, so it is as much an oracle as the assembler). The same discipline
applies to the whole-object differential: `asmdiff.py` takes `--objdump`
as well, its `betterok` groups accept a smaller encoding only when the
disassembler proves the two objects decode identically, and both asm-diff
gates (x86-64 and i686) pin the full 2.47 pair — an unpinned objdump is an
unpinned verdict authority, whatever binutils the host image ships.

## AArch64 instruments: gate mode, manual mode, and what proves what

Five scripts measure the AArch64 back end.  Each separates the mode that
*decides* (runs in CI, fails the build) from the mode that *rewrites* (manual:
it needs the pinned binutils prefix, and a generated golden belongs in a
reviewed diff, not in a gate that can silently refresh its own input).

| script | gate mode (where it runs) | manual mode (what only a human runs) |
|---|---|---|
| `aarch64_operand_legality_matrix.py` | `--check`, and `--check-lccc` for the encoder half: `ci.yml` step "AArch64 operand-legality matrix", `ci_local.sh` gates `aarch64-operand-legality` and `aarch64-operand-legality-encoder` (both SKIP without cross-binutils, which is why `ci_local.sh --fast` is not the whole story) | `--regenerate` rewrites `tests/aarch64/operand-legality.tsv` from GNU as; `--print` dumps it |
| `aarch64_legality_probe_differ.py` | no flag, i.e. the pinned-row diff: `ci.yml` step "AArch64 spelling differ and system-register table" | `--json`, the per-spelling dump a new pinned row is argued from |
| `aarch64_sysreg_table.py` | `--check`, same CI step: holds the generated table against the `.def` it came from and against `SYSREGS_DEF_SHA256`, in both directions | `--regenerate` re-emits the table; `--def` picks the source explicitly (prefix, `$LCCC_AARCH64_SYSREGS_DEF`, cache, tarball) |
| `aarch64_doccheck_2026_09.py` | `--xml-dir ... --goldens ...`: `ci.yml` step "Arm A64 documentation cross-check (2026-09)" | the same invocation with the cached-XML defaults, on a dev box |
| `gen_aarch64_dispatcher_goldens.py` | **not run in CI at all**: the generator needs GNU as.  What CI runs is its *output*, via `dispatcher_goldens_ratchet_one_row_per_arm` in `encoder/mod.rs`, which re-derives the arm set with the same rule and re-assembles every row through LCCC's own parser to leaf path -- no cross binutils, no network | the generation itself, on a host with a provisioned `$HOME/.cache/gas-2.47-aarch64-linux-gnu` |

## Evidence producers that are not gates (manual)

Two scripts in this directory exist to *measure* something rather than to decide
something, and neither runs in CI:

| script | what it produces | how to re-run |
|---|---|---|
| `gas_fill_census.py` | per-directive counts of alignment/fill emission inside function bodies, per target, over the compile corpus at several `-O` levels | `GAS247_PREFIX=$HOME/.cache/gas-2.47-<triple> python3 scripts/gas_fill_census.py [target]` (or `GAS247_AUTO_PROVISION=1`); needs the pinned binutils prefix |
| `wrap_ring_sweep.py` | the pointer-ring / wrap-semantics family sweep (per-family verdict table, exit 0 clean, 1 on any divergence or unusable reference, 2 on usage error) | `python3 scripts/wrap_ring_sweep.py` with `LCCC=<binary>` |

They are deliberately not gates: a full census pass is hundreds of
`TU x opt x arch` combinations (the recorded number is 636), and both tools
answer "how often does this shape occur", which is a *cost* question for a design
decision, not an invariant that can break.  The consequence that matters is
stated plainly: **numbers quoted from them in comments are historical evidence,
re-derivable but not enforced** -- so any law that must not regress belongs in a
gate instead.  `ci_local.sh --fast` and `.github/workflows/ci.yml` are the
enforcement surface; `check_ci_gate_parity.py` keeps the two in agreement, and it
does *not* list these two scripts.  When one of their findings becomes a rule (as
the `.zero`/`.fill` classification did), the rule moves into the classifier plus a
unit test, and the census citation stays as the measurement of what that rule
cost.

## The corpus-gate contracts (encdiff)

The `encdiff-corpus` gate runs `encdiff.py` offline, against the pinned
as + objdump 2.47 pair, over the two 64-bit law corpora
(`tests/encdiff-corpus/index-fold-64.insn` and
`data16-branches-64.insn`), with two checked-in contracts:

- **`--expect-histogram tests/encdiff-corpus/expected-verdicts.txt`** —
  the per-verdict row-count baseline. Counts only, never bytes: a BEATS
  row staying BEATS through better bytes needs no update, but any count
  change — a verdict drifting (BEATS → ok-best), a new row, a deleted
  row, a class appearing from zero — fails the gate until the baseline
  is consciously re-recorded **in the same commit that changed the
  corpus**. Zero-count classes are documentation (the class is pinned
  absent) and compare through the nonzero projection on both sides. The
  baseline also carries a `# rows-sha256:` digest over the
  comment-stripped, sorted row texts (with each row's byte-exact pin):
  counts alone are a NET contract — a compensating delete+add of
  same-verdict rows nets to zero — so the digest pins row identity, and
  any row-set or pin change is a baseline change too.
- **`# byte-exact <hex>` row annotations** — the strongest per-row
  contract. The annotation pins the row's LCCC bytes exactly and is
  checked BEFORE any law, canonicaliser or round-trip, so nothing
  downstream (not the dead-segment unification, not a same-rendering
  oracle) can launder a pinned byte difference into a verdict. It exists
  for rows whose bytes are the claim but whose verdict comparison cannot
  see them: the segment+fold flip rows (where the folded view
  legitimately differs from GAS's raw view by the dead prefix byte) and
  the APX x AVX-512 folds (whose correctness argument is byte-identity
  with the base-form spelling). A malformed annotation is a hard error,
  never a silently-ignored comment.

`encdiff.py` additionally needs outbound network access for the remote
oracles. Run it with `--offline` to restrict it to the local assembler; that
is what CI uses when it has no network. Remote answers are cached under
`.godbolt-cache/`, so a tuning loop hits the network once per instruction.

Any recent binutils works. To build a known-good one from source:

```bash
curl -O https://ftp.gnu.org/gnu/binutils/binutils-2.47.tar.xz
tar xf binutils-2.47.tar.xz && mkdir bu-build && cd bu-build
../binutils-2.47/configure --prefix=$PWD/../bu-247 \
  --disable-gdb --disable-gdbserver --disable-sim --disable-readline \
  --disable-libdecnumber --disable-nls --disable-werror \
  --disable-gprofng --disable-gprof --disable-plugins --with-system-zlib
make -j"$(nproc)" MAKEINFO=true all-gas all-binutils all-ld
```

`godbolt.py` uses Compiler Explorer when remote oracles are requested; explicit local references can also be compared offline:

```bash
scripts/godbolt.py list --filter icx
scripts/godbolt.py compare kernel.c --flags "-O3 -march=x86-64-v3" \
    --local ./target/fastbuild/lccc

# Offline assembly comparison; paths may be compilers or launchers.
# --offline fails before dispatch if any remote or provisioned oracle is requested.
# This is static assembly only, not a correctness or performance verdict.
scripts/godbolt.py compare --offline kernel.c --function hot_loop --flags=-O2 \
    --local target/fastbuild/lccc --oracles '' \
    --local-ref gcc=/path/to/gcc-16 --local-ref clang=/path/to/clang-23 \
    --artifact-dir /tmp/local-comparison

# AArch64 structural comparison (ARM64 GCC 16.1 on Compiler Explorer):
python3 scripts/codegen_oracle.py test.c --function hot_loop \
    --arch aarch64 --local target/fastbuild/lccc-arm \
    --local-flags '-O2' --flags '-O2' \
    --artifact-dir /tmp/aarch64-oracle

# For current GCC 17 development sources (pin the printed full commit on reruns):
python3 scripts/ensure_gcc_torture_git.py
# --suite ~/.cache/lccc-gcc17/gcc/testsuite/gcc.c-torture/execute
# uses that checkout instead of the GCC 16.2 release corpus.
# Reproduce the dedicated GitHub x86/i686 compile+execute lane locally:
GCC_BIN=gcc-16 GCC_TORTURE_FLAGS=-O2 GCC_TORTURE_JOBS=2 \
    bash scripts/ci_gcc17_torture.sh x86_64
GCC_BIN=gcc-16 GCC_TORTURE_FLAGS=-O2 GCC_TORTURE_JOBS=2 \
    bash scripts/ci_gcc17_torture.sh i686
# The lane's corpus revision is PINNED (c94b688a..., the revision the campaign
# report was measured against) and its reference compiler must be GCC 16.2, so a
# rerun means the same thing next week. GCC17_TORTURE_REF overrides the corpus:
# refs/heads/master tracks trunk, or pass a report's gcc_checkout_head to
# reproduce an exact revision.
# The native runner is NOT DejaGnu: conditional target directives, expected
# diagnostics, LTO option matrices, and nested suites are not fully interpreted.
# JSON schema 2 records selection, flags, binary/source hashes, and host identity.
# It checkpoints atomically every 25 cases and immediately on failures; only
# complete=true without fatal errors is a finished run. Reference multilib/tool
# preflight failures and all-skipped runs exit 2, never green. Resource failures
# retain a reference-fail verdict rather than disappearing into skips.

# Native x86-64: compile with LCCC, link through standalone lccc-ld, execute.
# The default is the full -O0/-O1/-O2/-O3/-Os matrix; use -j2 on constrained VMs.
python3 scripts/x86_gcc_torture.py \
    --suite /path/to/gcc/gcc/testsuite/gcc.c-torture/execute \
    --lccc target/fastbuild/lccc --lccc-ld target/fastbuild/lccc-ld \
    -j2 --json /tmp/x86-torture.json --failure-log /tmp/x86-torture.log

# AArch64 differential execution. The harness rejects every assembler whose
# --version is not 2.47.
python3 scripts/aarch64_execute_suite.py \
    /path/to/gcc/gcc/testsuite/gcc.c-torture/execute \
    --lccc target/fastbuild/lccc-arm \
    --assembler /path/to/binutils-2.47/aarch64-linux-gnu-as \
    --jobs 2 --flags=-O2 --json /tmp/aarch64-torture.json

# With Debian's aarch64-linux-gnu cross GCC/glibc and static Clang references,
# use --sysroot=/ (the cross driver layout is already target-qualified). Using
# /usr/aarch64-linux-gnu as Clang's sysroot can double-prefix linker-script paths.
# An explicitly requested Clang compile/link failure is HARNESS-ERROR, and an
# ORACLE-DISAGREE is nonzero; neither can silently degrade to a GCC-only PASS.
```

## Contracts the gates now pin (2026-10-13 audit session)

These three behaviours were audit findings; they are documented here because
each one is a contract build systems rely on, and each now has a gate:

* **`-o -` is stdout** for the text modes (`-S`, `-E`, `-M`/`-MM`, `-dM`), and
  `-MF -` sends the make rule to stdout. Object/executable modes reject
  `-o -` with a diagnostic instead of writing a file named `-`
  (`tests/regression/check_o_stdout.sh`).
* **Temporary files are private**: every temp object/assembly file is reserved
  inside an atomically created 0700 directory under `$TMPDIR`
  (`src/common/temp_files.rs`), so a pre-placed symlink or file at a predicted
  path can no longer redirect a write.  A TMPDIR that cannot be created is a
  normal driver error, never a panic
  (`tests/regression/check_temp_symlink.sh`).
* **`__builtin_cpu_supports` folds against the effective target feature set**,
  not against a CPU allowlist: `-march=x86-64 -mno-avx2` answers 0 for
  `"avx2"`, AArch64/RISC-V answer 0 for x86 feature names, and only features
  the driver can statically guarantee answer 1
  (`tests/regression/check_cpu_supports_runtime.sh`).

## New tools (and what they are for)

| script | purpose | gate |
|---|---|---|
| `audit_census.py` | static ratchet over panic-class sites, `unsafe`, giant functions, `CCC_*`/`LCCC_*` knobs, `#[ignore]`s | `ci_local.sh` (fast), CI | 
| `check_repro_replay.py` | replays every `artifacts/repros/*.c` against GCC with the ISA pinned on BOTH compilers (see `artifacts/repros/README.md`); `repros_replay.py` was the pre-merge variant and is gone | `ci_local.sh` (fast), CI |
| `verify_fib_perf.sh` | Fibonacci rec2iter perf probe (moved from `tests/`) | manual |
| `../tools/workspace_dirs.sh` | resolves the persisted workspace dir (`LCCC_WORKSPACE`, `/home/user`, `/mnt/user-data/outputs`) | sourced by the snapshot tool |


## Whole-tree audit replay (2026-10-14)

These tools support the A14 queue in root `backlog.md`; they do not certify
that a static inventory is a semantic audit.

- `python3 scripts/audit_tree_inventory.py --revision COMMIT --output REPORT`
  records every tracked blob by subsystem and exact source/tool identities,
  independent of build products and uncommitted changes. `--selftest` checks
  immutability and whitespace-bearing filenames as well as subsystem grouping.
- `python3 scripts/audit_contract_probes.py --compiler-probes --extended --check`
  replays the new census/type/ISA/assembler/linker/PGO/debug contracts. Requires
  already-built fastbuild target binaries, Clang, ld.lld, readelf and strace;
  no hidden compiler build or download. `--json REPORT` preserves observations.
  **Expected to exit nonzero on the audit baseline**: these are open defects,
  not a passing CI gate. `--selftest` validates process classification without
  requiring compiler tools. Unknown tools/timeouts/panics cannot count as a
  successful negative compiler test.
- `tests/workloads/zlib-ng-2.3.3/isolate_maketrees.py` replays actual Ninja
  compile flags on an existing configured source tree, mixing GCC/LCCC TUs
  and optimization levels. It creates only its own private scratch directory;
  output must be a fresh directory. Consult `--help` and the standalone
  reproducer under `engineering/evidence/2026-10-14-audit/zlib-maketrees/`.

The evidence directory retains commands, exact source hashes, raw original
reports, licenses and known-wrong outputs as **negative evidence**. Do not
update expected outputs to the wrong result or remove fail-closed checks to
make an audit replay appear green.


### Opt-in Intel ICX 2026.2 local oracle

`codegen_oracle.py --oracles icx2026.2` and `oracle_profile.verify` can use
Intel's 2026.2 release line locally; default remote oracle selections are
unchanged. This release-line key is not a claim that Compiler Explorer's
`icx` alias points at the same compiler.

```sh
scripts/ensure_icx_2026.sh
# Or use an existing compiler instead of provisioning:
LCCC_ICX_BIN=/absolute/path/to/icx python3 scripts/codegen_oracle.py \
  tests/regression/generator_map_vec.c --all-functions --no-local \
  --oracles icx2026.2
```

Provisioning requires Linux `apt-get`, `apt-cache`, `dpkg-deb`, Python 3 and
APT's HTTPS/signature support. It uses Intel's oneAPI APT repository, **not**
a Launchpad PPA. An isolated APT configuration fetches Intel's HTTPS-published
signing key and authenticates repository metadata, selects the newest
2026.2.x package build, and verifies the five downloaded archives by SHA-256.
The same exact package build is required across the compiler/support set;
missing or inconsistent releases fail closed, without falling back to 2026.1.
The signing-key trust bootstrap is HTTPS, not an independently pinned fingerprint.

Installation is rootless: host APT sources, installed packages and system
keyrings are not modified. A cross-process lock protects each prefix. The
compiler's predefined macro must agree with the package version **before**
the staged install is atomically published. The private prefix defaults to
`$HOME/.cache/intel-icx-2026.2`, or `$LCCC_ICX_PREFIX`; an explicit positional
prefix overrides both. `LCCC_ICX_MIRROR` can select another HTTPS repository,
but its metadata must still authenticate with Intel's key.

`PREFIX/.lccc-icx.json` records exact package versions and archive checksums,
repository metadata/key digests, and compiler identity. Existing prefixes are
validated, frozen snapshots. To explicitly refresh to a newer 2026.2.x build,
use a new prefix rather than mutating an existing measurement environment.
Oracle manifests record the actual patch release, macro, executable digest,
and the provisioning manifest when present; a release-line label is not binary
provenance by itself.

Offline tests run without network access or real compiler invocations:

```sh
python3 scripts/test_local_oracles.py
python3 scripts/test_icx_provision.py
```

The real probe test requires **both** `LCCC_TEST_REAL_ICX=1` and
`LCCC_ICX_BIN`. An explicitly requested real probe fails on a wrong or unusable
compiler instead of being converted to a skip. No compilation is performed
by that probe, only predefined-macro preprocessing.

### Switch lowering red-team contracts

`python3 -m unittest scripts.test_aarch64_execute_suite` exercises oracle
failure classification and bounded mismatch diagnostics without external tools.
`tests/regression/check_builtin_floor_narrowing.sh` separates call-routing
contracts from a numerical binary32 oracle: 42 signed boundary inputs and
65,536 deterministic samples, each checked through three call forms, on both
x86 ABIs at five optimization levels. It checks quiet-NaN classification and
signed zero, not NaN payloads or floating-environment exception flags.

`python3 tests/regression/check_switch_cross_execute.py` runs nine self-checking
switch fixtures with LUT lowering on/off under AArch64 and RV64 QEMU (36 runs).
It requires the fastbuild target drivers, Clang 23.1.4, GNU target linkers or LLD,
QEMU user emulators, and host C declaration headers. The executable uses a
minimal syscall startup and no cross libc; printf/puts are no-ops, so acceptance
is based on each fixture's exit-code oracle, not its printed output. `--clang`,
`--qemu-dir`, and `--lccc-dir` select tool locations; `--ld` selects one GNU/LLD
linker supporting both target emulations. Discovery prefers GNU triplet linkers,
then system LLD, then the building Rust toolchain's bundled `rust-lld`. These
freestanding fixtures do not require cross GCC or cross libc packages.
`python3 tests/regression/test_check_switch_cross_execute.py` tests discovery
and its fail-closed missing-tool diagnostics.
The assembler/linker CI lane runs this gate after cross-toolchain provisioning.

For a compile-time lookup witness (not a timing threshold gate):

```sh
cargo test --profile fastbuild --lib switch_definition_index_scaling_witness \
  -- --nocapture
```

The witness runs in the normal unit suite (no timing threshold) and asserts equality between linear-scan and indexed definition
results, and reports both timings for 2,000 queries. Production indexes only
original switch operands, rejects multiple definitions, and rebuilds return
path memoization after each IR transformation.
