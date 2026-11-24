# Assembler statements and the registers the compiler keeps (x86-64, i686)

A GNU `asm` statement names registers three ways: an operand constraint
(`"b"`, or the register of a register variable), the clobber list, and the
symbols its memory operands print. The compiler keeps registers too: the GOT
pointer, the frame pointer, the homes of live values. Where the two meet,
the compiler gives way. This file records where lccc did not, what GCC 16.2
does there, and what lccc does now. Everything is held by
`tests/regression/check_asm_registers.py` (below).

## What was wrong

Found by auditing `include/x86gprintrin.h`, whose `_mm_mwaitx` and enclave
intrinsics take `%ebx`; none of the defects was in the header.

| Statement | Before | GCC 16.2 |
|---|---|---|
| i686 `-fPIC`/`-fPIE`: takes `%ebx` (`"b"`, `cpuid`'s `"=b"`, a clobber, a register variable) | The GOT pointer stayed overwritten: the next global access or PLT call went through the wrong register. A PIE that asked `cpuid` and then called `printf` crashed, at every optimization level | The GOT pointer is a pseudo register and lives elsewhere meanwhile |
| i686: clobber `"bx"`, `"bl"`, `"bh"`, `"si"`, `"sil"`, `"di"`, `"dil"`, `"rbx"`, ..., and `%ebp` in every spelling | Not understood: a live value stayed in the destroyed register, or the register was not saved for the caller (16 of 35 spellings) | All are names of the register |
| x86-64: clobber `"rbp"` (any spelling) | The function was *given* a frame pointer because of it, and addressed every slot behind the statement through the destroyed register | The frame pointer is omitted and `%rbp` is one more callee-saved register |
| A function that needs its frame pointer, and a statement that takes it | Compiled, as above | `error: bp cannot be used in 'asm' here` |
| `"m"(global)` in position-independent code | x86-64: `global(%rip)`, which no linker makes a shared object of. i686: the absolute `global`, a text relocation (`DT_TEXTREL`) | Through the GOT; `local@GOTOFF(%reg)` for a local symbol |
| `"m"(thread_local_object)` | The address of the image threads are initialised from: `incl tls(%rip)` incremented that image and left the thread's variable alone | `%fs:tls@tpoff` and the other TLS models |
| A clobber that names nothing (`"r14l"`, `"EBX"`, `"st(0)"`) | Taken, and ignored | `error: unknown register name 'r14l' in 'asm'` |
| i686: a function after one with a variable-length array, `alloca` or `__builtin_frame_address` | Kept a frame pointer too: the decision was never reset, and cost every later function of the file one register and two instructions | Decided per function |

FFmpeg's `configure` asks two of these questions of the compiler and builds
its inline assembly on the answers: `ebx_available` is `"" :: "b"(0)` and
`"" ::: "%ebx"` (current master), `ebp_available` is
`__asm__ volatile ("xorl %%ebp, %%ebp" ::: "%ebp");` followed by a read of a
stack variable (releases 4.4 and 6.0; the current master no longer asks).

## What lccc does now

**The GOT pointer (i686).** A function that keeps the GOT pointer in `%ebx`
and has a statement that takes `%ebx` gets a frame slot for the pointer. The
prologue stores it once; behind each such statement, and on every edge of an
`asm goto`, one load brings it back. Inputs are loaded and outputs stored
without the pointer, so the reload is the last thing the statement does.

Alternatives, measured on the host (i686 code, Ice Lake, 400,000,000
executions of "take `%ebx`, restore the pointer, read a global through it",
best of seven):

| Restore | Time | Bytes per statement |
|---|---|---|
| Compute it again (`call __x86.get_pc_thunk.bx`, `addl $_GLOBAL_OFFSET_TABLE_, %ebx`) | 0.543 s | 11 |
| Reload from the frame slot (`movl 8(%esp), %ebx`) | 0.251 s | 4, and 4 once per function for the store |

Pushing and popping `%ebx` around the statement was not measured: it moves
`%esp` under a template that may call a function, and GCC never does.
Keeping the pointer in another register for the whole function, as GCC's
register allocator may, takes one of three callee-saved registers from
every instruction of the function to save one load per statement.

**Clobber names.** One table (`x86_gpr32_of_name` in
`src/backend/inline_asm.rs`) maps every spelling GCC takes to the register;
the register allocator's exclusion, the prologue's save list and the scratch
pool read it. `%ebp`/`%rbp` are callee-saved registers like the others where
the frame pointer is omitted.

**The frame pointer.** A statement that takes the frame pointer register no
longer forces a frame pointer on x86-64. Where the function cannot do
without one (no optimization, `-fno-omit-frame-pointer`, a variable-length
array, `__builtin_frame_address`, and lccc's own reasons: stack-clash
probing of a large frame, over-aligned call arguments), the statement is
refused in GCC's words at GCC's position, the closing brace of the function.
Code generation reports it through `CodegenOptions::codegen_errors`, the
first error only the backend can see; the driver prints it and fails.

**Symbols in memory operands.** The operand is what the symbol is: through
the GOT when another module may define it, relative to the GOT pointer or
the instruction pointer when it is local (through a register of its own on
i686 when the statement takes `%ebx`), and the thread's object for a
thread-local one: `%fs:sym@TPOFF` / `%gs:sym@NTPOFF` in the local-exec
model, the offset from the GOT in initial-exec, the computed address in the
dynamic models.

**Unknown clobbers.** Refused with GCC's message at GCC's position when
neither GCC 16.2 nor Clang 23.1.5 takes the name. The sub-register names of
`r8`-`r15` (`"r12d"`) and a few status registers are Clang's alone; lccc
takes them, and honours them. One `%` or `#` in front of a name is GCC's
register prefix and is stripped before the name is looked up, `"%cc"` and
`"%memory"` included.

**The i686 interoperability gate.** `tests/regression/check_i686_overalign_interop.sh`
failed two of its twenty combinations here, both with GCC as the caller,
one of them with GCC on both sides. Its test programs entered a C function
as `_start`, four bytes off the stack alignment the function is compiled
for, and GCC's code for the `aligned(16)` case faulted on a `movaps`. The
entry point is now a call; the gate passes with GCC 12 and GCC 16.2.

## The gate

```
python3 tests/regression/check_asm_registers.py --lccc target/fastbuild/lccc
```

| Check | Count | On the compiler before this work |
|---|---|---|
| Programs equal to GCC's: x86-64 and i686, position dependent, `-fPIC`, `-fPIE -pie`, -O0 to -Os | 92 | 77 differ |
| Shared libraries whose statements name globals, linked by GNU ld with `-z text`, sharing an object with the program | 2 | 2 refused |
| Refusals of the frame pointer register, compared by line | 10 | 6 compiled |
| Clobber names judged as GCC judges them | 212 | 74 taken that name nothing |
| Negative control (a program built to print something else) | 1 | |

`clobber_spellings.c` calls each of its 48 functions through a hand-written
caller that loads every callee-saved register with a known value first and
reports the ones that came back changed, so that a register the function
forgot to save is seen as well as a wrong result.

## Not done

* An i686 position-independent function sets up the GOT pointer for any
  assembler statement, whether or not the statement or anything else in the
  function needs it (`function_needs_got`: "inline asm, which may name any
  symbol in text we do not parse"). A function that is only a `cpuid` or a
  `lock cmpxchg8b` pays the thunk call, the add, a push and a pop that GCC
  does not emit. Deciding it from the operands is possible; from the
  template text it is not, and the rule stayed conservative.
* The address of a symbol memory operand is still computed into a slot the
  statement does not read (i686 position-independent code).
* GCC also refuses a register variable whose register the same statement
  clobbers ("'asm' specifier for variable 'x' conflicts with 'asm' clobber
  list"). lccc takes it.
* The other targets keep their rule: a statement that mentions the frame
  pointer register gives the function a frame pointer.
