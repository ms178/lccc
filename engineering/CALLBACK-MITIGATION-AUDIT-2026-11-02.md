# Callback capture and mitigation audit — intermediate, not readiness

## Reproduced failures

1. Global-cell call folding emitted `call *slot(%rip)` despite an explicit
   external/inline retpoline request. The new object-code assertion fails on
   the pre-fix compiler at O0. The completed kernel reported 271 indirect-call
   warnings, including check_hw_exists in arch/x86/events/core.c.
2. `F saved = slot; slot = replacement; return saved(3);` called replacement
   instead of saved. The standalone non-kernel reproduction fails at O0, O2
   and Os before the repair and passes all three afterwards.
3. Named GOT calls (-fno-plt) also emitted raw indirect calls. They now use the
   selected mitigation without corrupting argument registers or the SysV
   variadic SSE argument count in al.

## Design and alternatives

An emission-only retpoline guard would repair the immediate security warning
but leave target pre-staging/sibling-call selection inconsistent, and would
leave the snapshot miscompilation in unmitigated builds. A name whitelist for
pv_ops would retain the same C-language violation for that object. Neither is
acceptable.

The resolver now follows only address arithmetic to a symbol VALUE; it never
looks through a load. Captured data-cell values use the established register
call pipeline. Genuine symbol addresses remain direct, with the existing
interposition, version/TLS/absolute guards and checked offset arithmetic.
Actual paravirtual patch sites are explicit inline assembly and are unchanged.
This removes the misleading MemorySlot representation rather than adding more
special-case security policy around an unsound memory reread.

The GOT helper emits a load into r11 and delegates to the existing external or
inline retpoline emitter. r11 is caller-saved, not a C argument register in the
supported x86-64 SysV/Microsoft conventions, and not the SysV r10 static chain.
It preserves al after variadic argument staging. Unmitigated GOT calls retain
their original encoding. Sibling-call protection remains owned by the existing
return-transfer emitter. This is a correctness/security cost where protection
is requested, not an asserted code-size or runtime improvement.

## Executed evidence

* Optimized fastbuild: PASS, 45.97 s. Active swap: 4 GiB.
* All-target Clippy -D warnings: PASS, 2m47s.
* Expanded codegen gate: **215 object/runtime contracts PASS**; existing four
  LCCC targets retained, GCC exactly 16.2.0, Clang exactly 23.1.4.
* New non-kernel fixture covers scalar global cells, struct-member cells,
  register callbacks, seven integer arguments, and captured scalar/member/
  volatile callbacks followed by writes to their original cells.
* Each native compiler runs O0/O2/Os with external, inline and disabled
  mitigation. Object disassembly rejects every raw indirect call/jump in
  protected modes; unprotected mode is a positive control. Runtime calls use
  real speculation-trapping thunks, not unprotected jump stubs.
* Separate -fno-plt translation units exercise named GOT calls with named and
  variadic doubles: runtime results check floating argument delivery and al.
* All three Microsoft ABI suites: PASS. Symbol-version call regression: PASS.
* Actual arch/x86/events/core.c rebuilt using only LCCC: genuine
  objtool --retpoline --Werror --dry-run PASS, without suppressing warnings.
* Workflow shell, formatting, branch diff and CI/local parity are checked
  separately before checkpoint publication.

ICX is **unmeasured locally**, not counted as a pass. Its existing authenticated
Intel-PPA CI lane now executes this gate with --native-only --require-icx,
requiring ICX 2026.2 plus the pinned GCC/Clang. Default execution still requires
all four LCCC targets; native-only does not replace that coverage. CI is pending.

## Remaining audit findings

The previous image linked and passed qemu64/max system-QEMU smoke, but still
has 416 final objtool warnings and unsupported compiler-option warnings.
This callback revision needs a new full kernel link/guest run. UACCESS warnings
also reproduce when LCCC's assembly is assembled by GNU as, excluding a
native-assembler-only explanation. In check_zeroed_user a casted boolean merge
leaves an unnecessary post-STAC conditional path that objtool cannot correlate
with its constant incoming value. Dead stac/clac bodies are also retained by
conservative assembly-name scanning. These are under investigation, not fixed
or masked by this checkpoint. No external expert approval is asserted.
