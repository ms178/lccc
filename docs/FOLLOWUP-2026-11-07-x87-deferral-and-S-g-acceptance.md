# Two i686 defects the sampled invariance gate could not see

**Status: RESOLVED** (this PR). Both found by an exhaustive `STRIDE=1`
`-O0/-O1/-O2/-O3` sweep with CI-equivalent i686 header coverage, root-caused by
bisection through the compiler's own dump/disable knobs, fixed, and pinned by
48 new sentinel rows plus a unit test.

## How they were found

The shipped gate samples 1 source in 32 at `-O2`. Running it exhaustively
(`STRIDE=1 OPTS="-O0 -O1 -O2 -O3"`) over 10 646 jobs produced **19 failures in 6
distinct sources**, all i686, none of them in the sample:

| Class | Sources | Levels |
| --- | --- | --- |
| rejected only under `-g` | `asm_goto_jump_table.c`, `asm_quad_pcrel_key.c`, `kernel_altinstr_layout.c` | O0–O3 |
| `-g` changes the code | `call_return_type_function_pointer_shadow.c`, `fp_die_at_birth.c`, `unroll_const4_f64_distance.c` | O1–O3 |

All 48 target/level/source combinations (both x86 targets, four levels, six
sources) are now sentinels, so the sample can never hide them again.

## Defect 1 — a D6 endpoint was treated as an x87 merge point

### Bisection

| Experiment | Result | Conclusion |
| --- | --- | --- |
| `CCC_DUMP_EACH_PASS`, `-g` vs `-g0` | **byte-identical**, 6184 lines each | not the middle end |
| IR instruction census | 101 vs 101, no kind differs | `-g` adds no IR instruction |
| `CCC_NO_I686_PEEPHOLE=1` (verified to change output, 664 vs 678 lines) | still differs | not the peephole |
| `CCC_RA_GLOBAL_LOCATION=0`, `CCC_NO_SPLIT_RANGES=1`, `CCC_NO_LIVE_RANGE_SHRINKAGE=1`, `CCC_NO_MEM2REG=1`, `CCC_NO_IPA_PTA=1`, `CCC_NO_SIBCALL=1` | all still differ | not register allocation |
| `LCCC_DEBUG_PEEPHOLE_IN` dump | difference already present | codegen, before any peephole |
| 12 runs per variant | 1 distinct output each | deterministic, not hash order |

### Cause

`compute_x87_defer_values` lets an F64 binop keep its rounded result in `st(0)`
when the immediately following binop is its single consumer. The emitter tracks
that copy in `state.x87_pending`, and `CodegenState::x87_pre_emit` flushes it on
any line that could disturb the stack:

```rust
let is_x87_line = s.len() >= 5 && s.starts_with("    f");
if is_x87_line || s.ends_with(':') {          // <-- ANY label
    self.x87_pending = None;
    self.out.emit("    fstp %st(0)");
}
```

Flushing at a label is right for a real label: it is a merge point and a branch
could arrive with a different stack. But `transition_inline_markers` emits D6
range endpoints as labels too, and under `-g` one routinely lands between the
rounding store and its consumer. The result was measured directly:

```asm
; -g0                                ; -g (before the fix)
fstpl 12(%esp)                       fstpl 12(%esp)
fldl  12(%esp)                       fldl  12(%esp)
fldl  48(%esp)                       fstp %st(0)      <-- spurious pop
fadd  %st(1), %st                    fldl 48(%esp)
fstp  %st(1)                         fldl 12(%esp)    <-- reload
                                     faddp %st, %st(1)
```

Debug info bought **less** optimization — two extra x87 ops and a memory round
trip on a hot FP loop.

### Fix

A D6 endpoint is never a branch target: nothing branches to it, every peephole
deletes it from its view, and the DWARF writer only reads it as a range bound.
The flush gate now excludes exactly those labels, using the same exact-match
predicate (`peephole_common::is_inline_range_marker`) the peepholes and the
invariance filter already use, so a lookalike that *is* a real target still
flushes.

The probe runs only when a pending copy is live **and** the line ends with `:`,
so it is off the hot path twice over.

### Proof it restores rather than disables

| | `fstp %st(0)` | deferred `fadd %st(1), %st` | D6 markers | `.loc` rows |
| --- | --- | --- | --- | --- |
| `-g0` | 0 | 8 | – | – |
| `-g` before | 1 | 7 | 12 | 40 |
| `-g` after | **0** | **8** | **12** | **40** |

Code is byte-identical modulo debug rows, the deferral fires in both, and the
debug information is untouched.

## Defect 2 — `-S -g` rejected a unit `-S` accepted

`-S -g` assembles the generated text in memory to measure addresses for the line
program. On i686 a file-scope `.quad 791b` needs `BFD_RELOC_64`, which the 32-bit
assembler cannot represent, so the whole translation unit was rejected.

Measured against the oracle: GCC 16.2 `-m32 -S -g` **accepts**
`kernel_altinstr_layout.c`. (It rejects the other two with `impossible
constraint in 'asm'`, and GNU as rejects all three texts — but `-S` never asks
the assembler, which is the whole point.)

The existing fallback fired only when a proof attributed the failure to inline
assembly. That proof neutralizes the **instructions** inside `#APP` and
deliberately keeps directives — so a failure caused by a directive was
unattributable, the probe still failed, and `-S -g` rejected.

### Fix

`-S` requests assembly *text*. The in-memory assembly is an internal step for
line-table measurement, not part of the request, so **any** failure of it must
degrade to emitting the text with a note — never reject. Attribution is now a
diagnostic refinement that only chooses the wording; the assembler's own error
is always carried in the note, so nothing is silently swallowed.

This is safe because `append_debug_sections` is reached only from
`run_assembly_only`; the `-c` and link paths still assemble for real and still
reject. It also makes `-S` and `-S -g` agree **by construction** instead of by
enumerating the constructs the probe happens to understand.

## Verification

* All 24 i686 rows (6 sources × 4 levels) now PASS; `x86_64` rows unaffected.
* `x87_flush_gate_excludes_only_exact_d6_markers`: real labels flush, exact
  endpoints do not, four lookalikes (bad kind, missing id, empty function name,
  valid shape) are classified correctly, x87 ops flush, `.loc` is inert, no
  pending copy emits nothing, and `x87_suppress` inhibits everything.
* Build: zero warnings.
