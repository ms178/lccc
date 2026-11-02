#!/usr/bin/env python3
"""Assembler directive contract on every target, pinned against GNU as.

Every row is assembled by lccc for the targets it names and its outcome --
error, or the symbols plus the `.text`/`.data`/`.bss` sections -- compared
with the GAS verdict pinned in PINS: GNU as 2.47 for x86-64, i686 (`--32`)
and AArch64, Debian's 2.44 for RISC-V (`-march=rv64gc -mno-relax`).  The pins
are GAS's own output: regenerate them with `--oracle ... --print-pins` and
review the diff instead of editing them by hand.

Why this gate exists: the x86 parser silently ignored every directive it did
not know.  `.equ N,5` left N an undefined symbol with a relocation, `.local
x; .comm x,4,8` produced a GLOBAL common that the linker merges with other
translation units (on all three assemblers), `.lcomm y,12` produced no
symbol, `.base64` dropped its bytes, `.weak a, b` created one symbol named
"a," (x86 and AArch64), a malformed `.comm` size became 0, and a label
defined twice silently moved.  Unknown directives are now an error on x86 as
they already were on AArch64/RISC-V; the ones accepted without effect are an
explicit, justified allowlist in the x86 `parse_directive`.

DIVERGENCES are deliberate, each with its reason; every other row must match.
Error texts are not compared, only that both sides refuse.  Mapping symbols
(`$x`, `$d`, ...) are left out: they are not what these rows test.

Usage:
  check_asm_directive_contract.py --lccc target/fastbuild/lccc
      (lccc-i686, lccc-arm and lccc-riscv are taken from the same directory)
  check_asm_directive_contract.py --lccc ... --oracle x86_64=AS aarch64=AS riscv64=AS [--print-pins]
"""

import argparse
import os
import subprocess
import sys
import tempfile

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from elf_probe import SHT_NOBITS, Elf  # noqa: E402

SHT_PROGBITS = 1
# Width of the implicit-addend field of the i686 SHT_REL types the rows use
# (R_386_16/PC16 2, R_386_8/PC8 1, everything else 4).
REL_WIDTH = {20: 2, 21: 2, 22: 1, 23: 1}
EM_RISCV = 243
RISCV_ADD = {33: 8, 34: 16, 35: 32, 36: 64}  # R_RISCV_ADDn; SUBn = ADDn + 4

# name -> (source, targets).  "x86" = x86-64 only, "x86s" = x86-64 and i686,
# "all" = x86-64, i686, AArch64 and RISC-V.
ROWS = {
    # `.macro` binding (the shared ARM/RISC-V expander and x86's own), each
    # rule measured on GAS 2.47: a parenthesized default survives whole;
    # keywords bind by name with or without spaces around `=`; a positional
    # after a keyword, an unknown keyword, a surplus positional and a missing
    # `:req` are errors; an omitted argument is EMPTY, never "0".
    "mac_paren_default": (".macro O a b=(((1 << 4) | ( 3*32+21)))\n.long \\a, \\b\n.endm\n\t.data\nO 7\n", "all"),
    "mac_keyword": (".macro O a b=3\n.long \\a, \\b\n.endm\n\t.data\nO a=7, b=(2 | (1 << 8))\nO b = 5 a=1\n", "all"),
    "mac_kw_then_pos": (".macro O a b\n.byte \\a, \\b\n.endm\n\t.data\nO b=7, 9\n", "all"),
    "mac_unknown_kw": (".macro O a\n.byte \\a\n.endm\n\t.data\nO z=7\n", "all"),
    "mac_surplus": (".macro O a\n.byte \\a\n.endm\n\t.data\nO 1, 2\n", "all"),
    "mac_req_missing": (".macro O a:req\n.byte 1\n.endm\n\t.data\nO\n", "all"),
    "mac_missing_empty": (".macro E a b\n.byte \\a\\b\n.endm\n\t.data\nE 7\n", "all"),
    # `.org` targets: GAS resolves a label only when it is already defined; a
    # forward target is a fatal "Infinite loop encountered whilst attempting to
    # compute the addresses of symbols" (2.47), so both assemblers refuse it. A
    # backward target is legal, including a local numeric label, and it must
    # survive jump relaxation: the pad shrinks when a jump before it grows.
    "org_backward_local": ("\t.text\n1:\n\tnop\n\t.org 1b + 4, 0x11\n\tret\n",
                           "x86s"),
    "org_backward_named": ("\t.text\nstart:\n\tnop\n\t.org start + 4, 0x33\n\tret\n",
                           "x86s"),
    "org_relax_local": ("\t.text\n1:\n\tnop\n\tjmp 2f\n\t.org 1b + 12, 0x77\n"
                        "\t.space 200\n2:\n\tret\n", "x86s"),
    "org_dot_relative": ("\t.text\n\tnop\n\t.org . + 4, 0x90\n\tret\n",
                         "x86s"),
    "org_forward_local": ("\t.text\n\t.org 1f, 0xcc\n\tnop\n1:\n\tret\n",
                          "x86s"),
    "org_forward_named": ("\t.text\n\t.org tail, 0x55\n\tnop\ntail:\n\tret\n",
                          "x86s"),
    # `\@` counts invocations in pre-order (M=0, its I=1, the next M=2).
    "mac_counter": (".macro I\nL\\@i: .byte 0\n.endm\n.macro M\nL\\@m: .byte 1\nI\nL\\@z: .byte 2\n.endm\n\t.data\nM\nM\n", "all"),
    # Any number of labels may precede an invocation.
    "mac_label_prefix": (".macro E v\n.byte \\v\n.endm\n\t.data\na: b: E 7\n1: E 8\n", "all"),
    # A vararg is the rest as written (`2 3,4`), empty when nothing is left.
    "mac_vararg": (".macro V a, r:vararg\n.byte \\r\n.endm\n\t.data\nV 1, 2, 3 ,4\nV 5\n.byte 9\n", "all"),
    # A `;` in a body separates invocations; macros defined by an expanded
    # body are live afterwards; `.purgem` lets a name be redefined.
    "mac_semicolon_body": (".macro E v\n.byte \\v\n.endm\n.macro O n\nE \\n ; E \\n+1\n.endm\n\t.data\nO 4\n", "all"),
    "mac_def_in_body": (".macro D\n.macro Inner v\n.byte \\v\n.endm\n.endm\n\t.data\nD\nInner 6\n", "all"),
    "mac_purgem": (".macro E\n.byte 1\n.endm\n.purgem E\n.macro E\n.byte 2\n.endm\n\t.data\nE\n", "all"),
    # `x0, =sym` is two positional arguments (the AArch64 literal operand);
    # quotes around a keyword value are stripped.
    "mac_eq_after_comma": (".macro P a, b\n.ascii \"\\b\"\n.endm\n\t.data\nP x0, =sym\n", "all"),
    "mac_quoted_kw": (".macro Q s\n.ascii \"\\s\"\n.endm\n\t.data\nQ s=\"a b\"\n", "all"),
    # Argument splitting follows GAS's scrubber, which is target-specific:
    # whitespace before `-`, `*`, `(` separates on x86 only; character
    # literals become decimal values and swallow the following whitespace.
    "mac_split_minus": ('.macro M a b c\n.ascii "[\\a|\\b|\\c]"\n.endm\n\t.data\nM 1 -1\n', "all"),
    "mac_split_spaced": ('.macro M a b c\n.ascii "[\\a|\\b|\\c]"\n.endm\n\t.data\nM 1 - 1\n', "all"),
    "mac_split_plus": ('.macro M a b c\n.ascii "[\\a|\\b|\\c]"\n.endm\n\t.data\nM 1 + 2\n', "all"),
    "mac_split_paren": ('.macro M a b c\n.ascii "[\\a|\\b|\\c]"\n.endm\n\t.data\nM 1 (2)\n', "all"),
    "mac_split_close": ('.macro M a b c\n.ascii "[\\a|\\b|\\c]"\n.endm\n\t.data\nM (1) (2)\n', "all"),
    "mac_split_words": ('.macro M a b c\n.ascii "[\\a|\\b|\\c]"\n.endm\n\t.data\nM a b "x"\n', "all"),
    "mac_char_lit": ('.macro M a b c\n.ascii "[\\a|\\b|\\c]"\n.endm\n\t.data\nM \' \', \'a\' \'b\'\n', "all"),
    "mac_char_esc": ('.macro M a b c\n.ascii "[\\a|\\b|\\c]"\n.endm\n\t.data\nM \'\\n \'\\v \'\\\\\n', "all"),
    # A section name used as a symbol is offset 0 of that section (GAS), in
    # whichever section the expression is placed.
    "secsym_place": ("\t.section .rodata\n\t.byte 0\nf:\t.byte 0\n\t.section __ex_table,\"a\"\n\t.long 0\n\t.long f - __ex_table\n", "all"),
    "secsym_other": ("\t.section .rodata\n\t.byte 0\nf:\t.byte 0\n\t.data\n\t.long f - .rodata\n", "all"),
    "secsym_dot": ("\t.data\n\t.long 0\n\t.long . - .data\n", "all"),
    "secsym_undef": ("\t.data\n\t.long xtrn - .text\n", "all"),
    "sec_plus": ("\t.data\n\t.long 0\n\t.long .data + 2\n", "all"),
    # Data values: differences in one section cancel whatever the binding;
    # a lone global/weak label is relocated against by name.
    "pc_dot": ("\t.section .rodata\n\t.byte 0\nf:\t.byte 0\n\t.data\n\t.long 0\n\t.long f - .\n", "all"),
    "pc_dot_same": ("\t.data\nf:\t.byte 0\n\t.long f - .\n", "all"),
    "ext_minus_dot": ("\t.data\n\t.long 0\n\t.long ext - .\n", "all"),
    "glob_ref": ("\t.data\n\t.byte 0\n\t.globl g\ng:\t.byte 0\n\t.long g+2\n", "all"),
    "weak_ref": ("\t.data\n\t.byte 0\n\t.weak w\nw:\t.byte 0\n\t.long w\n", "all"),
    "glob_diff": ("\t.data\n\t.globl g\ng:\t.byte 0\nh:\t.byte 0\n\t.long h - g\n", "all"),
    "weak_diff": ("\t.data\n\t.weak w\nw:\t.byte 0\nh:\t.byte 0\n\t.long h - w\n", "all"),
    "shift_fwd": ("\t.data\n\t.long (e - s) >> 1\ns:\t.long 0\ne:\n", "all"),
    "neg_only": ("\t.data\n\t.long 5 - .\n", "all"),
    "two_ext": ("\t.data\n\t.long a + b\n", "all"),
    # Narrow data relocations (x86 R_*_8/16 and PC8/16, AArch64 ABS16/PREL16;
    # RISC-V has only ADD/SUB pairs below 4 bytes).
    "byte_ext": ("\t.data\n\t.byte ext\n", "all"),
    "short_ext": ("\t.data\n\t.2byte ext\n", "all"),
    "short_pc": ("\t.data\n\t.2byte ext - .\n", "all"),
    "byte_pc": ("\t.data\n\t.byte ext - .\n", "all"),
    "short_cross": ("\t.section .rodata\n\t.byte 0\nt:\t.byte 0\n\t.data\nd:\t.2byte t - d\n", "all"),
    # .set / .equ: the value is taken where the directive stands, and a
    # section-relative value stays section-relative (never an ABS offset).
    "set_dotlabel_alias": ("\t.data\n\t.byte 0\n.Lfoo:\t.byte 0\n\t.set bar, .Lfoo\n\t.long bar\n", "all"),
    "set_label_plus": ("\t.data\n\t.byte 0\nfoo:\t.byte 0\n\t.set baz, foo+4\n\t.long baz\n", "all"),
    "set_dot_plus": ("\t.data\n\t.byte 0\n\t.set here, .+2\n\t.long here\n", "all"),
    "set_forward": ("\t.set fwd, later+1\n\t.data\n\t.long fwd\nlater:\t.byte 0\n", "all"),
    "set_globl_alias": ("\t.data\nfoo:\t.byte 1\n\t.globl ali\n\t.set ali, foo\n", "all"),
    "set_glob": ("\t.data\n\t.byte 0\nfoo:\t.byte 0\n\t.globl s\n\t.set s, foo+1\n\t.long s\n", "all"),
    "set_undef_alias": ("\t.set u2, ext\n\t.data\n\t.long u2\n", "all"),
    "set_ext_plus": ("\t.set u3, ext+4\n\t.data\n\t.long u3\n", "all"),
    "set_same_diff": ("\t.data\na:\t.byte 0\nb:\t.byte 0\n\t.set d, b-a\n\t.long d\n", "all"),
    "set_secdiff": ("\t.section .rodata\n\t.byte 0\nt:\t.byte 0\n\t.data\n\t.set sd, t - .rodata\n\t.long sd\n", "all"),
    "set_chain": ("\t.data\n\t.byte 0\nfoo:\t.byte 0\n\t.set a1, foo\n\t.set a2, a1+1\n\t.long a2\n", "all"),
    "set_redefine": ("\t.set v, 1\n\t.data\n\t.long v\n\t.set v, 2\n\t.long v\n", "all"),
    "set_redef_use": ("\t.data\nv0:\t.byte 0\n\t.set v, v0\n\t.long v\n\t.set v, 7\n\t.long v\n", "all"),
    "set_cycle": ("\t.set c1, c2+1\n\t.set c2, c1+1\n\t.data\n\t.long c1\n", "all"),
    # .local / .comm / .lcomm -- GAS's sticky local flag and .bss subsection 1.
    "local_then_comm": ("\t.local x\n\t.comm x,4,8\n", "all"),
    "comm_then_local": ("\t.comm x,4,8\n\t.local x\n", "all"),
    "globl_local_comm": ("\t.globl x\n\t.local x\n\t.comm x,4,4\n", "all"),
    "local_globl_comm": ("\t.local x\n\t.globl x\n\t.comm x,4,4\n", "all"),
    "local_weak_comm": ("\t.local x\n\t.weak x\n\t.comm x,4,4\n", "all"),
    "local_comm_noalign": ("\t.local x\n\t.comm x,12\n", "all"),
    "local_comm_align0": ("\t.local x\n\t.comm x,4,0\n", "all"),
    "local_comm_align3": ("\t.local x\n\t.comm x,4,3\n", "all"),
    "local_comm_twice": ("\t.local x\n\t.comm x,4,4\n\t.comm x,8,8\n", "all"),
    "local_list": ("\t.local e, f\n\t.comm e,4,4\n\t.comm f,8,8\n", "all"),
    "bss_mix": ("\t.bss\n\t.byte 0\n\t.local x\n\t.comm x,4,8\n\t.byte 0\nz:\n", "all"),
    "local_defined": ("\t.globl x\n\t.local x\n\t.data\nx:\t.long 1\n", "all"),
    "defined_local_globl": ("\t.local x\n\t.globl x\n\t.data\nx:\t.long 1\n", "all"),
    "local_undefined": ("\t.local u\n\t.text\n\tcall u\n", "x86"),
    "lcomm": ("\t.lcomm y,12\n", "all"),
    "lcomm_sizes": ("\t.lcomm a,1\n\t.lcomm b,3\n\t.lcomm c,12\n\t.lcomm d,100\n"
                    "\t.lcomm e,0\n\t.lcomm f,2\n", "all"),
    "lcomm_align_operand": ("\t.lcomm y,12,16\n", "all"),
    "lcomm_globl": ("\t.globl y\n\t.lcomm y,4\n", "all"),
    "lcomm_then_label": ("\t.lcomm x,4\n\t.data\nx:\t.long 1\n", "all"),
    "lcomm_after_bss": ("\t.lcomm y,4\n\t.bss\n\t.zero 3\nw:\n", "all"),
    "comm_default_align": ("\t.comm a,1\n\t.comm b,3\n\t.comm c,12\n\t.comm d,100\n"
                           "\t.comm e,6\n", "all"),
    "comm_align0": ("\t.comm x,4,0\n", "all"),
    "comm_align3": ("\t.comm x,4,3\n", "x86"),
    "comm_expr": ("\t.comm x,2*4,1<<3\n", "all"),
    "comm_bad_align": ("\t.comm q,4,zz\n", "all"),
    "comm_twice": ("\t.comm x,4,4\n\t.comm x,8,8\n", "all"),
    "comm_twice_default_align": ("\t.comm x,4,8\n\t.comm x,100\n", "all"),
    "comm_twice_raise": ("\t.comm x,4\n\t.comm x,4,32\n", "all"),
    "comm_type_then_label": ("\t.type x,@function\n\t.comm x,4,4\n\t.data\nx:\n", "all"),
    "set_constant_expr": ("\t.set N, 3*4+1\n\t.set M, N+1\n\t.data\n\t.byte N, M\n", "all"),
    "comm_negative": ("\t.comm x,-4,4\n", "all"),
    "comm_after_label": ("\t.data\nx:\t.long 1\n\t.comm x,4,4\n", "all"),
    "comm_then_label": ("\t.comm x,4,4\n\t.data\nx:\t.long 1\n", "all"),
    # Symbol-list directives take comma-separated names.
    "weak_hidden_lists": ("\t.weak a, b\n\t.hidden c, d\n\t.data\na: b: c: d: .byte 0\n", "all"),
    "protected_internal_lists": ("\t.globl p, q, r, s\n\t.protected p, q\n\t.internal r, s\n"
                                 "\t.data\np: q: r: s: .byte 0\n", "all"),
    "label_redefined": ("\t.data\nx:\t.byte 0\nx:\t.byte 0\n", "all"),
    "numeric_label_reuse": ("\t.text\n1:\tjmp 1b\n1:\tjmp 1b\n", "x86"),
    # .equ is .set.
    "equ": ("\t.equ N, 5\n\t.data\n\t.long N\n", "all"),
    "equ_expr": ("\t.equ N, 3*4+1\n\t.data\n\t.byte N\n", "all"),
    "extern": ("\t.extern foo\n\t.text\n\tcall foo\n", "x86"),
    # .dc family.
    "dc_b": ("\t.data\n\t.dc.b 1,2\n", "x86"),
    "dc_w": ("\t.data\n\t.dc.w 0x102\n", "x86"),
    "dc": ("\t.data\n\t.dc 0x102\n", "x86"),
    "dc_l": ("\t.data\n\t.dc.l 7\n", "x86"),
    "dc_a": ("\t.data\n\t.dc.a 0x1020304\n", "x86s"),
    "dc_a_label": ("\t.data\n1:\t.dc.a 1b\n", "x86s"),
    "dc_s": ("\t.data\n\t.dc.s 1.5\n", "x86"),
    "dc_d": ("\t.data\n\t.dc.d 1.5\n", "x86"),
    "dc_q": ("\t.data\n\t.dc.q 1\n", "x86"),
    "dc_x": ("\t.data\n\t.dc.x 1\n", "x86"),
    # .base64 (GAS 2.42+).
    "b64_basic": ('\t.data\n\t.base64 "SGVsbG8="\n', "all"),
    "b64_nopad": ('\t.data\n\t.base64 "SGVsbG8"\n', "all"),
    "b64_two": ('\t.data\n\t.base64 "SGk=", "QQ=="\n', "all"),
    "b64_groups": ('\t.data\n\t.base64 "AAEC", "/w=="\n', "all"),
    "b64_pad2": ('\t.data\n\t.base64 "QQ=="\n', "all"),
    "b64_empty": ('\t.data\n\t.base64 ""\n', "all"),
    "b64_space": ('\t.data\n\t.base64 "SGVs bG8="\n', "all"),
    "b64_badchar": ('\t.data\n\t.base64 "SG!s"\n', "all"),
    "b64_noquote": ('\t.data\n\t.base64 SGVsbG8=\n', "all"),
    "b64_after_pad": ('\t.data\n\t.base64 "QQ==QQ=="\n', "all"),
    "b64_urlsafe": ('\t.data\n\t.base64 "-_8="\n', "all"),
    "b64_len1": ('\t.data\n\t.base64 "Q"\n', "all"),
    "b64_trailing_comma": ('\t.data\n\t.base64 "QQ==",\n', "all"),
    # Accepted without effect, and refused.
    "ident": ('\t.ident "GCC: 16.2"\n\t.data\n\t.byte 1\n', "x86"),
    "att_syntax": ("\t.att_syntax\n\t.att_syntax prefix\n\t.text\n\tret\n", "x86"),
    "att_syntax_noprefix": ("\t.att_syntax noprefix\n\t.text\n\tret\n", "x86"),
    "intel_syntax": ("\t.intel_syntax noprefix\n\t.text\n\tmov eax, 1\n", "x86"),
    "addrsig": ("\t.addrsig\n\t.addrsig_sym f\n\t.text\nf:\tret\n", "x86"),
    "prefalign": ("\t.text\n\t.prefalign 4, .Lend, nop\nf:\tret\n.Lend:\n", "x86"),
    "unknown": ("\t.bogus_directive 1\n", "x86"),
    "nops": ("\t.text\n\t.nops 4\n", "x86"),
    "end": ("\t.data\n\t.byte 1\n\t.end\n\t.byte 2\n", "x86"),
    "exitm_outside_macro": ("\t.exitm\n", "x86"),
    "subsection": ("\t.text\n\t.subsection 1\n\tnop\n\t.subsection 0\n\tret\n", "x86"),
    # An absolute reference to a label of its own section is its link-time
    # address: a relocation, never the section offset.
    "abs_self_data": ("\t.data\n\t.byte 0\nfoo:\t.byte 0\n\t.long foo\n\t.quad foo+2\n", "all"),
    # Data operands are classified in section algebra; anything that is not
    # `sym + k` / `a - b + k` folds after layout and must be absolute.
    "dot_long": ("\t.data\n\t.byte 0\n\t.long .\n", "all"),
    "div_dot": ("\t.data\nt:\t.long 0, 0\n\t.long (t - .) / 4\n", "all"),
    "num_scaled": ("\t.data\n1:\t.long 0\n2:\t.long (2b - 1b) * 4, 2b + 1 - 1b, (3f - 2b) >> 2\n3:\n", "all"),
    "shift_global": ("\t.data\n\t.globl gs, ge\ngs:\t.long 0\n\t.long (ge - gs) >> 1\nge:\n", "all"),
    "expr_set_redef": ("\t.data\ns:\t.long 0\n\t.set k, 2\n\t.long (e - s) >> k\n\t.set k, 1\ne:\n", "all"),
    "expr_cross": ("\t.section .rodata\nr:\t.byte 0\n\t.data\nd:\t.long (r - d) >> 1\n", "all"),
    "hex_not_numlabel": ("\t.data\n1:\t.byte 0\n\t.skip 0x1f - 0x1e\n\t.byte 0x1f\n", "all"),
    "ident_digits": ("\t.data\nx1b:\t.byte 0\n1:\t.byte 0\n\t.byte 1b - x1b\n", "all"),
    "set_numlabel": ("\t.data\n1:\t.byte 0\n2:\t.byte 0\n\t.set d, 2b - 1b\n\t.byte d\n", "all"),
    "dot_second_field": ("\t.data\n\t.long 0, .\n\t.long 0, 1f - .\n1:\n", "all"),
    "diff_trailing_addend": ("\t.data\na:\t.long 0\nb:\t.long b - a - 4, b - a + 4\n", "all"),
    "num_zero_back": ("\t.data\n0:\t.long 0\n\t.short . - 0b, 1f - 0b\n1:\n", "all"),
    "abs_self_text": ("\t.text\n\tnop\nfoo:\tmovl $foo, %eax\n\tpush $foo+1\n\t.long foo\n", "x86s"),
}

# (target, row) -> (lccc's outcome, reason).  "error" = lccc refuses;
# ("as", SRC) = lccc accepts the row with no effect: its output must equal
# its own output for SRC, the row without the ignored lines.
DIVERGENCES = {
    ("x86_64", "dc_x"): ("error", "x87 extended needs decimal-to-64-bit-mantissa rounding, "
                                  "which is not implemented; an f64 round trip would emit wrong "
                                  "low bits for literals like 0.1"),
    ("x86_64", "att_syntax_noprefix"): ("error", "lccc assembles %-prefixed AT&T syntax only"),
    ("x86_64", "intel_syntax"): ("error", "lccc assembles %-prefixed AT&T syntax only"),
    ("x86_64", "nops"): ("error", ".nops has its own NOP-selection rules in GAS (a `.nops 4` "
                                  "after one byte is 90 0f1f00); refused until implemented"),
    ("x86_64", "end"): ("error", ".end (stop reading input) is refused rather than honoured"),
    ("x86_64", "subsection"): ("error", "x86 refuses .subsection (see check_nobits_org_parity.py)"),
    ("x86_64", "exitm_outside_macro"): ("error", "x86 macro expansion does not implement .exitm, "
                                                 "so it is refused everywhere rather than ignored "
                                                 "(inside a macro, ignoring it expands the rest)"),
    ("x86_64", "addrsig"): (("as", "\t.text\nf:\tret\n"),
                            "LLVM extensions GAS does not know; without .llvm_addrsig lld treats "
                            "every symbol as address-significant, the conservative reading"),
    ("x86_64", "prefalign"): (("as", "\t.text\nf:\tret\n.Lend:\n"),
                              "LLVM extension GAS does not know; a hint by definition (the "
                              "required minimum is a separate .p2align)"),
    # `5 - .`: GAS writes a PC-relative relocation against symbol index 0.
    # lccc's relocations always name a symbol, so it refuses (never wrong
    # bytes) instead of inventing an undefined symbol `5`.
    ("x86_64", "neg_only"): ("error", "a symbol-less PC-relative relocation is not supported"),
    ("i686", "neg_only"): ("error", "a symbol-less PC-relative relocation is not supported"),
    ("aarch64", "neg_only"): ("error", "a symbol-less PC-relative relocation is not supported"),
    # GAS riscv refuses because linker relaxation needs the symbol itself in
    # the relocation; lccc never relaxes, so the first use relocates against
    # the value the symbol had there, exactly as GAS does on x86 and AArch64.
    ("riscv64", "set_redef_use"): (
        ".data:9:1:000000000007000000 @1:r1:.data+0 v:LOCAL:NOTYPE:ABS:7:0 "
        "v0:LOCAL:NOTYPE:.data:0:0",
        "lccc does not relax, so a redefined .set needs no symbol in the relocation"),
    # The only RISC-V oracle is GAS 2.44, which reads `=sym` after a comma as
    # a keyword argument with an empty name ("Parameter named `' does not
    # exist"). GAS 2.47 -- the reference on x86-64 and AArch64 -- binds it
    # positionally, and the shared expander follows 2.47 on every target.
    ("riscv64", "mac_eq_after_comma"): (".data:4:1:3d73796d",
                                        "GAS 2.44 quirk fixed in the 2.47 reference: `=sym` after "
                                        "a comma is a positional argument"),
}

# GAS verdicts, from `--oracle ... --print-pins`.
PINS = {
    ('x86_64', 'mac_paren_default'): '.data:8:1:0700000075000000',
    ('i686', 'mac_paren_default'): '.data:8:1:0700000075000000',
    ('aarch64', 'mac_paren_default'): '.data:8:1:0700000075000000',
    ('riscv64', 'mac_paren_default'): '.data:8:1:0700000075000000',
    ('x86_64', 'mac_keyword'): '.data:16:1:07000000020100000100000005000000',
    ('i686', 'mac_keyword'): '.data:16:1:07000000020100000100000005000000',
    ('aarch64', 'mac_keyword'): '.data:16:1:07000000020100000100000005000000',
    ('riscv64', 'mac_keyword'): '.data:16:1:07000000020100000100000005000000',
    ('x86_64', 'mac_kw_then_pos'): 'error',
    ('i686', 'mac_kw_then_pos'): 'error',
    ('aarch64', 'mac_kw_then_pos'): 'error',
    ('riscv64', 'mac_kw_then_pos'): 'error',
    ('x86_64', 'mac_unknown_kw'): 'error',
    ('i686', 'mac_unknown_kw'): 'error',
    ('aarch64', 'mac_unknown_kw'): 'error',
    ('riscv64', 'mac_unknown_kw'): 'error',
    ('x86_64', 'mac_surplus'): 'error',
    ('i686', 'mac_surplus'): 'error',
    ('aarch64', 'mac_surplus'): 'error',
    ('riscv64', 'mac_surplus'): 'error',
    ('x86_64', 'mac_req_missing'): 'error',
    ('i686', 'mac_req_missing'): 'error',
    ('aarch64', 'mac_req_missing'): 'error',
    ('riscv64', 'mac_req_missing'): 'error',
    ('x86_64', 'mac_missing_empty'): '.data:1:1:07',
    ('i686', 'mac_missing_empty'): '.data:1:1:07',
    ('aarch64', 'mac_missing_empty'): '.data:1:1:07',
    ('riscv64', 'mac_missing_empty'): '.data:1:1:07',
    ('x86_64', 'mac_counter'): '.data:6:1:010002010002 L0m:LOCAL:NOTYPE:.data:0:0 L0z:LOCAL:NOTYPE:.data:2:0 L1i:LOCAL:NOTYPE:.data:1:0 L2m:LOCAL:NOTYPE:.data:3:0 L2z:LOCAL:NOTYPE:.data:5:0 L3i:LOCAL:NOTYPE:.data:4:0',
    ('i686', 'mac_counter'): '.data:6:1:010002010002 L0m:LOCAL:NOTYPE:.data:0:0 L0z:LOCAL:NOTYPE:.data:2:0 L1i:LOCAL:NOTYPE:.data:1:0 L2m:LOCAL:NOTYPE:.data:3:0 L2z:LOCAL:NOTYPE:.data:5:0 L3i:LOCAL:NOTYPE:.data:4:0',
    ('aarch64', 'mac_counter'): '.data:6:1:010002010002 L0m:LOCAL:NOTYPE:.data:0:0 L0z:LOCAL:NOTYPE:.data:2:0 L1i:LOCAL:NOTYPE:.data:1:0 L2m:LOCAL:NOTYPE:.data:3:0 L2z:LOCAL:NOTYPE:.data:5:0 L3i:LOCAL:NOTYPE:.data:4:0',
    ('riscv64', 'mac_counter'): '.data:6:1:010002010002 L0m:LOCAL:NOTYPE:.data:0:0 L0z:LOCAL:NOTYPE:.data:2:0 L1i:LOCAL:NOTYPE:.data:1:0 L2m:LOCAL:NOTYPE:.data:3:0 L2z:LOCAL:NOTYPE:.data:5:0 L3i:LOCAL:NOTYPE:.data:4:0',
    ('x86_64', 'mac_label_prefix'): '.data:2:1:0708 a:LOCAL:NOTYPE:.data:0:0 b:LOCAL:NOTYPE:.data:0:0',
    ('i686', 'mac_label_prefix'): '.data:2:1:0708 a:LOCAL:NOTYPE:.data:0:0 b:LOCAL:NOTYPE:.data:0:0',
    ('aarch64', 'mac_label_prefix'): '.data:2:1:0708 a:LOCAL:NOTYPE:.data:0:0 b:LOCAL:NOTYPE:.data:0:0',
    ('riscv64', 'mac_label_prefix'): '.data:2:1:0708 a:LOCAL:NOTYPE:.data:0:0 b:LOCAL:NOTYPE:.data:0:0',
    ('x86_64', 'mac_vararg'): '.data:4:1:02030409',
    ('i686', 'mac_vararg'): '.data:4:1:02030409',
    ('aarch64', 'mac_vararg'): '.data:4:1:02030409',
    ('riscv64', 'mac_vararg'): '.data:4:1:02030409',
    ('x86_64', 'mac_semicolon_body'): '.data:2:1:0405',
    ('i686', 'mac_semicolon_body'): '.data:2:1:0405',
    ('aarch64', 'mac_semicolon_body'): '.data:2:1:0405',
    ('riscv64', 'mac_semicolon_body'): '.data:2:1:0405',
    ('x86_64', 'mac_def_in_body'): '.data:1:1:06',
    ('i686', 'mac_def_in_body'): '.data:1:1:06',
    ('aarch64', 'mac_def_in_body'): '.data:1:1:06',
    ('riscv64', 'mac_def_in_body'): '.data:1:1:06',
    ('x86_64', 'mac_purgem'): '.data:1:1:02',
    ('i686', 'mac_purgem'): '.data:1:1:02',
    ('aarch64', 'mac_purgem'): '.data:1:1:02',
    ('riscv64', 'mac_purgem'): '.data:1:1:02',
    ('x86_64', 'mac_eq_after_comma'): '.data:4:1:3d73796d',
    ('i686', 'mac_eq_after_comma'): '.data:4:1:3d73796d',
    ('aarch64', 'mac_eq_after_comma'): '.data:4:1:3d73796d',
    ('riscv64', 'mac_eq_after_comma'): 'error',
    ('x86_64', 'mac_quoted_kw'): '.data:3:1:612062',
    ('i686', 'mac_quoted_kw'): '.data:3:1:612062',
    ('aarch64', 'mac_quoted_kw'): '.data:3:1:612062',
    ('riscv64', 'mac_quoted_kw'): '.data:3:1:612062',
    ('x86_64', 'mac_split_minus'): '.data:7:1:5b317c2d317c5d',
    ('i686', 'mac_split_minus'): '.data:7:1:5b317c2d317c5d',
    ('aarch64', 'mac_split_minus'): '.data:7:1:5b312d317c7c5d',
    ('riscv64', 'mac_split_minus'): '.data:7:1:5b312d317c7c5d',
    ('x86_64', 'mac_split_spaced'): '.data:7:1:5b317c2d7c315d',
    ('i686', 'mac_split_spaced'): '.data:7:1:5b317c2d7c315d',
    ('aarch64', 'mac_split_spaced'): '.data:7:1:5b312d317c7c5d',
    ('riscv64', 'mac_split_spaced'): '.data:7:1:5b312d317c7c5d',
    ('x86_64', 'mac_split_plus'): '.data:7:1:5b312b327c7c5d',
    ('i686', 'mac_split_plus'): '.data:7:1:5b312b327c7c5d',
    ('aarch64', 'mac_split_plus'): '.data:7:1:5b312b327c7c5d',
    ('riscv64', 'mac_split_plus'): '.data:7:1:5b312b327c7c5d',
    ('x86_64', 'mac_split_paren'): '.data:8:1:5b317c2832297c5d',
    ('i686', 'mac_split_paren'): '.data:8:1:5b317c2832297c5d',
    ('aarch64', 'mac_split_paren'): '.data:8:1:5b312832297c7c5d',
    ('riscv64', 'mac_split_paren'): '.data:8:1:5b312832297c7c5d',
    ('x86_64', 'mac_split_close'): '.data:10:1:5b2831292832297c7c5d',
    ('i686', 'mac_split_close'): '.data:10:1:5b2831292832297c7c5d',
    ('aarch64', 'mac_split_close'): '.data:10:1:5b2831292832297c7c5d',
    ('riscv64', 'mac_split_close'): '.data:10:1:5b2831292832297c7c5d',
    ('x86_64', 'mac_split_words'): '.data:7:1:5b617c627c785d',
    ('i686', 'mac_split_words'): '.data:7:1:5b617c627c785d',
    ('aarch64', 'mac_split_words'): '.data:7:1:5b617c627c785d',
    ('riscv64', 'mac_split_words'): '.data:7:1:5b617c627c785d',
    ('x86_64', 'mac_char_lit'): '.data:10:1:5b33327c393739387c5d',
    ('i686', 'mac_char_lit'): '.data:10:1:5b33327c393739387c5d',
    ('aarch64', 'mac_char_lit'): '.data:10:1:5b33327c393739387c5d',
    ('riscv64', 'mac_char_lit'): '.data:10:1:5b33327c393739387c5d',
    ('x86_64', 'mac_char_esc'): '.data:11:1:5b313031313839327c7c5d',
    ('i686', 'mac_char_esc'): '.data:11:1:5b313031313839327c7c5d',
    ('aarch64', 'mac_char_esc'): '.data:11:1:5b313031313839327c7c5d',
    ('riscv64', 'mac_char_esc'): '.data:11:1:5b313031313839327c7c5d',
    ('x86_64', 'secsym_place'): '.rodata:2:1:0000 __ex_table:8:1:0000000000000000 @4:r2:.rodata+5 f:LOCAL:NOTYPE:.rodata:1:0',
    ('i686', 'secsym_place'): '.rodata:2:1:0000 __ex_table:8:1:0000000000000000 @4:r2:.rodata+5 f:LOCAL:NOTYPE:.rodata:1:0',
    ('aarch64', 'secsym_place'): '.rodata:2:1:0000 __ex_table:8:1:0000000000000000 @4:r261:.rodata+5 f:LOCAL:NOTYPE:.rodata:1:0',
    ('riscv64', 'secsym_place'): '.rodata:2:1:0000 __ex_table:8:1:0000000000000000 @4:pair32:.rodata-__ex_table+1 f:LOCAL:NOTYPE:.rodata:1:0',
    ('x86_64', 'secsym_other'): '.data:4:1:01000000 .rodata:2:1:0000 f:LOCAL:NOTYPE:.rodata:1:0',
    ('i686', 'secsym_other'): '.data:4:1:01000000 .rodata:2:1:0000 f:LOCAL:NOTYPE:.rodata:1:0',
    ('aarch64', 'secsym_other'): '.data:4:1:01000000 .rodata:2:1:0000 f:LOCAL:NOTYPE:.rodata:1:0',
    ('riscv64', 'secsym_other'): '.data:4:1:01000000 .rodata:2:1:0000 f:LOCAL:NOTYPE:.rodata:1:0',
    ('x86_64', 'secsym_dot'): '.data:8:1:0000000004000000',
    ('i686', 'secsym_dot'): '.data:8:1:0000000004000000',
    ('aarch64', 'secsym_dot'): '.data:8:1:0000000004000000',
    ('riscv64', 'secsym_dot'): '.data:8:1:0000000004000000',
    ('x86_64', 'secsym_undef'): 'error',
    ('i686', 'secsym_undef'): 'error',
    ('aarch64', 'secsym_undef'): 'error',
    ('riscv64', 'secsym_undef'): '.data:4:1:00000000 @0:pair32:xtrn-.text+0 xtrn:GLOBAL:NOTYPE:UND:0:0',
    ('x86_64', 'sec_plus'): '.data:8:1:0000000000000000 @4:r10:.data+2',
    ('i686', 'sec_plus'): '.data:8:1:0000000000000000 @4:r1:.data+2',
    ('aarch64', 'sec_plus'): '.data:8:1:0000000000000000 @4:r258:.data+2',
    ('riscv64', 'sec_plus'): '.data:8:1:0000000000000000 @4:r1:.data+2',
    ('x86_64', 'pc_dot'): '.data:8:1:0000000000000000 @4:r2:.rodata+1 .rodata:2:1:0000 f:LOCAL:NOTYPE:.rodata:1:0',
    ('i686', 'pc_dot'): '.data:8:1:0000000000000000 @4:r2:.rodata+1 .rodata:2:1:0000 f:LOCAL:NOTYPE:.rodata:1:0',
    ('aarch64', 'pc_dot'): '.data:8:1:0000000000000000 @4:r261:.rodata+1 .rodata:2:1:0000 f:LOCAL:NOTYPE:.rodata:1:0',
    ('riscv64', 'pc_dot'): '.data:8:1:0000000000000000 @4:pair32:.rodata-.data-3 .rodata:2:1:0000 f:LOCAL:NOTYPE:.rodata:1:0',
    ('x86_64', 'pc_dot_same'): '.data:5:1:00ffffffff f:LOCAL:NOTYPE:.data:0:0',
    ('i686', 'pc_dot_same'): '.data:5:1:00ffffffff f:LOCAL:NOTYPE:.data:0:0',
    ('aarch64', 'pc_dot_same'): '.data:5:1:00ffffffff f:LOCAL:NOTYPE:.data:0:0',
    ('riscv64', 'pc_dot_same'): '.data:5:1:00ffffffff f:LOCAL:NOTYPE:.data:0:0',
    ('x86_64', 'ext_minus_dot'): '.data:8:1:0000000000000000 @4:r2:ext+0 ext:GLOBAL:NOTYPE:UND:0:0',
    ('i686', 'ext_minus_dot'): '.data:8:1:0000000000000000 @4:r2:ext+0 ext:GLOBAL:NOTYPE:UND:0:0',
    ('aarch64', 'ext_minus_dot'): '.data:8:1:0000000000000000 @4:r261:ext+0 ext:GLOBAL:NOTYPE:UND:0:0',
    ('riscv64', 'ext_minus_dot'): '.data:8:1:0000000000000000 @4:pair32:ext-.data-4 ext:GLOBAL:NOTYPE:UND:0:0',
    ('x86_64', 'glob_ref'): '.data:6:1:000000000000 @2:r10:g+2 g:GLOBAL:NOTYPE:.data:1:0',
    ('i686', 'glob_ref'): '.data:6:1:000000000000 @2:r1:g+2 g:GLOBAL:NOTYPE:.data:1:0',
    ('aarch64', 'glob_ref'): '.data:6:1:000000000000 @2:r258:g+2 g:GLOBAL:NOTYPE:.data:1:0',
    ('riscv64', 'glob_ref'): '.data:6:1:000000000000 @2:r1:g+2 g:GLOBAL:NOTYPE:.data:1:0',
    ('x86_64', 'weak_ref'): '.data:6:1:000000000000 @2:r10:w+0 w:WEAK:NOTYPE:.data:1:0',
    ('i686', 'weak_ref'): '.data:6:1:000000000000 @2:r1:w+0 w:WEAK:NOTYPE:.data:1:0',
    ('aarch64', 'weak_ref'): '.data:6:1:000000000000 @2:r258:w+0 w:WEAK:NOTYPE:.data:1:0',
    ('riscv64', 'weak_ref'): '.data:6:1:000000000000 @2:r1:w+0 w:WEAK:NOTYPE:.data:1:0',
    ('x86_64', 'glob_diff'): '.data:6:1:000001000000 g:GLOBAL:NOTYPE:.data:0:0 h:LOCAL:NOTYPE:.data:1:0',
    ('i686', 'glob_diff'): '.data:6:1:000001000000 g:GLOBAL:NOTYPE:.data:0:0 h:LOCAL:NOTYPE:.data:1:0',
    ('aarch64', 'glob_diff'): '.data:6:1:000001000000 g:GLOBAL:NOTYPE:.data:0:0 h:LOCAL:NOTYPE:.data:1:0',
    ('riscv64', 'glob_diff'): '.data:6:1:000001000000 g:GLOBAL:NOTYPE:.data:0:0 h:LOCAL:NOTYPE:.data:1:0',
    ('x86_64', 'weak_diff'): '.data:6:1:000001000000 h:LOCAL:NOTYPE:.data:1:0 w:WEAK:NOTYPE:.data:0:0',
    ('i686', 'weak_diff'): '.data:6:1:000001000000 h:LOCAL:NOTYPE:.data:1:0 w:WEAK:NOTYPE:.data:0:0',
    ('aarch64', 'weak_diff'): '.data:6:1:000001000000 h:LOCAL:NOTYPE:.data:1:0 w:WEAK:NOTYPE:.data:0:0',
    ('riscv64', 'weak_diff'): '.data:6:1:000001000000 h:LOCAL:NOTYPE:.data:1:0 w:WEAK:NOTYPE:.data:0:0',
    ('x86_64', 'shift_fwd'): '.data:8:1:0200000000000000 e:LOCAL:NOTYPE:.data:8:0 s:LOCAL:NOTYPE:.data:4:0',
    ('i686', 'shift_fwd'): '.data:8:1:0200000000000000 e:LOCAL:NOTYPE:.data:8:0 s:LOCAL:NOTYPE:.data:4:0',
    ('aarch64', 'shift_fwd'): '.data:8:1:0200000000000000 e:LOCAL:NOTYPE:.data:8:0 s:LOCAL:NOTYPE:.data:4:0',
    ('riscv64', 'shift_fwd'): '.data:8:1:0200000000000000 e:LOCAL:NOTYPE:.data:8:0 s:LOCAL:NOTYPE:.data:4:0',
    ('x86_64', 'neg_only'): '.data:4:1:00000000 @0:r2:+5',
    ('i686', 'neg_only'): '.data:4:1:00000000 @0:r2:+5',
    ('aarch64', 'neg_only'): '.data:4:1:00000000 @0:r261:+5',
    ('riscv64', 'neg_only'): 'error',
    ('x86_64', 'two_ext'): 'error',
    ('i686', 'two_ext'): 'error',
    ('aarch64', 'two_ext'): 'error',
    ('riscv64', 'two_ext'): 'error',
    ('x86_64', 'byte_ext'): '.data:1:1:00 @0:r14:ext+0 ext:GLOBAL:NOTYPE:UND:0:0',
    ('i686', 'byte_ext'): '.data:1:1:00 @0:r22:ext+0 ext:GLOBAL:NOTYPE:UND:0:0',
    ('aarch64', 'byte_ext'): 'error',
    ('riscv64', 'byte_ext'): 'error',
    ('x86_64', 'short_ext'): '.data:2:1:0000 @0:r12:ext+0 ext:GLOBAL:NOTYPE:UND:0:0',
    ('i686', 'short_ext'): '.data:2:1:0000 @0:r20:ext+0 ext:GLOBAL:NOTYPE:UND:0:0',
    ('aarch64', 'short_ext'): '.data:2:1:0000 @0:r259:ext+0 ext:GLOBAL:NOTYPE:UND:0:0',
    ('riscv64', 'short_ext'): 'error',
    ('x86_64', 'short_pc'): '.data:2:1:0000 @0:r13:ext+0 ext:GLOBAL:NOTYPE:UND:0:0',
    ('i686', 'short_pc'): '.data:2:1:0000 @0:r21:ext+0 ext:GLOBAL:NOTYPE:UND:0:0',
    ('aarch64', 'short_pc'): '.data:2:1:0000 @0:r262:ext+0 ext:GLOBAL:NOTYPE:UND:0:0',
    ('riscv64', 'short_pc'): '.data:2:1:0000 @0:pair16:ext-.data+0 ext:GLOBAL:NOTYPE:UND:0:0',
    ('x86_64', 'byte_pc'): '.data:1:1:00 @0:r15:ext+0 ext:GLOBAL:NOTYPE:UND:0:0',
    ('i686', 'byte_pc'): '.data:1:1:00 @0:r23:ext+0 ext:GLOBAL:NOTYPE:UND:0:0',
    ('aarch64', 'byte_pc'): 'error',
    ('riscv64', 'byte_pc'): '.data:1:1:00 @0:pair8:ext-.data+0 ext:GLOBAL:NOTYPE:UND:0:0',
    ('x86_64', 'short_cross'): '.data:2:1:0000 @0:r13:.rodata+1 .rodata:2:1:0000 d:LOCAL:NOTYPE:.data:0:0 t:LOCAL:NOTYPE:.rodata:1:0',
    ('i686', 'short_cross'): '.data:2:1:0000 @0:r21:.rodata+1 .rodata:2:1:0000 d:LOCAL:NOTYPE:.data:0:0 t:LOCAL:NOTYPE:.rodata:1:0',
    ('aarch64', 'short_cross'): '.data:2:1:0000 @0:r262:.rodata+1 .rodata:2:1:0000 d:LOCAL:NOTYPE:.data:0:0 t:LOCAL:NOTYPE:.rodata:1:0',
    ('riscv64', 'short_cross'): '.data:2:1:0000 @0:pair16:.rodata-.data+1 .rodata:2:1:0000 d:LOCAL:NOTYPE:.data:0:0 t:LOCAL:NOTYPE:.rodata:1:0',
    ('x86_64', 'set_dotlabel_alias'): '.data:6:1:000000000000 @2:r10:.data+1 bar:LOCAL:NOTYPE:.data:1:0',
    ('i686', 'set_dotlabel_alias'): '.data:6:1:000000000000 @2:r1:.data+1 bar:LOCAL:NOTYPE:.data:1:0',
    ('aarch64', 'set_dotlabel_alias'): '.data:6:1:000000000000 @2:r258:.data+1 bar:LOCAL:NOTYPE:.data:1:0',
    ('riscv64', 'set_dotlabel_alias'): '.data:6:1:000000000000 @2:r1:.data+1 bar:LOCAL:NOTYPE:.data:1:0',
    ('x86_64', 'set_label_plus'): '.data:6:1:000000000000 @2:r10:.data+5 baz:LOCAL:NOTYPE:.data:5:0 foo:LOCAL:NOTYPE:.data:1:0',
    ('i686', 'set_label_plus'): '.data:6:1:000000000000 @2:r1:.data+5 baz:LOCAL:NOTYPE:.data:5:0 foo:LOCAL:NOTYPE:.data:1:0',
    ('aarch64', 'set_label_plus'): '.data:6:1:000000000000 @2:r258:.data+5 baz:LOCAL:NOTYPE:.data:5:0 foo:LOCAL:NOTYPE:.data:1:0',
    ('riscv64', 'set_label_plus'): '.data:6:1:000000000000 @2:r1:.data+5 baz:LOCAL:NOTYPE:.data:5:0 foo:LOCAL:NOTYPE:.data:1:0',
    ('x86_64', 'set_dot_plus'): '.data:5:1:0000000000 @1:r10:.data+3 here:LOCAL:NOTYPE:.data:3:0',
    ('i686', 'set_dot_plus'): '.data:5:1:0000000000 @1:r1:.data+3 here:LOCAL:NOTYPE:.data:3:0',
    ('aarch64', 'set_dot_plus'): '.data:5:1:0000000000 @1:r258:.data+3 here:LOCAL:NOTYPE:.data:3:0',
    ('riscv64', 'set_dot_plus'): '.data:5:1:0000000000 @1:r1:.data+3 here:LOCAL:NOTYPE:.data:3:0',
    ('x86_64', 'set_forward'): '.data:5:1:0000000000 @0:r10:.data+5 fwd:LOCAL:NOTYPE:.data:5:0 later:LOCAL:NOTYPE:.data:4:0',
    ('i686', 'set_forward'): '.data:5:1:0000000000 @0:r1:.data+5 fwd:LOCAL:NOTYPE:.data:5:0 later:LOCAL:NOTYPE:.data:4:0',
    ('aarch64', 'set_forward'): '.data:5:1:0000000000 @0:r258:.data+5 fwd:LOCAL:NOTYPE:.data:5:0 later:LOCAL:NOTYPE:.data:4:0',
    ('riscv64', 'set_forward'): '.data:5:1:0000000000 @0:r1:.data+5 fwd:LOCAL:NOTYPE:.data:5:0 later:LOCAL:NOTYPE:.data:4:0',
    ('x86_64', 'set_globl_alias'): '.data:1:1:01 ali:GLOBAL:NOTYPE:.data:0:0 foo:LOCAL:NOTYPE:.data:0:0',
    ('i686', 'set_globl_alias'): '.data:1:1:01 ali:GLOBAL:NOTYPE:.data:0:0 foo:LOCAL:NOTYPE:.data:0:0',
    ('aarch64', 'set_globl_alias'): '.data:1:1:01 ali:GLOBAL:NOTYPE:.data:0:0 foo:LOCAL:NOTYPE:.data:0:0',
    ('riscv64', 'set_globl_alias'): '.data:1:1:01 ali:GLOBAL:NOTYPE:.data:0:0 foo:LOCAL:NOTYPE:.data:0:0',
    ('x86_64', 'set_glob'): '.data:6:1:000000000000 @2:r10:s+0 foo:LOCAL:NOTYPE:.data:1:0 s:GLOBAL:NOTYPE:.data:2:0',
    ('i686', 'set_glob'): '.data:6:1:000000000000 @2:r1:s+0 foo:LOCAL:NOTYPE:.data:1:0 s:GLOBAL:NOTYPE:.data:2:0',
    ('aarch64', 'set_glob'): '.data:6:1:000000000000 @2:r258:s+0 foo:LOCAL:NOTYPE:.data:1:0 s:GLOBAL:NOTYPE:.data:2:0',
    ('riscv64', 'set_glob'): '.data:6:1:000000000000 @2:r1:s+0 foo:LOCAL:NOTYPE:.data:1:0 s:GLOBAL:NOTYPE:.data:2:0',
    ('x86_64', 'set_undef_alias'): '.data:4:1:00000000 @0:r10:ext+0 ext:GLOBAL:NOTYPE:UND:0:0',
    ('i686', 'set_undef_alias'): '.data:4:1:00000000 @0:r1:ext+0 ext:GLOBAL:NOTYPE:UND:0:0',
    ('aarch64', 'set_undef_alias'): '.data:4:1:00000000 @0:r258:ext+0 ext:GLOBAL:NOTYPE:UND:0:0',
    ('riscv64', 'set_undef_alias'): '.data:4:1:00000000 @0:r1:ext+0 ext:GLOBAL:NOTYPE:UND:0:0',
    ('x86_64', 'set_ext_plus'): '.data:4:1:00000000 @0:r10:ext+4 ext:GLOBAL:NOTYPE:UND:0:0',
    ('i686', 'set_ext_plus'): '.data:4:1:00000000 @0:r1:ext+4 ext:GLOBAL:NOTYPE:UND:0:0',
    ('aarch64', 'set_ext_plus'): '.data:4:1:00000000 @0:r258:ext+4 ext:GLOBAL:NOTYPE:UND:0:0',
    ('riscv64', 'set_ext_plus'): '.data:4:1:00000000 @0:r1:ext+4 ext:GLOBAL:NOTYPE:UND:0:0',
    ('x86_64', 'set_same_diff'): '.data:6:1:000001000000 a:LOCAL:NOTYPE:.data:0:0 b:LOCAL:NOTYPE:.data:1:0 d:LOCAL:NOTYPE:ABS:1:0',
    ('i686', 'set_same_diff'): '.data:6:1:000001000000 a:LOCAL:NOTYPE:.data:0:0 b:LOCAL:NOTYPE:.data:1:0 d:LOCAL:NOTYPE:ABS:1:0',
    ('aarch64', 'set_same_diff'): '.data:6:1:000001000000 a:LOCAL:NOTYPE:.data:0:0 b:LOCAL:NOTYPE:.data:1:0 d:LOCAL:NOTYPE:ABS:1:0',
    ('riscv64', 'set_same_diff'): '.data:6:1:000001000000 a:LOCAL:NOTYPE:.data:0:0 b:LOCAL:NOTYPE:.data:1:0 d:LOCAL:NOTYPE:ABS:1:0',
    ('x86_64', 'set_secdiff'): '.data:4:1:01000000 .rodata:2:1:0000 sd:LOCAL:NOTYPE:ABS:1:0 t:LOCAL:NOTYPE:.rodata:1:0',
    ('i686', 'set_secdiff'): '.data:4:1:01000000 .rodata:2:1:0000 sd:LOCAL:NOTYPE:ABS:1:0 t:LOCAL:NOTYPE:.rodata:1:0',
    ('aarch64', 'set_secdiff'): '.data:4:1:01000000 .rodata:2:1:0000 sd:LOCAL:NOTYPE:ABS:1:0 t:LOCAL:NOTYPE:.rodata:1:0',
    ('riscv64', 'set_secdiff'): '.data:4:1:01000000 .rodata:2:1:0000 sd:LOCAL:NOTYPE:ABS:1:0 t:LOCAL:NOTYPE:.rodata:1:0',
    ('x86_64', 'set_chain'): '.data:6:1:000000000000 @2:r10:.data+2 a1:LOCAL:NOTYPE:.data:1:0 a2:LOCAL:NOTYPE:.data:2:0 foo:LOCAL:NOTYPE:.data:1:0',
    ('i686', 'set_chain'): '.data:6:1:000000000000 @2:r1:.data+2 a1:LOCAL:NOTYPE:.data:1:0 a2:LOCAL:NOTYPE:.data:2:0 foo:LOCAL:NOTYPE:.data:1:0',
    ('aarch64', 'set_chain'): '.data:6:1:000000000000 @2:r258:.data+2 a1:LOCAL:NOTYPE:.data:1:0 a2:LOCAL:NOTYPE:.data:2:0 foo:LOCAL:NOTYPE:.data:1:0',
    ('riscv64', 'set_chain'): '.data:6:1:000000000000 @2:r1:.data+2 a1:LOCAL:NOTYPE:.data:1:0 a2:LOCAL:NOTYPE:.data:2:0 foo:LOCAL:NOTYPE:.data:1:0',
    ('x86_64', 'set_redefine'): '.data:8:1:0100000002000000 v:LOCAL:NOTYPE:ABS:2:0',
    ('i686', 'set_redefine'): '.data:8:1:0100000002000000 v:LOCAL:NOTYPE:ABS:2:0',
    ('aarch64', 'set_redefine'): '.data:8:1:0100000002000000 v:LOCAL:NOTYPE:ABS:2:0',
    ('riscv64', 'set_redefine'): '.data:8:1:0100000002000000 v:LOCAL:NOTYPE:ABS:2:0',
    ('x86_64', 'set_redef_use'): '.data:9:1:000000000007000000 @1:r10:.data+0 v:LOCAL:NOTYPE:ABS:7:0 v0:LOCAL:NOTYPE:.data:0:0',
    ('i686', 'set_redef_use'): '.data:9:1:000000000007000000 @1:r1:.data+0 v:LOCAL:NOTYPE:ABS:7:0 v0:LOCAL:NOTYPE:.data:0:0',
    ('aarch64', 'set_redef_use'): '.data:9:1:000000000007000000 @1:r258:.data+0 v:LOCAL:NOTYPE:ABS:7:0 v0:LOCAL:NOTYPE:.data:0:0',
    ('riscv64', 'set_redef_use'): 'error',
    ('x86_64', 'set_cycle'): 'error',
    ('i686', 'set_cycle'): 'error',
    ('aarch64', 'set_cycle'): 'error',
    ('riscv64', 'set_cycle'): 'error',
    ('x86_64', 'local_then_comm'): '.bss:4:8:nobits x:LOCAL:OBJECT:.bss:0:4',
    ('i686', 'local_then_comm'): '.bss:4:8:nobits x:LOCAL:OBJECT:.bss:0:4',
    ('aarch64', 'local_then_comm'): '.bss:4:8:nobits x:LOCAL:OBJECT:.bss:0:4',
    ('riscv64', 'local_then_comm'): '.bss:4:8:nobits x:LOCAL:OBJECT:.bss:0:4',
    ('x86_64', 'comm_then_local'): 'x:GLOBAL:OBJECT:COM:8:4',
    ('i686', 'comm_then_local'): 'x:GLOBAL:OBJECT:COM:8:4',
    ('aarch64', 'comm_then_local'): 'x:GLOBAL:OBJECT:COM:8:4',
    ('riscv64', 'comm_then_local'): 'x:GLOBAL:OBJECT:COM:8:4',
    ('x86_64', 'globl_local_comm'): '.bss:4:4:nobits x:LOCAL:OBJECT:.bss:0:4',
    ('i686', 'globl_local_comm'): '.bss:4:4:nobits x:LOCAL:OBJECT:.bss:0:4',
    ('aarch64', 'globl_local_comm'): '.bss:4:4:nobits x:LOCAL:OBJECT:.bss:0:4',
    ('riscv64', 'globl_local_comm'): '.bss:4:4:nobits x:LOCAL:OBJECT:.bss:0:4',
    ('x86_64', 'local_globl_comm'): '.bss:4:4:nobits x:LOCAL:OBJECT:.bss:0:4',
    ('i686', 'local_globl_comm'): '.bss:4:4:nobits x:LOCAL:OBJECT:.bss:0:4',
    ('aarch64', 'local_globl_comm'): '.bss:4:4:nobits x:LOCAL:OBJECT:.bss:0:4',
    ('riscv64', 'local_globl_comm'): '.bss:4:4:nobits x:LOCAL:OBJECT:.bss:0:4',
    ('x86_64', 'local_weak_comm'): '.bss:4:4:nobits x:WEAK:OBJECT:.bss:0:4',
    ('i686', 'local_weak_comm'): '.bss:4:4:nobits x:WEAK:OBJECT:.bss:0:4',
    ('aarch64', 'local_weak_comm'): '.bss:4:4:nobits x:WEAK:OBJECT:.bss:0:4',
    ('riscv64', 'local_weak_comm'): '.bss:4:4:nobits x:WEAK:OBJECT:.bss:0:4',
    ('x86_64', 'local_comm_noalign'): '.bss:12:1:nobits x:LOCAL:OBJECT:.bss:0:12',
    ('i686', 'local_comm_noalign'): '.bss:12:1:nobits x:LOCAL:OBJECT:.bss:0:12',
    ('aarch64', 'local_comm_noalign'): '.bss:12:1:nobits x:LOCAL:OBJECT:.bss:0:12',
    ('riscv64', 'local_comm_noalign'): '.bss:12:1:nobits x:LOCAL:OBJECT:.bss:0:12',
    ('x86_64', 'local_comm_align0'): '.bss:4:1:nobits x:LOCAL:OBJECT:.bss:0:4',
    ('i686', 'local_comm_align0'): '.bss:4:1:nobits x:LOCAL:OBJECT:.bss:0:4',
    ('aarch64', 'local_comm_align0'): '.bss:4:1:nobits x:LOCAL:OBJECT:.bss:0:4',
    ('riscv64', 'local_comm_align0'): '.bss:4:1:nobits x:LOCAL:OBJECT:.bss:0:4',
    ('x86_64', 'local_comm_align3'): 'error',
    ('i686', 'local_comm_align3'): 'error',
    ('aarch64', 'local_comm_align3'): 'error',
    ('riscv64', 'local_comm_align3'): 'error',
    ('x86_64', 'local_comm_twice'): 'error',
    ('i686', 'local_comm_twice'): 'error',
    ('aarch64', 'local_comm_twice'): 'error',
    ('riscv64', 'local_comm_twice'): 'error',
    ('x86_64', 'local_list'): '.bss:16:8:nobits e:LOCAL:OBJECT:.bss:0:4 f:LOCAL:OBJECT:.bss:8:8',
    ('i686', 'local_list'): '.bss:16:8:nobits e:LOCAL:OBJECT:.bss:0:4 f:LOCAL:OBJECT:.bss:8:8',
    ('aarch64', 'local_list'): '.bss:16:8:nobits e:LOCAL:OBJECT:.bss:0:4 f:LOCAL:OBJECT:.bss:8:8',
    ('riscv64', 'local_list'): '.bss:16:8:nobits e:LOCAL:OBJECT:.bss:0:4 f:LOCAL:OBJECT:.bss:8:8',
    ('x86_64', 'bss_mix'): '.bss:12:8:nobits x:LOCAL:OBJECT:.bss:8:4 z:LOCAL:NOTYPE:.bss:2:0',
    ('i686', 'bss_mix'): '.bss:12:8:nobits x:LOCAL:OBJECT:.bss:8:4 z:LOCAL:NOTYPE:.bss:2:0',
    ('aarch64', 'bss_mix'): '.bss:12:8:nobits x:LOCAL:OBJECT:.bss:8:4 z:LOCAL:NOTYPE:.bss:2:0',
    ('riscv64', 'bss_mix'): '.bss:12:8:nobits x:LOCAL:OBJECT:.bss:8:4 z:LOCAL:NOTYPE:.bss:2:0',
    ('x86_64', 'local_defined'): '.data:4:1:01000000 x:LOCAL:NOTYPE:.data:0:0',
    ('i686', 'local_defined'): '.data:4:1:01000000 x:LOCAL:NOTYPE:.data:0:0',
    ('aarch64', 'local_defined'): '.data:4:1:01000000 x:LOCAL:NOTYPE:.data:0:0',
    ('riscv64', 'local_defined'): '.data:4:1:01000000 x:LOCAL:NOTYPE:.data:0:0',
    ('x86_64', 'defined_local_globl'): '.data:4:1:01000000 x:GLOBAL:NOTYPE:.data:0:0',
    ('i686', 'defined_local_globl'): '.data:4:1:01000000 x:GLOBAL:NOTYPE:.data:0:0',
    ('aarch64', 'defined_local_globl'): '.data:4:1:01000000 x:GLOBAL:NOTYPE:.data:0:0',
    ('riscv64', 'defined_local_globl'): '.data:4:1:01000000 x:GLOBAL:NOTYPE:.data:0:0',
    ('x86_64', 'local_undefined'): '.text:5:1:e800000000 @1:r4:u-4 u:GLOBAL:NOTYPE:UND:0:0',
    ('x86_64', 'lcomm'): '.bss:12:8:nobits y:LOCAL:OBJECT:.bss:0:12',
    ('i686', 'lcomm'): '.bss:12:8:nobits y:LOCAL:OBJECT:.bss:0:12',
    ('aarch64', 'lcomm'): '.bss:12:8:nobits y:LOCAL:OBJECT:.bss:0:12',
    ('riscv64', 'lcomm'): '.bss:12:8:nobits y:LOCAL:OBJECT:.bss:0:12',
    ('x86_64', 'lcomm_sizes'): '.bss:126:8:nobits a:LOCAL:OBJECT:.bss:0:1 b:LOCAL:OBJECT:.bss:2:3 c:LOCAL:OBJECT:.bss:8:12 d:LOCAL:OBJECT:.bss:24:100 e:LOCAL:OBJECT:.bss:124:0 f:LOCAL:OBJECT:.bss:124:2',
    ('i686', 'lcomm_sizes'): '.bss:126:8:nobits a:LOCAL:OBJECT:.bss:0:1 b:LOCAL:OBJECT:.bss:2:3 c:LOCAL:OBJECT:.bss:8:12 d:LOCAL:OBJECT:.bss:24:100 e:LOCAL:OBJECT:.bss:124:0 f:LOCAL:OBJECT:.bss:124:2',
    ('aarch64', 'lcomm_sizes'): '.bss:126:8:nobits a:LOCAL:OBJECT:.bss:0:1 b:LOCAL:OBJECT:.bss:2:3 c:LOCAL:OBJECT:.bss:8:12 d:LOCAL:OBJECT:.bss:24:100 e:LOCAL:OBJECT:.bss:124:0 f:LOCAL:OBJECT:.bss:124:2',
    ('riscv64', 'lcomm_sizes'): '.bss:126:8:nobits a:LOCAL:OBJECT:.bss:0:1 b:LOCAL:OBJECT:.bss:2:3 c:LOCAL:OBJECT:.bss:8:12 d:LOCAL:OBJECT:.bss:24:100 e:LOCAL:OBJECT:.bss:124:0 f:LOCAL:OBJECT:.bss:124:2',
    ('x86_64', 'lcomm_align_operand'): 'error',
    ('i686', 'lcomm_align_operand'): 'error',
    ('aarch64', 'lcomm_align_operand'): 'error',
    ('riscv64', 'lcomm_align_operand'): 'error',
    ('x86_64', 'lcomm_globl'): '.bss:4:4:nobits y:GLOBAL:OBJECT:.bss:0:4',
    ('i686', 'lcomm_globl'): '.bss:4:4:nobits y:GLOBAL:OBJECT:.bss:0:4',
    ('aarch64', 'lcomm_globl'): '.bss:4:4:nobits y:GLOBAL:OBJECT:.bss:0:4',
    ('riscv64', 'lcomm_globl'): '.bss:4:4:nobits y:GLOBAL:OBJECT:.bss:0:4',
    ('x86_64', 'lcomm_then_label'): 'error',
    ('i686', 'lcomm_then_label'): 'error',
    ('aarch64', 'lcomm_then_label'): 'error',
    ('riscv64', 'lcomm_then_label'): 'error',
    ('x86_64', 'lcomm_after_bss'): '.bss:8:4:nobits w:LOCAL:NOTYPE:.bss:3:0 y:LOCAL:OBJECT:.bss:4:4',
    ('i686', 'lcomm_after_bss'): '.bss:8:4:nobits w:LOCAL:NOTYPE:.bss:3:0 y:LOCAL:OBJECT:.bss:4:4',
    ('aarch64', 'lcomm_after_bss'): '.bss:8:4:nobits w:LOCAL:NOTYPE:.bss:3:0 y:LOCAL:OBJECT:.bss:4:4',
    ('riscv64', 'lcomm_after_bss'): '.bss:8:4:nobits w:LOCAL:NOTYPE:.bss:3:0 y:LOCAL:OBJECT:.bss:4:4',
    ('x86_64', 'comm_default_align'): 'a:GLOBAL:OBJECT:COM:1:1 b:GLOBAL:OBJECT:COM:4:3 c:GLOBAL:OBJECT:COM:16:12 d:GLOBAL:OBJECT:COM:16:100 e:GLOBAL:OBJECT:COM:8:6',
    ('i686', 'comm_default_align'): 'a:GLOBAL:OBJECT:COM:1:1 b:GLOBAL:OBJECT:COM:4:3 c:GLOBAL:OBJECT:COM:16:12 d:GLOBAL:OBJECT:COM:16:100 e:GLOBAL:OBJECT:COM:8:6',
    ('aarch64', 'comm_default_align'): 'a:GLOBAL:OBJECT:COM:1:1 b:GLOBAL:OBJECT:COM:4:3 c:GLOBAL:OBJECT:COM:16:12 d:GLOBAL:OBJECT:COM:16:100 e:GLOBAL:OBJECT:COM:8:6',
    ('riscv64', 'comm_default_align'): 'a:GLOBAL:OBJECT:COM:1:1 b:GLOBAL:OBJECT:COM:4:3 c:GLOBAL:OBJECT:COM:16:12 d:GLOBAL:OBJECT:COM:16:100 e:GLOBAL:OBJECT:COM:8:6',
    ('x86_64', 'comm_align0'): 'x:GLOBAL:OBJECT:COM:4:4',
    ('i686', 'comm_align0'): 'x:GLOBAL:OBJECT:COM:4:4',
    ('aarch64', 'comm_align0'): 'x:GLOBAL:OBJECT:COM:4:4',
    ('riscv64', 'comm_align0'): 'x:GLOBAL:OBJECT:COM:4:4',
    ('x86_64', 'comm_align3'): 'x:GLOBAL:OBJECT:COM:3:4',
    ('x86_64', 'comm_expr'): 'x:GLOBAL:OBJECT:COM:8:8',
    ('i686', 'comm_expr'): 'x:GLOBAL:OBJECT:COM:8:8',
    ('aarch64', 'comm_expr'): 'x:GLOBAL:OBJECT:COM:8:8',
    ('riscv64', 'comm_expr'): 'x:GLOBAL:OBJECT:COM:8:8',
    ('x86_64', 'comm_bad_align'): 'error',
    ('i686', 'comm_bad_align'): 'error',
    ('aarch64', 'comm_bad_align'): 'error',
    ('riscv64', 'comm_bad_align'): 'error',
    ('x86_64', 'comm_twice'): 'x:GLOBAL:OBJECT:COM:8:4',
    ('i686', 'comm_twice'): 'x:GLOBAL:OBJECT:COM:8:4',
    ('aarch64', 'comm_twice'): 'x:GLOBAL:OBJECT:COM:8:4',
    ('riscv64', 'comm_twice'): 'x:GLOBAL:OBJECT:COM:8:4',
    ('x86_64', 'comm_twice_default_align'): 'x:GLOBAL:OBJECT:COM:4:4',
    ('i686', 'comm_twice_default_align'): 'x:GLOBAL:OBJECT:COM:4:4',
    ('aarch64', 'comm_twice_default_align'): 'x:GLOBAL:OBJECT:COM:4:4',
    ('riscv64', 'comm_twice_default_align'): 'x:GLOBAL:OBJECT:COM:4:4',
    ('x86_64', 'comm_twice_raise'): 'x:GLOBAL:OBJECT:COM:32:4',
    ('i686', 'comm_twice_raise'): 'x:GLOBAL:OBJECT:COM:32:4',
    ('aarch64', 'comm_twice_raise'): 'x:GLOBAL:OBJECT:COM:32:4',
    ('riscv64', 'comm_twice_raise'): 'x:GLOBAL:OBJECT:COM:32:4',
    ('x86_64', 'comm_type_then_label'): 'x:GLOBAL:FUNC:.data:0:0',
    ('i686', 'comm_type_then_label'): 'x:GLOBAL:FUNC:.data:0:0',
    ('aarch64', 'comm_type_then_label'): 'x:GLOBAL:FUNC:.data:0:0',
    ('riscv64', 'comm_type_then_label'): 'x:GLOBAL:FUNC:.data:0:0',
    ('x86_64', 'set_constant_expr'): '.data:2:1:0d0e M:LOCAL:NOTYPE:ABS:14:0 N:LOCAL:NOTYPE:ABS:13:0',
    ('i686', 'set_constant_expr'): '.data:2:1:0d0e M:LOCAL:NOTYPE:ABS:14:0 N:LOCAL:NOTYPE:ABS:13:0',
    ('aarch64', 'set_constant_expr'): '.data:2:1:0d0e M:LOCAL:NOTYPE:ABS:14:0 N:LOCAL:NOTYPE:ABS:13:0',
    ('riscv64', 'set_constant_expr'): '.data:2:1:0d0e M:LOCAL:NOTYPE:ABS:14:0 N:LOCAL:NOTYPE:ABS:13:0',
    ('x86_64', 'comm_negative'): '',
    ('i686', 'comm_negative'): '',
    ('aarch64', 'comm_negative'): '',
    ('riscv64', 'comm_negative'): '',
    ('x86_64', 'comm_after_label'): 'error',
    ('i686', 'comm_after_label'): 'error',
    ('aarch64', 'comm_after_label'): 'error',
    ('riscv64', 'comm_after_label'): 'error',
    ('x86_64', 'comm_then_label'): '.data:4:1:01000000 x:GLOBAL:OBJECT:.data:0:0',
    ('i686', 'comm_then_label'): '.data:4:1:01000000 x:GLOBAL:OBJECT:.data:0:0',
    ('aarch64', 'comm_then_label'): '.data:4:1:01000000 x:GLOBAL:OBJECT:.data:0:0',
    ('riscv64', 'comm_then_label'): '.data:4:1:01000000 x:GLOBAL:OBJECT:.data:0:0',
    ('x86_64', 'weak_hidden_lists'): '.data:1:1:00 a:WEAK:NOTYPE:.data:0:0 b:WEAK:NOTYPE:.data:0:0 c:LOCAL:NOTYPE:.data:0:0:HIDDEN d:LOCAL:NOTYPE:.data:0:0:HIDDEN',
    ('i686', 'weak_hidden_lists'): '.data:1:1:00 a:WEAK:NOTYPE:.data:0:0 b:WEAK:NOTYPE:.data:0:0 c:LOCAL:NOTYPE:.data:0:0:HIDDEN d:LOCAL:NOTYPE:.data:0:0:HIDDEN',
    ('aarch64', 'weak_hidden_lists'): '.data:1:1:00 a:WEAK:NOTYPE:.data:0:0 b:WEAK:NOTYPE:.data:0:0 c:LOCAL:NOTYPE:.data:0:0:HIDDEN d:LOCAL:NOTYPE:.data:0:0:HIDDEN',
    ('riscv64', 'weak_hidden_lists'): '.data:1:1:00 a:WEAK:NOTYPE:.data:0:0 b:WEAK:NOTYPE:.data:0:0 c:LOCAL:NOTYPE:.data:0:0:HIDDEN d:LOCAL:NOTYPE:.data:0:0:HIDDEN',
    ('x86_64', 'protected_internal_lists'): '.data:1:1:00 p:GLOBAL:NOTYPE:.data:0:0:PROTECTED q:GLOBAL:NOTYPE:.data:0:0:PROTECTED r:GLOBAL:NOTYPE:.data:0:0:INTERNAL s:GLOBAL:NOTYPE:.data:0:0:INTERNAL',
    ('i686', 'protected_internal_lists'): '.data:1:1:00 p:GLOBAL:NOTYPE:.data:0:0:PROTECTED q:GLOBAL:NOTYPE:.data:0:0:PROTECTED r:GLOBAL:NOTYPE:.data:0:0:INTERNAL s:GLOBAL:NOTYPE:.data:0:0:INTERNAL',
    ('aarch64', 'protected_internal_lists'): '.data:1:1:00 p:GLOBAL:NOTYPE:.data:0:0:PROTECTED q:GLOBAL:NOTYPE:.data:0:0:PROTECTED r:GLOBAL:NOTYPE:.data:0:0:INTERNAL s:GLOBAL:NOTYPE:.data:0:0:INTERNAL',
    ('riscv64', 'protected_internal_lists'): '.data:1:1:00 p:GLOBAL:NOTYPE:.data:0:0:PROTECTED q:GLOBAL:NOTYPE:.data:0:0:PROTECTED r:GLOBAL:NOTYPE:.data:0:0:INTERNAL s:GLOBAL:NOTYPE:.data:0:0:INTERNAL',
    ('x86_64', 'label_redefined'): 'error',
    ('i686', 'label_redefined'): 'error',
    ('aarch64', 'label_redefined'): 'error',
    ('riscv64', 'label_redefined'): 'error',
    ('x86_64', 'numeric_label_reuse'): '.text:4:1:ebfeebfe',
    ('x86_64', 'equ'): '.data:4:1:05000000 N:LOCAL:NOTYPE:ABS:5:0',
    ('i686', 'equ'): '.data:4:1:05000000 N:LOCAL:NOTYPE:ABS:5:0',
    ('aarch64', 'equ'): '.data:4:1:05000000 N:LOCAL:NOTYPE:ABS:5:0',
    ('riscv64', 'equ'): '.data:4:1:05000000 N:LOCAL:NOTYPE:ABS:5:0',
    ('x86_64', 'equ_expr'): '.data:1:1:0d N:LOCAL:NOTYPE:ABS:13:0',
    ('i686', 'equ_expr'): '.data:1:1:0d N:LOCAL:NOTYPE:ABS:13:0',
    ('aarch64', 'equ_expr'): '.data:1:1:0d N:LOCAL:NOTYPE:ABS:13:0',
    ('riscv64', 'equ_expr'): '.data:1:1:0d N:LOCAL:NOTYPE:ABS:13:0',
    ('x86_64', 'extern'): '.text:5:1:e800000000 @1:r4:foo-4 foo:GLOBAL:NOTYPE:UND:0:0',
    ('x86_64', 'dc_b'): '.data:2:1:0102',
    ('x86_64', 'dc_w'): '.data:2:1:0201',
    ('x86_64', 'dc'): '.data:2:1:0201',
    ('x86_64', 'dc_l'): '.data:4:1:07000000',
    ('x86_64', 'dc_a'): '.data:8:1:0403020100000000',
    ('i686', 'dc_a'): '.data:4:1:04030201',
    ('x86_64', 'dc_a_label'): '.data:8:1:0000000000000000 @0:r1:.data+0',
    ('i686', 'dc_a_label'): '.data:4:1:00000000 @0:r1:.data+0',
    ('x86_64', 'dc_s'): '.data:4:1:0000c03f',
    ('x86_64', 'dc_d'): '.data:8:1:000000000000f83f',
    ('x86_64', 'dc_q'): 'error',
    ('x86_64', 'dc_x'): '.data:10:1:0000000000000080ff3f',
    ('x86_64', 'b64_basic'): '.data:5:1:48656c6c6f',
    ('i686', 'b64_basic'): '.data:5:1:48656c6c6f',
    ('aarch64', 'b64_basic'): '.data:5:1:48656c6c6f',
    ('riscv64', 'b64_basic'): '.data:5:1:48656c6c6f',
    ('x86_64', 'b64_nopad'): '.data:5:1:48656c6c6f',
    ('i686', 'b64_nopad'): '.data:5:1:48656c6c6f',
    ('aarch64', 'b64_nopad'): '.data:5:1:48656c6c6f',
    ('riscv64', 'b64_nopad'): '.data:5:1:48656c6c6f',
    ('x86_64', 'b64_two'): '.data:3:1:486941',
    ('i686', 'b64_two'): '.data:3:1:486941',
    ('aarch64', 'b64_two'): '.data:3:1:486941',
    ('riscv64', 'b64_two'): '.data:3:1:486941',
    ('x86_64', 'b64_groups'): '.data:4:1:000102ff',
    ('i686', 'b64_groups'): '.data:4:1:000102ff',
    ('aarch64', 'b64_groups'): '.data:4:1:000102ff',
    ('riscv64', 'b64_groups'): '.data:4:1:000102ff',
    ('x86_64', 'b64_pad2'): '.data:1:1:41',
    ('i686', 'b64_pad2'): '.data:1:1:41',
    ('aarch64', 'b64_pad2'): '.data:1:1:41',
    ('riscv64', 'b64_pad2'): '.data:1:1:41',
    ('x86_64', 'b64_empty'): 'error',
    ('i686', 'b64_empty'): 'error',
    ('aarch64', 'b64_empty'): 'error',
    ('riscv64', 'b64_empty'): 'error',
    ('x86_64', 'b64_space'): 'error',
    ('i686', 'b64_space'): 'error',
    ('aarch64', 'b64_space'): 'error',
    ('riscv64', 'b64_space'): 'error',
    ('x86_64', 'b64_badchar'): 'error',
    ('i686', 'b64_badchar'): 'error',
    ('aarch64', 'b64_badchar'): 'error',
    ('riscv64', 'b64_badchar'): 'error',
    ('x86_64', 'b64_noquote'): 'error',
    ('i686', 'b64_noquote'): 'error',
    ('aarch64', 'b64_noquote'): 'error',
    ('riscv64', 'b64_noquote'): 'error',
    ('x86_64', 'b64_after_pad'): 'error',
    ('i686', 'b64_after_pad'): 'error',
    ('aarch64', 'b64_after_pad'): 'error',
    ('riscv64', 'b64_after_pad'): 'error',
    ('x86_64', 'b64_urlsafe'): 'error',
    ('i686', 'b64_urlsafe'): 'error',
    ('aarch64', 'b64_urlsafe'): 'error',
    ('riscv64', 'b64_urlsafe'): 'error',
    ('x86_64', 'b64_len1'): 'error',
    ('i686', 'b64_len1'): 'error',
    ('aarch64', 'b64_len1'): 'error',
    ('riscv64', 'b64_len1'): 'error',
    ('x86_64', 'b64_trailing_comma'): 'error',
    ('i686', 'b64_trailing_comma'): 'error',
    ('aarch64', 'b64_trailing_comma'): 'error',
    ('riscv64', 'b64_trailing_comma'): 'error',
    ('x86_64', 'ident'): '.data:1:1:01',
    ('x86_64', 'att_syntax'): '.text:1:1:c3',
    ('x86_64', 'att_syntax_noprefix'): '.text:1:1:c3',
    ('x86_64', 'intel_syntax'): '.text:5:1:b801000000',
    ('x86_64', 'addrsig'): 'error',
    ('x86_64', 'prefalign'): 'error',
    ('x86_64', 'unknown'): 'error',
    ('x86_64', 'nops'): '.text:4:1:0f1f4000',
    ('x86_64', 'end'): '.data:1:1:01',
    ('x86_64', 'exitm_outside_macro'): '',
    ('x86_64', 'subsection'): '.text:2:1:c390',
    ('x86_64', 'abs_self_data'): '.data:14:1:0000000000000000000000000000 @2:r10:.data+1 @6:r1:.data+3 foo:LOCAL:NOTYPE:.data:1:0',
    ('i686', 'abs_self_data'): 'error',
    ('aarch64', 'abs_self_data'): '.data:14:1:0000000000000000000000000000 @2:r258:.data+1 @6:r257:.data+3 foo:LOCAL:NOTYPE:.data:1:0',
    ('riscv64', 'abs_self_data'): '.data:14:1:0000000000000000000000000000 @2:r1:.data+1 @6:r2:.data+3 foo:LOCAL:NOTYPE:.data:1:0',
    ('x86_64', 'dot_long'): '.data:5:1:0000000000 @1:r10:.data+1',
    ('i686', 'dot_long'): '.data:5:1:0000000000 @1:r1:.data+1',
    ('aarch64', 'dot_long'): '.data:5:1:0000000000 @1:r258:.data+1',
    ('riscv64', 'dot_long'): '.data:5:1:0000000000 @1:r1:.data+1',
    ('x86_64', 'div_dot'): '.data:12:1:0000000000000000feffffff t:LOCAL:NOTYPE:.data:0:0',
    ('i686', 'div_dot'): '.data:12:1:0000000000000000feffffff t:LOCAL:NOTYPE:.data:0:0',
    ('aarch64', 'div_dot'): '.data:12:1:0000000000000000feffffff t:LOCAL:NOTYPE:.data:0:0',
    ('riscv64', 'div_dot'): '.data:12:1:0000000000000000feffffff t:LOCAL:NOTYPE:.data:0:0',
    ('x86_64', 'num_scaled'): '.data:16:1:00000000100000000500000003000000',
    ('i686', 'num_scaled'): '.data:16:1:00000000100000000500000003000000',
    ('aarch64', 'num_scaled'): '.data:16:1:00000000100000000500000003000000',
    ('riscv64', 'num_scaled'): '.data:16:1:00000000100000000500000003000000',
    ('x86_64', 'shift_global'): '.data:8:1:0000000004000000 ge:GLOBAL:NOTYPE:.data:8:0 gs:GLOBAL:NOTYPE:.data:0:0',
    ('i686', 'shift_global'): '.data:8:1:0000000004000000 ge:GLOBAL:NOTYPE:.data:8:0 gs:GLOBAL:NOTYPE:.data:0:0',
    ('aarch64', 'shift_global'): '.data:8:1:0000000004000000 ge:GLOBAL:NOTYPE:.data:8:0 gs:GLOBAL:NOTYPE:.data:0:0',
    ('riscv64', 'shift_global'): '.data:8:1:0000000004000000 ge:GLOBAL:NOTYPE:.data:8:0 gs:GLOBAL:NOTYPE:.data:0:0',
    ('x86_64', 'expr_set_redef'): '.data:8:1:0000000002000000 e:LOCAL:NOTYPE:.data:8:0 k:LOCAL:NOTYPE:ABS:1:0 s:LOCAL:NOTYPE:.data:0:0',
    ('i686', 'expr_set_redef'): '.data:8:1:0000000002000000 e:LOCAL:NOTYPE:.data:8:0 k:LOCAL:NOTYPE:ABS:1:0 s:LOCAL:NOTYPE:.data:0:0',
    ('aarch64', 'expr_set_redef'): '.data:8:1:0000000002000000 e:LOCAL:NOTYPE:.data:8:0 k:LOCAL:NOTYPE:ABS:1:0 s:LOCAL:NOTYPE:.data:0:0',
    ('riscv64', 'expr_set_redef'): '.data:8:1:0000000002000000 e:LOCAL:NOTYPE:.data:8:0 k:LOCAL:NOTYPE:ABS:1:0 s:LOCAL:NOTYPE:.data:0:0',
    ('x86_64', 'expr_cross'): 'error',
    ('i686', 'expr_cross'): 'error',
    ('aarch64', 'expr_cross'): 'error',
    ('riscv64', 'expr_cross'): 'error',
    ('x86_64', 'hex_not_numlabel'): '.data:3:1:00001f',
    ('i686', 'hex_not_numlabel'): '.data:3:1:00001f',
    ('aarch64', 'hex_not_numlabel'): '.data:3:1:00001f',
    ('riscv64', 'hex_not_numlabel'): '.data:3:1:00001f',
    ('x86_64', 'ident_digits'): '.data:3:1:000001 x1b:LOCAL:NOTYPE:.data:0:0',
    ('i686', 'ident_digits'): '.data:3:1:000001 x1b:LOCAL:NOTYPE:.data:0:0',
    ('aarch64', 'ident_digits'): '.data:3:1:000001 x1b:LOCAL:NOTYPE:.data:0:0',
    ('riscv64', 'ident_digits'): '.data:3:1:000001 x1b:LOCAL:NOTYPE:.data:0:0',
    ('x86_64', 'set_numlabel'): '.data:3:1:000001 d:LOCAL:NOTYPE:ABS:1:0',
    ('i686', 'set_numlabel'): '.data:3:1:000001 d:LOCAL:NOTYPE:ABS:1:0',
    ('aarch64', 'set_numlabel'): '.data:3:1:000001 d:LOCAL:NOTYPE:ABS:1:0',
    ('riscv64', 'set_numlabel'): '.data:3:1:000001 d:LOCAL:NOTYPE:ABS:1:0',
    ('x86_64', 'dot_second_field'): '.data:16:1:00000000000000000000000004000000 @4:r10:.data+4',
    ('i686', 'dot_second_field'): '.data:16:1:00000000000000000000000004000000 @4:r1:.data+4',
    ('aarch64', 'dot_second_field'): '.data:16:1:00000000000000000000000004000000 @4:r258:.data+4',
    ('riscv64', 'dot_second_field'): '.data:16:1:00000000000000000000000004000000 @4:r1:.data+4',
    ('x86_64', 'diff_trailing_addend'): '.data:12:1:000000000000000008000000 a:LOCAL:NOTYPE:.data:0:0 b:LOCAL:NOTYPE:.data:4:0',
    ('i686', 'diff_trailing_addend'): '.data:12:1:000000000000000008000000 a:LOCAL:NOTYPE:.data:0:0 b:LOCAL:NOTYPE:.data:4:0',
    ('aarch64', 'diff_trailing_addend'): '.data:12:1:000000000000000008000000 a:LOCAL:NOTYPE:.data:0:0 b:LOCAL:NOTYPE:.data:4:0',
    ('riscv64', 'diff_trailing_addend'): '.data:12:1:000000000000000008000000 a:LOCAL:NOTYPE:.data:0:0 b:LOCAL:NOTYPE:.data:4:0',
    ('x86_64', 'num_zero_back'): '.data:8:1:0000000004000800',
    ('i686', 'num_zero_back'): '.data:8:1:0000000004000800',
    ('aarch64', 'num_zero_back'): '.data:8:1:0000000004000800',
    ('riscv64', 'num_zero_back'): '.data:8:1:0000000004000800',
    ('x86_64', 'abs_self_text'): '.text:15:1:90b800000000680000000000000000 @2:r10:.text+1 @7:r11:.text+2 @11:r10:.text+1 foo:LOCAL:NOTYPE:.text:1:0',
    ('i686', 'abs_self_text'): '.text:15:1:90b800000000680000000000000000 @2:r1:.text+1 @7:r1:.text+2 @11:r1:.text+1 foo:LOCAL:NOTYPE:.text:1:0',
    # `.org` rows above. A forward target is a GAS fatal error (address
    # computation cannot converge), so both assemblers refuse it; the backward
    # forms pin the bytes, and org_relax_local pins the padding SHRINKING from
    # nine bytes to six when the `jmp` before it relaxes to a near jump.
    ('x86_64', 'org_backward_local'): '.text:5:1:90111111c3',
    ('x86_64', 'org_backward_named'): '.text:5:1:90333333c3 start:LOCAL:NOTYPE:.text:0:0',
    ('x86_64', 'org_relax_local'): '.text:213:1:90e9ce0000007777777777770000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000c3',
    ('x86_64', 'org_dot_relative'): '.text:6:1:9090909090c3',
    ('x86_64', 'org_forward_local'): 'error',
    ('x86_64', 'org_forward_named'): 'error',
    ('i686', 'org_backward_local'): '.text:5:1:90111111c3',
    ('i686', 'org_backward_named'): '.text:5:1:90333333c3 start:LOCAL:NOTYPE:.text:0:0',
    ('i686', 'org_relax_local'): '.text:213:1:90e9ce0000007777777777770000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000c3',
    ('i686', 'org_dot_relative'): '.text:6:1:9090909090c3',
    ('i686', 'org_forward_local'): 'error',
    ('i686', 'org_forward_named'): 'error',

}


def outcome(cmd, src, workdir, tag):
    s = os.path.join(workdir, tag + ".s")
    o = os.path.join(workdir, tag + ".o")
    with open(s, "w") as f:
        f.write(src)
    if os.path.exists(o):
        os.unlink(o)
    try:
        r = subprocess.run(cmd + [s, "-o", o], capture_output=True, text=True, timeout=60)
    except subprocess.TimeoutExpired:
        return "timeout"
    if r.returncode != 0:
        if r.returncode < 0 or "internal error" in r.stderr or "panicked" in r.stderr:
            return "crash: " + r.stderr.strip()[-200:]
        return "error"
    return canonical(Elf(o))


def canonical(elf):
    """Every non-empty PROGBITS/NOBITS section as name:size:align:bytes, its
    relocations, then the symbols.  A relocation is compared by meaning, not
    spelling: a reference to a defined LOCAL symbol is `section+offset`
    (GAS writes section symbol + addend where lccc may name the label), any
    other symbol stays by name.  SHT_REL implicit addends are read out of the
    bytes and their field zeroed, so the bytes compare only what is not
    relocated."""
    parts = []
    rels = {}
    for target, off, rtype, sym, add in elf.relocs:
        rels.setdefault(target, []).append((off, rtype, sym, add))
    for name, (typ, data, size, align) in sorted(elf.sections.items()):
        if typ not in (SHT_PROGBITS, SHT_NOBITS) or name == ".comment":
            continue
        if size == 0:  # an empty section's alignment is not under test (RISC-V
            continue    # GAS gives an empty .text 2 for RVC)
        data = None if data is None else bytearray(data)
        resolved = []
        for off, rtype, sym, add in sorted(rels.get(name, []), key=lambda r: r[0]):
            width = REL_WIDTH.get(rtype, 4) if add is None else 1
            if data is None or off + width > len(data):
                # A relocated field outside the section's bytes is a broken
                # object, reported as such (slicing past the end used to
                # extend the copy and hide it).
                resolved.append((off, rtype, "!overrun", 0))
                continue
            if add is None:  # SHT_REL: the addend lives in the field
                add = int.from_bytes(data[off:off + width], "little", signed=True)
                data[off:off + width] = bytes(width)
            base = ""
            if sym is not None:
                sname, bind, _t, ssec, value, _sz, _vis = elf.symbols[sym]
                if bind == "LOCAL" and ssec not in ("UND", "COM", "ABS"):
                    base, add = ssec, value + add
                else:
                    base = sname
            resolved.append((off, rtype, base, add))
        refs = []
        k = 0
        while k < len(resolved):
            off, rtype, base, add = resolved[k]
            nxt = resolved[k + 1] if k + 1 < len(resolved) else None
            if (elf.machine == EM_RISCV and rtype in RISCV_ADD and nxt
                    and nxt[0] == off and nxt[1] == rtype + 4):
                # An ADD/SUB pair is one value, add - sub; how the constant
                # is split between the two addends is spelling.
                refs.append(f"@{off}:pair{RISCV_ADD[rtype]}:{base}-{nxt[2]}{add - nxt[3]:+d}")
                k += 2
                continue
            refs.append(f"@{off}:r{rtype}:{base}{add:+d}")
            k += 1
        body = "nobits" if data is None else data.hex()
        parts.append(f"{name}:{size}:{align}:{body}")
        parts.extend(refs)
    for name, bind, typ, sec, value, size, vis in sorted(elf.symbols):
        # `.L` names are assembler-local temporaries (GAS riscv keeps the
        # one it makes for a `.`-relative SUB); no other code can see them.
        if typ in ("SECTION", "FILE") or not name or name.startswith(("$", ".L")):
            continue
        parts.append(f"{name}:{bind}:{typ}:{sec}:{value}:{size}"
                     + ("" if vis == "DEFAULT" else f":{vis}"))
    return " ".join(parts)


TARGETS = {"x86": ("x86_64",), "x86s": ("x86_64", "i686"),
           "all": ("x86_64", "i686", "aarch64", "riscv64")}
DRIVERS = {"x86_64": "lccc", "i686": "lccc-i686", "aarch64": "lccc-arm", "riscv64": "lccc-riscv"}


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("--lccc", required=True)
    ap.add_argument("--oracle", nargs="+", metavar="TARGET=AS",
                    help="GNU as per target: x86_64 (i686 via --32), aarch64, riscv64")
    ap.add_argument("--print-pins", action="store_true",
                    help="with --oracle: print GAS's verdicts as a PINS table")
    a = ap.parse_args()
    d = os.path.dirname(os.path.abspath(a.lccc))
    lccc = {t: [a.lccc if t == "x86_64" else os.path.join(d, b), "-c"] for t, b in DRIVERS.items()}
    for tgt, cmd in lccc.items():
        if not os.access(cmd[0], os.X_OK):
            sys.exit(f"missing {tgt} assembler driver: {cmd[0]}")

    def cells():
        for row, (src, spec) in ROWS.items():
            for tgt in TARGETS[spec]:
                yield tgt, row, src

    rows = failures = 0
    with tempfile.TemporaryDirectory(prefix="asmdir.") as td:
        if a.oracle:
            given = dict(o.split("=", 1) for o in a.oracle)
            gas = {}
            if "x86_64" in given:
                gas["x86_64"] = [given["x86_64"]]
                gas["i686"] = [given["x86_64"], "--32"]
            if "aarch64" in given:
                gas["aarch64"] = [given["aarch64"]]
            if "riscv64" in given:
                gas["riscv64"] = [given["riscv64"], "-march=rv64gc", "-mno-relax"]
            drift = 0
            fresh = {}
            for tgt, row, src in cells():
                if tgt not in gas:
                    continue
                got = outcome(gas[tgt], src, td, "g")
                fresh[(tgt, row)] = got
                if PINS.get((tgt, row)) != got:
                    drift += 1
                    if not a.print_pins:
                        print(f"ORACLE {tgt} {row}: GAS {got!r}, pinned {PINS.get((tgt, row))!r}")
            if a.print_pins:
                print("PINS = {")
                for (tgt, row), got in fresh.items():
                    print(f"    ({tgt!r}, {row!r}): {got!r},")
                print("}")
                return 0
            print(f"oracle: {drift} unexplained difference(s) over {len(fresh)} cells")
        missing = [(t, r) for t, r, _ in cells() if (t, r) not in PINS]
        if missing:
            sys.exit(f"unpinned cells (run --oracle ... --print-pins): {missing[:5]}")
        for tgt, row, src in cells():
            want, why = DIVERGENCES.get((tgt, row), (PINS[(tgt, row)], None))
            got = outcome(lccc[tgt], src, td, "l")
            if isinstance(want, tuple):
                want = outcome(lccc[tgt], want[1], td, "r")
                if want == "error" or want.startswith(("crash", "timeout")):
                    want = f"reference source refused: {want}"
            rows += 1
            if got != want:
                failures += 1
                print(f"FAIL {tgt} {row}: lccc {got!r}\n{'':>10}want {want!r}"
                      + (f" [divergence: {why}]" if why else ""))
    print(f"assembler directive contract: {rows - failures}/{rows} cells hold "
          f"({len(DIVERGENCES)} pinned divergences)")
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
