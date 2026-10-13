# Named address-space conversion fixtures and oracle policy

This directory holds the conversion fixtures for `__seg_gs` / `__seg_fs`
pointers. Each file is compiled by lccc and by GCC 16.2 with `gcc -w`, and the
file name is the expected verdict: `neg_*` must be rejected, `pos_*` accepted.

**The oracle is GCC 16.2, by major version.** Every seg gate resolves the
oracle through `tests/regression/lib/gcc_oracle.sh` (`GCC_ORACLE_MAJOR=16`), and
never runs a bare `gcc`. The default `gcc` on a runner is not the oracle, so
the gate does not use it even when it is present.

## What is oracle-compared

- **Accept / reject** for every fixture and every row of `parity_table.tsv`.
  lccc, the table, and GCC 16.2 must agree. A table that drifts from GCC fails
  the gate (`check_seg_conversion_parity.sh`).
- **Error versus warning class.** A shape GCC 16.2 errors on is an lccc error,
  and a shape GCC only warns on is an lccc warning that still accepts. Example:
  `q = (int *)gp` with `gp` in `__seg_gs`, a cast to a generic pointer from a
  disjoint space. GCC 16.2 warns with `cast to generic address space pointer
  from disjoint '__seg_gs' address space pointer`, and lccc emits the same
  warning. `check_seg_cast_warning.sh` checks the class without `-w`. The
  fixture gates compile with `-w`, so they check accept/reject only.

## What is NOT oracle-compared

- **Diagnostic wording.** lccc's messages name the types and the pointer level
  (for example `at pointer level 2`), which GCC's do not. A reject must carry
  an address-space, disjoint-space, or incompatible-pointer diagnostic, so a
  reject for an unrelated reason cannot pass, but the text itself is lccc's
  own and is not matched against GCC's.

## Null pointer constants

Only `(void *)0` (and `((void *)0)`, which is the same expression after parse)
is exempt from the named-space check. This matches GCC 16.2: `(void *)0`,
`0`, and `NULL` are accepted for a `__seg_gs` pointer; `(int *)0` and
`(char *)0` are rejected.

## Casts on the right-hand side

A cast is judged by the type it produces, not exempted wholesale. `gp = (int
*)q` is rejected because the cast result is generic, and
`gp = (int __seg_gs *)q` is accepted because the cast names the segment.

## Comparisons

`==`, `!=`, `<`, `<=`, `>`, `>=` between pointers in different top-level
pointee spaces are errors, with a null pointer constant exempt on either side.
Only the top-level pointee space is compared, which matches GCC 16.2:
`int __seg_gs **` against `int **` is accepted.

## Running the gates

The oracle resolves in this order: `$GCC` if set, then `gcc-16`, then `gcc` if
it reports major version 16. An explicit `$GCC` of another major version is a
failure. A missing GCC 16 is a failure everywhere, locally and under CI: a gate
never prints PASSED for a cross-check that did not run. The conversion gates
(`check_seg_conv_diagnostics.sh`, `check_seg_conversion_parity.sh`) run the
named-space positive control on both compilers before any fixture, and invoke
the resolved `$GCC_ORACLE_BIN`. See `tests/regression/lib/gcc_oracle.sh`.
