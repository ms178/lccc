# FOLLOWUP 2026-11-21 — flat case-label runs, three quadratic/depth defects, torture ratchet

Session scope: turn the gcc.c-torture compile-leg ratchet from 5 failures to
2, eliminate the measured near-quadratic and O(depth²) parse/analysis paths
that the flat-label work exposed, re-base on ms178/lccc main (a4145e2db), and
land self-contained regression fixtures for every fixed behaviour.

## Accomplished (all validated; see per-item evidence)

### 1. Flat case-label runs — `Stmt::Labeled(LabelRun)` (was: 4 chained Stmt variants)

* **Root cause.** `case 0: case 1: ... break;` parsed as a 100,000-deep
  `Box<Stmt>` chain.  Every derived walk (Drop, Clone, sema, flow, lowering,
  ref_collection, nested_functions) recursed per label and overflowed the
  64 MB compiler thread stack on gcc.c-torture/compile/limits-caselabels.c.
* **Fix.** One flat `LabelRun { labels: Vec<Label>, body: Box<Stmt> }` per
  run; `parse_label_run()` collects the run iteratively; `lower_labeled_stmt`
  registers every label block in source order (the exact block chain the
  nested shape produced) and lowers the shared body once.  All 11 consumer
  files converted; `cargo check --all-targets` clean.
* **Semantics preserved** (verified byte-identical against GCC 16.2 -O0/-O2
  and both lccc arches on a differential harness covering distinct-case
  dispatch, shared-body runs at 10/100/1000 labels, GNU `case a ... b`,
  fallthrough, named-loop `break L`/`continue L`, statement-expression label
  tails, `goto` into case bodies, mixed name+case runs, C23 trailing
  labels): trailing-name binding rule (`LabelRun::trailing_names` — a `case`
  between a name and the loop severs the C2y binding), duplicate-case /
  overlapping-range / multiple-default diagnostics, `pending_label_aliases`
  InlineAsm emission for name-label blocks, and the stmt-expr tail rule
  (only LEADING name labels get block creation + value capture).
* **Perf.** 100k-label switch now lowers linearly; see §2 for the two
  quadratic paths found downstream and fixed.

### 2. CFG successor dedup was O(row) per edge — `src/ir/analysis.rs`

* **Root cause (found by gdb sampling of the 100k-label compile).**
  `build_cfg`'s `push_edge` deduped successor rows with
  `succ_data[row_start..].contains(&t)`: a 100k-arm `Switch` built a
  100k-edge row and paid ~5·10⁹ comparisons — measured n^1.86 compile
  scaling (10k labels 0.19 s → 100k 13.8 s vs GCC 0.331 s).
* **Fix.** Epoch-stamp dedup (`succ_seen_stamp[t] == i` means "block i
  already has target t"): O(1) per edge, no clearing (block indices are the
  stamps), exact same successor-row semantics (duplicate-free, source-arm
  order; predecessor rows still keep duplicates).
* **Second quadratic in the same profile** (`mem2reg::promote::
  get_successor_labels`): `targets.contains(&label)` over the growing switch
  target list.  Replaced with an order-preserving `FxHashSet` dedup
  (default first, then case arms).
* **Result.** 100k labels: 13.8 s → **0.90 s** (GCC 0.33 s; fastbuild -O1
  host binary).  Scaling 10k→100k now ×11 for ×10 input (linear).

### 3. Parenthesized-expression chains — `paren_chain_run` (limits-exprparen.c)

* **Root cause.** `((((…0…))))` (10,000 nested parens in limits-exprparen.c)
  burned the full cast/unary/postfix/binary frame chain per paren and
  exhausted `PARSER_FRAME_BUDGET` (32,768 counted frames) at ~2.7k parens —
  a compile-fail where GCC 16.2 compiles in 0.026 s (Clang 23.1.5
  SEGFAULTS on this input).
* **Fix.** `paren_chain_run()` recognises a pure `(`ᵏ X `)`ᵏ group whose
  content X is bracket-balanced and non-empty; the arm peels the run
  iteratively and parses X once (parens are transparent in the AST, so the
  result is identical to the recursive parse).  Groups whose content is not
  balanced (`((a)+(b))`, `((int)3)`, `(f(x))`) return `None` and take the
  exact old path — casts inside chains are never mis-peeled.  A `(`ᵏ `{…` `)`ᵏ
  group parses the statement expression at the innermost level with the same
  single pedantic warning.  10k parens = O(1) Rust recursion.
* **Result.** limits-exprparen.c compiles in **0.03 s** both arches (GCC
  0.026 s); 100k-paren chain in 0.07 s.  Semantics diffed against GCC on
  19 constructed forms (assignments through parens, calls through parens,
  GNU `?:`, casts, stmt-expr tails, comma chains): byte-identical.

### 4. Nested record definitions — iterative trampoline + clone elimination (limits-structnest.c)

* **Root causes (two, both measured).**
  1. `struct { struct { … } x; }` member types recursed
     parse_struct_fields → parse_type_specifier → parse_struct_or_union one
     TYPE frame per level; the 2,048-frame budget rejected
     limits-structnest.c (10,000 levels) as a compile-fail.
  2. `fold_simple_derived` cloned the whole base `TypeSpecifier` per member
     declarator and `type_spec_has_enum_tag` re-walked the full nested
     subtree at every level — together an O(depth²) parse (measured n^1.8+:
     1k 0.15 s → 2k 0.76 s → 4k budget-rejected).
* **Fix.**
  1. `parse_struct_or_union` factored into `record_header` / `record_finish`;
     `parse_struct_fields` drives a `RecordLevel` stack when a member type is
     a bare (or qualifier-led) `struct`/`union` DEFINITION — one Rust frame
     per top-level definition, heap-bounded depth.  The pop path restores the
     body-boundary state, the const wrap, then runs `record_finish` and
     `consume_trailing_qualifiers`, then resumes the member's declarators —
     the exact recursive unwind order (a post-body `__attribute__` side
     effect survives the hand-back).  Non-member definitions (parameters,
     initializers) still use the recursive path behind the TYPE budget.
     `at_struct_or_union_def` is a bounded lookahead that never routes tag
     references (`struct S *p;`) or unbalanced groups to the trampoline.
  2. `fold_simple_derived` takes the base by value; a copy is made only when
     a comma actually introduces a second declarator.  `type_spec_has_enum_tag`
     is memoised on each record definition's `(file_id, start)` span
     (`Parser::record_has_enum_cache`).
* **Result.** limits-structnest.c (10,000 levels): compile-fail → **0.13 s**
  both arches (GCC 16.2: 1.87 s — 14× faster).  Depth scaling linear
  (250→4000: 0.010→0.045 s).  Semantics diffed against GCC on named /
  anonymous / union / const-led nesting, pointer+array declarators on nested
  definitions, and multi-declarator members: byte-identical on both arches
  at -O0/-O2.

### 5. Audit-hardening wave (same day, second pass)

Everything the red-team audit and the CI ratchets demanded after the four
fixes above landed:

* **Zero-slop cleanup** — `lower_labeled_stmt` lost its dead `span`
  parameter (`let _ = span;` wart) across all 3 call sites; an ENOSPC
  tail-append artifact (duplicated `#[test]` fn, rustc E0428) was removed
  and a fn-inventory diff taken against HEAD (only the 6 intended new fns).
* **ICE sites eliminated** — the two `.expect("second declarator without a
  saved base type")` sites in `types.rs` (clone-cascade fallbacks) became
  soft failures: an error diagnostic plus a `Void` placeholder base.  The
  parser invariant still holds; a future regression now degrades to a
  diagnostic instead of a compiler crash (audit census panic_sites 861→860).
* **`parse_primary_expr_inner` back under the 300-line census budget** —
  the `(` primary arm (StmtExpr / paren-chain peel / plain paren) moved to
  its own `parse_paren_group` method; one arm, one concern.
* **Sema record-nesting guard (root-caused with gdb)** — the 50k record
  chain did not crash in the parser any more but in SEMA:
  `field_ctype → build_qualified_ctype → resolve_type_spec_to_ctype →
  resolve_struct_or_union → convert_struct_fields → field_ctype` recursed
  one Rust frame chain per level and overflowed the 64 MB worker stack
  (SIGSEGV then abort).  Measured cliff: 20k compiles, 25k overflows.
  `resolve_struct_or_union` now carries an RAII depth guard
  (`MAX_RECORD_RESOLVE_DEPTH = 16_384`, one clean
  `type nesting too deep (exceeds 16384 levels)` diagnostic + `Void`
  placeholder, exactly once per TU).  10k (torture floor) compiles with
  1.6x margin; 16,384 keeps 1.5x headroom under the crash cliff.
* **Gate contracts updated to the new reality** — `check_parser_depth_limits`
  and `check_deep_nesting_robustness` pinned pure-paren/record rejection
  from the pre-peel/pre-trampoline era.  The shapes those legs named are
  now ITERATIVE and legal (and GCC compiles them — limits-exprparen.c and
  limits-structnest.c are torture must-compile files): the legs moved to
  the compile class with speed ceilings, and the rejection ends stay pinned
  by frame-consuming shapes that still recurse (mixed nested binary
  `(0+(0+...))` for the expression budget, 50k record chains through the
  new sema guard).
* **rustfmt + clippy gate repair** — `cargo-fmt` shim was missing from
  `~/.cargo/bin` after the Arena restore (the toolchain ships it); my new
  code had 15 rustfmt diffs, all fixed, `cargo fmt --all -- --check` clean.
* **pr115143 re-evaluation concluded: refusal is the correct contract** —
  `check_parser_depth_limits.sh` pins it in writing: “LCCC deliberately has
  no GIMPLE front end, so the contract is a bounded, panic-free rc=1
  refusal”.  `__GIMPLE`/`__BB`/`__PHI` is GCC's own internal SSA IR, not a
  C feature; a faithful implementation needs per-edge phi elimination
  (edge-splitting) for correctness and buys zero codegen/performance wins
  for lccc's benchmark corpus.  The 2 files stay clean refusals.

## Validation matrix (this session)

| Check | Result |
|---|---|
| `cargo test --lib` | 9495 passed, 0 failed (post-rebase count; re-run after the audit-hardening wave) |
| `tests/regression/run_regression.py` full corpus | 1537 passed, 0 failed, 3 skipped-compare (1540 total) |
| New fixture `flat_case_label_runs.c` | PASS; stdout identical to GCC 16.2 on x86-64/i686 |
| New fixture `paren_chain_deep.c` | PASS; stdout identical to GCC 16.2 on x86-64/i686 |
| New fixture `struct_nest_deep.c` | PASS; stdout identical to GCC 16.2 on x86-64/i686 |
| Differential labels/paren/nest harnesses vs GCC -O0/-O2 | byte-identical on x86-64 and i686 |
| `check_parser_depth_limits.sh` | 18/18 (pure-paren legs now compile class; mixed-nested-binary pins rejection) |
| `check_deep_nesting_robustness.py` | 14/14 (record 50k legs clean rejection through the sema guard) |
| `audit_census.py --check` | PASS — panic 860≤861, hot 94≤94, fns_over_300 233≤233 |
| `cargo fmt --all -- --check` | clean |
| `ci_local.sh --fast` | green (with clang-23.1.5 + gcc-16 + cargo-fmt on PATH) |

## To-do (next sessions)

1. **pr115143-2.c / pr115143-3.c** — RESOLVED as deliberate refusal (see
   §5): the checked-in contract pins a bounded rc=1 rejection for GCC's
   GIMPLE IR syntax; re-open only if GCC-parity on `-fgimple` ever becomes
   a user requirement.
2. Widen the record trampoline to parameter-list record nests (the TYPE
   budget still bounds those at 2,048 parser frames — acceptable parity;
   the sema guard's 16,384 ceiling only binds member-definition chains).
3. Linker oracle refresh per the user's toolchain matrix (GCC 16.2/17,
   GAS/bfd 2.47, lld 23.1, mold X86/i686, ICX 2026.2, Clang 23.1.5).
   Note: `~/.cache/lccc-toolchains/clang-23.1.5` is installed; its `bin/`
   must be on PATH for the oracle gates (the Arena restore dropped the
   PATH entry and `~/.cargo/bin/cargo-fmt` — both are provision-script
   gaps worth pinning in `arena_provision.py`).
4. godbolt_codegen_compare low-hanging-fruit sweeps on zlib-ng/gzip/expat
   workloads (ms178/archpkgbuilds extraction).
5. Torture re-run against the post-wave binaries (baseline 1995p/5f at
   c94b688a; 3 of the 5 failures root-fixed — expect 1998p/2f).

## Design notes for reviewers

* **Why an epoch stamp instead of a hash set in `build_cfg`:** the target
  space is block indices `[0..n)`, visited in increasing order, so
  `stamp[t] == i` is unambiguous and never needs clearing — O(1) per edge,
  2×u32 of memory per block, zero hashing.  A hash set would add per-edge
  hashing for no asymptotic gain; the old row scan was O(row) only for wide
  rows but poisoned exactly the workload (wide switches) that matters.
* **Why the paren peel is conservative:** mis-peeling `((int)3)` would turn a
  cast into a type-name parse.  The balanced-content requirement makes the
  peel sound for every shape and leaves all ambiguous groups on the
  battle-tested recursive path; the pathological input (pure chains) is
  exactly what the peel covers.
* **Why the trampoline restores in the recursive unwind order:** the parser's
  attribute save/restore contract is load-bearing (pr82210's `aligned`
  mishandling is documented in-tree).  Restoring the body boundary first and
  running `record_finish` afterwards is the only order that keeps post-body
  `__attribute__` effects observable to the parent member's declarator parse.
* **Why `fold_simple_derived` clones only on comma:** a member list with one
  declarator is the overwhelmingly common case and the only shape deep
  nesting produces; N declarators genuinely need N owned copies under the
  AST's by-value `TypeSpecifier`, so N-1 copies is optimal without an
  `Rc`-based type graph.

## 6. Modifiable-lvalue structural rule (2026-11-22 audit find; FIXED)

Red-team reproduction against GCC 16.2.0 exposed a pre-existing conformance
gap: `struct s5 { union { struct s6 n; int z; } u; }` with
`struct s6 { const int v; }` — `wu.u.n = w` is a C11 6.3.2.1p3 constraint
violation ("an lvalue of struct/union type with any const-qualified member,
recursively, is not modifiable") but lccc accepted it silently.

* **Root cause:** `object_const` is computed at layout-build time from the
  field's OWN declared `const` only (`parser/types.rs: object_const:
  self.attrs.parsing_const()`, plus a typedef-top-const OR at
  `analysis.rs` field conversion).  The C11 recursive clause ("any member
  (recursively, including all members of all contained aggregates and
  unions) that is const-qualified") had no implementation at all.
* **Fix:** `SemanticAnalyzer::type_structural_readonly` — an ITERATIVE
  worklist walk over `StructLayout` fields with a visited-key set (record
  chains reach 10k levels; shared layouts must not re-trigger walks;
  pointers stop the walk — `const int *p` as a member is a writable pointer
  object, measured).  Array members unwrap to their element type (measured:
  `struct { const int a[2]; }` is non-modifiable as an assignment target).
  `lvalue_readonly` = the existing qualification-path rule (renamed
  `lvalue_readonly_qualpath`) OR the structural rule on the lvalue's type.
* **Wording split:** GCC 16.2 distinguishes `assignment of read-only member
  'n'` (const lives inside the member's own type) from `assignment of member
  'n' in read-only object` (const flows from the access path).  The wording
  dispatch consults `lvalue_readonly_qualpath(base)` — the structural rule
  alone must NOT flip the wording to "in read-only object" (measured:
  `wu.u.n = w` says "read-only member 'n'"; `const struct s5 *p; p->n = w`
  says "member 'n' in read-only object"; `wu = v` says "read-only variable";
  `*p = v` says "read-only location").
* **What stays legal (measured on GCC 16.2):** a plain member whose own type
  carries no const is writable inside a const-containing aggregate
  (`wu.u.z = 1`, `hc.z = 7`), initialization of const-containing aggregates
  is not assignment (`struct s5 copy = wu;`), and clean aggregates assign
  freely.  All pinned in `tests/regression/const_member_modifiability.c`.
* **Rejection half pinned** in `tests/regression/check_sema_lvalue_gates.sh`
  (wired into `ci_local.sh` and `.github/workflows/ci.yml` for the gate-parity
  ratchet): 7 error-contract cases (exact GCC wording, exactly one
  positional diagnostic per violation, live gcc-16 cross-comparison of the
  error text) plus a multi-violation NO-SPAM case.  Diagnostic counting must
  exclude the `ccc: error: N error(s)` summary line.

## 7. Fixture inventory (post-wipe reconstruction; 7 files, all gcc-identical
##    on x86-64 AND i686, rc=0, byte-identical stdout vs GCC 16.2)

* `flat_case_label_runs.c` — dense/decade/bounded/fallthrough/default-first/
  sparse/loop/labels-run/nested/case-block switch forms (R10(base) decade
  macro; a naive R10×10 "R100" REPEATS case values 0–9 = gcc "duplicate
  case value" — parameterized bases are mandatory).
* `paren_chain_deep.c` — 10k pure chains + 23 operand walk-throughs +
  unbalanced-fallback pins.  A chain around a TYPE NAME (`LBR int RBR` used
  as a type) is NOT valid C at any depth — `((int))` re-parses as
  parenthesized expressions with a missing cast operand (measured; same
  class as the historical `((a))[0]` finding) — the cast-form walk-through
  `LBR (int)2.0 RBR` is the sanctioned type-path coverage.
* `struct_nest_deep.c` — the gcc.c-torture `LIM4(struct s)`/`RBR4` 10k
  NAMED chain + s6..s0 named-resolution chain + ga/gp/gm accessors.
  The chain must stay token-pasted NAMED: 10k-deep ANONYMOUS nesting is
  gcc-quadratic (2m09s measured) vs 1.9–3.1s for the named shape and would
  blow the runner's 90s oracle budget without testing anything extra.
* `deep_chain_integration.c` — cross-feature interference (label runs whose
  case bodies carry peeled paren chains on nested-member operands).
* `const_member_modifiability.c` — the ACCEPTED half of §6.
* `paren_chain_run.c` — peel-semantics breadth (operand classes, mixed and
  unbalanced fallbacks, stmt-expr interplay).
* `labels.c` — every label-run form: dense/sparse/range runs, default
  positions, empty-statement tails (`case N: ;` — a BARE `:` is not a
  statement and never was; the -O0 duplicated-terminator bug fired on
  empty-statement tails), stmt-expr tails, GNU label attributes, gotos,
  C2y named-loop binding (trailing names bind; case severs), nested switch
  fallthrough.

## 8. Wipe-recovery and workspace forensics (2026-11-22)

* The Arena wipe destroyed `labels.c` and `paren_chain_run.c` outright and
  the first restore pass missed them; the corpus COUNT (1540) vs file count
  arithmetic is what exposed the loss.  Both were recreated with the
  documented coverage and re-validated.
* A disk-exhaustion failure mode: `target/` (fastbuild 5.5G + debug 2.9G)
  plus throwaway check builds filled the 25G root to 100% mid-CI, producing
  27 phantom test failures ("No space left on device" in rustc IO) and a
  spurious gate failure.  Diagnosed via `df` + `du`; freed with
  `rm -rf target/debug target/fastbuild/incremental /tmp/check-target` +
  `apt-get clean` (6.7G free).  Any future "27 elf_writer failures at
  once" is disk-full until proven otherwise.
* The `check_peephole_alloc_before_filter` gate's self-test PRINTS a
  `FAIL:` line for its known-bad sample by design (detection proof); the
  gate verdict is the `PASS  peephole-alloc-before-filter-contracts` line —
  do not panic at the FAIL text.
* `clippy --all-targets` reports ~170 pre-existing macro-crate test-module
  errors (parse.rs includes test-only code under `cfg(test)`); `cargo clippy
  --lib -- -D warnings` is the established tree-wide gate and is clean.
