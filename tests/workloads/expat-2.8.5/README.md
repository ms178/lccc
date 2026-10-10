# Expat 2.8.5 — full-project correctness gate

```sh
LCCC="$PWD/target/fastbuild/lccc" \
ARTIFACT_DIR=/path/to/persisted/expat-evidence \
WORK_ROOT=/path/to/largest/work-drive \
bash tests/workloads/expat-2.8.5/run.sh
```

Requires active swap, native GCC, CMake, Ninja, curl, GNU timeout/tar,
Python 3 and the built LCCC. Builds are limited to two workers. The library,
examples, xmlwf and upstream C tests are compiled with each C compiler at
`-O2 -DNDEBUG`. This configuration contains only C sources; it is **not**
a C++ compiler test. Only the static, native UTF-8 configuration
is covered; shared-library, wchar, cross-target and fuzz configurations remain
separate work. It is a correctness gate, not a performance benchmark.

Both complete upstream CTest runs must contain tests and have no failures or
skips. The xmlwf binaries then parse five deterministic documents (attributes,
namespaces, internal entities, UTF-8/CDATA and a multi-buffer document), with
byte-identical nonempty canonical output required. Three malformed inputs
must return the **specific** XML rejection status, not merely any nonzero
exit. Command timeouts, signals, absent tools, stale/empty outputs and all-
skipped tests fail closed. An interrupted/failed run leaves `complete:false`;
only completion of every stage publishes a passing summary. Logs, CTest XML,
compiler digests and xmlwf binaries are retained; `KEEP_WORK=1` retains the
private source/build tree for diagnosis. The gate never deletes a user-provided
work root.

## Source pin and recipe discrepancy

Selected from `ms178/archpkgbuilds` commit
`39fcd576582ee3320379882965a549a1fe6120e1`, `packages/expat/PKGBUILD`.

- Upstream release: <https://github.com/libexpat/libexpat/releases/tag/R_2_8_5>
- Archive: `expat-2.8.5.tar.bz2`.
- SHA-256: `952c03c33a6b337f12dae7a9b0f9dee86f867550d35c994d6bdaaddd37dc8454`.
- SHA-512: `2184bb203df9f9f5bd49464397c6dfa70abf5a15ba5855af926e1ae7c2a0a0f72d0f11c8ae417be526a700c1ae9101c0a3d81a86fbec82f16af513868444b2ec`.

The package recipe's SHA-512 **does not match** these release bytes. It was
not ignored silently: the GitHub release asset's SHA-256 agrees, and the
2026-09-22 detached signature was verified on 2026-10-10 against the recipe's
pinned primary key `3176EF7DB2367F1FCA4F306B1F9B0E909AF37285`, signing subkey
`CB8DE70A90CFBF6C3BF5CC5696262ACFFBD3AEC6`. The session evidence retains the
release metadata and GPG `VALIDSIG` record. This gate verifies the resulting
exact archive SHA-256 on every run; it does **not** claim to recheck the PGP
signature on every invocation. Archive path/type checks provide additional
defence in depth before extraction. The recipe itself still needs correction
in its own repository.

The related integrated benchmark `expat_siphash24` is an independently scoped
kernel derived from this release's CC0 `lib/siphash.h`, not a replacement for
this full-project gate. See `tests/benchmark/WORKLOAD_PROVENANCE.md`.
