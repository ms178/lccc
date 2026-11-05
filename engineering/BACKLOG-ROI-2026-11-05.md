# Backlog ROI sweep — 20 measured items (2026-11-05)

Base: `3596c7bf4d39c8fb33befcbf0a07be6f668cf7c3` (`main`, post-merge #1109).
Method doctrine: [`engineering/agent/RULES.md`](../engineering/agent/RULES.md).
Executable half: [`scripts/check_backlog_roi.py`](../scripts/check_backlog_roi.py)
(20 invariants, 20 negative controls, `--self-test`, `--json`).
Regression half: [`tests/corpus/test_backlog_roi.py`](../tests/corpus/test_backlog_roi.py).

## Why these 20

The backlog is 3,045 lines in `backlog.md` plus a separate ranked catalog in
`engineering/agent/BACKLOG.md`.  A textual "is it closed?" reading is exactly
the guesswork the method doctrine forbids, so every candidate was **probed**
against the rebuilt compiler and the live tree before admission:

| Measurement | Result |
|---|---|
| `cargo build --profile fastbuild --locked`, `RUSTFLAGS=-D warnings` | exit 0, 4m20s, zero warnings |
| Toolchains restored from `harness-cache-8` | GCC 16.2.0, Clang 23.1.5, GDB 18.1, Rust 1.99.1, mold 3.0.0, QEMU 11.1.3, binutils 2.47 |
| GP-22 ledger rows executed against the binary (`-S` SHA-256 + `-dM -E` bytes) | 17 rows, **5 false classifications**, 17 dead `probe_id`s |
| GCC 16.2 / Clang 23.1.5 `-dM -E`, 4 dialects | 30 lccc-only, 13 GCC-only, 28 value-spelling differences (all numeric synonyms) |
| `tests/regression/sse2_codegen_matrix.json` | 122 rows, provenance still named Clang 23.1.4 |
| `gcc-16 -m32` before/after the pinned bookworm multilib bundle | FAIL (`bits/libc-header-start.h`) → PASS (4-byte pointer) |
| Full corpus | 1268 tests, 0 failures, 6 skips |

Selection rule: **ROI = provable defect × cost of the gate that prevents its
return ÷ implementation risk.**  Items whose premise was already false on this
tree were demoted to *ratchet* (keep them closed) rather than reimplemented;
items whose measurement was stale were re-derived.  No item is marked closed
without a test that fails when it regresses.

## The 20 items

| # | ID | Backlog source | Measured state before | Decision & change | Regression |
|---|---|---|---|---|---|
| 1 | GP-22/1 | `backlog.md` §12 | 17 ledger rows, 5 contradicted the binary | Reclassified 3 `Inert`→`Refused` (`-fprofile-arcs`, `-ftest-coverage`, `-static-pie`) and 2 `Inert`→`Implemented` (`-Ofast`, `-Og`), each verified by macro/assembly deltas | `option_effect_ledger.py` + `test_option_effect_ledger.py` |
| 2 | GP-22/2 | §12 | 17 `probe_id`s, **zero consumers** | New probe registry; every accepted row must resolve to an implemented probe | `static_problems` + 16 tests |
| 3 | GP-22/3 | §12 | registry could gain orphans silently | Exact set equality between ledger probes and registry; duplicates rejected | check 3 + negative control |
| 4 | GP-22/4 | §12 | priority set asserted only in prose | 7 priority spellings must all be ledgered | check 4 + Rust unit test |
| 5 | GP-22/5 | §12 | no gate executed the ledger | `check_driver_option_contract.sh` now runs `option_effect_ledger.py --lccc "$CCC"` in the existing fast lane | check 5 + gate run (exit 0) |
| 6 | GP-22/6 | §12 | local/hosted parity unaudited for this gate | Both `ci_local.sh` and `ci.yml` must name the driver gate | check 6 |
| 7 | GP-23/1 | §12 | macro-parity gate already existed and is wired | **Ratchet, not reimplementation**: pin the three budget tables and both wirings | check 7 |
| 8 | GP-23/2 | §12 | `BUFSIZ` positive control could be deleted | Pin the `FORBIDDEN_LIST` and its `BUFSIZ` sentinel | check 8 |
| 9 | GP-05 | §12 | historical drift `__GNUC__ 14`; live value is 16 | Pin `__GNUC__` to the GCC 16 major in `predefined_macros.rs` | check 9 |
| 10 | MB-01/1 | `backlog.md` §4 | `-fno-plt` described as a silent no-op; it emits GOTPCREL | **Closed upstream**; ratchet the effect probe so the stale claim cannot return | check 10 |
| 11 | MB-01/2 | §4 | same for `-fvisibility=hidden` | Ratchet the `.hidden` effect probe | check 11 |
| 12 | MB-40 | §9 | blanket `arg if arg.starts_with("-f") => {}` made refusals unreachable | Ratchet: strip comments, then reject that arm; also require `lookup` consultation | check 12 + ledger static |
| 13 | MB-41/1 | §9 | backlog headings had no parseable-ID invariant | Parse `### ID · class · evidence` headings; ≥20, well-formed ids | check 13 |
| 14 | MB-41/2 | §9 | `R` items with no reproducer invisible | Every `R` item must carry a fenced reproducer or an inline command | check 14 |
| 15 | MB-42 | §9 | capability probe's evidence basis could rot | Pin `NEGATIVE_CONTROLS` and the exit-status record | check 15 |
| 16 | MB-45 | §9 | "gap" counts could lose their oracle | Pin the named GCC/Clang comparison oracle in the performance catalog | check 16 |
| 17 | FOLLOWUP-7/2 | §7 item 2 | SIMD matrix provenance said "Measured with Clang 23.1.4" | **Re-derived** all 122 rows against Clang 23.1.5 / GCC 16.2.0 (`--update-baseline`); `--check` and `--self-test` PASS | `test_oracle_provenance.py` (6 tests) |
| 18 | FOLLOWUP-7/2b | §7 item 2 | `--update-baseline` **dropped the note** on regeneration | Generator now derives `note` + `rederived_with` from the banner that measured the data | 5 negative controls |
| 19 | FOLLOWUP-7/7 | §7 item 7 | CE serves 23.1.0 while the pin is 23.1.5 | **Closed upstream**; ratchet the alias-table provenance instruction | check 19 |
| 20 | CACHE-REDTEAM/1 | post-merge #1109 audit | stale cache tag before #1109: 5 obsolete blobs, no GDB | Ratchet the reviewed lock (tag 8, GDB entry, zero obsolete filenames) and the publisher's content guard | check 20 + `test_agent_cache_publisher.py` |

## Red-team findings on the merged patch (#1109)

1. **The ledger was decorative.**  `probe_id`s existed only as strings in
   `option_table.rs`; no gate consumed them.  Five rows were factually wrong.
   A trust contract that nobody executes is worse than no contract, because it
   invites a reviewer to believe it.  Fixed by item 1/2/5.
2. **`--update-baseline` destroyed provenance.**  It rebuilt `provenance`
   from scratch with only `recorded_with`, so the human-readable note naming
   the measuring oracle vanished on the very command a future pin bump would
   run.  That is a self-invalidating ratchet.  Fixed by item 18.
3. **The backlog overstates open work.**  MB-01's `-fno-plt` no-op,
   GP-23's missing gate, §7-7's CE provenance and GP-22's coverage rows are
   all closed on this tree.  Reimplementing them would be duplicated work;
   the honest action is a ratchet plus a measured triage, which is what the
   table above does.
4. **Local `-m32` capability is a provisioning gap, not a compiler defect.**
   `gcc-16 -m32` failed with `bits/libc-header-start.h: No such file` because
   the restored cache leaves the bookworm multilib bundle as `.deb` files.
   `sudo dpkg --unpack` on the 20 pinned packages makes it exit 0 with a
   4-byte pointer, unblocking the `zlib-compare256-correctness` gate and the
   195 previously-unmeasured i686 aggregate-ABI configurations.  This is
   recorded as an environment action, not claimed as a code fix.
5. **A second silent-ratchet defect in the DWARF DIE census.**  GDB 18.1
   showed LCCC reporting every parameter as `<optimized out>` at every
   optimisation level, while GCC 16.2 and Clang 23.1.5 resolve `a=5, b=7`.
   Tracing that into `readelf --debug-dump=info` exposed a worse problem than
   the gap itself: `check_dwarf_die_census.py` pinned only **3 of the 12 tags**
   LCCC actually emits and pre-declared **15 tags and 5 attributes** as
   "deferred" that the compiler already implements. Both halves of the gate
   ignored those names, so losing the entire C type graph, every variable DIE
   or every `DW_TAG_formal_parameter` would have produced a green check.
   Measured union (across `-gdwarf-2..5`, readelf 2.47): LCCC 12 tags /
   20 attributes, GCC 14 / 27, Clang 14 / 21; real gap = 3 tags
   (`enumeration_type`, `enumerator`, `lexical_block`) and 10 attributes.
   Fixed by pinning the full measured baseline, reducing the deferral list to
   the measured difference, adding an import-time disjointness assertion, and
   adding 7 corpus tests (including an end-to-end negative control that runs a
   gate copy with `DW_TAG_variable` removed and requires a non-zero exit).
6. **The gap itself is honest, so it is documented rather than faked.**
   Emitting a bare `DW_OP_regN` as a parameter location would claim the value
   stays in that register for the whole function, which is false as soon as
   the register allocator reuses it. GCC/Clang use entry-value expressions and
   location lists. A wrong location is worse than `<optimized out>`. The
   `parameter_location_is_never_wrong` test is a forward ratchet: LCCC may
   omit locations (measured, documented) or emit one GDB resolves to the true
   value; any half-correct implementation fails. The census docstring now
   records the GDB observation explicitly.
7. **The deferral list was still only statically checked.**  An import-time
   assertion compares two hard-coded tables; a new LCCC capability added
   without updating `BASELINE_TAGS` would still be masked by an old deferral.
   The gate now also requires the *measured* `ours_tags`/`ours_attributes` to
   be disjoint from the deferral lists, and a negative control removes a
   baseline entry, re-adds the stale deferral and requires the runtime
   diagnostic. Both layers are now ratcheted.
8. **The executable ledger only ever ran against one architecture.**  The
   driver option table is architecture-independent, but a target-specific
   `cli.rs` arm could diverge while the fast lane stayed green. All four
   fastbuild drivers were measured: `lccc`, `lccc-i686`, `lccc-arm` and
   `lccc-riscv` each report 17 rows and zero problems. The same probes now
   exercise four back ends in a pinned corpus sweep, so a fifth target cannot
   be added without deliberately extending the coverage.
9. **A line-by-line audit of the new gate found three edge-case defects in
   the gate itself.**  (a) A wrong `--lccc` path raised an uncaught
   `FileNotFoundError`, so CI saw a Python traceback instead of a named
   verdict. (b) `opt.s` was reused for all 17 rows; a compile that failed
   without writing it left the previous row's bytes, which were then hashed
   and could be reported as a codegen change. (c) A relative `--lccc` path
   only worked from the repository root. Fixed with `resolve_binary` (named
   problems, `PATH` lookup, repo-root fallback), `dest.unlink(missing_ok=True)`
   before every compile, and exact-duplicate suppression in the problem list.
   Seven new tests cover all three plus a non-executable file and a positive
   control proving a fresh compile is still hashed correctly.
10. **Host-CPU runtime behaviour was checked with a real debugger, not just
   assembly hashes.**  A four-frame `noinline` fixture compiled by LCCC,
   GCC 16.2 and Clang 23.1.5 at `-O2`, `-Ofast` and `-Og` produces identical
   results for inputs 5, -3, 0 and 12345 (`6n+16`). GDB's backtrace at `leaf`
   is `leaf -> middle -> top -> main` for all nine compiler/level pairs, line
   breakpoints resolve for all three, and `-fno-omit-frame-pointer` produces a
   saved `rbp` chain where default `-O2` reports only `rip`. These are the
   two optimisation levels reclassified in item 1, so their "implemented"
   classification is now backed by runtime evidence, not only a macro delta.

## Alternatives weighed

| Option | Pros | Cons | Verdict |
|---|---|---|---|
| Implement 20 large compiler features (PIE, stack protector, `gcov`, IFUNC…) | maximal headline scope | no measurable acceptance test in one session; high miscompile risk; violates "prove your claims" | rejected |
| Re-run every existing gate and fix only failures | safe | misses defects that are green because the gate never executed (the ledger, the provenance note) | insufficient |
| **Probe each candidate, fix provable defects, ratchet already-closed ones, measure the open ones** | evidence for every claim; zero miscompile risk; regression coverage per item | requires a triage document | **chosen** |
| Add the 20 checks to `ci_local.sh` | direct gate | would require editing `ci.yml` + `ci-lane-plan.json` + parity registries, widening the CI-failure surface | rejected; `unittest discover -s tests/corpus` already runs in both `ci_local.sh:2477` and `ci.yml:3223`, so `tests/corpus/test_backlog_roi.py` executes locally **and** hosted with no wiring change |

## Function-level efficiency audit of the new code

Every new hot path is static analysis or a single compiler invocation; none is
in a per-compilation loop. Measured on the 2-vCPU host (minimum of 3 runs):

| Function / gate | Complexity | Measured | Why this shape |
|---|---|---|---|
| `option_effect_ledger.parse_rows` | O(source bytes), one regex pass | — | Block parser with one `finditer`; no backtracking-prone nested quantifiers. Comments are handled by field regexes rather than a pre-pass that would copy the file. |
| `option_effect_ledger.dynamic_problems` | O(rows × probes), 17 rows / 11 probes | 0.472 s total for `--lccc --self-test` | Two compiler invocations per row (`-S`, `-dM -E`) are the irreducible cost of an *executable* contract; results are hashed, never diffed, so memory is O(1) per row. |
| `check_backlog_roi.Context.load` | O(sum of 17 file sizes), read once | 0.081 s for all 20 checks + 21 negative controls | One read per input; checks share a frozen context instead of re-reading the tree 41 times. |
| `check_backlog_roi.run_all` | O(20) | included above | Each check returns named problems, so `--json` can attribute a failure to one item. |
| `check_dwarf_die_census.census` | O(DIE lines), streaming | 0.364 s for 4 dialects × 3 compilers | Line-by-line state machine; no DOM, no retained DIE tree, so memory is O(tags + attributes). |
| `option_effect_ledger` × 4 arch drivers | O(4 × rows × probes) | 1.675 s for all 18 option-ledger tests | The same 17 rows are probed against four host executables that emit four targets' assembly; reuses parsed rows instead of re-reading the source four times. |
| `check_sse2_codegen_matrix` `--check` | O(122 rows × 3 compilers) | 6.230 s | Dominated by 366 real compilations; the checker itself is linear. |

No function allocates proportional to an untrusted input size beyond the file
it was written to read; no quadratic string concatenation; no shell
interpolation of paths in the Python gates (all `subprocess.run` with list
argv); no `eval`/`exec` on repository content in the production path. The
`test_dwarf_die_census_ratchet` negative control deliberately `exec`s a
mutated *copy* of the gate inside a temporary directory — that is test-only
and never reaches a compiler.

Design alternatives weighed for the ledger: (a) reuse the shell gate's
existing per-option `if` chain — rejected, it cannot enumerate the table and
would duplicate the source of truth; (b) parse `option_table.rs` with a Rust
build step — rejected, a Python gate can run in both the shell and corpus
lanes without a Cargo build; (c) emit a JSON contract from Rust and diff it —
rejected, that verifies *declarations*, not behaviour. The chosen design is
the only one that fails when the compiler's behaviour changes while the table
stays still.

## Verification

All results below are from the rebased tree (`origin/main` = `fe8d911`, which
merged 146 unrelated files), rebuilt with the fastbuild preset and the pinned
GCC 16.2.0 / Clang 23.1.5 / GDB 18.1 / Rust 1.99.1 oracles.

| Check | Result |
|---|---|
| `cargo build --profile fastbuild --locked`, `RUSTFLAGS=-D warnings` | **exit 0, zero warnings** (0 `warning` lines in the build log) |
| `cargo fmt --check` | **exit 0** |
| `cargo clippy --profile fastbuild --locked -- -D warnings` | **exit 0** (hosted Clippy lane passes too) |
| `cargo test --profile fastbuild --locked driver_option_contract_table` | **1 passed, 0 failed**, 8083 filtered |
| `option_effect_ledger.py --lccc … --self-test --json` | 17 rows, 11 probes, 7 priority options, **0 problems** |
| `check_driver_option_contract.sh --skip-trapv-matrix` | **exit 0**, including the executable ledger |
| `check_sse2_codegen_matrix.py --check / --self-test / --update-baseline` | **all PASS** (122 rows; re-derived against the live oracles) |
| `check_dwarf_die_census.py` (4 dialects × 3 compilers) | **PASS** with the tightened 12-tag / 20-attribute baseline |
| `check_backlog_roi.py --self-test --json` | **20/20 PASS, 0 negative-control failures** |
| `tests.corpus.test_dwarf_die_census_ratchet` | **8 tests OK**, incl. end-to-end lost-tag and stale-deferral negative controls |
| `tests.corpus.test_option_effect_ledger` | **24 tests OK**, incl. the 4-architecture dynamic sweep and 7 gate-robustness controls |
| `tests.corpus.test_gdb_option_runtime` | **7 tests OK** with pinned GDB 18.1 |
| `unittest discover -s tests/corpus -p 'test_*.py'` | **1302 tests, 0 failures, 5 skips** |
| `check_doc_links.py`, `check_script_imports.py`, `check_no_conflict_markers.py`, `check_ci_workflow_shell.py` | **all PASS** |
| Hosted CI on `fd3d490` | **22 pass / 3 skipping / 0 failures**, including the authenticated ICX 2026.2 integration lane |

Previously-failing local gates were traced to a missing pinned environment:
with `env.sh` sourced, `oracle-toolchain`, `zlib-compare256-correctness`,
`trapv-boundary-matrix-m32`, `gcc17-builtin-semantics`,
`public-native-simd-abi` and `memory-value-contracts` all pass. Hosted CI is
the authoritative gate.

### §10 audit of the merged cache refresh

`engineering/FOLLOWUP-2026-11-05-clang-2315-gdb-181.md` §10 was traced
line-by-line against the tree.  Every claim still holds: the publisher derives
`CLANG_VERSION` from the enforcing pin, stages the digest-verified Clang prefix,
builds GDB from the shared installer, and refuses retired payload names;
`agent-cache-lock.json` names `harness-cache-8` with a GDB entry; the six
surviving release tags carry zero obsolete payloads.  One residual risk was
found and closed: nothing prevented a future `--refresh` from reintroducing an
obsolete filename into the reviewed lock, so the guard is now ratcheted by
item 20 (`CACHE-REDTEAM/1`) rather than resting on a one-time manual audit.

**Not claimed:** ICX 2026.2 cannot be installed in this sandbox (the Intel PPA
is outside the network allowlist), so no local three-way comparison was run.
Hosted CI's `icx-oracle` job (`ci.yml:3450`) provisions the authenticated ICX
binary and runs `check_enum_forward_abi.py --require-icx` plus
`check_kernel_codegen_contracts.py --require-icx` alongside GCC 16.2 and
Clang 23.1.5, and the ICX dispatch fixtures compare lccc against ICX output;
that lane, not a local claim, is authoritative for three-way agreement.  No PMU
data is claimed — instruction counts and assembly hashes are deterministic
compile-time observables.  No runtime-performance claim is made for any item.
