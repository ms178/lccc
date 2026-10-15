# Post-reset upstream and provisioning red team

The S90 Git bundle recovered exact HEAD04d0b37b and tree72b6cd96 after the
workspace reset. The cumulative patch hash matched; the source tar did not:
it was the older S87 archive. Recovery correctly refused it. Fetching the
upstream base plus the verified bundle restored S90; regenerating its source
archive reproduced the original recorded SHA256d9149636... byte-for-byte.
The interrupted FAST run has no surviving completion receipt and is not passed.
Historical containers were compacted losslessly to keep persistence below the
128MiB limit; original gzip bytes were reconstructed and checked before removal.

A fresh Git fetch and uncached API lookup still both reported mainf07f4c51.
`git rebase origin/main` explicitly returned up to date. No invisible new commit
is assumed. archpkgbuilds advanced tod315f524, but the latest zstd recipe still
pins1.5.7/pkgrel16 and the same source/patch SHA256 identities measured previously.

## Confirmed provisioning defect

The first cache check used `_prefix_provisioned`, including consumer inputs.
The second check after taking the lock duplicated only binary/provenance checks.
A valid cached assembler pair missing `aarch64-sys-regs.def` therefore escaped
recovery: first check refused it, second check returned success without supplying
the file. This contradicts the script's claimed single-authority cache contract.

**Disagree** with that duplicate, weaker post-lock test. Both checks now call
the same complete-cache predicate. This retains the lock, supply-chain hash,
binary identity and functional canaries; it does not waive any tool version.
Three orchestration tests execute the actual provisioner with only independent
binary/provenance validation mocked and all networking blocked. Before repair,
the incomplete-cache case incorrectly returns0 (one failure); afterwards all
three pass. Complete caches and caches completed by the lock owner remain
reusable. The provisioner's existing self-test also passes. These mocks test
control flow, not an assertion that fake tools are genuine pinned assemblers.

## Restored environment

Debian GCC/G++16.2.0-3, Rust/cargo1.99.0, rustfmt1.10.0, Clippy0.1.99,
Clang/lld23.1.3, GAS/bfd2.47, Valgrind3.27.1 and6GiB active swap were restored.
Debian's mold candidate is3.0.0; the requested2.42.1 official release binary was
installed separately and selected explicitly. No mold source build was done;
future source builds retain the required X86_64/I386-only CMake preset.
Toolchain installations live under the snapshot-excluded cache with stable
symlinks, so gigabytes of tools cannot displace the user's patch/source evidence.
