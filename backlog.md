# LCCC Engineering Backlog

Live triage queues for measurable runtime/quality work. This file holds
**open items and their evidence**; session narratives, landed-work
history and measurement method live at the paths below — add history
there, not here.

- Sessions / measuremethod: [`engineering/journal/`](engineering/journal/README.md)
  (worst-10/worst-12 triage screens: `2026-09-W2.md`; daily entries tagged
  per-workstream: per-day edit).
- Method doctrine: [`engineering/agent/RULES.md`](engineering/agent/RULES.md).
- Per-area canonical knobs/kill-switches:
  [`engineering/subsystems/`](engineering/subsystems/README.md).
- The measured oracle baselines: [`engineering/evidence/benchmarks/`](engineering/evidence/README.md#benchmarks-and-comparisons).
- Deep research: [`engineering/DECISIONS.md`](engineering/DECISIONS.md) (measured
  negative-space, do-not-retry grounds), [`ideas/`](ideas/README.md).

Current audit adjudication: **2026-10-06**, upstream
`66c2452838eedef2322fbad44d242e3b990e6c10` (PR #771; contains
`6051e87304c6b07a2ed1a9a516aa5744d8110955`, `08f4e1a124d753e9eeddb1c43ce68b54d4d95f68`
and the merged `dd0127998aae32a0a56626091bd9c93b597d2aff`). See
[`engineering/FOLLOWUP-2026-10-07-ivsr-review-hardening.md`](engineering/FOLLOWUP-2026-10-07-ivsr-review-hardening.md)
for the review-response round on merged PR #772 (18 findings adjudicated, one
review recommendation measured and **reverted**, the descending-loop gap
discovered),
[`engineering/FOLLOWUP-2026-10-07-perf-ivsr-ptradd.md`](engineering/FOLLOWUP-2026-10-07-perf-ivsr-ptradd.md)
for the performance round (IVSR-PTRADD-1, the Callgrind kernel A/B, the two
measured rejections and the metric-method findings), and
[`engineering/FOLLOWUP-2026-10-06-ivsr-domain-audit.md`](engineering/FOLLOWUP-2026-10-06-ivsr-domain-audit.md)
for the second-round adjudication of the external review of the external review, the two new
miscompiles it did not find, and the four-vendor oracle table; see
[`engineering/FOLLOWUP-2026-10-06-codegen-audit.md`](engineering/FOLLOWUP-2026-10-06-codegen-audit.md)
for the first round's measured dispositions, regression commands and evidence.
The historical triage below is **not** a current compiler/runtime scoreboard.

Last historical broad triage rebuild: **2026-09-16**, base recorded as `8ca2fd4`.
The historical numbers below are from that earlier screen, not the current
baseline. On **2026-09-25**, `main` at `7ddb770f` was remeasured in 39
randomized paired, output-checked VM workloads against the candidate and GCC;
the only changed executable `.text` was LZ4. The sound byte-copy loop-idiom
fix retains ~10.8× speed over disabling that idiom on a separately scaled
LZ4 screen, while preserving forward-overlap semantics. See
[`engineering/evidence/2026-09-25-redteam/loop-idiom-redteam/README.md`](engineering/evidence/2026-09-25-redteam/loop-idiom-redteam/README.md).
On **2026-09-26**, follow-up work was based on upstream `e5bc1911`. A
41-workload randomized three-arm Xeon-VM screen found changed executable
`.text` only in `spectral_norm`; the paired 15-round focused result is
candidate/current-main **0.0361** (240 ms / 6.60 s), with exact benchmark
output agreement. The fix replaces the AVX2 strict-reciprocal lane pack's
legacy SSE instructions with VEX forms. The newly added match-rich LZ4 and
scaled find-bit arms reveal gaps hidden by the original short/no-match inputs.
The evidence and remaining P0 limitations are in
[`engineering/evidence/2026-09-26-followups/README.md`](engineering/evidence/2026-09-26-followups/README.md).
The i7-14700KF target has **not** been measured. Before accepting a new
**target-hardware** runtime improvement, use same-window, same-baseline,
output-checked paired A/B with ≥ 200 ms/arm and agreeing median/min, not
historical ratios.

An item with no reproducer does not belong here.

---

### ALIGN-1 · **CLOSED 2026-09-30** — the definition channel of `aligned()` now reaches codegen

Was: `__attribute__((aligned(N)))` on a function **definition** was parsed and
dropped, so only a *prototype* reached `IrModule::function_alignments` and thus
the emitted `.p2align`. Measured against GCC 14 (which honours both channels):

| function | attribute on | before | after | GCC 14 |
|---|---|---|---|---|
| `via_def` | definition only | `.p2align 4` | `.p2align 6` | `.align 64` |
| `via_proto` | prototype | `.p2align 6` | `.p2align 6` | `.align 64` |
| `plain` | none | `.p2align 4` | `.p2align 4` | `.p2align 4` |
| `proto_then_def` | prototype + definition | `.p2align 6` | `.p2align 6` | `.align 64` |

Fix: `FunctionDef` carries its own `alignment`; the lowering registers it under
the **emitted** name (the asm label when one exists). Gate:
`tests/regression/check_function_alignment_definition.sh`, verified to fail on a
stashed-fix build in exactly the one cell that changed, with `plain` as the
control against a blanket alignment change.

**Non-goal, measured:** nested function definitions. GCC applies no observable
alignment to one (`-O0`/`-O2`, with and without `noinline`), so there is no oracle
to match; LCCC ignores it too, and the wiring that made it honour the attribute
was tested, found not to fire, and removed rather than shipped.

### ALIGN-2 · **CLOSED by ALIGN-3** — redundant function alignment directive

Do not reopen the old report: ALIGN-3 below supersedes it.

### VOLATILE-BF-1 · **CLOSED 2026-09-30** — a volatile bitfield emitted non-volatile accesses

Eight hardcoded `volatile: false` sites in three lowering functions
(`store_bitfield` full-width branch, all four of `store_bitfield_split`, all three
of `extract_bitfield_from_addr`) meant an object-level volatile bitfield was
accessed non-volatilely. At -O2 that is a wrong answer, not a lost optimisation:

| probe | before | after | GCC 14 |
|---|---|---|---|
| N volatile full-width stores in a loop | 1 store | N stores | N stores |
| 4 unrolled volatile bitfield reads | 1 load | 4 loads | 4 loads |

Volatility is now resolved once at member-access entry and threaded to all three
functions; a split-load-only asymmetry is impossible by construction. Gate:
`tests/regression/check_volatile_bitfield_split.sh` (3 FAIL before, 0 after,
non-volatile twins as controls). The ratchet that would have caught the rename
that hid this is now in `scripts/check_volatile_destructuring.py` (parameter
rule, self-tested against the old signature verbatim).

### DEAD-BIND-1 · **CLOSED 2026-09-30** — 86 underscore bindings classified, 44 removed, 42 justified

`git grep -nE 'let _[a-z]' -- src/` returned 86. Deleted 44: pure dead statements
(a whole per-function map, two never-called closures, several aliases), plus the
call-preserving rewrites where the *expression* had effects
(`narrow_function(&mut func)`, 12× `add_shstrtab_name`, `parse_expr(lx)?`,
`fields.next()?`). One was a control-flow guard in disguise
(`let _out_name = match .. { None => continue }`), rewritten as a guard.
Kept 42, all RAII windows whose Drop restores state
(`EnvGuard`/`ScopedFlag`/`TriStateFlagWindow`/`EnvWindow`/`RestorePointerSize`/
`Target::set`/`LateMinMaxOnlyScope`) plus one uninitialised declaration.
Removing the dead `_iv_width_const` made its parameter unused, which the enforced
lint caught and which was then removed too — the cascade is the point.

### ALIGN-3 · **CLOSED 2026-09-30** — the attribute is honoured with parameters, and it replaces the default

Two follow-on defects in the same feature, both found by testing rather than by
the gate that was supposed to cover it (that gate's fixture was parameterless,
which is why it passed while both were live):

1. **A parameter list swallowed the attribute.** The alignment pending when a
   parameter list begins belongs to the ENCLOSING declaration, and the
   per-parameter attribute capture merged into that slot and took it. Measured:
   `aligned(64) void f(int x) { }` → `.p2align 4` where GCC emits `.align 64`;
   same for the prototype channel and `_Alignas`. Only the parameterless spelling
   ever worked. Fix: the list is parsed with the value held aside and restored at
   a single exit point.
2. **An attribute REPLACES `-falign-functions`, it does not merge with it.** GCC,
   `-O2`, `-falign-functions=32`: `aligned(2)` still `.align 2`, while an
   unannotated neighbour gets `.p2align 5`; `aligned(1)` emits **no directive**.
   LCCC emitted the attribute directive and let the default follow, so
   `.p2align` (which only advances) made the pair mean max(N, 16) — `aligned(2)`
   was 16-byte aligned and `aligned(1)` was 16-byte aligned. Fix: one value in
   one place, `Option<Option<u32>>` (absent / fixed / natural), consumed by every
   placement site; this also yields the single directive per entry that GCC
   emits.

Gate: `check_function_alignment_definition.sh` 4 → 14 assertions (parameterized
channels, replace-not-merge rows, one-directive-per-entry counts, two
unannotated controls), mutation-proven in both directions. Object-verified with
GNU as: after a one-byte pad, `aligned(2)` at offset 2, `aligned(1)` at an odd
offset, unannotated at a 16-byte boundary — as GCC's own `.s` produces.

### CI-FLAKE-1 · **CLOSED 2026-09-30 (burn-down continues)** — a gate reported SIGPIPE as a failure

`set -o pipefail` + a consumer that stops reading early (`head`, `grep -q`,
`grep -m1`, `grep -l`) makes the producer's SIGPIPE (141) the pipeline's status,
so a gate can print *"pattern not found"* about a comparison that succeeded.
Measured: 27/20000 false failures for `echo | grep -Eq`, 153/20000 for
`printf | grep -Eq`, **0/20000** for a here-string or `[[ =~ ]]`; reproduced end
to end as a 1-in-3 flake on the volatile gate against a byte-identical binary
(400 compiles, 1 distinct output), with `PIPESTATUS` showing `echo=141 grep=0`.

Fixed in every script CI runs: 47 pipelines whose consumer was `head` →
`sed -n '1,Np'`, 29 whose consumer was `grep -q` → `grep -c … >/dev/null` or a
here-string — 76 sites in 23 gates, one linker helper and the CI driver; each
keeps the producer's real status and reads to end of input.
`scripts/check_pipefail_sigpipe.py` (13-case adversarial self-test) enforces
it, wired into `ci.yml` and `ci_local.sh`. Verified scope:
89 shell scripts scanned, 0 gates that CI invokes left unscanned; previously
flaky gate now 0/30 runs.

**Remaining burn-down:** 82 sites across 30 developer-facing helper shell
scripts (`scripts/repro_claims.sh` 12, `tests/linker/setup_oracles.sh` 12,
`tools/linker/setup_oracles.sh` 7, `tests/regression/check_global_addr_cse.sh` 6,
…). `python3 scripts/check_pipefail_sigpipe.py --census` prints the list. These
are not CI reds, which is why they are a list rather than a gate.

## P0 — largest measured gaps

### TAG-ID-1 · **FIXED 2026-10-03** — nested same-tag structs no longer alias; the miscompile is gone

`f(s)` for the reproducer below is now a hard error matching GCC, and no
binary is produced. Pinned by
[`tests/regression/check_record_tag_identity.sh`](tests/regression/check_record_tag_identity.sh),
a 50-row differential against a probed GCC oracle, wired into
`ci_local.sh --fast` and hosted CI. Rows live in one table and the expected
count is derived from its length, so a row that silently stops running is itself
a failure -- the count had already drifted three times in prose (15, 16, 28)
while the gate ran something else. The summary prints the oracle's `--version`,
and the gate passes unchanged against both GCC 14.2 and GCC 16.2, which is the
evidence that the choice of oracle does not move a verdict. Disabling
`check_record_argument_compatibility` fails the gate, so it is not decorative.

The oracle is probed, not assumed. Hosted CI proved why: ubuntu-24.04's stock
gcc is 13.3.0, which does not know `-std=c23`, so five negative rows "agreed"
with lccc by both rejecting — GCC rejecting the FLAG, lccc rejecting the
PROGRAM — and the resulting red was misattributed to the compiler. The gate now
selects the first of `gcc-16 gcc-15 gcc-14 gcc` that compiles a trivial TU at
both `-std=c17` and `-std=c23`, fails with an explanation if there is none, runs
a positive control per dialect before any row, and requires every negative row
to be rejected *for the right reason* (`incompatible type` on both sides),
printing both diagnostics when a row goes red.

Three defects, not one:
1. `resolve_struct_or_union` keyed records by tag alone, so an inner
   `struct S { int c; }` and an outer `struct S { char c; }` were the same
   `CType`. Now a shadowing definition with non-corresponding members gets
   `struct.S#N`; corresponding members (N3037) keep the shared key so valid C23
   still compiles.
2. Every tag→`CType` site rebuilt the key from the raw source tag, so the
   distinct key never reached the type checker. A scoped `record_alias` map in
   `TypeContext` (unwound by `pop_scope`) now resolves them in
   `sema/type_checker.rs` and `sema/const_eval.rs`, including the four
   sizeof/alignof lookups, which are separate sites from the ones that build
   `CType`s.
3. `check_call_arguments` compared **only** arity and pointer/float mixing — it
   never compared record types at all. `check_record_argument_compatibility`
   now does, exempting anonymous records and `transparent_union` parameters,
   and reports the 1-based argument index the way GCC does.
   This third gap is why the first two fixes alone changed nothing, and it is
   the one worth remembering: a type-identity fix is inert if no comparison
   consumes the identity.

The plumbing has to be complete, not merely present. Two follow-on defects were
found by probing rather than by reading: the non-defining arm of
`resolve_struct_or_union` (a tag-only spelling such as `struct S b;` or a
parameter) returned the BASE key instead of the aliased one, which gave two
spellings of one type different identities inside the same scope and rejected
the valid assignment `struct S {int c;} a; struct S b; b = a;`; and
`record_layouts_correspond` compared member types by key, so anonymous members
— which get a fresh `__anon_struct_N` per conversion — never corresponded and a
valid C23 program was rejected outright. Both are gated.

Residual, asserted rather than hidden: definitions with **corresponding**
members share one key, which is what C23 requires and is too permissive pre-C23,
so `pos_n3037` and `pos_anon_member` under `-std=c17` are accepted by lccc and
rejected by GCC. Two rows pin that exact shape. It cannot miscompile —
corresponding members means identical layouts — and closing it needs `-std`
threaded into sema (which has no notion of it today) plus a mode-gated N3037
relation; making keys always distinct without that relation would reject valid
C23.

Original report, retained for the differential evidence. It is a `####`
sub-heading rather than an entry so this file keeps exactly one `### TAG-ID-1`
under P0 — two entries with the same id, one of them marked NEW, reads as an
open item and breaks any tool that scans entries by heading.

#### TAG-ID-1 · original report 2026-10-02 — nested same-tag structs shared one type identity (miscompile)

A correctness defect, ranked above the codegen gaps below because it silently
produces a wrong answer rather than a slow one. Reproducer and full evidence:
[`tests/bugs/nested_tag_struct_identity.c`](tests/bugs/nested_tag_struct_identity.c).

```c
struct S { char c; };
unsigned f(struct S p) { return (unsigned)(unsigned char)p.c; }
int main(void) { struct S { int c; } s = { 300 }; printf("%u\n", f(s)); }
```

lccc prints **44**; the correct answer is **300**. GCC 16.2 rejects the program
outright (`incompatible type for argument 1 of 'f'`). 300 is 0x12C and the
callee reads only the low byte.

Cause: struct/union types are keyed by tag alone
(`CType::Struct("struct.S")`) while struct *layouts* are scope-aware
(`TypeScopeFrame::struct_layouts_shadowed` restores them on `pop_scope`), so an
inner-scope definition of a tag silently rebinds the layout that the outer
declaration refers to and the two are indistinguishable downstream.

Differential vs GCC 16.2 — lccc accepts every row GCC rejects:

| case | gcc | lccc |
|---|---|---|
| member type differs | REJECT | accept |
| member name differs | REJECT | accept |
| member count differs | REJECT | accept |
| bit-field width differs | REJECT | accept |
| bit-field signedness differs | REJECT | accept |

And the defect is broader than C23: on the *valid* N3037 case GCC rejects under
`-std=c11` and `-std=c17` (6.7.2.3 makes an inner-scope definition a new,
incompatible type) and accepts under `-std=c23`; lccc accepts in all three, so
it is only accidentally right in C23 when the members happen to correspond.

Not a one-line fix, and that is why it is P0 rather than a patch: the tag-keyed
form is assumed across layers. `src/ir/lowering/` rebuilds keys by hand from the
AST tag (`format!("struct.{}", tag)` in `expr_access.rs`, `stmt.rs`,
`structs.rs`, `const_eval.rs`), `TypeSpecifier` carries the raw tag while
`CType` carries the prefixed key, and there are three `resolve_struct_or_union`
implementations behind the `TypeBuilder` trait. The plan in the reproducer is:
a per-definition key stack in `TypeContext` with one resolver used by every
lookup site; a distinct key only when a definition shadows a visible one, so
6.7.2.3p2 (same tag at file scope = same type) is preserved; then the N3037
compatibility relation for C23 only; then each matrix row pinned as a
`tests/regression/` case with GCC's verdict as the expectation.

Found by the EDG `src/Changes` distillation (E7): the top-scored C-relevant
entry in `docs/edg_changes_c_extract.md` is "C23: New tag compatibility rules"
(N3037, *C-score +10*), which is what made the differential worth running.

### PTR-COMPAT-1 · **FIXED 2026-10-04** — call arguments never checked pointer compatibility

`check_call_arguments` compared arity plus pointer/float mixing and nothing
else. TAG-ID-1's fix closed the by-value record path; a *pointer* to one of the
two distinct same-tag records walked straight through the `_ => return` arm, so
the miscompile's nearest neighbour was still accepted:

```c
struct S { char c; };
void f(struct S *p);
void g(void) { struct S { int c; } s; f(&s); }   /* GCC: error; lccc: accepted */
```

C23 6.5.2.2p7 makes argument/parameter compatibility a constraint, and GCC 14
onward rejects the pointer spelling by default, so this was not a warning
difference. The gap was much wider than records: `int *` where `char *` was
expected, `long (*)(void)` where `int (*)(void)` was expected, and `struct A *`
where `struct B *` was expected were all silently accepted.

`pointer_argument_compat` / `pointee_compat` now implement 6.3.2.3 plus
6.7.6.1p2, recursing through pointer and array chains so `struct S **` is caught
too. Every rule was pinned against GCC 16.2 rather than read off the standard,
because the standard's "compatible" and GCC's default diagnostics differ:

| case | GCC 16.2 | rule implemented |
|---|---|---|
| `int *` → `void *`, and back | accept | 6.3.2.3p2, **top level only** |
| `void **` → `int **`, both directions | REJECT | the exemption does not recurse |
| `int (*)[3]` vs `int (*)[4]` | REJECT | array size is part of the type |
| `int (*)(void)` vs `long (*)(void)` | REJECT | signature compared |
| `char *` → `signed char *` / `unsigned char *` / `int *` → `unsigned *` | accept | `-Wpointer-sign` is a warning |
| `_Bool *` → `int *` | REJECT | not a sign pair |
| `long *` → `long long *`, `float *` → `double *` | REJECT | different types |
| `f(0)`, `f(NULL)` | accept | null constant: the check stays silent |

`Unknown` is a first-class verdict meaning "these shapes are not modelled
precisely enough to decide". This check is being introduced where nothing ran
before, so firing on a shape it does not understand would reject valid code — a
strictly worse failure than the one it closes. Measured: a 81-case differential
against GCC 16.2 went from 11 divergences to 4, with **zero false rejections**,
and the 887-case regression corpus is unchanged at 874 passed / 0 failed.

Two things this found on the way, both fixed:

* **`FunctionType` derived `PartialEq` over `params: Vec<(CType, Option<String>)>`,
  so parameter NAMES were part of function-type compatibility.** C23 6.7.6.3p15
  keeps names out of the type. Comparing with `==` rejected `call(dbl)` where
  `dbl` is `int dbl(int x)` and the parameter is `int (*)(int)` — i.e. every
  callback in C. `function_types_compatible` compares return type, arity and
  parameter types only, and a unit test asserts the derived `PartialEq` still
  differs so the distinction cannot be silently re-broken.
* **Parameter types were converted twice, and only one converter was correct.**
  `type_builder::convert_param_decls_to_ctypes` resolved just `p.type_spec`,
  while `SemanticAnalyzer::param_decl_ctype` also handled `fptr_params`.
  A prototype reaches the compiler through the former and a definition through
  the latter, so the same signature was typed differently depending on whether
  the body had been seen: `void f(int (*p)(int));` typed `p` as `int *` and
  every call to `f` with a real function pointer was a hard error, while the
  identical call to the *definition* was fine. The analyzer's own doc comment
  already recorded this failure mode for the SQLite amalgamation; it had been
  fixed on one side of the duplication and not the other. One implementation now
  lives in `type_builder::param_decl_to_ctype` and both sides call it.

### ANON-KEY-1 · **NEW 2026-10-04** — anonymous record keys are not canonical across construction paths

An anonymous `struct { ... }` gets a fresh `__anon_struct_N` key per conversion,
and source-level spellings are stable — verified on nine (typedef by value, by
pointer, as a return type, as a member, in an array, as an array element, behind
a pointer member, in `_Generic`, and a declarator pair). But a builtin's
parameter type is built by a different path than the header typedef it matches,
so `__m128i` can hold two different keys for one type.

Consequence: both `check_record_argument_compatibility` and `pointee_compat`
must exempt `__anon_` keys, which costs one missed diagnostic (an anonymous
record passed where a named one was declared — GCC rejects, lccc accepts).

Removing the exemption was tried and measured, which is why the cost is known:
70 semantic errors in `include/emmintrin.h` alone and **19 corpus failures**, all
of them SSE/AVX (`_mm_loadu_si128`, `_mm_storeu_si128`, `_mm_cmppd`). The
exemption is load-bearing, not lazy. Canonicalising anonymous keys across
construction paths would let both checks tighten and would close the missed
diagnostic; until then any attempt must be measured against the SIMD corpus.

### PARSER-ALIGN-1 · **NEW 2026-10-04** — `_Alignof` in `_Static_assert` ignores block scope

```c
struct S { char c; };
int h(void) {
    struct S { int c; } a; (void)a;
    _Static_assert(_Alignof(struct S) == 4, "reads 1");   /* fails */
    return 0;
}
```

`_Static_assert` is evaluated by the **parser** (`parse_static_assert`), which
resolves `_Alignof(struct S)` through its own `struct_tag_alignments` map. That
map is keyed by bare tag and is never unwound at block scope — the parser has no
block scoping at all, only a save/restore around nested functions — so inside a
shadowing scope it still holds the outer record's alignment.

Bounded precisely: `sizeof` is unaffected (it resolves in sema), and the same
`_Alignof` is correct in an array bound and in an enum constant, which also go
through sema. Only `_Static_assert` is wrong. Pinned by the
`gap_alignof_static_assert` gate row as a stated divergence, so it cannot widen
and a fix turns the row red. Fixing it means giving the parser block-scope
tracking, which is a parser-core change well outside a type-identity fix.

### CONST-QUAL-1 · **NEW 2026-10-04** — `CType` does not encode qualifiers

`CType` has no `const`/`volatile` variant; the flags ride on `ParamDecl`
(`is_const`, `is_volatile`, `is_restrict`) for parameter lists only. So
`struct S { const int i; }` and `struct S { int i; }` have identical member
types and correspond, where C23 6.7.3p10 says they do not and GCC rejects the
call. Fixing it means threading qualifiers through the type enum, every
comparison, IR and the ABI — a project, not a patch, and out of scope for a
record-identity fix. Recorded because the differential found it, not guessed it.

### INTPTR-1 · **NEW 2026-10-04** — no integer/pointer conversion diagnostic

`int call(int (*p)(int)) { return p; }` is `-Wint-conversion`, an error by
default since GCC 14; lccc accepts it. Same class as PTR-COMPAT-1 but on the
return path rather than the argument path, and it needs the scalar-conversion
half of 6.5.1.6 rather than the pointer half.

### REDEF-1 · **NEW 2026-10-04** — redefining a record in one scope is not diagnosed

```c
struct S { char c; };
int h(void) { struct S { int c; } a; struct S { int d; } b; ... }
```

GCC: `error: redefinition of struct or union 'struct S'`. lccc accepts it. This
is also the shape that would exercise the alias-undo bookkeeping twice in one
frame, so `set_record_alias_from_ref` now records at most one undo entry per
base key per frame — without that, `pop_scope` removes the `added` entry and
then replays the `shadowed` one, resurrecting an alias that belongs to a scope
which no longer exists. The guard is in place even though the diagnostic is not;
a unit test covers the double-set/pop round-trip.

### IVSR-PTRADD-1 · **FIXED 2026-10-06** — the pointer scan never saw LCCC's own addressing form

`try_lower_pointer_arithmetic` (`src/ir/lowering/expr_ops.rs`) lowers **every** C
subscript to `Add(ptr, scale_index(i, elem_size))` in pointer-width integer
arithmetic — `scale_index` is the identity for `elem_size == 1` and a
`Mul(i, elem_size)` otherwise. It does not emit `GetElementPtr`. IVSR's
`find_derived_exprs` collected only `GetElementPtr` offsets, so the pointer
recurrence never fired on the most common addressing idiom in C: byte-buffer
walks got no recurrence at all, and wider-element arrays found the `Mul` but no
GEP consuming it, so the whole group was dropped.

`tests/bench/k_varint.c` `-O3`, `bench_run`, before:

```
.LBB6:  movslq %esi, %rdx          # rebuild &v[i] from scratch
        leaq   v(%rip), %r8        #   ... every single iteration,
        addq   %rdx, %r8           #   ... including the invariant global
        movzbl (%r8), %r9d
```

after: `leaq v(%rip), %rdi` is hoisted to the preheader and the three loads
become `movzbl (%rdi,%rdx)`, `movzbl 1(%rdi,%rdx)`, `movzbl 2(%rdi,%rdx)`.

Measured (Callgrind `Ir` for `bench_run`, deterministic; the pinned cache
geometry is in the evidence file):

| kernel | before | after | GCC 14.2 | after/GCC |
|---|---|---|---|---|
| `varint` | 3,578,595 | **3,235,371** (−9.59%) | 2,533,635 | 1.412 → **1.277** |
| `matchlen` | 10,340,483 | **10,333,427** (−0.07%) | 12,019,475 | 0.860 |
| suite total (11 kernels) | 71,225,717 | **70,875,430** (−0.49%) | 59,498,729 | 1.197 → **1.191** |

All 11 kernels: pre/post/GCC **checksums identical**. Wall clock was NOT used as
the decision metric — `scripts/bench_kernels.py` itself reported 7 of 11 kernels
moving beyond its 3% tolerance with byte-identical codegen on this host, a ~15%
layout-noise floor that swamps a 9.59% effect.

Static blast radius, measured rather than assumed:

- benchmark + oracle corpus, 66 sources × 6 configurations = **402 comparisons,
  0 changed**, 87386 → 87386 instructions;
- regression corpus x86-64 `-O2`, **841 comparisons, 6 changed**, 212203 →
  212207 instructions (+4), 31001 → 30998 stack refs (−3): `simd_vecreg` −2/−2,
  `accumulator_pointer_load` −1, `memcpy_unaligned_load_fwd` +1,
  `temp_promotion_window` +1, `simd_crc_adler` +2/−1, `simd_avx2_defer_chain` +3.

The +4 static / −350,287 dynamic trade is stated plainly: this is a runtime-shape
win that costs a few instructions in four synthetic x86-64 regression files.

**ILP32 is excluded**, on measurement not taste: without the gate,
`simd_crc_adler.c` `-O2 -m32` grew **142 → 191 stack refs (+49)** for +2
instructions, while the same source on x86-64 was +2/−1. The recurrence adds a
loop-carried pointer web and a 6-GPR file parks it in a slot at the latch — the
documented cost that keeps `CCC_IVSR_SCALAR_DERIVED` opt-in. With the gate every
`-m32` difference disappeared (18 changed files → 6). Correctness on ILP32 is
still covered: `tests/regression/ivsr_address_add.c` runs and passes on `-m32`
at `-O0..-O3`.

Guards beyond the ILP32 gate: the Add must be in the **pointer ring**, and its
result must be **used as an address** (`Load.ptr`, `Store.ptr`,
`GetElementPtr.base`, `Memcpy`/intrinsic pointer argument, delegated to
`IntrinsicOp::reads_pointer_arg`/`writes_memory_via_args` so a new vector memory
opcode is picked up automatically). An integer accumulation `invariant + iv` that
nobody dereferences is left exactly alone — that is the scalar-derived net loss.
Kill switch `CCC_NO_IVSR_PTR_ADD`.

**Applicability is bounded, and the bound was measured on real code.** The win is
on *index-spelled* byte walks. It does **not** generalise to every byte-buffer
codebase, and claiming otherwise would be overclaiming:

| corpus | comparisons | changed | net |
|---|---|---|---|
| **gzip 1.14** (136 real sources, `-O2` and `-O3`) | **238** | **0** | 44430 → 44430 insns, 7980 → 7980 stack refs |

Verified per-file with `CCC_NO_IVSR_PTR_ADD=1` A/B on the six hot translation
units — `deflate.c`, `inflate.c`, `zip.c`, `unzip.c`, `trees.c`, `bits.c` — all
six produce **byte-identical assembly with the arm on and off**, i.e. the arm never
fires there. The reason is in the source: gzip already spells its hot loops as
pointer increments, so there is no `Add(ptr, iv)` left to fold —

```c
} while (*(ush*)(scan+=2) == *(ush*)(match+=2) && ...   /* deflate.c:436 */
} while (*++scan == *++match && *++scan == *++match ... /* deflate.c:469 */
do { putc(window[start++], stderr); } while (--length != 0);  /* deflate.c:515 */
```

The code is *already in the form this optimization produces*. So IVSR-PTRADD-1
helps the `p[i]` spelling — which is what `k_varint.c` models, and that kernel is
SQLite-derived, matching the `sqlite_varint` shape — and is exactly neutral on
hand-written pointer-walking C. Both facts are the result: a real −9.59% on the
shape it targets, and a measured zero on a real compressor that does not use that
shape. Reproduce with `./configure` in an unpacked gzip 1.14 (archive SHA-256
`01a7b881bd220bfdf615f97b8718f80bdfd3f6add385b993dcf6efd14e8c0ac6`, pinned in
`artifacts/`) and compiling with `-I. -Ilib -include lib/config.h`.

This also sharpens where the remaining `varint` gap lives: at 1.277× GCC after the
fix, the residue is not addressing any more. Of 13 instructions in the common
1-byte path, 4 are pure data movement (`movl %r9d,%r11d`, `movq %r11,%rbx`,
`movl %r12d,%r8d`, and an unconditional `jmp` from block layout) — copy
coalescing and block layout, i.e. RA-CSAVE-1 class, not IVSR.

Tests: `tests/regression/ivsr_address_add.c` (byte stride 1, nonzero and
affine starts, wide-element multiply, backward walk, `size_t` index; each checked
against an independently computed reference), four unit tests
(`address_forming_add_becomes_a_pointer_recurrence`,
`pointer_width_add_that_nobody_dereferences_is_left_alone`,
`ptr_add_parameter_is_a_real_kill_switch`,
`address_add_arm_is_gated_by_ilp32` — the last two were split out of
`address_add_arm_is_gated_by_the_kill_switch_and_ilp32` by the review-hardening
round, which is why an older revision of this entry names one test), wired into
`check_ivsr_domains.sh` on x86-64 **and** i686.
Evidence: `engineering/evidence/2026-10-07-perf-ivsr-ptradd/callgrind-kernel-ab.json`.

### SELECT-CHAIN-FOLD · **PROPOSAL REJECTED 2026-10-06** — fewer static instructions, more executed ones

`k_classify` is 2.057× GCC and `k_namechars` 1.653×, the two worst ratios in the
kernel suite. C's `a || b || c || d` if-converts to a chain
`Select(c, 1, Select(c2, 1, Select(c3, 1, Y)))`, and `c ? K : (c2 ? K : Y)`
collapses to `(c|c2) ? K : Y` by case analysis alone — no domain, range or
no-wrap assumption. Implemented, with the OR formed in the *narrower* carrier
after peeling integer casts off known-boolean conditions (widening measured
worse: it sign-extended each byte term to 64 bits before an `orq`).

Result: `bench_run` 49 → 48 instructions, **but Callgrind `Ir` 5,962,067 →
6,118,811 (+2.63%)** on `classify`, `namechars` unchanged, suite total −0.49% →
−0.27%. The folded form makes every term unconditional where the chain previously
short-circuited through the branch. **Reverted byte-identically** (`cmp` against
the pre-fold build).

The gap it targeted is still open and is **not** an IR-shape problem. The
residual cost is backend boolean materialisation: each `setcc` is followed by a
redundant `movzbl` *and* a `movsbq`/`movsbl` of the same byte, and the surviving
Select still costs `movl $1, %ecx; movq %rN, %rM; cmovneq %rcx, %rM`. That is
register allocation and copy coalescing (RA-CSAVE-1 class). Do not retry this at
the IR level.

### IVOPTS-1 · **PARTIALLY CLOSED 2026-10-06** — shared scalar/SIMD address recurrences

The scalar half of this item is now closed by IVSR-PTRADD-1 above: the
address-forming `Add` that LCCC's lowering actually emits is recognised, so
scalar GEP-free subscripts get a pointer recurrence. **Still open:** the vector
half. `is_used_as_address` already recognises `VecLoad*`/`VecStore*` pointer
arguments via `IntrinsicOp::reads_pointer_arg`/`writes_memory_via_args`, so the
shapes are now *visible* to the scan, but SLP `VecLoadF64x2`/`VecStoreF64x2`
carry base and byte offset as separate operands rather than as one `Add`, so
there is still no single address value for the recurrence to attach to. That is
the remaining boundary and it needs the intrinsic operand form, not the scan.

### IVOPTS-1 · **RE-SCOPED 2026-10-06** — shared scalar/SIMD address recurrences

IVSR is **already implemented** in `src/passes/iv_strength_reduce.rs`, with
innermost-first selection, same-base recurrence grouping and affine forms.
`global_addr_cse`, LICM, IV widening and un-IVSR also exist. The missing work
is coverage/profitability/proof across their interfaces, not adding a first IV pass.

The old **110 vs 14** claim compared unidentified loop scopes. On pinned
upstream, `hot_loop_metric.py --symbol main --all-loops` classifies `.LBB7`
as a **composite outer loop**, not the pair-loop body. The actual `.LBB12`
pair loop has **43 instructions**, versus **29** in GCC 16.2 `.L18`; after
FP-LANE-1 it has **37**. These are static counts per pair, not cycle counts.
Whole-TU LCCC/GCC is 302/215 before and 296/215 after. The claimed zero stack
traffic is false for this code: the pair loop stores/reloads two extracted FP
lanes. The old mnemonic census mixed memory loads/stores with register copies
and SSE with VEX spellings; it cannot establish redundant FP moves.

**Concrete remaining boundary:** SLP `VecLoadF64x2`/`VecStoreF64x2` carry
base/byte-offset operands directly; IVSR enumerates only `GetElementPtr`
offset uses. A multiply retained for an intrinsic reader cannot die just
because a scalar GEP acquired a pointer recurrence. Reproducer:
`tests/benchmark/programs/nbody.c`, `-O2 -march=x86-64-v3`,
`LCCC_DUMP_IR=1 CCC_IVSR_DEBUG=1` (both dumps go to stderr).

Next experiment: share a *proven* byte-address recurrence between scalar GEPs
and vector memory intrinsics, preserving displacement, real/hidden uses,
SSA dominance and nonzero-loop guards. Reject if the pressure cost exceeds
the removed scaling. Do not enable scalar-derived IVs or GLA spill gaps globally.
Before expansion, prove wrap/cast legality: IVSR-WRAP-1 below was real wrong code.

### IVSR-DOMAIN-1 · **FIXED 2026-10-06** — a signedness reinterpretation was called "widening"

`find_derived_exprs` admitted any cast with `to_ty.size() >= from_ty.size()` as
IV-derived. Equal size therefore passed, so `U32 -> I32` was treated as a
value-preserving step of the chain and a following `I32 -> I64` sign-extended
it. `p[(int32_t)i]` with `i` crossing `UINT32_MAX` must visit subscripts
`-2,-1,0,1`; the derived pointer recurrence instead walked forward from
`+4294967294 * stride`. Two new reproducer programs (`ivsr_signedness_domain.c`,
`ivsr_unsigned_sparse_wrap.c`) segfaulted at `-O2`/`-O3` on upstream `6051e87`
and return the correct answer with GCC 14/16.2, Clang 23.1, ICC 2021.10 and ICX
latest. No pre-existing shipped benchmark was affected: the defect needed an
index that crosses `UINT32_MAX`, which none of the 55 corpus programs has.

The predicate is now `cast_preserves_offset_value`, which separates two
questions the old size comparison conflated:

- **value preservation over the whole source domain** — widening an unsigned
  source, or a signed source into a signed target. `U32 -> I32` fails this.
- **preservation of the value that reaches the pointer ring** — a same-width
  signedness reinterpretation *at or above* the pointer width (`U64 <-> I64` on
  LP64, `U32 <-> I32` on ILP32) cannot change the pattern the GEP consumes, so
  the ordinary `size_t`/`ptrdiff_t` spelling stays reducible.

Getting this distinction wrong in either direction is measurable: the
value-only predicate silently cost `histogram` +5, `linux_rbtree` +12
instructions/+7 stack refs, `linux_find_bit` +1, `i686_alu_chains` +2 and
`zlib_ng_adler32_combine` +1 over the 55-program default corpus, because their
64-bit indices are spelled `U64 -> I64`. With the ring-aware predicate the
corpus is byte-for-byte identical to baseline (8139 vs 8139 instructions) while
both miscompiles are gone.

Regression: `tests/regression/ivsr_signedness_domain.c` (32- and 64-bit signed
views plus the affine `i + 0x80000000` form), and the four-vendor oracle program
`tests/oracle/programs/ivsr_index_domains.c`. Gate: `check_ivsr_domains.sh`.
Unit: `derived_signedness_then_widening_is_not_a_pointer_iv`,
`offset_cast_proof_adds_only_the_pointer_ring_reinterpretation`,
`derived_cast_proof_is_about_values_not_storage_size`.

### IVSR-DOMAIN-2 · **FIXED 2026-10-06** — an unproven modular unsigned counter became a linear pointer

IVSR-WRAP-1's own closing caveat ("this does **not** prove all unsigned 32-bit
pointer recurrences sound") was the defect. A `uint32_t` counter with no
provable bound is modular; zero-extending it into `p += step*stride` is linear
only while the executed backedges cannot wrap. `p[i]` over a sparse 16 GiB
`uint32_t` index space with `i` crossing `UINT32_MAX` segfaulted at `-O2`/`-O3`
on `6051e87`.

`unsigned_iv_bound` now discharges the obligation locally: the loop must be
single-latch with the header *not* its own latch, the header's conditional
terminator must branch on a `Cmp` of **this exact phi** in **the phi's own
unsigned type**, the polarity must match the step sign after normalising for
which target stays inside the loop, the other operand must be a constant or a
loop-invariant value, and the step must be exactly ±1 with a strict limit. It
returns the proven body range and whether the limit was a compile-time
constant. No bound ⇒ no pointer recurrence; the modular index recurrence is
still available and always correct.

Regression: `tests/regression/ivsr_unsigned_sparse_wrap.c` (16 GiB virtual,
four committed pages, guard pages everywhere else — a wrong recurrence faults
instead of returning a plausible number).

### IVSR-OFFSET-1 · **NEW 2026-10-06** — the offset product's own ring was never checked

Even with a proven non-wrapping IV, `offset = (iv + k) * stride` is evaluated in
`mul_ty`, and the recurrence is formed in the pointer ring. Those agree only if
the product cannot wrap inside `mul_ty`. A signed `mul_ty` cannot wrap in a
defined program (C17 6.5/5 — the theorem `iv_widen` already relies on) and a
product at or above the pointer width wraps in the ring the recurrence lives
in, so both are equivalent by construction. An **unsigned product narrower than
the pointer ring** is a genuine hazard and is now discharged by
`offset_product_cannot_overflow`, which needs the *exact* constant bound from
`unsigned_iv_bound`. This replaces the blanket "veto every sub-pointer-width
multiply" that a first attempt used and that would have silently forbidden the
very common constant-trip `i * 4` in 32 bits. Unit:
`offset_product_proof_needs_an_exact_bound_only_below_the_pointer_ring`,
`unsigned_bound_records_whether_the_limit_was_a_constant`. No shipped workload
currently reaches the narrow-unsigned-product arm; it is a proof, not a
measurement.

### IVSR-SAMEWIDTH-1 · **PROPOSAL REJECTED 2026-10-06** — relaxing the backedge cast peel to equal width

An external review proposed widening `look_through_casts` from `from_ty == to_ty`
to `from_ty.size() == to_ty.size()`, arguing that a phi's backedge cast is
"unavoidably narrowing" for sub-int counters and that same-width
signedness-only reinterpretation carries no wraparound risk.

Measured: building the relaxation and re-running the 55-program default corpus
gives **8139 vs 8139 instructions — zero difference, no program changed**. It
recovers no optimization on this tree.

It is also not free of soundness cost. The "signed overflow is UB" theorem that
licenses a linear recurrence for a signed IV does not cover a signed counter
*incremented in unsigned arithmetic*: C17 6.3.1.3p3 makes `INT_MAX -> INT_MIN`
implementation-defined, not undefined, so the sequence
`2147483646, 2147483647, -2147483648, -2147483647` is legal, is not linear, and
has consecutive bit patterns — exactly the shape a same-width peel would accept.
`tests/regression/ivsr_signed_wrap_impldef.c` pins the end-to-end behaviour over
a 16 GiB sparse mapping. In practice LCCC's frontend lowers
`i = (int)((unsigned)i + 1u)` straight to `Add(I32)`, so the shape does not
currently arise from C source; that is an implementation detail of the frontend,
not a theorem, and it is not a reason to weaken the pass.

**Decision: keep `from_ty == to_ty`.** Zero measured benefit, a real (if
currently unreachable) soundness cost, and a frontend lowering that already
normalises the idiom away. Revisit only with a workload where it fires.

### CAST-PREDICATE-1 · **DONE 2026-10-06** — one rule, three spellings, and the weakest one miscompiled

The `to_ty.size() >= from_ty.size()` test that caused IVSR-DOMAIN-1 was not a
novel mistake; it was a *third* copy of a rule the tree already had.
`src/backend/generation.rs`'s SIB-index peel carried the correct whole-domain
value-preservation condition inline, complete with a comment describing the
`I32 -> U32 -> I64` miscompile it prevents; `src/passes/loop_carried_forward.rs`
carried a same-width peel that is sound only because that analysis never follows
a sub-pointer-width integer. Neither was stated as a predicate another pass could
call, so `iv_strength_reduce.rs` grew its own and got it wrong.

Both notions are now methods on `IrType` — `cast_preserves_integer_value` (value
preserved over the whole source domain) and `cast_preserves_offset_value` (that,
or a same-width reinterpretation inside the target's pointer ring) — with the
exhaustive tests beside them in `src/common/types.rs::cast_predicate_tests`.
`generation.rs` calls the first; `loop_carried_forward.rs` documents which arm of
the second its structural invariant already implements.

The replacement in `generation.rs` is provably equivalent, and that proof was
measured rather than asserted: **5388 assembly byte-comparisons over 911 sources
(`-O2`, `-O3`, `-O2 -march=x86-64-v3` × x86-64 and i686) are 5388 identical,
0 differing**. Evidence:
[`engineering/evidence/2026-10-06-ivsr-domain/refactor-neutrality.json`](engineering/evidence/2026-10-06-ivsr-domain/refactor-neutrality.json).

Do not add a fourth spelling. If a new transform needs to know whether a cast may
disappear into a linear address expression, it calls one of these two methods and
says which one it means.

### SETMEM-SHAPE-1 · **NEW 2026-10-06** — `set_membership` cannot see either shape the classifier kernels actually produce

`classify` (2.057× GCC Ir, 1.208× wall) and `namechars` (1.653× Ir, 1.248× wall)
are the two remaining GENUINE gaps in the `tests/bench` suite — worse on **both**
metrics (see METRIC-TRAP-1 for why that conjunction is the filter). They are the
same defect, and it is a **shape-recognition** gap in `src/passes/set_membership.rs`,
not a backend codegen gap. `set_membership`'s own module doc names the target:
GCC's `subl $45,%edi; cmpb $50,%dil; ja miss; btq %rdi,MASK` classify idiom,
citing Expat `xml_name_continue` and SQLite varint — exactly these kernels.

Both forms miss, for **different** verified reasons.

**(a) `classify` — the value form never has the block chain at all.**
`c += (unsigned)is_name_char(s[i])` returns `a || b || c || d` as a *value*, and
the **frontend** lowers that straight into a three-link `Select` chain inside one
block:

```
Select v80 = v65 ? 1 : v70        # v70 = (I64)('.' == ch)
Select v81 = v61 ? 1 : v80        # v61 = '_' == ch
Select v82 = v74 ? 1 : v81        # v74 = (I64)(digit range test)
```

`set_membership` requires `T_i: <pure tests>; CondBranch cond -> JOIN, T_{i+1}`
with `JOIN: Phi [(Const(1), T_1), ...]`. That shape **does not exist at any point
in the pipeline** for this kernel, so transformation #2 (BIT-MASK CLUSTER, the
`btq` idiom) is unreachable. Verified: `Select` count in the final IR of
`bench_run` is **3 under every one of** `CCC_DISABLE_PASSES=` (none), `ifconv`,
`cfg`, `ifconv,cfg`, `boolthread`, and `ifconv,cfg,boolthread,set_membership`.

> **Falsified hypothesis, recorded so it is not retried.** The obvious guess is
> pass ordering — `if_convert` runs at `src/passes/mod.rs:2389` and
> `set_membership` at `:2489`, so if-conversion would destroy the chain before the
> membership pass could see it. It is **not** the cause: disabling `ifconv`
> changes neither the Select count (3) nor the assembly (49 instructions both
> ways). The Selects are produced by the frontend's lowering of `||` in a value
> context, so no reordering of optimization passes can expose a chain that was
> never built.

`set_membership` still fires on the residual `Range` pair and performs
transformation #1 (CASE-FOLD PAIR MERGE, the `andl $-33` ASCII letter fold):
disabling it costs **+7 static instructions** (49 → 56). So the pass is doing
real work here; it is blocked from the *bitmap* half, which is where GCC's
25 instructions-per-byte against LCCC's 52 comes from. No `bt`/`movabsq` appears
in `bench_run` under any of the six configurations.

### RANGEFOLD-OR-1 · **NEW 2026-10-07** — the case-fold pair merge has no `Or`-spelling counterpart (this is the bounded form of SETMEM-SHAPE-1(b))

SETMEM-SHAPE-1(b) recorded that `set_membership` does not fire on `namechars` and
attributed it to the counting-form join Phi. That is true but it is **not the
first blocker**, and the first one is much smaller and precisely located.

`range_fold` (`src/passes/range_check.rs`, `run_function`) DOES fire — on each
conjunct separately. The IR it leaves behind:

```
Load  v19 = *p                       ty I8
Sub   v67 = v19 - 97                 # 'a'
Cmp   v68 = Ule(v67, 25)             # [a-z]  folded to the unsigned-bias form
Sub   v69 = v19 - 65                 # 'A'
Cmp   v70 = Ule(v69, 25)             # [A-Z]  folded too
Or    v73 = v68 | v70                # <-- NOT merged; 5 instructions, not 3
CondBranch v73 -> JOIN, next_test
```

`try_fold_bool_op` is the function that merges an `And`/`Or` of two range tests,
and it delegates to `extract_range(&bound1, &bound2, and_form, cast_defs)`, which
requires **both bounds to compare the same value**. After `range_fold` has done
its job the two operands are `v67 = ch-97` and `v69 = ch-65` — *different*
values — so `extract_range` returns `None` and the `Or` survives. The pass that
folds the ranges is the same pass that then cannot see through its own output.

What is missing is `set_membership`'s transformation #1 (CASE-FOLD PAIR MERGE)
applied to this spelling. Its exactness conditions are already written down and
already tested there (`case_fold_requires_bit5_clear`):

```
a2 == a1 + 32  &&  b2 == b1 + 32  &&  (a1 & 32) == 0  &&  a1 >= 0
                                   &&  (a1 >> 5) == (b1 >> 5)
```

which `[65,90]` / `[97,122]` satisfies (65&32==0, 65>>5 == 90>>5 == 2). Clearing
bit 5 maps `[a+32,b+32]` onto `[a,b]` and maps nothing else into it, so the merge
is exact for every input in the domain — for a byte classifier that is checkable
by exhausting all 256 values, which is the test this should ship with.

The rewrite is `Or(Ule(Sub(x,a2),s), Ule(Sub(x,a1),s))` →
`Ule(Sub(And(x, ~32), a1), s)`: **5 instructions → 3** in the hottest part of the
loop. On `k_namechars` that is ~41.8 → ~39.8 Ir per byte-iteration (**−4.8%**,
1.653× GCC → ~1.57×); suite-wide ≈ −0.3%. It also removes an inconsistency rather
than just chasing a number: `classify` gets this fold through `set_membership`'s
block-chain path and `namechars` does not, from the *same* C source idiom
(`isalpha`-style `(c>='a'&&c<='z') || (c>='A'&&c<='Z')`) spelled once as a value
and once as a branch.

### RANGEFOLD-OR-2 · **IMPLEMENTED, MEASURED, REVERTED 2026-10-07** — the case-fold `Or` merge is correct but does not fire where it was needed

RANGEFOLD-OR-1 specified this transform and estimated **−4.8% Ir** on
`k_namechars`. It was then implemented, to test that estimate rather than argue
it. The estimate was wrong, and the reason is more useful than the number.

**What was built.** `try_case_fold_or` in `src/passes/range_check.rs`, reached as
a fall-through from `try_fold_bool_op` when `extract_range` declines (which it
does whenever this pass has already folded each conjunct, because the two operands
are then `x-97` and `x-65` rather than one shared value). It recognises
`Cmp(Ule, Sub(x, lo), span)` on both sides of an `Or` via the `binop_defs` map
`DefMaps` already collects, requires one shared value *and* one shared cast chain
*and* equal spans, and applies `set_membership`'s exactness preconditions
verbatim (`a2==a1+32`, `b2==b1+32`, `(a1&32)==0`, `a1>=0`, `a1>>5==b1>>5`).
Bounds are normalised through the pass's own `domain_of`/`dom_value`/`fits_domain`
so a constant arriving as a sign-extended bit pattern is refused rather than
reinterpreted. Four unit tests, including one that violates each precondition
separately and one that offers the same range constants over two *different*
values. **4152 lib tests passed.** The transform is correct in isolation.

**What it did end-to-end: nothing at all.** Differential over
`tests/benchmark/programs` + `tests/oracle/programs` + `tests/bench`, `-O2`, both
targets — **158 comparisons, 0 changed, 31619 → 31619 instructions, 8611 → 8611
stack refs.** The transform is *provably inert*: it never fires anywhere in this
corpus, on any target, at any of the flag sets tried. `k_namechars` and
`k_classify` — the two kernels it was written for — are byte-identical with and
without it.

> **A measurement error made here, and how it was caught.** The first differential
> was run against a compiler binary saved *before* the ILP32 gate was added to
> IVSR-PTRADD-1, and reported "2 of 158 configurations changed, one better
> (`k_matchlen -O2 -m32`, −2 insns / −5 stack) and one worse (`k_varint -O2 -m32`,
> +4 / +3)". That was read as the case-fold firing incidentally with mixed sign.
> It was not: both differing configurations are `-m32`, and re-running the same
> differential between the *ungated* and *gated* IVSR builds reproduces exactly
> those two rows and no others. The 2 diffs were the ILP32 gate doing its job,
> and the case-fold contributed zero. The lesson is the one this repo's own
> `differential_corpus.sh` header states — an A/B is only as good as the
> provenance of its two arms, and "which build is the baseline" has to be checked,
> not assumed. Every number in this entry is now from arms whose provenance is
> pinned: (A) case-fold build vs reverted build = 0/158 changed; (B) gated
> pre-fold build vs reverted build = 0/158 changed, so the revert is clean;
> (C) ungated vs gated = exactly those 2 `-m32` rows.

**Why it does not fire, and the correction this makes to RANGEFOLD-OR-1.** The
blocking shape is real and appears in the *final* IR, but it does not exist while
`range_fold` is running: the `Or` of two already-folded range tests is produced
*after* that pass's last invocation, so the fall-through never sees it. This is
**not** the `if_convert`-ordering hypothesis that SETMEM-SHAPE-1 already falsified
for `classify`; it is a different and later ordering problem, between `range_fold`
and whichever pass emits the boolean `Or`. Activating the transform therefore
needs a pipeline change, not a pattern change — and a pipeline change is precisely
what cannot be validated on a VM forbidden from running the benchmark-output gate.

Two further corrections to the estimate in RANGEFOLD-OR-1:

* The −4.8% figure assumed the fold would fire on `k_namechars`. It does not, so
  the realised benefit there is **0%**, not −4.8%.
* Because the transform fires on **zero** configurations, this corpus can say
  nothing at all about its eventual value — not even its sign. A successful
  pipeline change would have to be justified on the real-world `isalpha`-style
  classifiers (Expat, SQLite, gzip, glibc parsers) that the kernel suite
  under-represents, and measured on those, before shipping.

**Reverted**, and the revert is verified by codegen differential, not by `cmp`:
separate builds embed distinct build metadata, so binary identity is the wrong
test and initially reported a spurious difference. The right test is assembly
identity against the gated pre-fold build — **0 of 158 configurations changed** —
so no codegen in this delivery is affected. The unit tests went with the revert. Recorded rather than kept, because a correct transform
that does not fire is dead weight in a hot pass, and shipping it would have put an
unexercised algebraic rewrite in front of every classifier in the corpus for zero
measured gain.

If this is picked up: the transform is small and its tests are written — the work
is the ordering, and the first question is which pass emits the boolean `Or` and
whether `range_fold` can be re-run after it (the `-O2` tier is already a fixpoint
loop, so a placement change may be enough). Do not ship it without a measurement
on a real classifier corpus; this one cannot even establish the sign, because the
transform never fires on it.

**Why it was not implemented here.** It needs a `sub_defs` map threaded into
`try_fold_bool_op` (which currently receives only `cmp_defs` and `cast_defs`), and
it is a correctness-critical algebraic rewrite on the classifier path every parser
in the corpus exercises. The expected suite gain is ~0.3% with an unmeasured wall
effect, and METRIC-SPLIT-1 is the demonstration that an instruction-count win with
an unmeasured wall effect is not a win. The only validator for that pair is the
benchmark-output gate, which this VM is instructed not to run. Bounded, specified,
and deliberately not guessed at.

**(b) `namechars` — the counting form has the chain but not the recognized join.**
`if (pred) c++;` keeps a genuine block chain (verified in the IR: blocks 6/7/8/9
each end in `CondBranch -> JOIN`), so the shape is *almost* right. But the pass
does not fire at all: `CCC_DISABLE_PASSES=set_membership` yields
**byte-identical assembly** (35 instructions either way). The reason is the join:
the documented shape wants a Phi materializing the 0/1 predicate
(`Phi [(Const(1), T_1), ...]`), whereas the counting form's join Phi selects the
*accumulator* (`c` vs `c+1`). Static size is close (35 vs GCC's 30); the 1.653×
is dynamic — 41.8 vs 25.3 Ir per byte-iteration — because every test block in the
chain executes for every byte instead of one bitmap test.

**What closing this needs** (neither attempted: both are pass surgery whose only
validator is the benchmark-output gate, out of scope on this VM by instruction):

1. teach `set_membership` a **value-form member kind** — a `Select` chain whose
   `true_val` is a constant and whose conditions are pure tests of one value is
   the same disjunction the block chain expresses, and can feed the same cluster
   selection. The chain-collapse algebra is already proven in
   SELECT-CHAIN-FOLD below; what that experiment showed is that collapsing the
   chain *without* reaching a bitmap makes things **worse** (+2.63% Ir), so the
   two must land together or not at all.
2. accept a **counting-form join** — a Phi selecting `acc` vs `acc + 1` on the
   same condition set is a predicate Phi with the increment hoisted out, and is
   maskable by the identical cluster logic.

Reproduce all of the above in under a minute:

```bash
L=target/fastbuild/lccc; INC=$(gcc -print-file-name=include)
for cfg in "" ifconv cfg boolthread set_membership ifconv,cfg,boolthread,set_membership; do
  CCC_DISABLE_PASSES="$cfg" $L -O3 -S tests/bench/k_classify.c -o /tmp/z.s -I tests/bench -I$INC
  printf '%-46s %s\n' "${cfg:-<none>}" \
    "$(sed -n '/^bench_run:/,/\.size.*bench_run/p' /tmp/z.s | grep -vc '^\s*\(\.\|#\|$\|.*:\)')"
done   # -> 49 49 49 49 56 56 ; GCC 14.2 reference: 30
```

### METRIC-TRAP-1 · **NEW 2026-10-06** — `strlen_scan`'s 1.333x Ir ratio is not a defect; "fixing" it destroys a measured win

Any future triage of the `tests/bench` kernel suite will rank `strlen_scan` as the
worst absolute gap (29,371,411 Callgrind `Ir` vs GCC's 22,026,115, **+7.35M**) and
be wrong. The entire ratio is **one instruction**: LCCC's inner loop is
`cmpb $0,(%r8); je; addq $1,%r8; jmp` (4) against GCC's rotated
`addq $1,%rax; cmpb $0,(%rax); jne` (3), and 4/3 = 1.333 exactly.

LCCC keeps the **top-tested** form and hoists `leaq buf(%rip), %rdi` out of the
32-iteration outer loop; GCC re-materialises `leaq buf(%rip), %rax` inside it. On
wall clock LCCC is **faster** (ratio 0.984). The kernel header already records the
measurement: 49.3 ms top-tested vs 59.2 ms rotated on this host — LCCC ~15.9%
faster on a byte-at-a-time scan — and states that any transform which rotates it
"has to beat that number, not merely be defensible". See
`docs/SESSION_FOLLOWUP_S21_LOOP_ROTATION.md` §5.

**Rule this establishes for the whole suite: neither metric may be used alone.**
Cross-metric triage of all 11 kernels at `-O3` (`Ir` from Callgrind, wall from
`scripts/bench_kernels.py`, both `lccc/gcc` so >1 is worse):

| verdict | kernels |
|---|---|
| **GENUINE GAP** (both metrics worse) | `classify` 2.057 Ir / 1.208 wall; `namechars` 1.653 / 1.248; `varint` 1.277 / 1.577 |
| wall-only (same Ir, slower → layout/branch) | `strcmp_signed` 1.002 Ir / **1.261** wall |
| Ir-only (more Ir, wall equal or faster) | `strlen_scan` 1.333 / 0.984; `adler32_do8` 1.112 / 0.963 |
| parity or better | `adler32`, `hashmix`, `map64_sub` (0.630 Ir), `matchlen` (0.860 Ir), `memchr` |

`strcmp_signed` is the mirror-image trap: **1.002× Ir but 1.261× wall.** An
identical instruction count running 26% slower is block layout and branch
prediction, and no instruction-count work will move it.

The two genuine targets that remain, `classify` and `namechars`, are the same
defect — see SELECT-CHAIN-FOLD below for the measured proof that it is NOT
reachable from the IR.

### CONSTLOOP-FOLD-1 · **NEW 2026-10-06** — constant-trip-count loops are not evaluated at compile time

Four-vendor oracle, `-O2`, `tests/oracle/programs/int_alu.c`: LCCC **92**
instructions, GCC 16.2 **68**, Clang 23.1.0 **23**, ICC 2021.10 **81**, ICX
latest **28**. Clang and ICX fold the entire 64-iteration FNV-1a chain to a
single `movabsq` and strength-reduce the 256-iteration narrow-cast loop to an
8-instruction closed form; LCCC and GCC execute both.

LCCC's complete unroller caps trip at 16 with a 512-expanded-instruction budget
(half for FP bodies) — see `src/passes/loop_unroll.rs`. Raising the cap blindly
is code bloat, and this VM cannot run the slow/benchmark gates that would catch
it. The self-limiting design worth building: speculatively complete-unroll a
call-free, side-effect-free, constant-trip loop, run the constant folder, and
**revert unless the folded result is smaller than the loop it replaced**. That
makes the transform monotone by construction rather than tuned by a threshold.
Reproducer: `python3 scripts/godbolt.py compile cclang2310
tests/oracle/programs/int_alu.c --flags -O2`.

### GATE-SPLIT-1 · **DONE 2026-10-06** — one CI step carried four unrelated fixes

`tests/regression/check_audit_loop_contracts.sh` had accumulated the IVSR,
FP-extract-home and `va_arg_pack_len` invocations, so a failure named none of
them. Split into `check_ivsr_domains.sh`, `check_fp_extract_homes.sh` and
`check_va_arg_pack_len.sh`, leaving the original script to the bottom-tested
branch contracts it is named for. All four are wired into both
`scripts/ci_local.sh` and `.github/workflows/ci.yml`;
`scripts/check_ci_gate_parity.py` reports PASS (144 commands, 17 registered
invocation contracts). The FP gate now also covers **i686**, because
`result_type()` for scalar lane extracts is unconditional and drives
`compact_i686_values`, `is_wide_on_32bit` and the i686 prologue's compaction
veto — not just the x86-64 small-slot class.

### RESULT-TYPE-BLAST-1 · **AUDITED 2026-10-06** — every unconditional consumer of the scalar lane type

`dd01279` changed `result_type()` for `VecExtractLaneF32x4/x8` and
`VecExtractLaneF64x2/x4` from `None` to `Some(F32)`/`Some(F64)`. That is
**always-on**, not behind `CCC_FP_EXTRACT_HOMES`, and it has **17 call sites in
9 files**, not the 8 consumers / 8 files an external review counted. Two of the
files it named do not exist in this tree: its `src/passes/provenance.rs` was `src/ir/provenance.rs`,
and its `src/backend/i686/prologue.rs` was `src/backend/i686/codegen/prologue.rs`.
It also missed `src/backend/regalloc.rs`'s
own two sites and counted 4 of `slot_assignment.rs`'s 8. Every site is now
enumerated with its verdict in a comment at the changed arm in
`src/ir/instruction.rs`.

Net effects: an F32 lane result gets a width-partitioned **4-byte** slot instead
of the 8-byte fallback (correct — F32 is 4 bytes, and slot sharing is
partitioned by exact size class, so the stale-upper-half hazard cannot recur);
F64 lane results become `wide_values` on i686 and now veto the prologue's 32-bit
compaction they previously slipped through as `None`; `reassoc_latency`'s GPR
residency weight for a lane result becomes 0 (an XMM value occupies no GPR)
instead of the `None` default 1; `ir/provenance.rs`, `generation.rs` and
`loop_carried_forward.rs` are provably unchanged. Pinned by the new
`scalar_lane_slot_tests` (four simultaneously-live lanes, asserted
non-overlapping spans, byte-exact reload across a real adjacent 4-byte boundary,
with `fp_extract_homes` asserted **off**) and by
`tests/regression/fp_extract_slot_boundary.c` on x86-64, x86-64-v3 and i686 ×
`-O0..-O3` × default / `CCC_FP_EXTRACT_HOMES=1` / `CCC_NO_SMALL_SLOTS=1`.

### IVSR-WRAP-1 · **FIXED 2026-10-06** — truncating backedge was treated as a copy

`unsigned char x=254; ... a[x]; ++x` must visit 254,255,0,1. Baseline returned
509 instead of 510; disabling IVSR returned 510. `look_through_casts` now
looks through only same-type casts and Copies. A narrowing or signedness-
changing conversion cannot disappear from recurrence matching. Regression:
`tests/regression/ivsr_narrow_wrap.c` (8-/16-bit indices, zero trips and wraps).
This does **not** prove all unsigned 32-bit pointer recurrences sound; a reusable
no-wrap proof for every derived expression remains a hard prerequisite.

### VAPACK-LEN-1 · **FIXED 2026-10-06** — fortified open broke full gzip linkage

The pinned gzip 1.14 end-to-end build failed with undefined
`__lccc_va_arg_pack_len`. Reduced to `_FORTIFY_SOURCE=3; open(p,flags,mode)`.
Inliner plan IDs were in the callee namespace; cloned blocks were already
remapped. The old check added the offset a second time and never matched;
the would-be replacement also added it twice. Zero extra arguments were
incorrectly excluded. Match each original ID plus the offset to the already-
remapped destination, preserve that destination and materialize count zero too.
`va_arg_pack_len_inline.c` covers 0/1/4 extras, branches and distinct sites;
`fortify_open_va_pack.c` covers the actual libc wrapper. Both pass O1–O3;
baseline fails linking. Integrated into the audit contract gate.

### METRIC-SPLIT-1 · **NEW 2026-10-07** — `Ir` and wall clock disagreed by 74 points, and `Ir` was the one that was wrong

`CCC_FP_EXTRACT_HOMES=1` on `tests/benchmark/programs/nbody.c`, `-O2
-march=x86-64-v3`, same compiler binary, output byte-identical between arms
(`cmp` on stdout):

| metric | homes OFF | homes ON | ON/OFF |
|---|---|---|---|
| Callgrind `Ir` (`scripts/callgrind_ab.py`, correctness-gated) | 2,700,122,542 | **2,400,122,541** | **0.8889 (−11.1%)** |
| I1miss / D1miss / LLmiss / Bcm / Bim | 1378 / 1618 / 2739 / 2968 / 180 | 1380 / 1616 / 2740 / 2967 / 179 | unchanged |
| wall clock, median of 15 interleaved CPU-pinned reps | 224.51 ms | **366.42 ms** | **1.6321 (+63.2%)** |
| wall clock, min of 15 | 211.98 ms | 331.94 ms | 1.5659 (+56.6%) |
| vs GCC 14.2 (median) | 1.076× | **1.756×** | — |

**11.1% fewer executed instructions, 63% slower**, with every simulated
cache and branch-misprediction metric flat. The regression is far outside this
host's ~15% layout-noise floor (METRIC-TRAP-1) and reproduces on both median and
min, so it is not measurement artifact.

What this establishes, and why it is recorded as a method rule rather than a
one-off number:

* `Ir` counts **instructions retired**, not stalls. A change that removes stack
  round-trips by keeping FP lanes in XMM registers removes instructions and can
  simultaneously introduce a store-forwarding or dependency stall in the tightest
  loop of the program. `Ir` is structurally blind to that, and no amount of
  pinned cache geometry fixes the blindness — the geometry models misses, and the
  misses did not move.
* `scripts/bench_kernels.py`'s own docstring already said this ("a vectorised
  loop with more instructions usually wins, and a shorter loop that spills does
  not"). This is that sentence with a 74-point number attached.
* **The acceptance rule this session should have stated up front and now does:**
  a codegen change must pass *both* metrics to be promoted. `Ir` is the right
  tool for detecting and *sizing* a shape change and for A/B-ing when wall clock
  is inside the noise floor; wall clock is the only tool that can *accept* a
  promotion. Either one alone is unsound — and this session used each alone at
  least once before finding the pair that disagreed.
* Both of this session's own decisions survive the paired test, which is why they
  stand: IVSR-PTRADD-1 improves `Ir` (−9.59% on `varint`) **and** wall clock
  (66.5 → 56.8 ms, −14.6%); SELECT-CHAIN-FOLD worsened `Ir` (+2.63%) with no
  wall-clock win and was reverted. FP-LANE-1 is the third case and the one that
  only the pair catches.

**Consequence for FP-LANE-1: it stays opt-in, and the earlier evidence for that
decision was understated.** The first round declined to promote it on wall-clock
grounds that looked layout-dependent (+52.9% without `-lm`, −20.5% with it,
−10.6% on matched data addresses) and therefore arguable. This measurement is
not arguable: +63.2% on median and +56.6% on min, same binary, same flags,
identical output, no `-lm` ambiguity. The instruction-count improvement is real
and the promotion is still wrong. Do not re-propose enabling it on the strength
of an instruction count, a `.text` size, or a static matrix.

The mechanism is still **not proven** — the first round's page-split-store
hypothesis (fifth velocity store crossing a 4 KiB boundary only in the losing
candidate) is plausible and unconfirmed, and this VM has no PMU to confirm it.
What is proven is the size and reproducibility of the effect. Establishing the
mechanism needs a Raptor Lake host with working counters.

Reproduce in about a minute:

```bash
L=target/fastbuild/lccc; INC=$(gcc -print-file-name=include)
$L -O2 -march=x86-64-v3 -I$INC tests/benchmark/programs/nbody.c -o /tmp/nb-off
CCC_FP_EXTRACT_HOMES=1 $L -O2 -march=x86-64-v3 -I$INC tests/benchmark/programs/nbody.c -o /tmp/nb-on
cmp <(/tmp/nb-off) <(/tmp/nb-on)                       # identical output
python3 scripts/callgrind_ab.py /tmp/cc-homes-on /tmp/cc-homes-off \
    "-O2 -march=x86-64-v3" nbody                       # Ir 0.8889
for i in $(seq 15); do for e in /tmp/nb-off /tmp/nb-on; do
    /usr/bin/env taskset -c 0 $e >/dev/null; done; done  # interleave, then time
```

### FP-LANE-1 · **TYPING FIXED; ALLOCATION OPT-IN 2026-10-06** — SLP scalar extract results lacked FP classification

Four F32/F64 lane-extract intrinsics now expose their scalar `result_type`.
Both RA filters consume that shared type: non-GPR exclusion AND scalar-FP
candidacy. Updating the second filter alone is ineffective (measured).
With `CCC_FP_EXTRACT_HOMES=1`, existing x86 scalar-home emitters remove the nbody lane stack round trips:
43 -> 37 inner-pair instructions; no other instruction/stack count changed
in the 55-program same-source candidate A/B screen. Unit + scalar/SLP execution
tests pin the contract. **Do not enable by default:** the longer pinned test
without `-lm` regressed 52.9% on median, while `-lm` improved 20.5%; matched-data-
address control improved 10.6%. Link layout shifts `bodies` so the fifth velocity
store crosses a page. Both adverse/positive evidence are retained in the follow-up.
Default lane homes remain conservative; Raptor Lake is unmeasured.

### ORACLE-MEM-2 · **FIXED 2026-10-06** — address expressions were counted as memory traffic

The AT&T parser split inside SIB addressing and counted LEA/NOP as loads.
Indexed store, read/modify/write and stack tests now cover the distinction.
`stack_refs` is explicit; legacy JSON `spills` is a compatibility alias, NOT
an allocator diagnosis. Equal call counts now mean **unproven**, not comparable;
disparate call counts flag wins as well as losses. Tests:
`python3 scripts/test_codegen_oracle.py`.

### ORACLE-SEM-2 · **FIXED 2026-10-06** — valid ICC memcmp result called a miscompile

The four-compiler execution oracle found ICC -1 versus libc/LCCC -213.
C specifies the sign, not the magnitude. Normalize only this C semantic boundary
in `tests/oracle/programs/strings.c`, with explicit less/greater/equal/zero-length
checks; never normalize arbitrary output in the harness. The `all-vendors` preset
now includes ICC as well as ICX. Eight programs agree with all four oracles.

### METRIC-1 · **CLOSED 2026-09-30** — the oracle metric was ranking an inlining artifact

`codegen_oracle.py --rank` reported a 1899-instruction deficit, worst first
`zlib_ng_adler32::main` "204 behind icc=73". ICC's `main` is 73 instructions
because ICC left `zlib_ng_adler32_c` out of line as two copies the
single-function view never measured; whole translation unit, ICC is 43 % larger
than us over the six files that table put on top. 16 of the 77 "behind" rows
compared functions with different call counts and carried **781 of 1899 = 41 %**
of the headline, including both top rows.

Closed by adding a `calls` column, a per-row comparability marker, and a warning
naming the rows that cannot be ranked; `jmp`/`b`/`j` are deliberately not
counted as calls because a tail jump to an out-of-line copy is exactly the shape
that fakes a smaller function. Guarded by
`scripts/test_codegen_oracle.py` (11 cases, mutation-verified both ways) and
registered in both CI drivers. Full method and numbers:
[`ORACLE-METRIC-1`](engineering/evidence/ORACLE-METRIC-1/README.md).

Two corollaries that outlive the fix: a corpus **total** is the wrong statistic
here (four recursion benchmarks where GCC explodes carry the whole aggregate —
all-51 says −12.5 %, the 47 say +15.8 %), so quote the **median per-file ratio
and the larger/smaller counts**; and static instruction count remains a
screening metric only, never PMU evidence.

### PF-SN-1 · AVX2 strict-reciprocal pack, target confirmation pending
Current main `e5bc1911` emits legacy `movd`/`pinsrd` immediately before
`vcvtdq2pd`/`vdivpd`. The follow-up replaces these with VEX forms **only
when AVX2 is available**; a focused 15-round, output-checked Xeon-VM screen
found candidate/main 0.0361 (240 ms vs 6.60 s), and a 41-workload scan found
changed `.text` only in `spectral_norm`. This is a reproducible VM result,
**not** an i7-14700KF timing. Target P-core validation is pending; use the
runner and evidence linked above. Negative-trip, repeated-vector, scalar-tail,
changing-sign divisor, and XMM-disabled tests protect semantics.

### PF-LZ4-1 · Byte-copy semantics fixed; byte-compare still open
The default-on v2 copy matcher covers both relevant LZ4 loops. The old
unconditional `memmove` rewrite was **incorrect** for forward-overlap smear,
and naively disabling the pass caused a 10.8× scaled-workload slowdown on the
Xeon VM. A guarded memmove fast path now preserves the scalar overlap loop.
The original input executed **zero** match extensions in an instrumented
12,288-pass run; its near-parity time is *not* evidence for byte-compare parity.
A separately registered match-rich arm exercises 49,299 long matches in
24 passes. On current main, the 9-round scaled Xeon-VM LCCC/GCC 14 ratio is
~1.29; candidate/main `.text` is identical, so the byte-compare gap remains
**open**. Widening a compare without proving **both** operand ranges valid
can overread near an object boundary. No speculative widening or incorrect
copy rewrite shipped. Require sanitizer/page-edge and overlap tests, a range
proof, all four oracles, and an output-checked target i7 paired A/B.

### PF-MB-1 · Mandelbrot hot scalar FP loop (open)
Current `e5bc1911` 9-round Xeon-VM ratio to GCC 14 is ~1.30; candidate and
main `.text` match. GCC 16, Clang 23.1, ICC, and ICX oracles **also use scalar
FP instructions** in the compared main loop; static AVX-instruction counts
do not prove vectorization. Further cross-pixel widening needs legal SSA/exit
proof, and scalar loop/scheduling improvements require paired measurements.
Reproducer: `tests/benchmark/programs/mandelbrot.c`. Done = output-verifying
paired A/B with a proven general change on the i7-14700KF.

### PF-FB-1 · `linux_find_bit` loop structure (open)
Current `e5bc1911` scaled 9-round Xeon-VM ratio to GCC 14 is ~1.40;
candidate/main `.text` matches. `bsfq` is present; the open question is
branch/index structure, not the loop-copy idiom. A 21-round output-checked,
one-instruction assembly splice removing an apparently redundant post-`andn`
`test` gave median no-test/original 1.013 on this VM; **not** a win, so no
peephole was shipped. Done = classified, semantics-proven general CFG change
with output-checking and target-CPU paired evidence, or a justified bound.

### RA-GLA-04 · GLA Phase 2 — full-identity register remat (sha256 hot)
Phase 1 shipped (`CCC_RA_GLOBAL_LOCATION=1`); source-less remat is default-ON.
The checked Phase-2 *spill-gap* experiment was **not** shipped: paired
`sqlite_varint` and `expat_xml_scan` regressed 1.60% and 4.51% respectively
on an earlier VM despite a few better static counts. On current `e5bc1911`,
`sha256_transform` is ~1.18× GCC 14 on the 9-round Xeon-VM screen;
candidate/main `.text` is identical and its runtime ratio's interval spans
1.0. **No allocator speedup or regression is established.** Require
post-allocation slot-traffic feedback and a small, verified general change
before reconsidering full register-source-spanning remat.
**2026-09-28: the slot-traffic feedback prerequisite now exists** —
`scripts/stack_census.py --per-slot N` reports per-slot load/store/addr
traffic, and the census shows `sha256_transform` has **0 spill** and 12
`csave` references, so the Phase-2 remat experiment cannot pay for itself on
this function in its current shape. Re-open only for a function the census
shows as genuinely spill-dominated.
Oracle targets: `sha256_transform ≤ 1.5×`, epilogue ref-count −50 %.
Design refs: [`engineering/DECISIONS.md`](engineering/DECISIONS.md)
RA-GLA-01/02/03. Aligns with **R3** (RA span supply + phi/ORI lowering —
skip the 1.93× sha256 without tripling the web count; compare
GHASH/mulpack for the multi-source shape rule).

## Tier 1 — measured, largest first

### MINMAX-1 · **LANDED 2026-09-29** — integer min/max reductions on x86-64
`min`/`max` over `int[]` stayed scalar on x86-64 although the packed
machinery was finished: the reduction only becomes a `Select` after
`if_convert`, which runs AFTER the main vectorizer, and the late rerun that
catches it was gated on `matches!(target, Aarch64)` while its own comment
promised the x86-64 rerun. Fixed (shared early/late dispatch table, opened
to x86-64), plus `ReductionKind::Min` (`vpminsd`, new
`VecHorizontalMinI32x8`) and admission of the ordinary IV-indexed-GEP shape
at `iv_init == 0`.
Measured `-O3 -march=x86-64-v3`, steady-state density: `min_i32`/`max_i32`
**1.5 -> 0.1250 insn/byte** (12x; level with gcc 16.2 and icx, behind clang
23.1's 4x-unrolled 0.0547 only). Pinned oracles: clang 23.1 0.0547, gcc 16.2
0.1250, icx 0.1250, icc 0.5625.
Second defect found and fixed on the way (MINMAX-1b): a dynamic loop bound
was divided by the vector width with a `UDiv` inserted into the loop HEADER,
so the min/max steady state carried a real 64-bit `divq` — 6 of its 10
instructions. Now emitted once in the preheader as `LShr` (power-of-two
width): 10 -> 4 insns per trip. Both the AVX2 and the SSE2 transform, corpus
A/B shows no regression. See
[`engineering/evidence/MINMAX-1/README.md`](engineering/evidence/MINMAX-1/README.md).

* **MINMAX-5 · extract the duplicated dynamic-limit hoist (LOW-3).** The
  ~110-line hoist is cloned verbatim between `transform_reduction_avx2` and
  `transform_reduction_sse2`. Mechanical, and the right call eventually, but
  it must be its OWN commit with its own re-measurement: it touches the single
  largest behavioural change in MINMAX-1, and spending a correctness session's
  validation budget on a maintainability wart with no defect behind it is the
  wrong order. Deferred deliberately, with the reasoning recorded, rather than
  skipped silently.

**Gate/observability follow-ups from the PR #681 audit** (detail and
measurements in
[`engineering/evidence/PR681-AUDIT-RESPONSE/README.md`](engineering/evidence/PR681-AUDIT-RESPONSE/README.md)):

* **OBS-1 · `src/lib.rs` carries `#![allow(unused_variables)]` crate-wide.**
  This, not any rustc limitation, is why a deleted `!*volatile` guard in
  `licm.rs` reached a green Clippy job: rustc *does* lint refutable-pattern
  bindings (verified: `rustc -D warnings` errors on the exact shape), and this
  crate has the lint switched off. Until it is addressed, a dropped
  observable-access guard in any pass is invisible to the build.
  `scripts/check_volatile_destructuring.py` covers the one field where silence
  is a miscompile. The right follow-up is a **counted ratchet** over
  `unused_variables` sites — measure today, never raise it — in the style of
  `check_env_test_hygiene.sh`, migrating the safety-relevant sites first.
* **OBS-2 · widen the ratchet to the other observable-access fields.**
  `Instruction` carries a second flag, `semantic_volatile`, which has the same
  silent-drop exposure and currently **no instrument at all**. `AtomicLoad` /
  `AtomicRmw` / `AtomicStore` are excluded on the *argument* that `_Atomic`
  accesses are already unremovable; that is an argument, not a measurement.
* **OBS-3 · `check_volatile_spin_loop.sh` asserts on the INNERMOST loop.**
  A volatile access hoisted from an inner loop into an outer one would pass.
  Not reachable today (no pass sinks outward), but the helper should support
  "inside any enclosing loop" and the gate should say which it means.
* **OBS-4 · `loop_preheader` is 528 lines of CFG surgery, default-on at -O2,
  and fires ~0 times at default settings.** Measured: byte-identical output
  with and without it on every guard-at-top shape, including the SQLite
  `if (p == 0) return 0;` case its own docstring cites; it fires under
  `CCC_LOOP_ROTATE=1` (18x, per the W3 journal) and on a plain `-O2` `do`-while
  shape. Either find the shape family it is actually good for and measure the
  insertion rate over the golden workloads, or gate it off by default. Today
  it is paid for on every `-O2` compile and its value is invisible.
  `tests/regression/check_loop_preheader.sh` now pins both directions.
* **OBS-5 · an access COUNT cannot see a HOIST.**
  `check_volatile_pointer_subscript.sh` passed green on a compiler that hoists
  a volatile MMIO load out of its spin loop, because hoisting preserves the
  count. Measured, not assumed. Any future gate asserting on emitted code
  should ask about POSITION; `tests/regression/lib_loop_bounds.sh` is the
  shared primitive.

* **OBS-6 · `reloc_pc32_out_of_range_diagnosed_on_script_path` cannot pass on
  a host whose non-reference linkers cannot parse the fixture's script.**
  Measured on this box: 300 pass / 1 fail, with
  `only 1 of 2 oracles could express an opinion, need 2` -- bfd refuses, and
  mold 2.37 answers `unknown linker script token` for the fixture's
  `ENTRY(probe)`, so it is `inapplicable` and the applicability floor of 2 is
  never met. This is the fail-closed rule working **as designed** (see the
  "two inapplicable out of three is NOT a cross-check" known-answer case), and
  it is not a regression: on a host with bfd + lld + wild it passes. The
  defect is in the *fixture*, not the rule: a fixture whose linker script
  needs a token some oracles lack makes the test host-dependent in a way that
  reads as "lccc is unconformant". Fix by giving the fixture a script every
  configured oracle can parse, or by reporting an incapable oracle as an
  explicit `SKIP` for the case with the reason attached, rather than folding
  it into a FAIL that reads like a conformance verdict. Do NOT lower the
  floor.

**Named follow-ups, in value order** (all measured, all refused today so
they stay CORRECT rather than fast):

* **MINMAX-4 · the rest of the late rerun (measured, deliberately off).**
  The unrestricted rerun also fires the Adler-32 epic and the guarded-sum
  transforms on `zlib_ng_adler32`. Measured on the benchmark program itself
  (`-O3 -march=x86-64-v3`, `valgrind --tool=callgrind`, whole program):
  **396,349,832 -> 400,499,785 retired instructions (+1.05 %)**, static
  instructions 335 -> 370 (+10.4 %), stack references 32 -> 44 (+37.5 %) —
  even though the steady-state loop it produces is denser (14 -> 12 insns per
  32 bytes). The prologue, the extra accumulator traffic and the spills cost
  more than the packed body saves *on this workload*. The rerun is therefore
  scoped to min/max (`LateMinMaxOnlyScope`), which keeps the 12x min/max win
  and leaves every other kernel byte-identical (adler32 is now 332 static
  instructions and 0.3750 insn/byte — better than the 335 / 0.4375 baseline,
  from the preheader-division fix alone). Re-open with a PROFITABILITY guard
  (prologue + epilogue cost vs trip count), not by deleting the transform.

* **MINMAX-2 · multi-accumulator min/max.** `for (...) { if (a[i]<mn) ...;
  if (a[i]>mx) ...; }`, and the same loop with a sum. Two of the corpus's
  worst kernels are this shape (`moving_stats`: lccc 149 insns/trip vs icc
  0.625 insn/byte; `fir_filter`, `conv_u8_3x3` are nearby). The pattern
  models ONE accumulator, so it refuses; the follow-up is to populate
  `SecondaryAccumulator` with a `kind` and wire the min/max epilogue per
  accumulator. The refusal exists because NOT doing this produced
  `sum == 0` for every n below the vector width.
* **MINMAX-3 · 16-bit and unsigned lanes.** `vpminsw`/`vpmaxsw` are SSE2
  baseline (`vpmin*`/`vpmax*` for 8-bit need SSE4.1, `vpminud`/`vpmaxud`
  too). `moving_stats` is `short` data: lccc 3.0 insn/byte vs gcc 0.125.
* **LOOP-PREHEADER-2 · fix `loop_rotate`'s condition lowering, then enable
  rotation (measured blocker).** `loop_rotate` makes the loop body the header,
  which is what turns LICM's derived-pointer load hoisting from dead to live —
  with rotation plus the dedicated preheaders from LOOP-PREHEADER-1 the
  `fir_filter` FIR hot loop goes **56 -> 38 instructions (-32 %)**. Rotation is
  OFF by default because it is **+209 static instructions (+2.6 %)** across the
  51 benchmark programs at `-O3 -march=x86-64-v3`, and the entire cost is one
  lowering defect: the rotated latch materialises the loop condition as an `i1`
  value instead of keeping a comparison, so the backend emits `setl %bl;
  movzbl %bl,%ebx; testb %bl,%bl; jne` where a single `jl` belongs. Three
  instructions per iteration, in every rotated loop. Fix the condition
  lowering, re-measure, then re-evaluate enabling rotation at `-O2+` — which
  would simultaneously make LOOP-PREHEADER-1 pay corpus-wide. Evidence:
  [`engineering/evidence/LOOP-PREHEADER-1/`](engineering/evidence/LOOP-PREHEADER-1/README.md).
  **Blocking defect, already known:** `loop_rotate` leaves invalid SSA on 2 of
  the 51 benchmark programs (`fir_filter.c`, `moving_stats.c`): under
  `CCC_VALIDATE_SSA=1` both abort with `SSA PHI-ARITY VIOLATION after phase
  'after iter=0 loop_rotate'` — phi incoming-label sets that do not match the
  rewritten CFG. Verified NOT caused by LOOP-PREHEADER-1 (reproduces with
  `CCC_DISABLE_PASSES=loop_preheader`, and fires before that pass runs). Fix the phi
  rewriting before touching the condition lowering.

* **LOOP-PREHEADER-3 · ~~the pass has no dedicated gate~~ CLOSED.**
  Landed: `tests/regression/check_loop_preheader.sh` +
  `tests/regression/loop_preheader_shapes.c`, wired into `ci_local.sh` and
  `ci.yml`, and the shapes below are asserted on **emitted assembly**
  (a preheader that is structurally valid but hoists nothing is invisible to
  every runtime comparison and every Rust-side unit test).
    1. *Effect.* `guarded_sum` (the SQLite NULL-guard shape) loads `p->nUsed`
       once, outside the loop.
    2. *Soundness.* That load sits **after** the `p == 0` early return; hoisted
       into the guard block instead it would dereference NULL.
    3. *Negative control.* Under `CCC_DISABLE_PASSES=loop_preheader` the same
       loop reloads `p->nUsed` every iteration. Without this delta a pass that
       never fired would also show one load outside the loop and pass
       vacuously -- verified by disabling `enabled()`: three contracts fail.
    4. *Refusal.* Four shapes stay byte-identical between the two arms:
       `invariant_ptr` (nothing to unlock), `switch_entered` and
       `computed_goto` (the miscompile shapes -- counting only Branch edges
       would claim one of two entering blocks as *the* preheader, leaving the
       hoisted bound's def not dominating its use on the other path), and
       `already_dedicated`.
    5. *Idempotence.* With `CCC_DEBUG_LOOP_PREHEADER=1` the fixpoint inserts
       once and then reports "already has a dedicated preheader".
  Note on the spec's original first shape,
  `int f(const int *c,int n){int t=0;for(i<n;i++)t+=c[0];}`: it is **not** a
  preheader shape in this pipeline. Measured, it is byte-identical with the
  pass on and off -- LICM already hoists the unguarded load, so the pass has
  nothing left to unlock. The gate therefore asserts refusal for it rather
  than effect, which is the true behaviour.

* **RED-WIDEN-1 · widening reductions.** `int s; for (i) s += a[i];` with
  `a` of `short`/`unsigned char`: lccc stays scalar (3.0 / 5.0 insn/byte)
  where gcc does 0.28 / 0.53. Needs the detector to accept
  `accumulator_type != element_type` and a widen-then-fold body
  (`vpmovsxwd`/`vpmovzxbd` + `vpaddd`).


### ZERO-REM-1 · **LANDED 2026-09-28** — dead vectorizer remainder loops
The map vectorizer emitted its scalar mirror (the `N % W` tail loop, which
doubles as the runtime dependence guard's fallback) even when a constant trip
count made it unreachable: 17 instructions of guard, resume-index arithmetic,
materialised stream pointers and a dead scalar body **per vectorized loop**.
`transform_map_vector` now omits it when `N % packed_width == 0`,
`pattern.guarded_streams` is empty and the escaping counter can be re-pointed
at the trip count (materialised in the preheader with the mirror's own
`Cast`-to-`iv_ty` form, so it works in bare-`Value` use positions too).
Measured at `-O2` on upstream `main` `93f2a43b` (A/B against the kill
switch): `sha256_transform` **151 → 135** insns / 20 → 19 rrmov / 1 → 0
stkref, a 16-element `unsigned` copy 61 → 28, `shape_u32_exact` 39 → 18.
Pinned-oracle targets for the same function: GCC 16.2 = 142, Clang 23.1 =
129. Kill switch `CCC_NO_MAP_ZERO_REM=1`.
Evidence: [`engineering/evidence/ZERO-REM-1/README.md`](engineering/evidence/ZERO-REM-1/README.md);
gates `tests/regression/check_vec_dead_remainder.sh` +
`tests/regression/check_vec_remainder_shapes.py` +
`tests/regression/vec_dead_remainder.c` and the assembly-shape fixture
`tests/regression/vec_shapes/vec_dead_remainder_shapes.c` (a subdirectory, so
`run_regression.py`'s program corpus does not try to link it).
Not done: the reduction and stencil vectorizers build their own tails; the same
constant-trip-count reasoning applies there.

### SPILL-01 · **TOOL LANDED 2026-09-28** — causal stack-reference census
`ra_quality_census` counted stack references; nothing said *why* they exist.
`RegAllocResult` now publishes the allocator's own `eligible` set and
`src/backend/stack_layout/slot_census.rs` turns it into a per-slot cause
(`alloca`/`address`/`wide`/`spill`/`nongpr`/`temp`, opt-in `CCC_SLOT_CENSUS=1`);
`scripts/stack_census.py` joins it with the POST-PEEPHOLE assembly and
attributes every emitted stack reference (plus `csave`/`argout`/`incoming`).
Corpus gate met: **98.74 %** coverage (1251/1267) vs the 95 % criterion.
Finding that re-orders the queue: **44.4 % of stack references are
callee-save save/restore**, 23.8 % structural (alloca/address/wide/nongpr) and
only **10.7 % are spills** — and `sha256_transform`, the RA-PRESSURE-3 target,
has **zero** spill references.
Evidence: [`engineering/evidence/SPILL-01/README.md`](engineering/evidence/SPILL-01/README.md).

### DO-WHILE-BRANCH-1 · **CLOSED AS STALE 2026-10-06**

On upstream `08f4e1a`, the published `int f(int n){int i=0; do{i++;}while(i<n);
return i;}` already emits `addl; cmpl; jl`, without materializing a boolean.
Five variants (`<`, `<=`, `!=`, step 2, explicit break) are now pinned by
`tests/regression/check_audit_loop_contracts.sh`, mirrored in local/hosted CI.
No new peephole or claimed speedup is warranted for already-correct code.

### LOOP-PREHEADER-3 · **PROPOSAL REJECTED 2026-10-06; optimization still open**

The former instruction to hoist a load because its block is *dominated by the
header* was unsound: that is true for every block of a natural loop, including
conditional/zero-trip/early-exit paths. Example:
`for(int i=0;i<n;i++) if(take) sum+=*p;` with `n=17,take=0,p=NULL` is defined.
Hoisting `*p` to the preheader introduces a fault. An early break before the
load is another counterexample. Both are in `audit_loop_contracts.c`.

A future improvement needs must-execute proof on the **first entered iteration**
(or independently safe speculation), including exits and potentially nonreturning
operations before the load, plus alias, volatile, atomic and lifetime checks.
Keep the current conservative LICM gate. Do NOT change the existing preheader
assembly contract from one load to zero on header-dominance alone.

### LOOP-PREHEADER-4 · **CLOSED 2026-09-29** — the pass's module doc motivated itself with a loop it does not fire on
*(numbered -4, not -2: LOOP-PREHEADER-2 is already taken by the `loop_rotate`
default-enable item in `engineering/journal/2026-09-W3.md`.)*
`src/passes/loop_preheader.rs` opened with `for (i = 0; i < n; i++) t += c[0];`
as the shape a dedicated preheader unlocks. Measured: the pass prints nothing
for that function, because its own profitability gate mirrors LICM's
must-execute rule and the load sits in the body, not the header — that spelling
is the `while_sum` shape, which the pass *refuses* on purpose.

Closed: the docstring now leads with the do-while (guard outside the loop, so
the load is in the header) and keeps the counted `for` as an explicit
counter-example, and both shapes are pinned by
`tests/regression/check_loop_preheader.sh` contracts 1 and 2, so the comment
cannot drift away from the behaviour again.

### WR-COND-RETEST · **NEW 2026-09-29** — second conditional store re-tests a live condition
`void wr_cond(volatile u32 *p, volatile u32 *q, int c){ *(c?p:q)=1; *(c?p:q)=2; }`
emits `testl %edx, %edx` twice — once for `cmovneq %rdi, %r8` and again for
`cmovneq %rsi, %rdi` — instead of reusing the first select's result. One
redundant `test` per pair of same-condition selects. Small; worth folding into whichever pass
the now-closed DO-WHILE-BRANCH-1 used to describe. That closure does not
establish this separate vectorizer layout proposal.

### RA-CSAVE-1 · **NEW 2026-09-28** — callee-save save/restore traffic
Historical SPILL-01: 562 of 1267 stack references (44.4 %) are
callee-saved registers being saved on entry and restored on exit, more than
spills and allocas combined. Every register taken from the callee-saved pool
costs two stack references per call. Before touching eviction policy: measure
how many of those registers are *used* after allocation (`[RA-STATS]
callee-homes=`) versus saved defensively, and whether the corpus's hot
functions would rather spill a caller-saved value. Done = a census-driven,
output-checked change that reduces `csave` references on the corpus without an
instruction-count regression, or a documented bound.

2026-10-06 census (`--corpus`, programs + kernel corpus, `-O2 -march=x86-64-v3`):
Final default: **589 / 1298 = 45.4%** callee-save references;
**128 / 1298 = 9.9%** classified spills, 14 unknown and 262 `temp` references.
(The opt-in FP candidate had 1294 total, 12 unknown and 260 `temp`.) This is a static census, not
execution frequency: once-per-call saves cannot be priced as once-per-iteration
loads. `temp` classification is not proof those references are unavoidable.
No default allocation-policy change is justified by percentages alone.

### RA-PRESSURE-3 · **RE-SCOPED 2026-09-28** — sha256 round loop
The historical framing ("land a generic phi-copy cycle resolver") is
obsolete: a general resolver already ships (Kahn decomposition in
`plan_edge_copies`, exhaustive semantic-equivalence test over EVERY
parallel-copy graph for n=2..5, default-on policy, and
`check_phi_acyclic_order.sh` pinning rotation < legacy statically).
Re-measured at `-O2` on `6f8ace9c`: `sha256_transform` is **148 insns /
14 rrmov / 10 stkref** (GCC 142/19/8, Clang 129/24/26) — not the
historical 218 insns / 64 spills, and lccc now has FEWER stack
references than Clang. The residual gap is instruction selection and
*physical* rotation, not copy resolution.
New sub-problems: (1) express the rotation as a register permutation
(rotate-by-immediate / `rorx`-class) rather than copies — check
`ui: alu/phi` admission, see `span_recurrence`; (2) 2×/4× unroll so the
rotation runs once per body; (3) close the 19-insn gap to Clang.
Negative space unchanged: the valve count-coupled supply is falsified
(W2 09-11); RA-06 intra-block splitting-measurement is recorded dead
(W1 09-05).

### RA-PRESSURE-4 · Aggregate/struct register pressure — `struct_copy` (1.15–1.45×)
156 insns vs clang 76 (**77 spills**). `CCC_NO_AGGREGATE_SPLIT` escape.
Done = ≤1.05× on Raptor-Lake report.

### PF-CHACHA-1 · chacha20 ARX schedule — remaining ICX gap (118 insns vs icx 74 @-O2, icx 63 @v3)
Refreshed 09-27 (oracle, static counts): PR #638's ARX work already took
the `-O2` body from the old 592 and the v3 body from 222 down to **118**
(oracle: lccc 118 < clang 176 < gcc16.2 180; icc 1104). The remaining gap
is ICX only (74 @`-O2`, 63 @v3): the v3 ARX-lane schedule (superword diag +
horizontal ARX; W2 09-08) plus ICX's tighter quarter-round folding at base.
Done = v3 closer to ICX's 63, `-O2` static ≤1.15× of ICX's 74 (runtime
parity on the author's box no longer counts as done — keep static and
runtime numbers separate).

### PF-SCHEDULER-1 · sha256 message-schedule vectorization
GCC vectorizes the sliding-window expansion even at -O2 (`m[i-2..i-16]`
SIGTA → SSE). Blocker: strided/sliding recurrence — unaligned vector
loads at constrained offsets + sliding-window SLP (`vec_arx` /
`arx_vectorize` first-light: `s10/next/solve/split` → v-slp flags).
Done = schedule vectorized at -O2, kernel ≤1.15×.

### PF-TLS-1 · TLS segment access — **DIRECT LOCAL-EXEC FORM LANDED (2026-09-28)**
Legal Local-Exec accesses now lower to ONE instruction,
`%fs:symbol@TPOFF(+N)`, instead of base materialization + access.
Measured at `-O2` on `6f8ace9c` → this tree (`ra_quality_census`):
`set_all` 54 → **36** insns, `sum_all` 41 → **28**, a 3-store `set`
12 → **6** (byte-identical to GCC), benchmark `tls_pass` 34 → **30**
(GCC 37, Clang 31), whole focused TU 123 → 91.
Legality rule (measured against GCC, not guessed): Local-Exec is a
link-time constant, so it is used for every executable — PIE included —
but **never** for `-shared`, and only for a symbol this module owns
(extern TLS keeps Initial-Exec, as GCC does). The backend therefore
carries a new `shared_lib` flag: `pic_mode` alone cannot tell `-fPIC`
from `-shared`, and the two have opposite legality here.
Two prerequisites were repaired first, both reproducible on the review
baseline: (1) the assembler dropped the relocation modifier for
`sym@MOD+N` (`R_X86_64_32S` against a symbol literally named
`sym@TPOFF` — a silent wrong address; lccc's relocations now match GNU
as 2.47 exactly); (2) `lccc -fPIC -shared` could not link ANY
`static __thread` ("R_X86_64_TPOFF32 ... can not be used when making a
shared object") because shared output used Local-Exec — it now uses
Initial-Exec, verified by a `dlopen` round-trip.
Runtime: **not established** — the scaled `tls_seg_access` screen
(`-DPASSES=20000000U`, 7 interleaved rounds, output identical to GCC)
has a per-binary spread of 0.67–0.94 s, so the before/after medians are
inside the noise. UNVERIFIED ON TARGET.
Evidence: [`engineering/evidence/PF-TLS-1/README.md`](engineering/evidence/PF-TLS-1/README.md);
gates `tests/regression/check_tls_model_selection.sh` +
`tests/regression/tls_local_exec_direct.c`.
Remaining: two of three `&tls_slots` computations still not CSE'd; a
variable-index TLS read still pays the base (unavoidable); `%gs:` and
i686 coverage not attempted.

### PF-CLS-1 · Byte-classifier chains (expat_xml_scan 1.31–1.37×)
**Re-measured 2026-09-28 at `-O2`: `expat_utf8_name_length` is 70 insns
vs gcc 79** — lccc is now statically AHEAD, so the 1.31–1.37× runtime
gap is pure branch/layout behaviour and instruction-count work here is
wasted effort. Use `scripts/callgrind_ab.py` (deterministic `Ir`,
I1/LL misses, branch mispredictions) or a target-CPU PMU run; this
host's wall-clock noise floor is ~15 %.
Historically: 79 vs gcc 78 insns — branch-prediction/scheduling-bound,
not insn count. Structural block: `a||b||c` last-member critical edge + shared
increment block starve if-conversion (two attempts reverted W1 09-01g).
**Split the critical edge on the last member first**; the counting
spelling `pred && n++` needs the same `Select`s as the boolean one.
Sub-items: (1) length-table load DCE; (2) char-class increment-starved
`vectorize` falling back to `if_convert` (1.17×): sequence-first
vectorization.

## Tier 2 — MachInst / isel

### MI-CLOBBER-1 · Clobber modelling, then lower `Call` (7.8 % of insns)
Correct-by-rejection today (the flush boundary *is* the clobber model).
`CallTyped` carries declared clobbers (W1 09-02e); extend per-instruction
clobbers before lowering `Call` — adding `Call` uses before that is a
miscompile.

### MI-XMM-1 · Vector/FP register class (1.3 %)
119 rejected `Store(float)`. MachInst models only the 16 GP families.

### MI-PARAM-1 · Remaining `ParamRef` (1.1 %)
No-code subset landed (963 → 96). Remainder is emissive: alloca-homed
params and stack-passed args; the fallback reads the parameter's
**incoming** register even when a caller-saved pre-store of a different
parameter aliased the name.

### MI-ROTATE-1 · Sub-word rotates
Native `RotateLeft/Right` landed (W2 09-07). Open: sub-word rotates
whose complements meet only at the narrow width (truncation-aware
pattern / a consuming `Cast(i32→u16)` rewrite). Count-staging polish is
cosmetic; do not chase.

### MI-ENCODE-1 · Encoding-level differential
Suite at 7 layers/36 tests incl. execution vs GAS 2.47; byte-level arm
via `scripts/insndiff.py` / `encoding_diff.py` + GAS 2.47 (INF-GAS-1).

## Tier 3 — verifier / infra

### FE-RELRO-1 · CLOSED — multi-level pointer const leaked into `.data.rel.ro` placement (2026-09-27)
The parser OR-ed `decl_flag::POINTER_CONST` across every star of a
declarator, so `short * const * gp` (outer object **writable**) was
classified read-only and `classify_global` routed it to `.data.rel.ro`;
RELRO page-protection then made the first run-time store SIGSEGV
(Csmith seed 20260928, differential vs GCC at -O0; wild store to the
globals page, gdb-confirmed read-only mapping at run time). Fix:
each star iteration now *clears* the flag, so only the identifier-
adjacent (outermost) level's const survives — which also fixes
cross-declarator leakage (`int * const a, *b;` made `b` read-only).
Regression: `tests/regression/pointer_const_multi_level_relro.c`
(SIGSEGVs on the pre-fix build; byte-compares vs GCC).
Do-not-reopen: the single-star `T *const p` case is
`pointer_const_data_rel_ro.c` (kernel zstd contract); the volatile
sibling (`short * volatile * gp` marks the declaration volatile —
conservative, safe, missed-opt only) is deliberately NOT changed in
the same commit; measure before touching it.

### VER-PHIARITY-1 · CLOSED — φ-arity contract is two-sided: reachable ⊆ named ⊆ static (2026-09-27, refined after corpus run)
phi elimination materializes one edge copy per (reachable pred,
incoming) pair and never visits unreachable blocks, so (a) a statically
present but dead predecessor legally has no φ incoming — the frontend
lowering leaves such dead edges routinely — and (b) an entry for a dead
predecessor is inert (the edge copy sits on a path that never runs;
switch/loop lowering produces these, e.g. switch_dispatch's 16-pred
join). The first cut of the reachable contract enforced EQUALITY with
the reachable set and false-positived on 15 corpus files, all of the
`extra [dead-pred]` class. Final contract, identical in BOTH checkers:
every REACHABLE predecessor must be named (missing = live-edge defect,
the miscompile class), and only real CFG predecessors may be named
(stray = carelessly retargeted edge); an entry for a dead predecessor
sits legally in the gap. GLA check 7 (src/backend/location_alloc/
verifier.rs) and the `CCC_VALIDATE_SSA` PHI-ARITY check (src/passes/
mod.rs) implement it with unit tests for both sides. Do not weaken the
reachable-preds coverage side: that is the real defect detector
(Csmith 20260981's compile abort). Also added a
`lowering:entry` validator checkpoint (at -O0 the optimizer loop is
empty, so without it a frontend IR defect was only visible at
`backend:pre-eliminate_phis`) and dump-before-validate ordering there.

### PHI-SINK-1 · CLOSED (superseding entry) — composed with #645; profitability = live phi home (2026-09-27, S66)
The unguarded sink regressed `spectral_norm` (289 > 287 budget): sinking the
accumulator's add across the body/latch boundary separated it from its mul
and lccc lost `vfmadd231sd`. Final rule: sink ONLY when the phi home is
live across the window — an instruction between definition and relay (same
block) or in the strictly-dominated region reads or writes the home (the
rotation's `f <- e` copy). Dead-home accumulator shapes are refused; the
allocator coalesces them anyway and the fold would only distance the
computation from its operands. Both behaviors unit-pinned
(`computed_incoming_folds_to_its_copy_slot`, `dead_home_computed_incoming_is_not_sunk`,
`fold_sinks_definition_without_touching_readers` — rewritten after it was
found to pass vacuously under refusal). Composed with #645 (Agent B's
rotation lags + residency guard + lea addend fold — audit:
`engineering/AUDIT-PR645-reassoc-latency.md`): rot 55/3 → 53/3, sha256
144→142, corpus −5 insns/−5 rrmov with 183/186 functions identical, full
lib 3621/0. Gate ratchet: escape-off census pinned to the exact composed
shape (k_i ≤ 53 / k_s ≤ 3); the beat-gcc and beat-legacy-by-10 contract
unchanged. Do-not-reopen: the predicate gates profitability only —
correctness always stays with the RA's disjoint-interval coalescing rule.

### CC-O0CALL-1 · **CLOSED 2026-09-28** — secondary-cache (`%rcx`) clobber by GP argument 4
The previously proposed cause (`-O0` accumulator-address loads trusting a
non-SSA cache hit) is **falsified**: applying that guard leaves Csmith
20260945 SIGSEGVing, and its regression test passes on the broken
baseline, so it does not discriminate.
Actual cause: `%rcx` is both the secondary value cache and SysV's fourth
GP argument register. Argument 4 staged a scalar into `%rcx` without
invalidating the cached pointer identity, so the next (aggregate)
argument rematerialized the "pointer" from `%rcx` and dereferenced the
scalar that had just been written — `movq %rcx,%rax; movq (%rax),%r8`
with a NULL base. Independent of the peephole and of GLA (also fails
with `CCC_NO_PEEPHOLE=1` and `CCC_RA_GLOBAL_LOCATION=0`).
Fix: invalidate the secondary cache after a GP argument writes ABI slot
3 and before the next argument is staged, for every classification that
can write it (scalar, i128/i64 register pairs, by-value aggregates, both
mixed struct classes); the next-iteration placement also covers the
early-`continue` staging paths.
Evidence: reduced reproducer
`tests/regression/call_secondary_cache_clobber.c` (8 ABI shapes), gate
`tests/regression/check_call_secondary_cache.sh` (20 output-checked
arms: `-O0..-O3/-Os` × {default, no-peephole, no-GLA, both}), the full
Csmith seed 20260945 now prints `checksum = 39DEBCF9` / exit 0 like GCC,
and GCC `-fsanitize=address,undefined` is clean on the reduced case.
Historical reproducer kept:
`artifacts/repros/miscompile_csmith_20260945_-O0.c`.

### INF-HARNESS-1 · Subtract startup / scale short benchmarks
lz4's 1.94× hid a 10× work gap (~2 ms fixed cost in a 7 ms measure).
Report rule: no benchmark with fixed cost > 10 % of measured time
reported bare; short kernels scaled or decomposed.

### INF-BENCHGATE-1 · Benchmark output gate unconditional in CI
`scripts/check_benchmark_outputs.sh` runs every `tests/benchmark/
programs/*.c` at all four levels (~4 min, no timing) — a miscompile
once passed all regression tests because those kernels were only ever
built by the timing harness.

### INF-FRESHCLONE-1 · CI builds from a fresh clone
Historical failure mode: files missing from a commit while the author's
tree was fine.

### INF-GAS-1 · GAS 2.47 re-provision per session
`scripts/ensure_gas_247.sh` — caches/snapshots do not persist it;
`/usr/bin/as` 2.44 is **not** an oracle.

### INF-LINK-1 · Linker oracle pins
lld 23.1 (`release/23.x`), mold 2.42.1, bfd 2.47 — verify versions
before trusting a prebuilt (W3: a stale `wild` 0.7.0 produced two false
lccc failures). Setup: `tools/linker/setup_oracles.sh`,
`tests/linker/setup_oracles.sh`.
2026-10-06: restored bfd/mold binaries now must pass actual `--version` checks,
not merely exist in version-named directories. Reproducer/tests:
`tests/regression/check_linker_oracle_versions.sh`. Mold's upstream v2.42.1
CMake option was verified as `-DMOLD_TARGETS='X86_64;I386'`; it is already set.

---

### PERF-1 · Byte-width and masked sum reductions are not vectorized

Isolated with a four-case experiment (`-O3 -march=x86-64-v3`, vector
instructions emitted per loop):

| shape | LCCC | GCC 16.2 |
|---|---:|---:|
| `c += p[i]`, `int *` | 18 | 25 |
| `c += p[i]`, `signed char *` | **2** | 43 |
| `if (p[i]) c++`, `int *` | **1** | 22 |
| `if (p[i]) c++`, `signed char *` | **1** | 46 |

So plain I32 sum reductions vectorize and nothing else does — neither byte
width nor masked reduction, at any width.

Consequence: `sieve` runs 1.121x slower than GCC, and LCCC emits **zero**
vector instructions for the whole file. GCC vectorizes the prime-counting
loop as `vpcmpeqb` -> `vpmovsxbw` -> `vpsubd`.

Blocker: `src/ir/intrinsics.rs` has only I32->I64 widening
(`VecWidenAddI32x4ToI64x2`, `VecLoadWidenI32ToI64x2`). There is no I8->I32
widening op in the IR or the x86 backend, so this needs a new intrinsic, its
`vpmovsxbwd`+`vpaddd` lowering, vectorizer pattern matching for I8 element
types, and remainder handling — three layers, and a miscompile if rushed.
Start from the four-case table, not from the symptom.

### PERF-2 · No loop interchange pass (matmul 1.399x, worst kernel)

`grep -rn "interchange" src/passes/` finds nothing. GCC rewrites
`C[i][j] += A[i][k]*B[k][j]` from i,k,j into i,j,k so `C[i][j]` is contiguous
and `A[i][k]` is broadcast, and unrolls k by 2. LCCC keeps source order and
stores the accumulator to memory on every k iteration.

`matmul` is the worst ratio in the benchmark corpus at 1.399x. Interchange
needs dependence analysis to prove legality plus index remapping to rewrite
the nest — a project, not a patch.

### PERF-3 · Register-move gap in zlib_ng_adler32 is code size, not speed

LCCC emits 39 reg->reg moves against GCC's 12, which looks alarming. Measured
per block: the moves are spread across cold blocks, and the hot inner loop
(`.LBB13`, 29 instructions) contains only **2** of them. The kernel already
runs **faster** than GCC (0.930x). Recorded so the next engineer does not
spend a day on it: it is a size defect on a kernel that already wins.

### PERF-4 · **NEW 2026-10-07** — descending loops get no IV strength reduction at all

`find_basic_ivs` matches only `IrBinOp::Add` with a constant step, and LCCC's
frontend emits `i--` as `Sub`, so the two most common descending spellings never
become a `BasicIV` at all:

```
$ CCC_IVSR_DEBUG=1 lccc -O2 -S -o /dev/null desc.c
[IVSR] header=1 no basic ivs          # for (unsigned i = 64; i-- > 0;) s += p[i];
block 5: v7 = Sub(v22, 1) : U32 ; v11 = Cmp Ne (v22, 0) : U32
```

**But "no descending loop forms a `BasicIV`" is wrong**, and the distinction
matters for any future work here. A 16-program probe against the merged compiler
(`CCC_IVSR_DEBUG=1`, `-O2`; table in
[`engineering/FOLLOWUP-2026-10-07-ivsr-audit-response.md`](engineering/FOLLOWUP-2026-10-07-ivsr-audit-response.md)
§2) shows the `i += -1` family DOES form one — `i += -1`, `i = i + -1`,
`i = i + (unsigned)-1`, `i = i + 0xFFFFFFFFu`, `i = i + (0u - 1u)`, `i += -(u32)1`,
and the `i32`/`u64` equivalents — because the frontend renders the decrement as an
`Add` of a zero-extended constant:

```
BinOp v17 op=Add lhs=Value(28) rhs=Const(I64(4294967295)) ty=U32   ; step reads as +2^32-1
Cmp   v29 op=Ne  lhs=Value(28) rhs=Const(I64(0))          ty=U32   ; `i > 0` canonicalises to Ne
```

Two consequences:

1. **A step misrecording.** `find_basic_ivs` takes the step from the raw carrier,
   so a modular decrement is recorded as `step = +2^32 - 1` — a huge positive
   increment. No firing path can carry it (a narrow unsigned IV must present an
   `unsigned_iv_bound`, whose polarity gate admits only `(Ult, +1)` and
   `(Ugt, -1)`), so this is latent rather than live, but it is the first thing a
   descending-loop implementation must fix: reading the step through
   `const_in_iv_domain` turns `+2^32 - 1` into `-1` and makes the descending arm
   of the bound live.
2. **The exit test is `Ne`, not `Ugt`.** `unsigned_iv_bound` declines `Ne` by
   design, so accepting descending loops needs the `Ne`-against-zero polarity
   handled too (sound for a unit decrement: `i != 0` and `i > 0` agree on every
   value a descending unsigned IV takes), not just the opcode and the step.

No descending recurrence fired in any of the 16 probes, for a third reason that is
independent of both: the derived-expression collector found no offset instruction
at all (`[IVSR] no derived exprs`) — a backwards byte walk is already SIB-indexed
(`movzbl (%rdi,%rsi)`), and where an offset does exist it is scaled after a
widening `Cast`, which is PERF-6.

**Do not expect a large win from this alone**: the load in these shapes is already
SIB-indexed (`movzbl (%rdi,%r8)`), so the address is not being recomputed. Measured
against GCC 14.2 on `revvarint` (`-O2`), LCCC is 11 instructions/iteration to GCC's
7, and the decomposition is: no loop rotation (2 branches vs 1), two redundant
extensions (`movl %r8d,%r8d` after `leal`, `movslq %r11d` after `andq $127`), three
instructions of copy around a shift GCC does in place, and no vectorization (GCC's
`g` is 4-wide SSE at ~1.75 insns/element). Ranked: PERF-6 > PERF-5 > rotation >
this. Evidence and full assembly:
[`engineering/FOLLOWUP-2026-10-07-ivsr-review-hardening.md`](engineering/FOLLOWUP-2026-10-07-ivsr-review-hardening.md)
§4.1.

### PERF-5 · **NEW 2026-10-07** — no redundant-extension elimination (`nonzero_bits`)

LCCC emits `movl %r8d, %r8d` to zero-extend a value that `leal -1(%rsi), %r8d`
already zero-extended (every 32-bit x86-64 op does, by definition), and
`movslq %r11d, %r10` to sign-extend a value that `andq $127, %r11` already made
non-negative. The IR carries the first one literally as `Cast { from_ty: U32,
to_ty: U32 }` — an identity cast that survives to codegen because it is load-bearing
for the register allocator's "upper 32 bits are zero" invariant.

GCC tracks `nonzero_bits`/`sign_mask` through `combine`/`fwprop` and deletes both.
LCCC has no such tracking, so the invariant can only be re-established by emitting
the extension.

This is very likely the same root cause as the two worst benchmark kernels against
GCC — `classify` 2.057x `Ir` and `namechars` 1.653x — previously diagnosed as
"backend boolean materialisation, RA/copy-coalescing class" (a redundant `movzbl`
plus `movsbq`/`movsbl` of the same 0/1 byte). `revvarint` is a far crisper
reproducer than either kernel: two extensions, four lines of assembly, no
vectorization or register pressure to confound it. Fixing this is the highest-value
item in the descending-loop decomposition that does not require a vectorizer.

### PERF-6 · **NEW 2026-10-07** — narrow-scaled offsets are not collected by `find_derived_exprs`

`(I64)(i << 2)` — scale inside the narrow ring, then widen — is not collected,
while `(I64)i * 4` — widen, then scale — is. Probed in both loop directions rather
than assumed:

```
PROBE descending=false narrow_shl fired=0     PROBE descending=false wide_cast fired=1
PROBE descending=true  narrow_shl fired=0     PROBE descending=true  wide_cast fired=1
```

Direction is irrelevant, so this is independent of PERF-4. No corpus shape currently
needs it (402 comparisons, 0 changed either way), so the benefit is unmeasured; it
is filed because it is a hole in the collector's pattern set and the next person to
look will otherwise re-derive it from a failing test.

### INF-PGOPID-1 · **NEW 2026-10-07** — `-fprofile-generate` output is not reproducible

The `.profraw` name bakes in the PID, emitted both as a `.byte` list and as
`__lccc_pgo_dump_<hash>_<pid>`. The same binary compiled twice differs from itself:

```
< .hidden __lccc_pgo_dump_3028b24e91f41319_99551
> .hidden __lccc_pgo_dump_3028b24e91f41319_99557
```

Consequence: any assembly A/B over `tests/regression` reports **four** phantom
changes (`pgo_branchy`, `pgo_split_label`, `switch_table`, `value_profiling`) as
"same size, different code". `ab_regression.py` normalises both forms and proves the
normalisation by a self-A/B of one binary against itself. Worth a deterministic-name
option, or at least a comment where the name is formed.

### INF-M32HDRS-1 · **NEW 2026-10-07** — missing 32-bit headers silently disable every `-m32` gate

Three separate package groups, three separate failure modes, all of them looking
like compiler bugs, all of them caused by a wiped workspace losing installed
packages while keeping the repository:

| missing | symptom | gate affected |
|---|---|---|
| `gcc-multilib`, `libc6-dev-i386` | `bits/libc-header-start.h: No such file or directory`, exit 1 | `check_ivsr_domains.sh`; every `-m32` leg of a static A/B **silently skips** |
| `g++-multilib` (`libstdc++-14-dev:i386`) | `bits/c++config.h: No such file or directory` in `i386_dso_emit_semantics` | `linker-suite`: 301 pass / **1 fail** → 302 / 0 after installing |
| `libstdc++-14-dev-i386-cross` | *not* a substitute for the above: it installs under `/usr/i686-linux-gnu/include/c++/14`, which `g++ -m32` does not search | — |

The silent-skip one is the dangerous case: the benchmark + oracle A/B reported
`compared=284 skipped=252` while broken and `compared=402 skipped=0` once the
packages were installed — the same verdict, on 70% of the evidence. Two lessons
recorded:

* a gate that fails identically on both arms of an A/B is not evidence about the
  change — compare the arms against each other first;
* **skip counts are part of a result**, not metadata. 284 comparisons out of a
  possible 402 is not coverage.

This hit because a wiped workspace loses installed packages while keeping the
repository.

### EDG-TRANSPLANT · **NEW 2026-10-01** — mined the open-sourced EDG front end

Register + evidence: [`docs/EDG_TRANSPLANT_ANALYSIS.md`](docs/EDG_TRANSPLANT_ANALYSIS.md)
(14 items E1–E14, subsystem-by-subsystem, licensing adjudicated). Session
narrative + mid-session harness-wipe doctrine:
[`engineering/FOLLOWUP-2026-10-01-edg-transplant.md`](engineering/FOLLOWUP-2026-10-01-edg-transplant.md).

Closed already: **E1** corpus miner (`scripts/edg_corpus_mine.py` selftest
18/18 + `tests/corpus/`: 1 434 Apache Clang-C tests mined with SPDX
attribution and `corpus-index.json` metadata (expectations, per-invocation
`option_sets`, flag translation), runner
`tests/corpus/run_clang_c_corpus.py`, 18 188 GPL-safe GNU test names
manifested), **E2** (pinned Callgrind geometry in `scripts/callgrind_ab.py`
with `manifest.json` evidence — no-PMU measurements now host-reproducible),
**E7 accelerated** (`scripts/edg_changes_mine.py` dual-era parser: 13 562
`Changes` entries losslessly parsed, 1 567 C-relevant extracted to
`docs/edg_changes_c_extract.md` (918 KB) + full index — remaining work is
curation into regression tests).  All three were re-opened and perfected
against the PR #719 and #721 review audits (see
[`docs/REVIEW_719_ADJUDICATION.md`](docs/REVIEW_719_ADJUDICATION.md) and
[`docs/REVIEW_721_ADJUDICATION.md`](docs/REVIEW_721_ADJUDICATION.md)).

Open, priority order (each needs a reproducer/measurement to move):
**E1 remainder** corpus-vs-LCCC triage (runner exists; collect failures);
**E7 remainder** curation;
**E5** work-stack const-eval + step budget (`crash_synth_*` class);
**E3** builtin-signature scrape+generate pipeline (kernel/glibc/expat
fidelity); **E6** bit-field ABI differential corpus; **E4** diagnostic
catalog+tags; **E8** macro expansion diagnostic notes; **E9**
initializer/VLA corpus; **E13** expect recording + flake sonar; **E10** IR
dump/diff; **E11** lccc daemon; **E12** shareable-const interning. T3
(do-not-do, recorded in the analysis): template machinery/IFC, tree-IL
transplant, committing generated giant headers.

## Closed this cycle (do not re-open without new evidence)
<!-- durable here for grep-ability; narrative in engineering/journal/2026-09-W2.md and 2026-09-W3.md -->

VER-DOM-1 (def-dominates-use in the IR verifier — landed as
`src/passes/verify.rs` checks 7 & 8: SSA single definition and
def-dominates-use over an interval-encoded dominator tree, with the
diamond/loop/terminator test family in `verify/tests.rs`; the 2026-09-27
triage found this item stale — the dominance check has been in place
since the IR-verifier birth arc);

`__builtin_memcpy` → native Memcpy + the two latent aliasing fixes it
exposed; chacha20/ARX 10.01× → ~1.05×; `iv_widen` constant-scale firing
(sieve −21.6 %); order-preserving block layout; `CCC_VERIFY_IR` 0
violations/6 levels; machine-level loop inversion (memchr → parity);
S05 inline-single-site-static; S06 cmp-branch-fusion (4 holes, 16
tests); S07 ifcombine profitability (+4.7 % lz4); S09 vectorizer
cond-store use-without-def; S10 vectorizer IV-live-out (`wsum=511`);
bool-pair tail-jmp; loop-memset (lz4 −1.75 % Ir); native rotates; imm32
bit-pattern hoisting; duplicate `GlobalAddr` merge (expat −23.3 %,
lz4 −15.3 %); RA web-wide in-loop-use supply (+3.63/+4.33 % sha256,A/B)
+ boolean-only valve supply; phi acyclic copy order (opt-in
`CCC_PHI_ACYCLIC_ORDER=1`); SROA split `0.0`/`-0.0` bit-exact zero
(CG-07); epilogue-suffix sharing (CG-09); OP-42 transactional sinking;
RA mode-6 stays opt-in (RA-28/28b); TLS CSE merge (3→2).
