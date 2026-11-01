# Microsoft x64 ABI: implementation and validation status

This is a **partial implementation**, not completion of the kernel/QEMU task.
The non-default ABI is represented on function types and call instructions,
including typedefs, function pointers, composite types and indirect calls.

## Implemented ABI paths

- Four shared argument positions: RCX/RDX/R8/R9 or XMM0–3 by position.
- Caller-owned 32-byte home area, overflow arguments and stack alignment.
- C-record arguments/returns: integer carriers for 1/2/4/8-byte objects,
  hidden return pointers otherwise, and 16-byte-aligned caller-owned copies
  for indirect arguments. Callee copies read exactly the object bytes without
  clobbering uncaptured incoming argument registers.
- Signed/unsigned integer128 returns use the GNU Microsoft ABI single-XMM0
  carrier. Fixed and variadic integer128 arguments use 16-byte-aligned private
  images and one pointer position each. Semantic IR retains integer128 values
  through optimization and inlining; backend-only legalization rewrites both
  parameter capture and call transport together.
- Floating-point return and variadic callers, including GP duplicates of FP
  register arguments. Microsoft variadic definitions use GP home slots and
  eight-byte char-pointer cursors, separate from the 24-byte SysV va_list
  record. Start/copy/end and indirect wide/record payload reads are supported.
  Malformed Microsoft builtin arity, cursor types and start contexts are
  rejected in both syntax-only and code-generation modes.
- Preservation of RDI/RSI and, when SSE is enabled, XMM6–15, with synthesized
  DWARF save records. The current save policy is conservative; no performance
  superiority is claimed.
- Optional written conventions in declaration ASTs distinguish absence (which
  inherits an established ABI) from explicit `sysv_abi` (which can conflict).
- Specifier attributes propagate to comma-separated declarators. Scalar argument
  checking applies ordinary array/function decay.
- SysV-only typed-call, sibling-call and register-allocation optimizations do not
  reinterpret foreign-ABI calls as SysV calls. The general ABI-aware call path
  handles those sites.

## Reproducible evidence (2026-11-01)

`bash tests/regression/check_ms_abi.sh` uses GCC **only for non-kernel oracle
objects**, and LCCC for the implementation under test. Passed with GCC 16.2.0
at `-O0`, `-O2`, `-Os`, and `-O2 -fstack-clash-protection`:

- Both GCC-to-LCCC and LCCC-to-GCC scalar interoperability.
- Mixed positional GP/FP arguments, FP returns and six-argument overflow.
- Direct, volatile indirect, callback parameter, array and struct-field calls.
- Assembly witnesses for home-slot writes, stack alignment and preservation of
  RDI/RSI/XMM6–15 across a Microsoft-to-SysV clobbering call.
- Mixed Microsoft/SysV integer calls in the same function (a probe that exposed
  and now guards the incorrectly admitted SysV `CallTyped` path).
- Variadic FP duplication, string/function decay, declaration inheritance and
  rejection of conflicting explicit conventions.
- 32-KiB frames, page probing, actual libgcc unwinding through Microsoft frames,
  and `readelf` save-record checks for all twelve extra nonvolatile registers.
- Integer-only `-mno-sse -mno-sse2` emission without vector registers.

The earlier kernel5 compiler SHA256 was
`883b8b34d4b13e7685de9426729bca9827c76bd8aa99c485e885895200e1a87c`.
The frozen kernel6 scalar/record/variadic compiler is
`6cce8a32fa17c14fd17608ede004a617d802c8ceb30d6922fd57d404f9218b39`.
These are intentionally separate executables; kernel artifacts are not
attributed to the newer compiler. Hosted CI must validate the submitted source.

Run `check_ms_abi_records.sh` and `check_ms_abi_varargs.sh` separately too (both
are wired into hosted and local CI). The latter passes GCC-to-LCCC Microsoft
variadic definitions at O0/O2/Os: mixed register/overflow FP/GP slots, cursor
copies and helper consumption, five fixed arguments, hidden return pointers,
and simultaneous SysV variadic use. Payloads include integer128, long double,
IEEE binary128, three-byte records, all three complex precisions and a 16-byte
vector. Integer128 fixed signatures and variadic callers are now separately tested.
Other fixed wide signatures and other wide variadic callers remain diagnosed
rather than silently assigned SysV transport.

The actual patched CachyMod 6.18.58 default configuration was retained (SHA256
`3d06a10a7773a3ca9fff4f425b8adff0cf373c656b849a7202744924034d1e5d`).
The latest boot-only run passed at **31,024 / 32,768 bytes**, with 24 ISA-scanned
objects and a byte-identical GNU-linker parity check. No GCC target compilation
was used. The full build subsequently compiled `arch/x86/kernel/cpu/common.o`,
the translation unit previously stopped by fourteen `ms_abi` diagnostics.
That full run subsequently stopped in `drivers/gpu/drm/i915/i915_pmu.c`: a
valid static address of a nested array-compound-literal member was rejected.
The rebuilt compiler passes that actual translation unit in a focused compile
and a ten-address mutable-object reduction at O0/O2/Os. The reduction also
caught and now guards transient AST-pointer reuse in literal caches and missing
multidimensional initializer strides. Existing compound-literal verdict,
runtime and relocation checks pass all 41 rows (35 accepted).

See `KERNEL-PATTERN-REVIEW.md` for source-inventory coverage, remaining semantic
review obligations, and measured non-kernel variadic-home-save improvements.

## Remaining acceptance criteria

- C-record interoperability now passes at O0/O2/Os for sizes 1..33/64/128,
  including FP-field integer carriers, hidden return pointers, register/stack
  indirect parameters and mutation isolation of caller-owned copies. The tested
  executable is `/home/user/compiler-next/lccc`, not the running kernel compiler.
- Fixed wide scalar arguments/returns other than integer128, complex/vector
  ABI signatures and wide variadic callers other than integer128 still require
  implementation; these remain rejected.
  Fetching those payloads in Microsoft variadic definitions is separately
  tested and does not remove that guard.
- Parser attribute placement and inherited conventions need further adversarial
  coverage, including post-asm attributes and typedef-derived declarations.
- `-Wundef` now has evaluated-operand diagnostics, not option silencing.
  The warning-precedence gate passes 109 GCC 16.2 differential cases, with
  its 18 undef cases checked in compilation, preprocessing and syntax-only
  modes. The frozen kernel6 compiler predates this warning implementation;
  its warning stream is not claimed clean.
- Further kernel compiler/linker defects and unsupported-option warnings must
  be resolved. The previous full build failed at i915; a clean, content-identified
  rebuild with the validated fix is required, not a successful kernel yet.
- Latest main (`2503b046e`) has been merged into the draft branch. The new
  boot-first rebuild uses compiler SHA256 `6cce8a32fa17c14fd17608ede004a617d802c8ceb30d6922fd57d404f9218b39`
  and rebuilt linker SHA256 `6a2ac22794120f984caf8a71ec7bc0a1858ce4c52a3244c691831c3c821399d3`.
  Preparation and the 31,024-byte boot-only gate pass again, including all
  24 ISA scans and byte-identical GNU-linker parity. The clean full build
  has restarted; its final link and QEMU result remain outstanding.
- A complete kernel link, QEMU boot and green hosted CI remain required before
  production readiness. No full-kernel success or performance improvement is
  asserted by the scalar tests.

## Integer128 return extension

The existing `check_ms_abi.sh` gate also checks 25 full-width signed/unsigned
bit patterns at O0/O2/Os against non-kernel GCC 16.2 oracle objects. Direct and
volatile indirect returns, callback parameters, dereferenced and computed
function pointers, mixed GP/FP register and overflow arguments,
Microsoft-to-Microsoft and SysV crossings, and Microsoft
variadic definitions are covered. Return emission fails closed when SSE is
disabled. The record and Microsoft variadic gates also pass unchanged.
These tests are not kernel runtime or code-size measurements. The running
full6 build intentionally uses the frozen compiler and does not include this
extension or the later warning changes.

## Additional warning implementation

`-Wmissing-prototypes` and `-Wmissing-declarations` now diagnose externally
visible function definitions against explicit prior file-scope declarations.
Implicit builtin knowledge and unrelated block-scope declarations do not
silence them. Static/inline definitions, main, old-style declarations, prior
prototypes and combined warning-leaf precedence are checked against GCC 16.2.
The expanded gate passes 109 cases: 34 new declaration/prototype cases each in
compilation and syntax-only modes, plus the previous warning cases. Actual
i915 syntax-only compilation with its original kernel flags passes too.

The driver core contract passes. A separately attempted full driver gate
passed its 64-bit trapv matrix (910 expected traps, zero mismatches), but its
32-bit leg failed because this sandbox lacks the required 32-bit libc headers.
It is not counted as a full-driver pass. This does not relax hosted coverage.
The full6 kernel compiler predates these diagnostics, so its tolerated-option
warnings remain explicitly outstanding.

## Integer128 argument extension and oracle red-team result

`check_ms_abi.sh` now covers fixed integer128 arguments and variadic callers,
including four simultaneous private copies, register/overflow positions,
signed and unsigned bit patterns, mixed GP/FP arguments, callbacks,
always-inline helpers, hidden record returns combined with a named wide
parameter and variadic payloads, and no-SSE integer-return functions. Assembly
witnesses check all four argument-image addresses for 16-byte alignment and
mutate a callee-owned image to prove isolation of the caller's live SSA value.
The runtime gate passes at O0/O2/Os and O2 with an 8-byte preferred stack
boundary, against both GCC 16.2 and Clang 23.1.4 callers/fixed-signature callees.

The oracle self-check exposed a GCC 16.2 receiver limitation: its Microsoft
integer128 variadic caller passes indirect images, but its own `va_arg` reads
aligned inline bytes. The minimal non-kernel
`tests/regression/fixtures/ms_abi/wide_variadic_self_control.c` exits **1** with
GCC 16.2 and **0** with Clang 23.1.4. Therefore the gate requires the pinned
Clang receiver for that direction and verifies Clang-to-Clang agreement before
comparing LCCC's caller. GCC still supplies the callers and fixed-signature
oracles. This is not a claim that the broken GCC receiver passed, and missing
Clang is a failure, not a skip.

Three backend unit tests pass: unchanged functions retain a borrowed view;
physical views preserve the semantic input, source-span pairing, unique IDs
with stale cached bounds, alloca definition order and idempotence; and SysV
calls retain their integer128 operands. Record and Microsoft variadic gates
also pass unchanged, as do the 109 warning-precedence cases. The new pass is
x86-64-only; other backends retain the default borrowed view.

Design alternatives: mutating only call operands would break inlining's typed
parameter contract. Normalizing the entire optimizer IR would couple every
semantic pass to physical Microsoft transport. A backend-only physical view
avoids both problems and adds no copy of unrelated function IR. Caller images
must be private because callees may overwrite them. The current callee capture
is deliberately conservative; copy elision and nonvolatile-save pruning still
need alias/provenance, debug/unwind and performance evidence. No optimality or
kernel runtime/code-size benefit is claimed from this ABI extension alone.

## Scoped diagnostics and function-pointer casts

The warning gate now passes **146 warning-precedence cases** against GCC 16.2,
including 37 additional function-pointer cast cases. `-Wcast-function-type`
uses GCC's deliberately permissive predicate, not strict C compatibility:
equal-width non-boolean integers, pointer parameters, old-style signatures,
variadic prefixes and the `void (*)(void)` wildcard have distinct rules.
LP64 and ILP32 syntax-only cases check width-dependent decisions.

GCC diagnostic push/pop, ignored, warning and error classifications are now
preserved in preprocessed output, including at the exact position of a
same-line `_Pragma`. Parser/sema/lexer reporting selects settings by
preprocessed byte offset, not a possibly repeated original file/line.
Preprocessor warnings retain their occurrence's own settings. Source-name
provenance for emitted markers is independent of user redefinitions of
`__FILE__` and is restored across includes.

The hosted warning gate also runs **58 scoped-diagnostic mode checks** against
GCC 16.2, checking severity and source line in compilation, syntax-only, and
(where relevant) preprocessing modes. Coverage includes nested and unmatched
pop, blanket `-w`, CLI promotion/demotion, macro keyword collisions, repeated
includes, scopes crossing includes, and `#line`. GCC's positive-classification
analysis flags are sticky: an ignored prototype warning must not spuriously
fall through to a declaration warning. Unknown pragma-option diagnostics and
other pragma families are not claimed as complete.

The actual `kernel/cgroup/rstat.c` command now passes syntax-only with its
original Kbuild flags. The previous rejection of `bpf_rstat_flush` was caused
by ignored GCC diagnostic scopes in the BPF hook macros; no BPF/weak-function
exemption or warning-flag removal was introduced. This is frontend evidence,
not a successful kernel codegen, final link, boot or performance result.
