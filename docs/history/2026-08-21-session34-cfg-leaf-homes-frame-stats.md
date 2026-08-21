# 2026-08-21 session 34 — CFG-leaf ABI homes, empty-frame removal, and RA statistics

Base: `8fb1573bf2167f7b36a1c500320fa7be8ab23591` (PR #159). Round-6 patch is included cumulatively because upstream had not yet incorporated it when fetched.

## Generated-code improvements

### Call-free CFG leaf parameter homes (RA-26)

The one-block-only x86 ordered-parameter-home rule now admits multi-block call-free leaves when all ParamRefs execute in the entry prefix and the signature has at most the six SysV register arguments. Calls, inline asm, late/non-entry ParamRefs, stack arguments, and mixed ABI signatures fail closed. The existing ordered parallel-copy prologue remains the only path allowed to overlap ABI argument registers.

On the two-exit pointer/integer leaf used by the regression:

- control: 16 body instructions, two callee-save pushes/pops, and two entry parameter moves;
- treatment: 8 body instructions, no frame, no saves, no entry parameter moves;
- Compiler Explorer: LCCC 10 instructions vs GCC 16.2 / ICC 8 and Clang 22.1 / ICX latest 9.

A nine-argument regression initially vetoed a broader version: stack arguments changed offsets. The final `<=6` gate was added and `param_home_scratch_reg` passes.

### Empty local frame elision (AB-09)

The x86 layout starts with a conservative `n_saves*8+8` collision reserve. In a call-free, non-variadic leaf where no real local/caller-save slot consumes bytes beyond that reserve, push/pop already represents the complete stack use. The compiler now returns zero local-frame bytes rather than emitting redundant `subq/addq $24,%rsp`. CFA uses the actual push depth (`24` for two saves), not the elided frame size. Calls, varargs, and any real slot retain the frame.

Kill switch: `CCC_NO_EMPTY_LOCAL_FRAME_ELISION`.

### Allocation statistics (MS-02)

`CCC_TRACE_ALLOCSTATS[=function-filter]` emits one deterministic line with eligible values, scan values, assigned/spilled counts, segments/holes, and callee/caller homes. The structural regression verifies filtering and byte-identical assembly with tracing enabled.

## Workload evidence

- gzip 1.14: 30/30 tests.
- `longest_match`: 330 instructions, 118 stack references (one fewer than the prior 119-reference veto baseline).
- Paired gzip VM screening, 20 rounds: treatment/control median ratio 0.992. This is screening evidence, not Raptor Lake hardware evidence.
- Whole gzip text: treatment +16 bytes vs control; no size-win claim is made.
- Expat `expat_utf8_name_length`: current local 114 instructions vs GCC 16.2 78; BTQ classification remains an actual gap.

Artifacts: `artifacts/r7-cfg-leaf`, `r7-gzip-longest-match.txt`, `r7-expat`, `r7-crc`, and `r7-adler`.

## Validation

- 971 unit tests passed, 6 ignored.
- 50/50 correctness.
- 374/374 lccc-only regressions after installing the complete host multilib runtime.
- i686 alias differential: 540/540.
- phi CFG: O2/Os 400/400; latest upstream already mismatches 125/200 O0 seeds (confirmed with an independently built pristine `8fb1573b` compiler), so these are not attributed to this patch.
- gzip stack-memory veto passed: 118 <= 119.
