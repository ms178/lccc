#!/usr/bin/env bash
# check_function_alignment_definition.sh — `aligned(N)` on a function reaches the asm, in
# every spelling, and REPLACES -falign-functions rather than merging with it.
#
# THE GAP THIS PINS, PART 1 -- the definition channel
# ---------------------------------------------------
# Function alignment reaches codegen through `IrModule::function_alignments`,
# keyed by the emitted function name.  Only `lower_global_decl` -- the PROTOTYPE
# path -- ever wrote to it, so a definition's own attribute was parsed and
# dropped: `__attribute__((aligned(64))) void f(void) { }` emitted `.p2align 4`.
#
# THE GAP THIS PINS, PART 2 -- the parameter list swallowed it
# ------------------------------------------------------------
# After part 1 was fixed the definition channel still only worked for
# PARAMETERLESS functions.  The reason was in the parser, not the emitter: the
# alignment pending when a parameter list starts (`parsed_alignas`) belongs to
# the enclosing declaration, and the per-parameter attribute capture inside the
# list merged into that slot and TOOK it.  `f(void)` and `f()` kept the
# attribute; `f(int x)` lost it, in both the definition channel and the
# prototype channel.  The first version of this gate could not see that, because
# every function in its fixture was parameterless -- hence `*_params` below.
#
# THE GAP THIS PINS, PART 3 -- replace, do not merge
# --------------------------------------------------
# GCC: `-falign-functions` applies to functions with NO attribute, and an
# attribute REPLACES it.  Measured, GCC 14 `-O2`, default and
# `-falign-functions=32`/`=8`:
#
#     -falign-functions unset (16):   plain -> .p2align 4   aligned(2) -> .align 2
#     -falign-functions=8:            plain -> .p2align 3   aligned(2) -> .align 2
#     -falign-functions=32:           plain -> .p2align 5   aligned(2) -> .align 2
#     aligned(1):                     NO directive at all, i.e. the default is
#                                     suppressed, not merged (GCC 14 -O2)
#
# LCCC emitted the attribute directive and then the default one after it; since
# `.p2align` only ever advances the location counter, the pair meant max(N, 16)
# and `aligned(2)` came out 16-byte aligned.  Measured on GCC 14 as the oracle:
#
#     function              attribute on            GCC 14       LCCC before  LCCC now
#     --------------------------------------------------------------------------------
#     via_def               definition only         .align 64    .p2align 4   .p2align 6
#     via_proto             prototype               .align 64    .p2align 6   .p2align 6
#     plain                 (none)                  .p2align 4   .p2align 4   .p2align 4
#     proto_then_def        both                    .align 64    .p2align 6   .p2align 6
#     via_def_params        definition, params      .align 64    .p2align 4   .p2align 6
#     via_proto_params      prototype, params       .align 64    .p2align 4   .p2align 6
#     via_decl_small        decl aligned(2)         .align 64    .p2align 4   .p2align 6
#                           + def aligned(64)                   (merge=max)
#     replaces_default      aligned(2)               .align 2     .p2align 4   .p2align 1
#     natural_alignment     aligned(1)              (none)       .p2align 4   (none)
#     plain_params          (none), params          .p2align 4   .p2align 4   .p2align 4
#
# `plain` and `plain_params` are the controls: a fix that aligned everything
# would satisfy the positive rows and break these two.
#
# THREE MEASUREMENT TRAPS this file exists to avoid repeating
# ----------------------------------------------------------
# 1. "The last directive before the label wins" is WRONG.  LCCC used to emit
#    `.p2align 6` and then a redundant `.p2align 4` before the same label, and
#    `.p2align` only advances: the effective alignment is the MAXIMUM over the
#    directive run since the previous label.  `effective_align_of` below
#    implements the max, which is also how it agrees with GNU as.
# 2. This fixture must contain a function WITH PARAMETERS.  Every failure mode
#    in parts 1-3 above is invisible on a parameterless fixture, and that is
#    exactly how the original version of this gate passed while the bug was
#    live.
# 3. Exactly one placement directive belongs in front of a function entry, and
#    `directive_count_of` asserts that for the attributed cases: two directives
#    where one is an attribute and the other a default read as if the default
#    overrode the attribute, even when the padding happens to come out right.
#
# The nested-function case is deliberately NOT asserted.  Measured: GCC applies
# no observable alignment to a nested function definition at -O0 or -O2, with or
# without `noinline`, so there is no oracle to match, and LCCC ignoring the
# attribute there is parity rather than a gap.
set -euo pipefail

CCC=${CCC:-./target/fastbuild/lccc}
[[ -x $CCC ]] || { echo "check_function_alignment_definition: lccc not found at $CCC" >&2; exit 1; }
tmp=$(mktemp -d "${TMPDIR:-/tmp}/lccc-fn-align.XXXXXX")
trap 'rm -rf "$tmp"' EXIT

cat > "$tmp/a.c" <<'EOF'
__attribute__((aligned(64))) void via_def(void) { }
__attribute__((aligned(64))) void via_proto(void);
void via_proto(void) { }
void plain(void) { }
__attribute__((aligned(64))) void proto_then_def(void);
void proto_then_def(void) { }
__attribute__((aligned(64))) void via_def_params(int x) { (void)x; }
__attribute__((aligned(64))) void via_proto_params(int x);
void via_proto_params(int x) { (void)x; }
_Alignas(64) void via_alignas_params(int x) { (void)x; }
__attribute__((aligned(2))) void via_decl_small(int x);
__attribute__((aligned(64))) void via_decl_small(int x) { (void)x; }
__attribute__((aligned(2))) void replaces_default(int x) { (void)x; }
__attribute__((aligned(1))) void natural_alignment(int x) { (void)x; }
void plain_params(int x) { (void)x; }
EOF
"$CCC" -O2 -S "$tmp/a.c" -o "$tmp/a.s"

# window_of <fn> <asm>
#   The directives between the previous label and this one, one per line.
#   Every directive in that window applies, and each only ever increases the
#   padding, so max is the effective alignment -- see trap 1 above.
window_of() {
    awk -v fn="$1" '
        /^[[:space:]]*\.(p2align|align)[[:space:]]/ { buf = buf $2 " "; next }
        /^[A-Za-z_.][A-Za-z0-9_.]*:$/ {
            if ($0 == fn ":") { print buf; exit }
            buf = ""
        }
    ' "$2"
}

#   Both helpers always succeed and always print something, because a pipeline
#   that can fail inside a command substitution aborts the whole script under
#   `set -e`: `grep` exits 1 on an empty window (`aligned(1)`, where the correct
#   answer is "no directive"), which surfaced as the script stopping dead with
#   no FAIL line -- the least debuggable way for a gate to fail.
effective_align_of() {
    local v
    v=$(window_of "$1" "$2" | tr ' ' '\n' | grep -E '^[0-9]+$' | sort -n | tail -1) || true
    echo "${v:-0}"
}
directive_count_of() { window_of "$1" "$2" | wc -w | tr -d ' '; }

fail=0
expect() { # expect <fn> <log2> <why>
    local got; got=$(effective_align_of "$2" "$tmp/a.s"); got=${got:-0}
    if [ "$got" = "$1" ]; then
        echo "    ok   $2: .p2align $got"
    else
        echo "FAIL $2: effective alignment is .p2align $got, want .p2align $1 -- $3" >&2
        fail=1
    fi
}

expect_directives() { # expect_directives <count> <fn> <why>
    local got; got=$(directive_count_of "$2" "$tmp/a.s")
    if [ "$got" = "$1" ]; then
        echo "    ok   $2: exactly $got placement directive(s)"
    else
        echo "FAIL $2: $got placement directives in the window, want $1 -- $3" >&2
        fail=1
    fi
}

echo "  the definition channel (the original gap):"
expect "6" via_def "a definition's own aligned(64) must reach the asm"
echo "  the prototype channel:"
expect "6" via_proto ""
expect "6" proto_then_def "declaration and definition both aligned(64)"
echo "  a parameter list must not swallow it (part 2):"
expect "6" via_def_params "leading attribute on a definition WITH parameters"
expect "6" via_proto_params "prototype channel WITH parameters"
expect "6" via_alignas_params "_Alignas on a definition with parameters"
expect "6" via_decl_small "aligned(2) declaration + aligned(64) definition merges to the max"
echo "  an attribute replaces -falign-functions instead of merging with it (part 3):"
expect "1" replaces_default "aligned(2) is 2-byte aligned, not max(2, default 16)"
expect "0" natural_alignment "aligned(1) suppresses the directive entirely, as GCC does"
echo "  one directive per entry, so the attribute is not shadowed by a default (part 3b):"
expect_directives 1 via_def "attribute + default used to sit in front of the same label"
expect_directives 1 replaces_default "a lowered attribute is still a single directive"
expect_directives 1 plain "the default path still emits its own directive"
echo "  the controls (a blanket alignment change fails here):"
expect "4" plain ""
expect "4" plain_params ""

if [ "$fail" -ne 0 ]; then
    echo "--- asm ---" >&2
    grep -vE '^\s*\.(cfi|loc|file|size|type|ident|weak)' "$tmp/a.s" >&2
    exit 1
fi
echo "check_function_alignment_definition: ok"
