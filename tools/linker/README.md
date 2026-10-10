# Linker verification gates

Four tools, each of which produced a verdict that changed code in this series.
They live in the tree rather than in a scratch directory so a reviewer can
re-derive every claim in the commit messages instead of trusting them.

## `reloc_number_audit.py` — the relocation *number* gate

Checks every relocation constant in the tree against the authoritative source
(`/usr/include/elf.h` — the ABI headers binutils and the kernel are built from)
and reports any constant whose value disagrees with its name.

    python3 tools/linker/reloc_number_audit.py     # expect: 0 wrong

It found six on first run: five rotated AArch64 `R_AARCH64_TLSLE_MOVW_TPREL_*`
constants — whose `_NC` ("no check") suffixes made the resulting mis-encodings
silent, since the arm that should have rejected an out-of-range value was the
arm that got the wrong number — plus an i686 `R_386_TLS_LE_32` and a name,
`R_386_32S`, that does not exist in the i386 ABI at all. Run it after touching
any `R_*` constant.

## `oracle_cmp.py` — differential comparison against another linker

Links the same inputs with lccc-ld and with an oracle (GNU ld by default) and
reports divergences both in the accept/reject verdict and in the bytes written.

    python3 tools/linker/oracle_cmp.py --help

## `setup_oracles.sh` — build the corroborating linkers

Pinned oracles (user directive): **GNU ld 2.47** (bfd + ld only), **mold
2.42.1** built with `-DMOLD_TARGETS='X86_64;I386'` (mold's own CMake cache
variable — the x86 preset cuts template instantiation ~10x on the 2-vCPU host),
**LLVM lld 23.1.x** (apt.llvm.org release build; the script asserts the `23.1.`
banner) and wild at git HEAD. The binutils release download is SHA-256-pinned
like the GAS oracle. Its actual banner is `2.47.20260726`, which the version
check accepts explicitly alongside `2.47`; it still rejects `2.470`, `2.47.1`,
and other dated snapshots. This is not permission to substitute any version
whose directory happens to contain `2.47`.

`WITH_WILD=0` removes wild from the oracle set
for that run: no build, no `wild` wrapper (a stale one is deleted) and no
inventory entry; an already-built binary stays cached for a later
`WITH_WILD=1` run. Installed binaries live
in the persisted `/home/user/artifacts/oracles`; tarballs and build trees go to
the snapshot-excluded `~/.cache/lccc-oracle-src` and are deleted after install.
`ORACLES.lock` is a generated inventory, not an input: every run rewrites it
with the exact `--version` banners of the oracles it exposed (exactly those
with wrappers in `/home/user/artifacts/bin`); the pins live in the script. GNU ld is
the *primary* oracle — every "matches GNU ld" claim in this series was produced
with it; the others corroborate. Idempotent.

    bash tools/linker/setup_oracles.sh

## `recover_env.sh` — restore the build environment

Toolchain paths, the 8 GiB swap file (this work runs on a 2-core, ~2 GB machine)
and the fastbuild preset. Run first in a fresh session.

    bash tools/linker/recover_env.sh

## The suites themselves

    # linker acceptance suite: 178 pass / 0 fail / 0 warn, differential vs bfd
    python3 tests/linker/run_linker_tests.py --lccc target/fastbuild/lccc-x86

    # unit tests, including the --defsym classifier and the field-width table
    cargo test --profile fastbuild --lib

    # the reloc tag alone: r_offset sweeps + per-field-width range verdicts
    python3 tests/linker/run_linker_tests.py --lccc target/fastbuild/lccc-x86 --tag reloc -v

Always build with the fastbuild preset (`scripts/build_lccc_fast.sh`); a debug
build of this tree does not fit in 2 GB.
