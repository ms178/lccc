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

A candidate fix was implemented and measured as
`pic_codegen_effective(pic, pie, fixed_exec_link) = pic || (pie &&
!fixed_exec_link)` (a PIE request is honoured only when the image is really
relocatable), plus `-pie`/`-no-pie` setting the codegen half so the two can no
longer drift apart. It delivers the §2 numbers on default builds and leaves
`-S`/`-c` output byte-identical (absolute forms stay gated on
`fixed_exec_link`, exactly like absolute switch tables). It is **not landed**
because of the blocker below.

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

1. Fix the `-O1` non-PIC miscompile (§4): make the non-PIC symbol-SIB arms go
   through the same index-form normalisation as the PIC arm, and add the
   `gzip_longest_match`-at-`-O1` case to `scripts/check_benchmark_outputs.sh`
   as a `-O1 -fno-pie` variant so the path is covered by CI from then on.
2. Re-land `pic_codegen_effective` (the candidate patch is described in §3).
   Expected: -306 instructions, -179 `leaq`, -54 `push`/`pop` on the linked
   benchmark binaries; `-S`/`-c` output unchanged.
3. Register-pressure work beyond that is small: pool size (+%rax/%rcx) buys
   -4 %, save-set compaction is already worth -30 pushes, and 88 % of the
   remaining pushes live in call-free functions where they are the ABI cost of
   keeping more than six values live — not a defect.
