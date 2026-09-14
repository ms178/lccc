# AUDIT 2026-09-13 — i686 TLS IE relaxation: the `C7 /reg` #UD bug

**Status:** FIXED, regression-gated, CI-green.
**Blast radius:** every i686 executable whose initial-exec TLS sequence
relaxes to a destination register other than `%eax` (i.e., almost every
program with more than one TLS variable, or any variable the allocator
puts in ecx/edx/esi/edi/ebp).

## Symptom

`lccc -m32` linked executables using `__thread` variables crashed with
SIGILL (`trap invalid opcode`, `error:0`) at IPs that *looked* random and
decoded as "valid" in isolation. Two wrong theories were chased and
killed before the real one was found (see "Dead ends" below).

## Root cause

The main-image GOTIE→LE relaxation
(`src/backend/i686/linker/reloc.rs`, `relax_gotie_to_le`) rewrites

    movl  slot(%ebx), %reg     ; 8b /r disp32   (6 bytes)
    addl  %gs:0, %reg          ; 65 03 …        (7 bytes)

into the local-exec form

    movl  $tpoff, %reg         ; C7 /0 imm32    (6 bytes, same footprint)
    addl  %gs:0, %reg          ; untouched

The opcode write was

    out_sec.data[off - 2] = 0xc7;
    out_sec.data[off - 1] = 0xC0 | (reg << 3);   // BUG

`C7 /0` (MOV r/m32, imm32) fixes the ModRM **REG field to 000** and takes
the destination in the **r/m field**. Writing `0xC0 | (reg << 3)` puts the
destination in the REG field, selecting the *undefined* `C7 /reg` form.
Only `%eax` (reg=0) survives, because there the REG and r/m fields happen
to coincide (`0xC0`). Every other destination register emits an illegal
opcode and the kernel/QEMU raise #UD on first execution.

Measured proof (real CPU, no VM involved):

    $ cat > c7test.s   # _start: .byte 0xc7,0xd0,1,0,0,0; test ecx; …
    $ gcc -m32 -nostdlib -static c7test.s -o c7test && ./c7test
    Illegal instruction (SIGILL)          # C7 D0 = C7 /2, undefined

The same bytes GCC would emit (`B9` for `mov ecx, imm32`, or `C7 C1` for
the r/m form) run fine. QEMU-user (`qemu-i386`) reproduces the #UD, which
is what finally separated "binary defect" from "host CPU quirk".

## Why it was invisible for so long

* The pre-S01 linker had **no** GOTIE relaxation at all — the pair kept
  its GOT-slot form and worked (unoptimized). The relaxation is new
  session work; the bug shipped with it.
* The first regression gate only exercised `%eax` destinations (compiler
  allocation happened to use it), where the encoding is accidentally
  valid.
* The faulting IP is far from the write site, and the byte sequence at
  the fault decodes "legitimately" when you start disassembly at the
  wrong alignment — so disassembly-based inspection kept producing
  false leads.

## Fix

1. `src/backend/i686/linker/reloc.rs`
   * `0xC0 | (reg << 3)` → `0xC0 | reg`, with an explicit "encoding trap"
     doc comment so the mistake is documented at the site.
   * Removed the dead "3-byte `add %gs:0`" detection branch: the absolute
     `%gs:0` operand requires a disp32 in every encoding, so the 7-byte
     form is the only possible one. The branch could only ever
     false-match a self-add (`add %reg, %reg`) and relax garbage.
2. `src/backend/i686/assembler/elf_writer.rs` — `is_tls_reloc` corrected
   and commented against the real ABI numbers (elf.h, EM_386):
   `14..=19 | 24..=37 | 39..=41`.  (The earlier list missed the
   GD/LDM-sequence parts 24–29 and `TLS_DESC` 41, and its comment
   contained wrong type numbers; 38 = R_386_SIZE32 is NOT TLS.)
3. `src/backend/x86/assembler/elf_writer.rs` — `is_tls_reloc` corrected:
   `16..=23 | 34..=36`. (The earlier list contained unverifiable 44–51
   "CODE_*" numbers; the lccc assembler never emits the binutils 2.41+
   relaxable extensions, and this predicate only ever sees relocations
   produced by the lccc assembler, so the documented standard set is the
   correct one.)
4. Audit of every other `C7` writer in the tree: x86-64 GOTIE relax
   (`emit_exec.rs:2429`, already `0xc0 | reg`), x86-64 TLSDESC→LE
   (hardcoded `c7 c0`, rax-only by construction), `0x81`/`0x01`-family
   relaxations (legitimate `/ext` field), and the i686 compiler-assembler
   `C6/C7` memory-store path (`reg` field = 0). All correct.

## Regression gate

`tests/regression/check_i686_tls_ie_relax.sh` (registered in
`scripts/ci_local.sh` as `i686-tls-ie-relax`) now includes, in addition to
the original structure checks:

* a **deterministic per-register probe** (hand assembly, no compiler
  allocation luck): one TLS variable, six GOTIE pairs driving
  eax/ecx/edx/esi/edi/ebp, exact-value readback (765432; exit-code mod
  256 compare);
* a **static C7 /0 scan** over the executable LOAD segment(s) of the
  linked probe: every `C7` byte must have ModRM REG field == 000, so any
  regression to `C7 /reg` fails the gate even before execution.

Proven to catch the bug: with the fix reverted, the gate dies with exit
132 (SIGILL from the probe itself); with the fix, it prints OK.

## Validation

* `ci_local.sh --fast`: green (see ledger entry for this snapshot).
* TLS runtime probes all match gcc byte-for-byte in output:
  `mini.c` (inline-asm LE + IE, prints `x=0` on both), `tls32.c`
  (`le=6 ld=6 g=10`), `tprobe.s` (raw-syscall probe, exit 80 = 20304 mod
  256 on both), `tpv.c` (C equivalent, exit 80 on both), plus the
  per-register gate probe.
* All pre-existing i686 gates green: `i686-atomics`,
  `i686-code16-relax`, `i686-const-div` (run with `CCC=target/fastbuild/lccc`
  — the gate defaults to `./target/release/lccc`, a pre-existing
  convention), `i686-overalign-interop` (20 combos).

## Dead ends (do not retry)

1. **"Missing/corrupted thunk"** — the `__x86.get_pc_thunk.bx` bytes
   (`8b 1c 24 c3`) are present and correct in every crasher.
2. **`_start` passes a misaligned main / PLT call off-by-one** — both
   were analysis artifacts of an off-by-one in my own next-IP arithmetic
   during manual decode (`call rel32` next-IP is e8+5, not e8+4). The
   PLT stubs, the trampoline, and the displacements are all correct.
3. **ndisasm offset trap** — disassembling a stream at the wrong base
   produces garbage that looks like proof of corruption. Always confirm
   the true function start (entry-relative) before trusting a decode.

## Notes for future agents

* `C7 /0` is the classic "one field is fixed" trap. Same shape as the
  x86-64 REX.R→REX.B transplantation the x86-64 relaxer documents. When
  writing a relaxation that moves a register between ModRM fields, the
  doc comment must state which field is fixed and which carries the
  operand.
* The i686 linker's GD→LE relaxation (`lea sym@tlsgd + call
  ___tls_get_addr` → `mov %gs:0,%eax; add $tpoff,%eax`) only handles an
  `%eax` destination by construction (`dest == 0`); a non-eax GD
  destination is a hard error. That is deliberate (the rewrite has no
  register-agnostic 6-byte+5-byte footprint) — keep it that way.
* `objdump -d` prints nothing for lccc i386 executables: they carry no
  section headers (program headers only). Use `ndisasm`/qemu tracing or
  raw byte scans for post-link inspection.
