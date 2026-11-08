# SIMD forwarding: integer and absolute addresses

## Scope and proof gaps

This is a follow-up to `FOLLOWUP-2026-11-08-vector-provenance.md`; the earlier
cast-only hardening was not sufficient. `pointer_root_analysis` inferred a
root for any integer add/subtract with exactly one currently-known rooted
operand and for every `GetElementPtr` (GEP). The IR does not mark a GEP as
in-bounds, and frontend lowering can erase pointer/integer conversions.
Consequently the absence of a root for one operand does not show that it is a
small object-relative offset. For example, modulo address-width arithmetic can
construct the address of B as `A + (B - A)`. Treating that result as derived
from A makes a write to B falsely disjoint from a vector snapshot sourced at B.

The `integer_address_arithmetic_may_reach_another_alloca` unit test encodes
both an integer addition and a GEP version. Before this change it observed
`[1, 1]` forwarded loads where `[0, 0]` was required. This is an IR-level
counterexample. The C fixture `vector_integer_address_snapshot.c` tests the
same high-level concern with two write offsets, but is **not** a before/after
failure: the old compiler passes it too. Its optimizer can simplify the
address expression before the forwarding pass.

Independently, an absolute numeric source is not necessarily disjoint from a
stack alloca: a stack may be mapped at a fixed address. The old special case
retained the forwarding even for a direct alloca write. The updated
`constant_address_source_may_alias_an_alloca_write` was red before the fix.
This is a missing IR proof, not a C-level miscompile report.

## Design choice

* Preserve roots through copies, pointer-to-pointer casts and a GEP with literal
  zero offset. The zero check uses the full integer value, not a truncating
  `i64` conversion. Preserve direct-alloca disjointness and the existing
  rooted-parameter contracts.
* For all other GEPs and integer add/subtract, keep graph edges for recurrence
  detection but do not infer an object root. Opaque arithmetic in an SCC also
  disables the optimistic single-seed phi shortcut: otherwise the phi could be
  rooted before the unknown backedge is discovered. An identity-only cycle
  still propagates its root. Unrooted writes already invalidate candidates.
* For a numeric source, invalidate on every write, including an alloca write.
  An exact range proof might permit some of these forwards; a fixed mapping or
  pointer inequality alone does not establish one.

This is intentionally conservative. A more precise design would carry
non-wrapping address offsets, object bounds, access sizes and explicit
provenance through lowering; it must account for integer round trips and
cross-object addresses. Merely accepting constant offsets, remembering a
single root, or treating the absence of a second root as disjoint is not such
proof. The new restriction may lose forwards; the evidence below samples but
does not bound the performance impact.

## Evidence

At tree `b8b3f632` (the following `9228f391` adds only the C fixture and flags):

* Fastbuild, formatting, audit census, 50 vector-temp unit tests and 3 targeted
  large-snapshot unit tests passed. The first two described regressions failed
  on the old implementation, then passed with the guards. Positive controls
  cover direct distinct allocas, literal-zero GEP and an identity-only phi
  cycle; an arithmetic cycle is a negative control. An I128 offset with only
  high bits set is not mistaken for zero.
* Against the pre-change fastbuild compiler, 24/24 benchmark-kernel assembly
  files were byte-identical. On an Intel Xeon VM with AVX-512, the 485-case
  mask-facts matrix produced byte-identical LCCC/GCC 16.2/Clang 23.1.5 outputs;
  the enabled build remained at 7,403 static instructions. Its forced-pass-off
  mutation was detected. This is not a program-runtime speedup claim and does
  not measure every vector workload.
* AVX FP contracts: 72 build, execution and frame checks passed on x86-64 and
  i686 with LCCC, GCC 16.2 and Clang 23.1.5. Snapshot interop: 10 AArch64 and
  RV64 QEMU fixture executions passed across O0/O1/O2/O3/Os. These do not
  substitute for physical-machine concurrency or performance testing.
* The C integration fixture passed with both pre-change and post-change LCCC,
  GCC 16.2 and Clang 23.1.5 at O0/O1/O2/O3/Os (20 executions). In GDB 18.1,
  a closely related `-g -O2` fixture's input was 1 and saved vector lane was 1
  before the store; after a single step the input was 100 while the saved lane
  remained 1; the process returned 2 normally. It traces that one execution.
* The pinned GitHub cache supplied the tested toolchains. Local ICX 2026.2
  provisioning was attempted and failed with TLS EOF because the Intel host
  is outside this sandbox's network allowlist. Hosted ICX integration is useful
  but is not evidence that it compared these exact new test cases.

## Remaining work

Final same-tree hosted CI and a gated delivery snapshot remain necessary.
The rest of the provenance graph, asm/memory effects, vector roundtrips,
real-machine AArch64/RV64 memory-model and performance testing, all outstanding
follow-ups, and the 20 requested backlog items are not complete. No exhaustive
correctness or optimality claim is made.
