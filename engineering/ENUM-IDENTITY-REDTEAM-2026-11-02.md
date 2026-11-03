# Enum identity: kernel CI defect and red-team follow-up

**Status: validated targeted repair, not whole-PR production readiness.**
No external human approval is claimed. The reasoning and executable checks below
are review evidence, not a claim that every compiler function is optimal.

## Reproduced production failure

At `bf51dd9f2`, the general CI workflow passed, but kernel check
`125075454389` (run `41382248854`) failed in `kernel/irq/manage.c` at lines
2665 and 2723. Both errors claimed an implicit conversion from
`enum irqchip_irq_state` to the same spelling under `-Werror=enum-conversion`.
The Checks API annotations retain this evidence when the runner's redirected
Azure log host is inaccessible.

The bug was introduced by comparing complete `EnumType` snapshots for diagnostic
identity. A function-pointer member can capture an incomplete enum before a later
definition supplies its enumerators. These are two representations of **one tag**,
not two enumeration declarations. The previous simple-enum fixture suite missed
this ordering. Merely comparing names would repair this example but incorrectly
merge distinct inner-scope declarations with the same spelling and values.

## Selected design and alternatives

* Reject structural snapshot equality as diagnostic identity: completion changes
  the fields, and walking every enumerator is not an identity operation.
* Reject tag-name equality: lexical shadowing must remain distinguishable.
* Reject process-global or pointer-address IDs: they add global lifetime/state or
  allocation-address dependence. Use TU-local, monotonically allocated nominal
  IDs, with scoped undo for visible tag names and source-position keys for
  repeatedly resolved anonymous definitions.
* Use `Option<NonZeroU64>`: its tested representation is **8 bytes**, versus the
  extra discriminant word of `Option<u64>`. No extra per-type heap allocation is
  needed. Counter exhaustion is checked, never wraps/recycles, and yields unknown
  identity rather than adding a hot-path panic. Existing IDs still resolve.
* Borrow named object/enumerator metadata through `Cow`; only complex expressions
  require owned inference. Two tests with **4,096 enumerators** assert pointer
  identity of the borrowed data. This proves that this accessor does not clone
  those tables; it is **not** a measured whole-compiler throughput claim.
* Reject non-enum destinations before querying source-scoped warning policy.
  Warning promotion/pragma handling and the language type of enumerators remain
  unchanged (an enumerator can still have type `int`, separate from provenance).

The GNU unfixed inner forward-declaration fixture exposed a GCC/Clang behavioral
difference. The regression keeps the GCC diagnostic expectation for the tested
callback reference before the later definition; this is not presented as universal
agreement between the two compilers on enum extensions.

## Executed validation

Reviewed cache `harness-cache-7` was actually fetched and hash-verified: Rust
1.99.1, GCC 16.2.0, Clang 23.1.4, binutils 2.47 and QEMU-user 11.1.2. A **4 GiB**
swap file was enabled. Compiler/linker binaries use normal optimized `fastbuild`.
Focused Rust unit tests use the fastbuild preset with opt-level/debug overrides
of zero to avoid repeatedly optimizing the test harness under memory pressure;
these commands do not replace the optimized compiler executables.

| Check | Result |
|---|---|
| Final optimized compiler/linker rebuild | PASS, 1m21s |
| Enum diagnostic differential, including forward callbacks/typedefs, same-spelling shadows, anonymous types and scope restoration | PASS, **720** non-kernel GCC comparisons |
| Callback ABI: incomplete enum member prototypes, ordinary/packed/wide enums, volatile indirect calls, noinline callees, O0/O2/Os | PASS, **54** executions: four LCCC targets plus native GCC/Clang; zero unmeasured cross executions |
| Enum scope/compactness/exhaustion units | PASS, **6** |
| Borrowed metadata, integer enumerator language type and ordinary-name shadowing units | PASS, **2** |
| Existing diagnostic engine units | PASS, **17** |
| Integrated warning gate | PASS: original precedence suite, **60** pragma, **697** include-path, **636** date/time, **2,940** layout/unused, **6** invalid numeric-option checks, plus enum checks above |
| Verbose assembly/native ELF equivalence | PASS, **12** cross-target modes |
| Native/GNU linker contracts | PASS: ELF32/64 RWX contracts, **16** discard-none comparisons, AArch64/RISC-V QEMU-user execution |
| Microsoft scalar/wide ABI script | PASS |
| Census | PASS; no metric grew and no baseline was relaxed |
| Workflow shell and lane-plan structural checks | PASS; no gate removed |

All C fixtures in these checks are **non-kernel** fixtures. No kernel target code
was compiled with GCC or Clang. ABI checks do not replace a real kernel QEMU boot.

Local authenticated ICX 2026.2 provisioning was attempted but failed with a TLS
EOF at the restricted network boundary. The new dedicated hosted ICX regression
requires the provisioned compiler, checks its 2026.2 version macro, retains the
runtime log, and fails if it is missing. Local results explicitly mark ICX
unmeasured; no local ICX execution or three-oracle agreement is claimed.

## Open findings — do not erase these on a green fixture run

1. **Prototype-scope incomplete enums remain a separate defect.** Reduction:

   ```c
   void sink(enum E);
   enum E { K };
   void f(enum E x) { sink(x); }
   ```

   GCC 16.2 and Clang 23.1.4 reject the call because the prototype's enum is
   incomplete and not the later file-scope tag. LCCC currently accepts it. Fixing
   this requires correct prototype scope and completion constraints, not a
   same-name diagnostic exemption or rejection of every incomplete snapshot
   (which would break the valid kernel callback pattern repaired here).
2. The previous sandbox's local full-kernel process/log did not survive the wipe;
   its completion result is unknown. The hosted kernel failure above is verified.
   A fresh full link, real objtool validation and system-QEMU kernel boot remain
   required; QEMU-user fixture success is not a substitute.
3. Designated-init, frame-larger-than, vla-larger-than, implicit-fallthrough,
   format-overflow, format-truncation, missing-format-attribute, restrict and
   stringop-truncation, plus the remaining inventoried options, are still open.
   Unimplemented-option warnings must not be silenced or called implementations.
4. No correctness-backed kernel runtime/code-size improvement has been measured
   for this repair. No superiority over GCC/Clang/ICX is asserted.
5. Latest main `d202421fe` is integrated. Refreshed hosted CI, including the actual
   kernel and mandatory ICX leg, must pass before readiness can be claimed.
