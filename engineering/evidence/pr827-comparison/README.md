# PR827 comparison evidence

- `inventory.json`: exact fixture/value-domain counts, latest Rust release API result, actual local tool versions/Zig executable hash, swap observation, and immutable-tree diff size.
- `lld22-current-contract.json`: all 466 real LLD invocations with fixture hashes, command lines, actual statuses and diagnostics. Acceptance agrees for 230/230 valid invocations; all 64 new range/alignment controls reject. Remaining malformed disagreements are retained, never relabeled as passes.
- `host-harness-gcc.json`, `host-harness-clang.json`: nine executed host/reference tests per compiler, not local LCCC integration claims.

These are immutable local experiment snapshots. The three-way built-compiler comparison is performed by the scoped hosted research job in `.github/workflows/pr826-redteam.yml`; its source/binary identities, full contract rows, PGO probes, syscall/memory-tool logs and 270 object-compilation records are retained as GitHub artifacts and compact API notices. The isolated compiled run 38603881290 completed successfully on 987bca9; `compiled-comparison-summary.json` preserves its actual 364/402/466 totals, runtime rows, distinct binary hashes and 90/90 object identities. `compiled-contract-summary.json` records the independent 466/72/9 checks and 45 workloads at each O2/O3. `compiled-validation-jobs.json` anchors job identities to the tested head. `cross-source-preflight.json` records fail-closed duplicate-binary/optimized-Python controls. Full raw records remain in hosted artifacts 11914488320 and 11914433303, not locally downloaded. Earlier shared-build-fingerprint results were invalidated and discarded. Historical successful PR826 checks do not validate later code changes.

Reproduce the local real-linker experiment:

```
python3 tests/linker/check_riscv_relocation_contract.py --oracle-only --oracle /path/to/ld.lld --json results.json
```

A nonzero result on the full malformed contract is disagreement evidence. To gate only well-formed byte/ISA controls, add `--only valid-`.

Reproduce the compiled comparison with Rust 1.99.0, Debian GCC 16.2.0, strace, Valgrind and both immutable source objects available:

```
bash tests/linker/check_pr827_comparison.sh
```

It uses git archives and never changes the working branch. It is historical scoped research, not a universal requirement that subsequent optimizer work reproduce an old object's bytes. Generated binaries/objects and large tool caches remain outside Git.
