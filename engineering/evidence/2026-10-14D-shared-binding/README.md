# Shared binding: measured evidence

Final binary-specific results use `-final` names and `callgrind-final/`.
Earlier files retain pre-strength checkpoint evidence; `before` means an
independent upstream arm and `initial` identifies a superseded experiment.
Final exact-tree fast CI and strict Clippy results are in the external delivery
receipt/log so recording them cannot change the tree whose stamp they describe.

## Callgrind

Bounded registered kernels, O2/x86-64-v3/-fwrapv, equal-length A/B paths.
These are whole-program simulated counts, including startup/library work.
No PMU, 14700KF timing, full-package result or universal speedup is inferred.

| Kernel | Candidate / upstream Ir | Candidate / distro GCC Ir | Ratio vs GCC |
|---|---:|---:|---:|
| gzip_crc32 | 801055 / 801055 | 801069 / 808232 | 0.991137 |
| zlib_ng_adler32 | 463894 / 463894 | 463866 / 510908 | 0.907925 |
| expat_xml_scan | 1666721 / 1666721 | 1666776 / 1442124 | 1.155779 |
| sqlite_varint | 833128 / 833128 | 833169 / 780227 | 1.067855 |
| linux_find_bit | 5957762 / 5957762 | 5957817 / 5413764 | 1.100494 |
| glibc_memcmp | 722937 / 722937 | 722951 / 692255 | 1.044342 |

Within each baseline pair, executable hashes and every simulated event total
are identical. Separate runs can have different startup counts; do not mix arms.
`callgrind-final/` includes the raw `.cg` files and per-pair manifests. An external
archive additionally preserves executable copies; its SHA-256 is recorded.

## Reproduce from the checkout

Build base and candidate separately using `scripts/build_lccc_fast.sh`, with
`CARGO_PROFILE_FASTBUILD_DEBUG=0 CARGO_INCREMENTAL=0`, O1/j2 and active swap.
Never reuse the candidate executable as the baseline. Exact hashes and source
files are in `build-provenance-final.json`. `$R` below is the independent base binary.

```bash
python3 tests/linker/test_riscv_shared_binding.py
python3 tests/linker/check_riscv_shared_binding.py --lccc target/fastbuild/lccc-riscv --oracle ld.lld --json /home/user/shared.json
python3 tests/linker/check_riscv_relocation_contract.py --lccc target/fastbuild/lccc-riscv --json /home/user/relocations.json
python3 tests/regression/check_pgo_io.py --lccc target/fastbuild/lccc --json /home/user/pgo.json
python3 tests/linker/check_riscv_shared_loader.py --lccc target/fastbuild/lccc-riscv --clang clang-19 --linker ld.lld --qemu qemu-riscv64 --sysroot /usr/riscv64-linux-gnu --json /home/user/loader.json
python3 scripts/codegen_sweep_compare.py --base "$R" --new "$PWD/target/fastbuild/lccc" --roots tests/benchmark/programs --levels=-O0,-O1,-O2,-O3 --jobs 2 --out /home/user/sweep
python3 scripts/callgrind_corpus.py --mine "$PWD/target/fastbuild/lccc" --ref "$R" --only gzip_crc32,zlib_ng_adler32,expat_xml_scan,sqlite_varint,linux_find_bit,glibc_memcmp --out /home/user/callgrind
CARGO_PROFILE_FASTBUILD_DEBUG=0 CARGO_INCREMENTAL=0 bash scripts/ci_local.sh --fast
```

Change `--ref` to `gcc` for the separately labeled distro comparison.
Pass the actual selected tool paths; the supplemental loader test requires
an installed RV64 glibc sysroot. No tests here silently skip unavailable tools.

CE `oracle.json` is whole-TU O2/v3 evidence from GCC16.2, Clang23.1 and ICX.
`icc.json` is separate O2/core-avx2 evidence. `oracle-version-probes.json`
retains actual compiler diagnostics, not just channel labels. Assembly from
`oracle_asm.py` and `tune_oracle.py` is retained. `density.json` explicitly
marks loop/work equivalence unverified; its ratios are not hardware speedups.
`siphash.txt` correctly reports GCC did not emit the requested standalone
function: a missing function is not zero instructions or a certified win.

## Pinned local competitors (final compiler)

Archive and executable identity: `pinned-tool-provenance.json`.
All twelve compiler/workload comparisons completed with matching program output.
Each pair uses the same environment; startup counts can differ between separate
runs, so never substitute a candidate count from another table or run.

| Kernel | LCCC / GCC16.2 Ir | Ratio | LCCC / Clang23.1 Ir | Ratio |
|---|---:|---:|---:|---:|
| gzip_crc32 | 801272 / 805829 | 0.994345 | 801028 / 603111 | 1.328160 |
| zlib_ng_adler32 | 464091 / 502922 | 0.922789 | 463907 / 447685 | 1.036235 |
| expat_xml_scan | 1667015 / 1439749 | 1.157851 | 1666776 / 1367941 | 1.218456 |
| sqlite_varint | 833394 / 755321 | 1.103364 | 833133 / 721642 | 1.154496 |
| linux_find_bit | 5958056 / 5075024 | 1.173996 | 5957817 / 4314732 | 1.380808 |
| glibc_memcmp | 723190 / 689576 | 1.048746 | 722910 / 700507 | 1.031981 |

Raw counters/manifests: `callgrind-pinned/`. Exact LLD23 semantics are in
`shared-binding-lld23.*`; the two loader arms are `loader-pinned-{before,after}.*`.

To restore the reference tools, fetch the two URLs in the provenance file,
verify each archive against its recorded SHA-256, and extract under
`/home/user/.cache/oracles` (or use explicit equivalent tool paths). On this
constrained host Clang extraction was restricted to `bin/`, shared `.so`
libraries, and `lib/clang/` resource files; static development libraries were
not needed. Deleted download archives are reproducible from their pins. Install
host multilib libc development files and the RV64 glibc sysroot separately.
Run the compilers' 64-/32-bit canaries; verify the exact banners before use.
Set `GCC` explicitly for fast CI as shown in the follow-up: otherwise its named
address-space gates correctly reject the older distro GCC instead of skipping.
