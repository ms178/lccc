# Codegen mode, `leaq` materialisation and `push`/`pop` pressure — findings and the blocker

Date: 2026-10-31. Scope: the two quantified census gaps (GCC 16.2 comparison on
the 86-program `tests/benchmark/programs` corpus, `-O2`).

This note records what the census actually measures, what it does *not* say,
and the one defect that blocks the largest available fix. Nothing here changes
the compiler; the follow-up work is described in "Next steps".

## 1. The census, decomposed

Per-function counting over `-O2 -S` output, 86 programs:

| category | lccc | gcc-16 | delta |
|---|---:|---:|---:|
| `leaq` (all) | 1,069 | 414 | **+655** |
| `push`/`pop` (both directions) | 795 | 271 | **+524** |
| `movz`/`movs` (extensions) | 702 | 385 | +317 |
| `jcc` | 1,579 | 1,310 | +269 |
| `cmp`/`test` | 1,282 | 1,052 | +230 |
| `cmov` | 164 | 69 | +95 |
| `mov` | 3,358 | 3,622 | -264 |
| `call` | 281 | 584 | -303 |
| SIMD/FP | 948 | 1,985 | -1,037 |
| **total** | **15,050** | **14,324** | **+726 (+5.1 %)** |

The two named axes are the two largest positive deltas. Both were traced to a
single codegen decision (§2); the remaining deltas are structural (lccc keeps
32-bit integer values in canonical zero-extended form, hence `movz`/`movs`;
gcc splits hot/cold functions, hence more `call`s; gcc vectorizes more).

Counting method note: `-S` output only. A binary-level count (`objdump -d` over
all 86 linked programs) gives 22,613 instructions and must be used when judging
a link-mode change, because `.s`/`.o` output and linked executables can
legitimately differ (see §3).

## 2. `leaq` and `push`/`pop` are the same decision

x86 has no RIP-relative SIB form. A symbol+index global access therefore has
exactly two legal shapes, and **both arms already exist** in the backend
(`src/backend/x86/codegen/memory.rs`, `indexed_sym_operand` and
`emit_leaq_sym_index_impl`):

```text
# position-independent codegen (pic_mode)
leaq  sym(%rip), %rcx            # rebuild the symbol in the reserved scratch
movl  (%rcx,%r10,4), %eax

# position-fixed codegen
movl  sym(,%r10,4), %eax
```

The same decision also decides whether a symbol address is *hoisted into a
register and held across a loop* (`leaq sym(%rip), %rbx` + `(%rbx,%idx,scale)`)
or folded into the operand. Freeing those bases removes homes from the save
set, and the prologue `pushq` / epilogue `popq` disappear with them.

Measured, with the whole corpus compiled in the non-PIC mode (`-fno-pie`):

| metric | PIC (default) | non-PIC | delta |
|---|---:|---:|---:|
| instructions (`-S`) | 13,918 | 13,661 | -257 (-1.8 %) |
| `leaq` (`-S`) | 1,069 | 897 | -172 (-16.1 %) |
| `push`+`pop` (`-S`) | 795 | 741 | -54 (-6.8 %) |
| `pushq` (`-S`) | 397 | 370 | -27 (-6.8 %) |

and in linked binaries (`objdump -d`, all 86 programs):
instructions 22,613 → 22,307 (-306), `leaq` 1,076 → 897 (-179, -16.6 %),
`push`+`pop` 1,415 → 1,361 (-54, -3.8 %).

**Conclusion: the `leaq` excess is not a peephole, allocator or rematerialisation
defect.** It is the price of position-independent codegen. What remains after
the change is what a position-fixed image needs, plus the LEA-as-ALU idioms
(`leaq (%r9,%r9,4)` for `x*5`, `leal 1013904223(%r8)` for an add) that GCC emits
too — lccc's non-symbol `leaq` count is dominated by those.

Corollary for the other direction of the search: a peephole that folds
`leaq sym(%rip), %base` into a single-use memory operand has **nothing to do**:
all 432 symbol-address `leaq` sites are bases with several uses (loop bases),
and exactly **one** corpus site is single-use-and-dead. Likewise adding %rax
and/or %rcx to the main pool (measured by temporarily admitting them) removes
only 16 of 397 pushes (-4 %) — pool size is not what drives the save count.

## 3. The driver inconsistency (the actual defect)

`Driver::pie` defaulted to `true` (the Debian x86-64 `-fPIE` codegen default),
while the link half only received `-pie` when the user typed it.
`linker_pie_effective()` — the predicate that already gates absolute switch
tables (`CodegenOptions::fixed_exec_link`) — answers `false` for a default
argument list.

So every default build was linked **position-fixed** while its codegen still
paid the full PIC price from §2. The two halves of the driver disagreed, and
the losing half was the one that costs instructions, `leaq`s and registers.

The fix is `pic_codegen_effective(pic, pie, fixed_exec_link, pie_explicit)`
(a PIE request is honoured only when the image is really relocatable, or when
the user spelled `-fPIE`), plus a `-pie` link forcing the codegen half so the
two can no longer drift apart. It delivers the §2 numbers on default builds and
leaves `-S`/`-c` output byte-identical (absolute forms stay gated on
`fixed_exec_link`, exactly like absolute switch tables). **Landed** — see §6 for
the final shape; the blocker below is what had to be fixed first.

## 4. Blocker: a latent miscompile in non-PIC codegen at `-O1`

The candidate fix routes *default* builds through the non-PIC path, which CI
had never exercised: every gate builds with default flags, i.e. PIC.

`tests/benchmark/programs/gzip_longest_match.c` (gzip 1.14 `longest_match`) is
miscompiled by the non-PIC path **at `-O1` only**:

```text
$ lccc -O1 -fno-pie gzip_longest_match.c -o glm && ./glm
gzip_longest_match checksum=5145201118685017974 reference=15308096299102938189 MISMATCH
$ lccc -O1 -fpie gzip_longest_match.c -o glm && ./glm     # PIC: correct
gzip_longest_match checksum=15308096299102938189 reference=15308096299102938189 OK
```

Opt-level matrix for the non-PIC build: `-O0` OK, `-O1` **MISMATCH**, `-O2` OK,
`-O3` OK, `-Os` OK. The failure reproduces with `CCC_DISABLE_PASSES=all`,
`licm`, `gvn`, `sroa` and `CCC_NO_PEEPHOLE=1`, so it is not an IR pass and not
a peephole: it is base codegen. The relocations emitted for the absolute forms
are correct (`R_X86_64_32S` against `.bss` + addend, resolved to
`0x405010`/`0x415120`/`0x435120`/`0x475120`, matching the symbol table).

The only difference between the correct (`-O1 -fpie`) and the wrong
(`-O1 -fno-pie`) assembly is the address forms, and one hunk points at the
suspect:

```text
    PIC                       non-PIC
    leaq window(%rip), %r9    movl %r15d, %ebp
    addq %r15, %r9            movzbl window(%rbp, %rsi), %edi
    movzbl (%r9, %rsi), %edi
```

The non-PIC path materialises the address contribution into a *base* register
with a 32-bit move (`movl %r15d, %ebp`, zero-extending) where the PIC path adds
it with a 64-bit `addq`. If `%r15` holds a value whose canonical form is not
zero-extended, the two address computations differ. Prime suspect:
`emit_leaq_sym_index_impl` (`memory.rs`) does **not** call
`ensure_sib_index_form` (unlike `indexed_sym_operand`), so the non-PIC arm can
consume an index/base register without the widening/`movslq` normalisation the
PIC arm gets for free from `ensure_sib_index_form`'s type switch.

Impact today: `-fno-pie` (already supported) inherits it; `-no-pie` does not
(its codegen half is ignored), which is why CI is green.

## 5. Next steps

1. ~~Fix the `-O1` non-PIC miscompile (§4)~~ — done:
   `tests/regression/check_reg_off_sym_fold.sh` contract 5 runs the gzip
   longest-match kernel at `-O1 -fno-pie` against gcc and the kill switch, so
   the level and mode that used to answer "foldable" for a live base are
   covered by CI from then on.
2. ~~Re-land `pic_codegen_effective` (§3)~~ — done, see §6. Expected: -306
   instructions, -179 `leaq`, -54 `push`/`pop` on the linked benchmark
   binaries; `-S`/`-c` output unchanged.
3. Register-pressure work beyond that is small: pool size (+%rax/%rcx) buys
   -4 %, save-set compaction is already worth -30 pushes, and 88 % of the
   remaining pushes live in call-free functions where they are the ABI cost of
   keeping more than six values live — not a defect.

## 6. Landed: the codegen mode follows the link mode

`Driver` carries two facts, not one:

* `pie` — what the `-f` flags say. It still drives `__PIE__`/`__PIC__`,
  because those macros describe the option, not the image.
* `pie_explicit` — whether `-fPIE`/`-fpie` was *spelled*. It exists for exactly
  one case: an explicit request for position-independent CODE must not be
  overridden by a position-fixed LINK.

`compile_translation_unit` then computes, once, from the same argument list the
linker receives:

```rust
let link_pie         = linker_pie_effective(&self.build_linker_args());
let fixed_exec_link  = matches!(self.mode, CompileMode::Full)
                       && !self.shared_lib && !self.relocatable && !link_pie;
let pic_codegen      = self.pic || self.shared_lib;
let pie_codegen      = (self.pie && (!fixed_exec_link || self.pie_explicit)) || link_pie;
```

`fixed_exec_link` already existed for the absolute switch-table form; symbol
addressing now rides the same predicate, so there is one answer to "can this
image be relocated" rather than two that could disagree — which is what the
original defect was.

What each invocation gets:

| invocation | image | codegen | why |
|---|---|---|---|
| `lccc -O2 a.c -o a` (default) | ET_EXEC | position-fixed | the image cannot move; PIC buys nothing |
| `lccc -O2 -c a.c` / `-S` | object | position-independent | a later link may be `-pie` |
| `lccc -O2 -pie a.c -o a` | ET_DYN | position-independent | `R_X86_64_32S` is unresolvable in ET_DYN |
| `lccc -O2 -fpie a.c -o a` | ET_EXEC | position-independent | the user asked for PIC code |
| `lccc -O2 -fPIC` / `-shared` | — | full PIC | interposition-safe |

`tests/regression/check_default_codegen_mode.sh` pins all six arms, in the
linked images (the only place a FULL invocation's codegen mode is observable,
since `-S`/`-c` are deliberately mode-independent): the absolute symbol+index
SIB form `0x404040(,%rax,4)` exists by default and never in a `-pie`/`-fpie`/
`-c` image, `-fPIC` still reaches a preemptible global through
`@GOTPCREL`, every arm prints gcc's own output, and `-S` is byte-identical with
and without `-no-pie`.

The security posture is unchanged and worth stating: lccc's default link was
already non-PIE (`linker_pie_effective` defaults to false), so this change does
not disable ASLR for code — it stops paying for position independence the
resulting binary never had. A hardened build is one flag away and now honoured
end to end: `-pie` (or `-fPIE`) produces position-independent codegen *and* an
ET_DYN image.

## 7. What this unlocks, and what it does not

The mode change is the whole of the measured address-generation gap, so the
next levers are elsewhere. Ranked by measured size, with the reason each is a
project rather than a commit:

1. **ChaCha20 quarter-round SIMD** (`engineering/STATE.md` backlog 1). LCCC 592
   instructions against ICX 74, Clang 176, GCC 232 on the same kernel: a 3-8x
   code-size gap driven by scalar quarter-rounds under spill pressure. This is
   vectorisation work in the loop/SLP passes, not a codegen tweak.
2. **Leaf functions and the callee-saved bank.** After the mode change the
   corpus still emits 741 `push`/`pop` against GCC's 271. The MachInst
   allocator already prefers caller-saved registers; the mature linear-scan
   path allocates from the callee-saved bank, so a leaf function that keeps
   seven values live pays `push`/`pop` for registers it need not preserve at
   all. Changing the pool changes every function the allocator touches, so it
   needs its own differential run against the corpus, not a ride-along.
3. **Loop rotation default-enable (PF-17).** The pass exists and removes the
   extra entry jump in hot loops; it stays off until the remaining 15 edge-case
   loop shapes are hardened.

**Narrow integer ALU width — analysed, and why it is not in this change.**
After the divide work, narrow integer arithmetic still runs at 64-bit width.
It looks like a mechanical `addq` -> `addl` sweep and it is not: `addl` and
`addq` are only equivalent on canonical operands when the *result* also stays
canonical. `0xFFFFFFFF + 1` is `0` in 32 bits (CF=1, ZF=1) and `0x100000000` in
64 (CF=0, ZF=0), so the width decides both the value's upper half and every
flag a consumer reads; and the consumers — `setcc`, `cmov`, `jcc`, the
`check_narrow_cmp_flag_law.sh` contracts — are chosen at the same width today.
Making the ALU 32-bit is therefore a whole-lattice change: the ALU, the flag
law and every flag consumer have to move together, or nothing moves. It is
provably free of *lost* information (the canonical form is 32 bits) but it is
not free of *work*, and a change of that size has to be measured against the
corpus with a compiler in the loop, not reasoned about.

