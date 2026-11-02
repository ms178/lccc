# Follow-up — whole-tree audit, 2026-10-14

> Historical audit record. The later implementation session rebased to PR #820
> (`e047aa1`), repaired A14-29/30, and reduced the queue to 31 open entries.
> See [current fixes, validation and limits](FOLLOWUP-2026-10-14-constant-gep-fix.md).
> Original evidence and claims below retain their original dates and bases.

## Accomplished

- Started from main `3b66cd1d73743a7a79f912c01df48ac15cc646ef`; final refresh
  advanced to `1e83776f41529e2d754380d1d934ff901e25a097`. Rebased the session,
  retained initial evidence identities, closed A14-05 and revised A14-01/32/34.
  There are 34 entries, 33 still open; the original 217/0/5 CI run is not reused
  as a pass for the rebased tree.
- Activated 2 GiB swap on the largest available filesystem. Built LCCC using
  **fastbuild, Rust -O1, two jobs**, not the release/thin-LTO recipe.
- Inventoried all tracked files / 514 Rust sources and checked the shell/Python
  surface syntactically; semantically examined every subsystem's selected
  high-risk contracts. See the coverage ledger for explicit depth limitations.
- Added **A14-01 through A14-34** to root backlog.md, with source locations,
  reproductions, evidence grade, implementable actions and acceptance tests.
- Reproduced target-size, ADR overflow, RISC-V expression/relocation,
  AArch64 kernel-ISA, PGO-flush, native debug-info and ELF name-identity defects.
- Exercised existing CE oracle with GCC16.2/Clang23.1/ICX/ICC; verified aliases
  and the existing mold target setting against upstream source. Recorded ICC's
  incompatible v3 flag rather than suppressing the warning.
- Ran six extracted-workload filters at four optimization levels: 28 cases
  (Adler filter also includes combine), no failures/skips.
- Built/tested full gzip: 30/30 upstream tests in each LCCC/GCC arm, all six
  compressed stream hashes identical. Excluded the concurrent one-round timings
  from performance evidence.
- Built and tested full pinned Expat and zlib-ng. Expat has 4,932 passing checks
  in each arm but its JUnit postprocessor rejects a valid `status=run` report.
  zlib-ng has 68/69 upstream tests plus passing compression round trips.
- Isolated zlib-ng's **real table-generation miscompile** to maketrees.c;
  -O0/-O1, reference GCC/Clang and no-IV-widen control agree; -O2/-O3 do not.
  Saved a standalone licensed two-TU extraction, expected/wrong outputs and
  independent GCC-linker/TU-mix/ASan+UBSan evidence.
- Used the repository's hardened version of the supplied snapshot script.
  Cumulative ms178-1.patch, compressed named checkpoints, verified source
  archive, exact session bundle and ledger are published under the workspace.
  Interim saves explicitly say UNGATED; no CI stamp was manufactured.

## Implementation handoff — concrete order

1. **A14-30 first.** Replay the standalone case before further performance work.
   Dump the IV-widen/LICM/GVN interaction; find the first incorrect value or
   address. The current evidence isolates a trigger, not the faulty theorem.
   Fix, add a normal regression gate, obtain full 69/69 zlib checks, snapshot.
2. **A14-17 / 19 / 14 / 15 / 13.** Unknown shared relocations, forbidden NEON,
   ADR immediate bounds, unresolved data expressions and target-aware vector
   sizing are small, well-bounded correctness targets. Each has a negative test
   and positive controls. Prefer an honest rejection over fabricated bytes.
3. **A14-20.** Keep profile flushing within the file lock; deterministic
   multi-writer/error-injection tests must precede any PGO-driven optimization.
4. **A14-01 / 02 / 03 / 29.** Repair evidence machinery: census lexer, per-sample
   checksums, invalid layout-noise waiver, JUnit status semantics. Do not make
   unknown/error outcomes pass merely to obtain a green dashboard.
5. **A14-24 / 25.** Resume RA/codegen experiments only after correctness gates.
   Reuse existing RA IDs and kill switches; measure actual hot function work
   and per-workload regressions. No new allocator or ML framework is justified
   by this audit alone. PMU-backed 14700KF results remain unavailable here.
6. Remaining P1/P2 contracts are fully specified in backlog.md: typed AST,
   metadata/invalidation, runtime ownership, debug objects, CLI capabilities,
   script/temp/snapshot safety and stale capability claims.

## Validation boundaries

The exact final fast-CI status is in the **delivered snapshot receipt/ledger**.
The initial fast run began before documentation/evidence commits and therefore
cannot certify the final tree hash even if its gates pass; a final unchanged-tree
run is required. The standard fast suite deliberately omits slow gates and does
not contain this audit's new failing compiler contracts. Do not conflate its
status with resolution of A14-30 or a full parent-project pass.

The new audit probe runner's --selftest passes; its --check is expected to fail
on the audited compiler because it checks desired, currently violated behavior.
The zlib isolation tool similarly returns 1 on the open defect. This is honest
negative evidence, not permission to weaken CI or bless the wrong output.
No Linux kernel boot, complete glibc/SQLite test run, full fuzz campaign,
non-x86 runtime validation or bare-metal throughput superiority is claimed.

## Recovery and continued work

- Keep /home/user/ms178-1.patch and artifacts intact. .cache points to transient
  storage and is not a recovery artifact. Source+patch+bundle receipts are.
- Restore with the delivered LCCC-RECOVER.py, verify hashes, fetch upstream main,
  rebase the session and re-anchor LCCC_BASE_REF before making a new snapshot.
- Recreate swap and fastbuild toolchains. Do not silently turn missing oracles
  into successful tests; provision exact research revisions or label blocked.
- S01..S04 preserve validated audit milestones. Later named generations carry
  subsequent documentation/validation. Consult the ledger rather than guessing
  the newest contents from timestamps or older session narratives.


## Final-base rerun

On 1e83776, the compiler was rebuilt fastbuild/-O1/-j2. Full gzip, Expat,
zlib-ng, the 28 reduced kernel cases, zlib TU-mixing/standalone sanitizer matrix,
ELF byte-name fixture, and 30 current contract observations were rerun. Results
agree with the open findings (13 contract violations; 68/69 zlib tests; Expat
parser false-failure). A14-05 is closed upstream, leaving 33 open entries.
See `rebase-validation.json` and the report's final-base section for exact scope.
