# Review follow-up decisions — 2026-10-19

This is a decision record, not a validation log. Exact commands and the final
Git tree belong in the change review so they cannot become stale prose here.

## Lexical depth and binary128 zero

`SymbolTable` and `TypeContext` intentionally have different storage models but
the same public depth convention: file scope is 0 and lexical scopes are 1+.
`TypeContext` frames are undo logs, so adding a disposable file frame would
corrupt shadow detection and scope unwind. Cross-map tests pin depths 0–3.
`SymbolTable::pop_scope` now also refuses to destroy its final file scope and
diagnoses an unbalanced pop in debug builds.

The actual integer `IrConst` variants are `I8`, `I16`, `I32`, `I64`, and
`I128`; every zero form is covered. Floating constants are excluded because
`-0.0` must keep its sign. The runtime regression checks all 128 bits after both
integer zero and `-0.0F128` cross the same ABI boundary.

## Builtin policy and inlining

A name-based ban on inlining TU-defined libcalls was removed after mutation
measurement. Deleting the ban did **not** break the complete libcall synthesis
gate: ordinary external `memset` was inlined while its custom semantics and all
runtime rows remained correct. This is expected: `builtin_policy::apply`
withdraws synthesis for a TU definition before inlining, and the body carries
that policy.

The experiment exposed a narrower real defect. A GNU extern-inline libc body
has no out-of-line TU definition, so ordinary calls may still denote libc; yet
once its body is cloned, later loop-idiom synthesis must not rewrite the custom
inline implementation into a call to libc. GNU-inline bodies now carry a local
withdrawal and inlining conservatively merges builtin policies. End-to-end rows
pin ordinary, `always_inline`, and GNU-inline libc-named definitions. This is
more precise than either globally disabling libc optimization or imposing a
spelling-based inlining performance cliff.

Commit `21772cf83` is valid and is an ancestor of the original change; the
review claim that it did not exist was a harness/repository-history error. The
review was correct that the old shell assertion allowed either a call or a
semantically correct inline body and therefore did not justify a call-retention
claim.

## TLS models

The first follow-up deliberately rejected unimplemented dynamic models rather
than approximating them. Native x86-64 and i386 codegen now implements all four
GNU models, automatic linkage/PIC-sensitive selection, and per-declaration
`tls_model` overrides. x86-64 uses GNU2 descriptors so dynamic accesses stay
linker-relaxable without an ordinary call's register clobbers on either ABI. A DSO loaded after a worker thread exists verifies
real dynamic allocation on both ABIs. AArch64 and RISC-V remain explicit
Local-Exec-only errors until their object writers and dynamic linkers support
the complete contract. Optional `gcc_m16` delegation continues to hand GCC the
original model option unchanged.

## Peephole operand classes

The `movq` extension fusion test now drives an XMM→RAX producer through all six
consumer kinds and retains a positive GPR control for each. Removing the GPR
family bound makes the test fail by indexing beyond the 16-family alias table.
GPR→XMM and memory-source forms are tested at classification, where they are
rejected, rather than presented as vacuous fusion tests. The memory form is not
pinned as a permanent missed optimization.

## Harness and workload safety

The SysV generator's `--emit-only` mode no longer requires a fake LCCC path and
is exercised in local and hosted CI with all three generated files checked.
The linker process wrapper handles an empty command, reports a missing working
directory separately from a missing executable, and has focused tests.

The glibc workload treats patch bytes—not an unrelated repository HEAD—as the
input authority. Git revision remains logged provenance. Host compiler values
are explicitly bare paths; one recognized compiler can derive its sibling,
while arbitrary wrappers must provide both paths. Same-target validation,
destructive-cleanup confinement, symlink-escape rejection, version fallback,
and failure-log reporting are covered by a fast hermetic shell test.

## Reproducible measurements

The pointer-width correction remains a correctness fix with no invented speed
claim. Across the deterministic 186-shape SysV grid, assembly before and after
was byte-identical: zero assembly-byte and zero spill-byte delta. The commands
and hashes remain in the glibc follow-up document.

The repository census is `python3 scripts/audit_census.py --selftest` followed
by `--check`; there is no `cargo run --bin audit` interface. A held ratchet is
left unchanged rather than regenerated gratuitously.
