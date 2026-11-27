# Public CCC fork network: admission audit, 2026-11-27

## Scope, not a blanket certificate

Baseline: `ms178/lccc` **7604253fa62bbce385b740ea33d7ce573f21cf5a**.
Original network root: `anthropics/claudes-c-compiler`
**6f1b99acb2f4ec2414592136c2009fe7713deec3**.

The reproducible census covers **261 repository records**, including the root
and target, **150 distinct external branch tips**, **147 trees**, and **133
`src`/`include` tree signatures**. All advertised branches were enumerated;
looking only at `main` would miss most of the useful changes. LCCC's own
historical research branches are excluded from the external-tip analysis.

**This is not a claim that every line of every fork was semantically verified.**
The entire visible network was inventoried at immutable revisions; selected
candidates received detailed semantic/admission review and local reproducers.
Five repositories' heads remained inaccessible. Detached mirrors, unadvertised
refs, concurrent remote changes and inaccessible histories are not certified.
Among the 254 accessible external forks, 31 have a differing `src`/`include`
tree on at least one branch. An unchanged source tree does **not** certify a
repository's build scripts or dependencies as safe.

Evidence:

- [heads.json](evidence/fork-network-2026-11-27/heads.json): repository/branch census, including errors;
- [deltas.json](evidence/fork-network-2026-11-27/deltas.json): exact modes/blob/tree identities, changed source and execution-boundary paths, and entry comparisons against LCCC;
- [lccc-tree-summary.json](evidence/fork-network-2026-11-27/lccc-tree-summary.json): the LCCC baseline's **10,715 tracked blobs / 785 Rust files**, subsystem counts and full-manifest hash;
- [follow-up](FOLLOWUP-2026-11-27-fork-network-audit.md): implementation, validation, measurements and open work.

Subjects and filenames are untrusted data. No foreign CCC checkout, hook,
workflow, build script or executable was run. Fork objects were fetched into a
separate bare repository. Exact entry equality is provenance evidence, **not**
a semantic adoption proof. Existing script `audit_levkropp_since.py --commit`
was also used: its first-parent comparison for `a9987d0e8e97` reported 14
identical entries, 14 already-absent deletions, 36 absent additions and 159
entries requiring review. These counts are not merge approval.

## Admitted and perfected changes

| Source commit in `sametyilmaztemel/claudes-c-compiler` | Current-LCCC witness | Adaptation, not a blind cherry-pick |
|---|---|---|
| `5d3f86f8dec1133c00ac0dcea7ed3559ad11ffa2` | `#if (1 ? -1 : 0U) < 0` selects the wrong branch | Common type from both arms, proper middle-operand comma grammar, preserved inactive-arm diagnostics; separate preprocessing and C-expression tests. |
| `a50a017f26f3894a12cb61101180f388c712ed25` | Negative narrow LHS of unsigned `/=`/`%=` is zero-extended at the wrong width | Adapted to `promote_for_op`, preserving both widening steps through 128 bits and distinguishing C common type from widened carrier. Tests avoid out-of-range final signed results. |
| `c6e8d717b88ac575d9ce3d63e3ff52f55dcf14e1` | `typedef char T; ... int T = sizeof(T);` initializes to 1; later `(T)-1` can parse as a cast | Per-declarator binding, exactly-once scope undo, sibling qualifier metadata, local alias-size deferral and GNU inferred-type exception. Preserves prior outer bindings rather than copying the fork's simpler symbol sets. |

Their `LICENSE` remains the original CC0 dedication; the small adaptations and
new tests retain explicit source attribution. No compiler rewrite or dependency
bundle was imported. The separately added zlib-ng benchmark is **Zlib-licensed**,
with source hashes and the unchanged license text identified in
`tests/benchmark/WORKLOAD_PROVENANCE.md` and `LICENSING.md`.

## Concrete exclusions and integration blockers

### Misleading or replacement trees

- `wadsaek/claudes-c-compiler`,
  `bdaa75cad6d7dd13dcc93b62b2dcd9b7bd818b54`: subject “Edit README.md”, but
  the parser substitutes unrelated text for string literals based on random
  input. **Rejected by source inspection; never executed.**
- `rosubra/claudes-c-compiler`,
  `31823e8179d276943bfa8eca0beb55cb8f324e63`: the tip contains only a README;
  the compiler was removed. There is no compiler improvement to transplant.
- `Matr1x-101/claudes-c-compiler`,
  `9b09382894d0dfd57497fd56d7f2c6a72942b40b`: Cargo/main/compiler sources are
  deleted and the root has a GCC source-tree layout (`gcc/`, `lib*`, `COPYING*`,
  configure machinery). This is not a surgical CCC change. No build or import.
- Several `adwait` experimental branches delete whole semantic/lowering
  subsystems. Treat them as experiments, not ready-made fixes.

### Already addressed or structurally obsolete

- `trademomentumllc` `57d17369fc712f198736e2129d5d9bf1c1a0af5d` fixes narrowing
  casts in CFG truth folding. Current LCCC's global resolver already uses
  `cast_integer_const`, with broader full-width integer handling. Its local
  resolver can fall back to that path. Copying the old narrower implementation
  would not establish an improvement.
- `ChaseWNorton` `1827f0d9940bb69e66f9ea757098e5820fd1ea9d` removes an old
  allocator's false ECX clobber for constant GEPs. Current liveness/allocator
  organization is different; the old `ecx_clobber_points` edit is not a patch
  against current LCCC. The fork's “101 bytes” observation is not current
  x86/i686 performance evidence.
- Older SCCP/use-def/peephole changes in `CrazyTodd-one` and `rurban` overlap
  substantially evolved current passes. A matching pass name is not proof of
  equivalence; an old benchmark number is not a reason to replace the pipeline.

### Promising Darwin/allocator work that must not be imported as-is

`macncrash/claudes-c-compiler` branch `macos-arm64`,
`badae0fd07a647e867f1cb3a4231ff150acee769`, has 38 incoming source paths absent
from LCCC and 151 divergent source entries. Several pass designs deserve
prototypes, but they carry platform/IR assumptions:

1. **`pressure_hoist.rs`: insertion must respect definitions.** Its target is
   the deepest operand-defining block, but it inserts before a final compare
   used by that block's branch. Consider this legal SSA shape:

   ```text
   entry: x = ...; y = ...; c = cmp(arg, 0); condbr c, left, right
   left:  br join
   right: br join
   join:  r = select(c, x, y); return r
   ```

   `x` and `y` each have one user, so the pressure test admits the select. All
   definitions are in `entry`, and the reachability test admits that target.
   Inserting before its final `cmp` would use `c` before its definition.
   This is an **admission counterexample to the pass in isolation**, not a
   claim that the foreign full pipeline was executed. A port needs an
   instruction-level insertion bound and dominance verification, not just
   block-level dominance. Floating-point environment/trap assumptions also
   need explicit treatment before the broad “pure” whitelist is reused.

2. **`sincos.rs` is Darwin-specific.** It emits `__sincos*_stret` with an HFA
   return convention and assumes Darwin libm's errno behavior. It scans across
   intervening instructions/calls. Linux ABI, symbol visibility, math flags
   and observable floating-point environment changes need their own proof.
   Do not copy it into a strict x86/Linux pipeline as a generic libcall fold.

3. **`ext_elide.rs` depends on AArch64 register-extension facts and late pass
   placement.** A load's register high bits are not a target-independent IR
   theorem, especially after store forwarding or physical-register reuse.
   Its idea is useful; its representation contract must first be mapped to
   LCCC's narrowing/range facts and backend.

4. **`ret_dup.rs` has bounded but real code-growth costs.** Two body instructions
   and eight predecessors do not bound duplicated epilogues or actual register
   pressure. Prototype against current CFG cleanup and profile/layout behavior;
   do not equate a removed branch with a measured win.

5. **`mod_ref.rs`, `mem_inline.rs`, `step_to_load.rs`: boundary review only.**
   Respect call/capture effects, target-alignment guarantees, provenance and
   post-phi non-SSA behavior. Their whole implementations are not certified by
   this session's metadata or header review.

`levkropp/lccc` `a9987d0e8e97db148dbf91bf307b72a93357eb60` also contains
`phi_antidep_split.rs`. The reviewed implementation inserts an instruction and
replaces `instructions` without extending nonempty `source_spans`; that violates
LCCC's parallel-vector invariant. Its cached definition indices also need
reconsideration after edits. A port needs span/analysis maintenance and measured
register-allocation benefits. The quoted Mandelbrot result is not ours.

## Inventory-level coverage of the other source-changing repositories

The following is **not** a safety or semantic-completeness verdict. It records
which visible repositories remain in the source-changing set after deduplication;
the numbers count their distinct source-changing tips (shared tips can occur in
more than one repository). Detailed outcomes above apply only to named changes.

| Repository owner(s) | Source-changing tips | Admission status |
|---|---:|---|
| `ChaseWNorton` | 86 | Selected GEP review above; other topics inventoried, no blanket import. |
| `sametyilmaztemel` | 13 | Three adapted changes validated; other type/driver/lexer topics remain separately reviewable. |
| `adwait` | 7 | Experimental restructurings/deletions; no wholesale import. |
| `RandyMcMillan` | 3 | Build/docs/source boundaries inventoried; no validated compiler transplant. |
| `fermat-hkrc` | 3 | Large test/report campaign; no foreign generator executed. |
| `MaxwellCalkin` | 2 | Keyword/constant-predicate changes; require current dialect/FP-semantic witnesses. |
| `Mr-HRJ`, `liuxiaopai-ai` | 2 each | Shared preprocessor topics inventoried; not separate independent implementations. |
| `rurban` | 2 | Compatibility and old optimizer changes; no pipeline replacement. |
| `wkias` | 2 | Historical LCCC-derived trees, not a reason to revert current main. |
| `macncrash` | 1 | Selected pass admission review above; not a whole-fork correctness certificate. |
| `levkropp` | 1 | Exact-entry inventory and selected allocator review above. |
| `thanhtoantnt` | 1 | PBT branch inventoried; consult the existing 2026-10-03 audit rather than importing its campaign wholesale. |
| `regehr`, `sscalpone` | 1 each | Differential-testing lineage/source boundaries reviewed; no full campaign replay in this constrained session. |
| `CrazyTodd-one` | 1 | Evolved-pass overlap; no unmeasured optimizer transplant. |
| `arindam89`, `blitzy-public-samples`, `wowngasb` | 1 each | The same shared branch tip, not three independent compiler improvements. |
| `N-I-ckel`, `TholeG`, `ZhdanovP`, `figitaki`, `hamzahamidi`, `shrehanrajsingh`, `wangdalian`, `yishangzhang` | 1 each | Source/build/target-boundary inventory only; no newly validated current-LCCC transplant. |
| `trademomentumllc` | 1 | Narrowing-cast candidate already addressed, as discussed above. |
| `wadsaek`, `rosubra`, `Matr1x-101` | 1 each | Concrete exclusion reasons above. |

## Reproduce, without trusting fork scripts

```sh
python3 scripts/audit_fork_network.py discover --out /large/drive/heads.json
# Exit 2 with a report is an explicit coverage gap, NOT a clean census.
python3 scripts/audit_fork_network.py inspect /large/drive/heads.json \
  --cache /large/drive/forks.git --fetch --out /large/drive/deltas.json
# Import only the target's immutable baseline into that object store for
# exact entry comparison; no worktree is checked out.
git -C /large/drive/forks.git fetch --no-tags --depth=1 \
  https://github.com/ms178/lccc.git \
  7604253fa62bbce385b740ea33d7ce573f21cf5a:refs/audit/lccc-base
python3 scripts/audit_fork_network.py inspect /large/drive/heads.json \
  --cache /large/drive/forks.git \
  --against 7604253fa62bbce385b740ea33d7ce573f21cf5a \
  --out /large/drive/deltas-against-lccc.json
python3 -m unittest discover -s tests/corpus -p test_fork_network_audit.py
```

Never infer semantic adoption from the `against_entry_counts` field. A valuable
candidate still needs a failing current-LCCC reproducer, an adapted fix,
correctness tests, generated-code review, measured scope and an immediate Arena
snapshot. That admission rule is what kept the delivered patch small in compiler
behavior despite the large fork network.
