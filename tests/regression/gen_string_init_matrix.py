#!/usr/bin/env python3
"""Generate tests/regression/array_string_init_matrix.c.

The matrix crosses every character element width (char, char16_t,
wchar_t, char32_t) with every initializer shape lccc must lower for arrays
of scalars and strings (C11 6.7.9p14-p22): short/exact/unsized/braced
strings, multi-dimensional arrays with elided and explicit braces,
designators and designator continuations, struct and union members,
arrays of structs with and without element braces, pointer-carrying
structs (relocation path), nested designator chains and flexible array
members, in block-scope, static and global storage.  Each object is hashed
bytewise after a stack-dirtying call, so both stale padding and misplaced
elements show up in the gcc-vs-lccc comparison of run_regression_suite.sh.

Usage: gen_string_init_matrix.py [OUTPUT]   (default: next to this script)
Regenerate and commit the output whenever the matrix changes.
"""
kinds = [("char", "", "\"", 1), ("char16_t", "u", "\"", 2), ("wchar_t", "L", "\"", 4), ("char32_t", "U", "\"", 4)]
out = ['#include <stdio.h>', '#include <string.h>', '#include <uchar.h>', '#include <wchar.h>',
       'static unsigned long H = 1469598103u;',
       'static void mix(const char *tag, const void *p, size_t n) { const unsigned char *b = p; unsigned long h = 1469598103u; for (size_t i = 0; i < n; i++) h = (h ^ b[i]) * 16777619u; printf("%s %zu %lx\\n", tag, n, h); H ^= h; }',
       '__attribute__((noinline)) static void dirty(void) { volatile unsigned char junk[8192]; memset((unsigned char *)junk, 0x5a, sizeof junk); }']
bodies = []
for T, P, _, W in kinds:
    big = r"\U0001F600" if W == 2 else ("\\xe9" if W == 1 else r"\U0001F600")
    lit = lambda s: f'{P}"{s}"'
    n = T.replace("_t", "")
    decls = [
        (f"{n}_1d_short", f"{T} %s[7] = {lit('ab')};"),
        (f"{n}_1d_exact", f"{T} %s[2] = {lit('ab')};"),
        (f"{n}_1d_unsized", f"{T} %s[] = {lit('a' + big + 'b')};"),
        (f"{n}_1d_braced", f"{T} %s[5] = {{ {lit('xy')} }};"),
        (f"{n}_2d", f"{T} %s[3][4] = {{ {lit('ab')}, {lit('wxyz')}, {lit('q')} }};"),
        (f"{n}_2d_braced", f"{T} %s[2][3] = {{ {{ {lit('ab')} }}, {{ {lit('c')} }} }};"),
        (f"{n}_2d_desig", f"{T} %s[3][3] = {{ [2] = {lit('hi')}, [0] = {lit('a')} }};"),
        (f"{n}_2d_unsized", f"{T} %s[][3] = {{ {lit('ab')}, {lit('c')} }};"),
        (f"{n}_3d", f"{T} %s[2][2][3] = {{ {{ {lit('ab')}, {lit('c')} }}, {{ {lit('d')} }} }};"),
        (f"{n}_member", f"struct {{ {T} a[4]; int g; {T} b[4]; int h; }} %s = {{ {lit('ab')}, 7, {lit('cdef')}, 9 }};"),
        (f"{n}_member_braced", f"struct {{ int g; {T} a[4]; }} %s = {{ 1, {{ {lit('ab')} }} }};"),
        (f"{n}_member_desig", f"struct {{ int g; {T} a[4]; int k; }} %s = {{ .a = {lit('z')}, .k = 3 }};"),
        (f"{n}_member_2d", f"struct {{ {T} a[2][4]; int g; }} %s = {{ {{ {lit('ab')}, {lit('c')} }}, 5 }};"),
        (f"{n}_member_flat2d", f"struct {{ {T} a[2][4]; int g; }} %s = {{ {lit('ab')}, {lit('c')}, 5 }};"),
        (f"{n}_nested", f"struct {{ int g; struct {{ {T} s[4]; int k; }} in; }} %s = {{ 1, {{ {lit('a' + big)}, 2 }} }};"),
        (f"{n}_arr_of_struct", f"struct {{ {T} s[4]; int k; }} %s[2] = {{ {{ {lit('ab')}, 1 }}, {{ {lit('c')}, 2 }} }};"),
        (f"{n}_arr_of_struct_flat", f"struct {{ {T} s[4]; int k; }} %s[2] = {{ {lit('ab')}, 1, {lit('c')}, 2 }};"),
        (f"{n}_union", f"union {{ {T} s[8]; long long x; }} %s = {{ {lit('ab')} }};"),
        (f"{n}_2d_override", f"{T} %s[2][4] = {{ [0] = {lit('abc')}, [0] = {lit('x')}, [1] = {{ 1, 2, 3 }}, [1] = {lit('y')} }};"),
        (f"{n}_2d_excess", f"{T} %s[2][3] = {{ {lit('ab')}, {lit('c')}, {lit('d')} }};"),
        (f"{n}_2d_braced_override", f"{T} %s[2][3] = {{ [0][2] = 9, [0] = {{ 4 }}, [1][1] = 5, [1] = 6 }};"),
        (f"{n}_3d_mid_desig", f"{T} %s[2][2][4] = {{ [1] = {{ [1] = {lit('hi')} }}, [0][1] = {lit('j')}, {lit('k')} }};"),
        (f"{n}_member_desig_chain", f"struct {{ int g; {T} a[3][4]; int k; }} %s = {{ .a[1] = {lit('xy')}, {lit('z')}, .k = 3 }};"),
        (f"{n}_member_desig_elem", f"struct {{ int g; {T} a[2][4]; int k; }} %s = {{ .a[1][2] = 65, 66, 67, .g = 4 }};"),
        (f"{n}_member_2d_desig_rows", f"struct {{ {T} a[3][3]; int k; }} %s = {{ {{ [2] = {lit('p')}, [0] = {{ 1 }} }}, 8 }};"),
        (f"{n}_union_2d", f"union {{ {T} s[2][3]; long long x[2]; }} %s = {{ {lit('a')}, {lit('bc')} }};"),
    ]
    for name, d in decls:
        for storage in ("auto", "static"):
            fn = f"t_{name}_{storage}"
            v = "v"
            pre = "static " if storage == "static" else ""
            bodies.append(fn)
            out.append(f"__attribute__((noinline)) static void {fn}(void) {{ {pre}{d % v} mix(\"{fn}\", &{v}, sizeof {v}); }}")
    # compound literals (block scope)
    cls = [
        (f"{n}_cl_unsized", f"const {T} *p = ({T}[]){{ {lit('a' + big)} }}; mix(\"%s\", p, sizeof(({T}[]){{ {lit('a' + big)} }}));"),
        (f"{n}_cl_sized", f"const {T} *p = ({T}[5]){{ {lit('ab')} }}; mix(\"%s\", p, 5 * sizeof({T}));"),
        (f"{n}_cl_2d", f"const {T} (*p)[3] = ({T}[2][3]){{ {lit('ab')}, {lit('c')} }}; mix(\"%s\", p, 6 * sizeof({T}));"),
        (f"{n}_cl_struct", f"struct cs_{n} {{ {T} a[4]; int g; }}; const struct cs_{n} *p = &(struct cs_{n}){{ {lit('ab')}, 4 }}; mix(\"%s\", p, sizeof *p);"),
    ]
    for name, d in cls:
        fn = f"t_{name}"
        bodies.append(fn)
        out.append(f"__attribute__((noinline)) static void {fn}(void) {{ {d % fn} }}")
    # file-scope objects
    gl = [
        (f"g_{n}_2d", f"static {T} g_{n}_2d[2][4] = {{ {lit('ab')}, {lit('c' + big)} }};"),
        (f"g_{n}_member", f"static struct {{ {T} a[3]; {T} w[3]; }} g_{n}_member = {{ {lit('ab')}, {lit('cd')} }};"),
        (f"g_{n}_arr_struct", f"static struct {{ {T} a[3]; int k; }} g_{n}_arr_struct[2] = {{ {{ {lit('ab')}, 1 }}, {{ {lit('e')}, 2 }} }};"),
        (f"g_{n}_fam", f"static struct {{ int n; {T} s[]; }} g_{n}_fam = {{ 3, {lit('ab')} }};"),
        (f"g_{n}_ptrmix", f"static struct {{ const {T} *p; {T} a[3]; }} g_{n}_ptrmix = {{ 0, {lit('ab')} }};"),
        (f"g_{n}_ptrmix_2d", f"static struct {{ const {T} *p; {T} a[2][3]; int k; }} g_{n}_ptrmix_2d = {{ g_{n}_2d[1], {lit('ab')}, {lit('c')}, 4 }};"),
        (f"g_{n}_ptrmix_2d_desig", f"static struct {{ const {T} *p; {T} a[2][3]; int k; }} g_{n}_ptrmix_2d_desig = {{ .a[1] = {lit('d')}, .p = 0, .k = 2 }};"),
        (f"g_{n}_fam_big", f"static struct {{ int n; {T} s[]; }} g_{n}_fam_big = {{ 3, {lit('a' + big)} }};"),
        (f"g_{n}_nested_ptr", f"static struct {{ const void *p; struct {{ {T} s[4]; {T} m[2][3]; int k; }} in; }} g_{n}_nested_ptr = {{ 0, {{ {lit('ab')}, {lit('c')}, {lit('d')}, 5 }} }};"),
        (f"g_{n}_arr_ptr_flat", f"static struct {{ const void *p; {T} s[3]; {T} m[2][2]; }} g_{n}_arr_ptr_flat[2] = {{ {{ 0, {lit('a')}, {lit('b')} }}, 0, {lit('bc')}, {lit('d')}, {lit('e')} }};"),
        (f"g_{n}_nested_ptr_desig", f"static struct {{ const void *p; struct {{ int k; {T} m[3][3]; }} in; }} g_{n}_nested_ptr_desig = {{ .in.m[1] = {lit('x')}, {lit('yz')}, .p = 0, .in.k = 7 }};"),
        (f"g_{n}_3d_desig", f"static {T} g_{n}_3d_desig[2][2][3] = {{ [1][1] = {lit('z')}, [0] = {{ {lit('a')}, {lit('b')} }} }};"),
    ]
    for name, d in gl:
        out.append(d)
        fn = f"t_{name}"
        bodies.append(fn)
        # FAM payload: "ab" is 3 elements with the terminator; "a<big>" is 3,
        # or 4 in UTF-16 where <big> is a surrogate pair.
        fam = {"_fam": 3, "_fam_big": 4 if W == 2 else 3}
        suffix = next((k for k in fam if name.endswith(k)), None)
        sz = f"sizeof {name}" if suffix is None else f"sizeof {name} + {fam[suffix]} * sizeof({T})"
        if name.endswith("_ptrmix_2d"):
            # The pointer's value differs between executables: hash from `a` on.
            out.append(f"__attribute__((noinline)) static void {fn}(void) {{ mix(\"{fn}\", {name}.a, sizeof {name} - (size_t)((char *){name}.a - (char *)&{name})); if ({name}.p != g_{n}_2d[1]) puts(\"{fn} BADPTR\"); }}")
            continue
        out.append(f"__attribute__((noinline)) static void {fn}(void) {{ mix(\"{fn}\", &{name}, {sz}); }}")
for T in ("int", "short", "double", "unsigned char", "_Bool"):
    n = T.replace(" ", "_")
    decls = [
        (f"{n}_md_flat", f"struct {{ {T} a[2][2]; {T} g; }} %s = {{ 1, 2, 3, 4, 5 }};"),
        (f"{n}_md_partial_row", f"struct {{ {T} a[2][2]; {T} g; }} %s = {{ {{ 1, 2, 3 }}, 5 }};"),
        (f"{n}_md_short_row", f"struct {{ {T} a[2][3]; {T} g; }} %s = {{ {{ 1, 2 }}, 7 }};"),
        (f"{n}_md_desig", f"struct {{ {T} a[2][2]; {T} g; }} %s = {{ .a[1][0] = 9, .g = 3 }};"),
        (f"{n}_md_inner_desig", f"struct {{ {T} a[2][2]; {T} g; }} %s = {{ {{ [1] = {{ 4 }}, [0][1] = 2 }}, 1 }};"),
        (f"{n}_md_nested", f"struct {{ {T} k; struct {{ {T} a[2][2]; }} in[2]; }} %s = {{ 1, {{ {{ {{ {{ 1, 2 }}, {{ 3 }} }} }}, {{ 5, 6, 7 }} }} }};"),
        (f"{n}_md_nested_flat", f"struct {{ {T} k; struct {{ {T} a[2][2]; }} in[2]; }} %s = {{ 1, 2, 3, 4, 5, 6, 7 }};"),
        (f"{n}_3d_desig", f"{T} %s[2][2][2] = {{ [1][0] = {{ 1, 2 }}, [0] = 3, 4 }};"),
        (f"{n}_md_then_scalar_members", f"struct {{ {T} a[2][2]; {T} b[2]; {T} g; }} %s = {{ 1, 2, 3, 4, 5, 6, 7 }};"),
        (f"{n}_md_excess", f"struct {{ {T} a[2][2]; {T} g; }} %s = {{ {{ {{ 1, 2, 9 }}, {{ 3 }}, {{ 8 }} }}, 5 }};"),
        (f"{n}_md_desig_continue", f"struct {{ {T} a[2][3]; {T} g; }} %s = {{ .a[0][2] = 1, 2, 3, .g = 4 }};"),
        (f"{n}_md_braced_override", f"{T} %s[2][3] = {{ [0][2] = 9, [0] = {{ 4 }}, [1][1] = 5, [1] = 6 }};"),
        (f"{n}_md_nested_level_desig", f"{T} %s[3][2][2] = {{ {{ 1 }}, {{ [1] = {{ [1] = 4 }}, [0][0] = 2 }}, 3 }};"),
        (f"{n}_md_values", f"{T} %s[2][3] = {{ {{ -1, 2, 0 }}, {{ 256, 3, 0 }} }};"),
        (f"{n}_md_cl", None),
    ]
    for name, d in decls:
        if d is None:
            # compound literal of a multi-dimensional scalar array
            fn = f"t_{name}"
            bodies.append(fn)
            out.append(f"__attribute__((noinline)) static void {fn}(void) {{ const {T} (*p)[3] = ({T}[2][3]){{ [1][1] = 7, 8, [0] = {{ 1 }}, 2 }}; mix(\"{fn}\", p, sizeof({T}[2][3])); }}")
            continue
        for storage in ("auto", "static"):
            fn = f"t_{name}_{storage}"
            pre = "static " if storage == "static" else ""
            bodies.append(fn)
            out.append(f"__attribute__((noinline)) static void {fn}(void) {{ {pre}{d % 'v'} mix(\"{fn}\", &v, sizeof v); }}")
out.append("int main(void) {")
for b in bodies:
    out.append(f"  dirty(); {b}();")
out.append('  printf("ALL %lx\\n", H); return 0; }')
import os
import sys

dest = sys.argv[1] if len(sys.argv) > 1 else os.path.join(
    os.path.dirname(os.path.abspath(__file__)), "array_string_init_matrix.c")
with open(dest, "w") as f:
    f.write("/* GENERATED by gen_string_init_matrix.py -- do not edit. */\n")
    f.write("\n".join(out) + "\n")
