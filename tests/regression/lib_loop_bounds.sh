#!/usr/bin/env bash
# lib_loop_bounds.sh — shared "is this access inside the loop?" primitives.
#
# WHY A LIBRARY
# -------------
# Two gates need the same non-trivial capability, and getting it subtly
# different in each is how a suite ends up with one instrument that can see a
# hoist and another that cannot.  `check_volatile_spin_loop.sh` and
# `check_loop_preheader.sh` both assert on the POSITION of an emitted
# instruction relative to a loop, which an access *count* cannot express:
# hoisting an access leaves the count at one, so a count-based assertion
# passes on miscompiled code (measured on the pre-fix tree, where
# `while (!regs[4]) {}` hoisted the read and
# `check_volatile_pointer_subscript.sh` still reported ok).
#
# Sourced, not executed: `shellcheck source=tests/regression/lib_loop_bounds.sh`.
#
# Requires the caller to have set:
#   ASM_S   path to the .s file
#   CCC     unused here, kept out of scope deliberately

# asm_fn_lines <asm> <fn>
#   Echo the lines of one function's body, without directives, without a
#   leading `.size`/`.type`, and with a per-function line counter.  A function
#   body starts at `<fn>:` and ends at the next non-local label, so a static
#   helper like `count_mem_movs` in the subscript gate cannot drift past the
#   end of the function it was asked about.
asm_fn_lines() {
    awk -v fn="$2" '
        $0 ~ "^" fn ":" { inside = 1; next }
        inside && /^[A-Za-z_.][A-Za-z0-9_.]*:/ && $0 !~ /^\.L/ { inside = 0 }
        inside { print }
    ' "$1"
}

# loop_body_bounds <asm> <fn>
#   Echo "<start> <end>": the half-open, 1-based line range of the innermost
#   loop body, measured from the label a backward branch targets up to that
#   branch.  Empty when the function has no backward branch, which callers
#   must treat as a failure: a probe with no loop asserts nothing.
loop_body_bounds() {
    asm_fn_lines "$1" "$2" | awk '
        BEGIN { line = 0 }
        {
            line++
            if ($0 ~ /^[A-Za-z_.][A-Za-z0-9_.]*:/) {
                lbl = $0; sub(/:.*/, "", lbl)
                pos[lbl] = line
                next
            }
            if ($0 ~ /^[[:space:]]*[a-z]+[[:space:]]/) {
                tgt = ""
                for (i = 1; i <= NF; i++)
                    if ($i ~ /^\.L/) { tgt = $i; break }
                if (tgt != "" && (tgt in pos) && pos[tgt] < line) {
                    lo = pos[tgt]; hi = line
                }
            }
        }
        END { if (lo != "") print lo, hi }
    '
}

# count_mem_in_range <asm> <fn> <lo> <hi>
#   Memory-operand instructions of `fn` in the half-open line range [lo, hi).
#   A folded `addl 4(%rdi), %edx` is a real second access, so only `lea` — an
#   address computation that touches no memory — is excluded.
count_mem_in_range() {
    asm_fn_lines "$1" "$2" | awk -v lo="$3" -v hi="$4" '
        {
            line++
            if (line > lo && line <= hi &&
                $0 ~ /^[[:space:]]*[a-z]+[[:space:]]/ && /\(/ && $1 !~ /^lea/) n++
        }
        END { print n + 0 }'
}

# mem_accesses_in_loop <asm> <fn>
#   Convenience: how many memory accesses of `fn` sit inside its innermost
#   loop body.  Prints nothing (and returns 2) when there is no loop.
mem_accesses_in_loop() {
    local bounds
    bounds=$(loop_body_bounds "$1" "$2")
    [ -n "$bounds" ] || return 2
    count_mem_in_range "$1" "$2" "${bounds%% *}" "${bounds##* }"
}

# require_loop_accesses <label> <asm> <fn> <min-or-"eq"> <count>
#   The one assertion both gates share, with the "no loop proves nothing"
#   case folded in so neither caller can forget it.
require_loop_accesses() {
    local label="$1" asm="$2" fn="$3" mode="$4" want="$5" got
    if ! got=$(mem_accesses_in_loop "$asm" "$fn"); then
        echo "FAIL $label: '$fn' has no backward branch; the probe proves nothing" >&2
        return 1
    fi
    case "$mode" in
        ge) [ "$got" -ge "$want" ] && return 0
            echo "FAIL $label: '$fn' has $got memory access(es) in the loop body, need >= $want" >&2
            return 1 ;;
        eq) [ "$got" -eq "$want" ] && return 0
            echo "FAIL $label: '$fn' has $got memory access(es) in the loop body, want exactly $want" >&2
            return 1 ;;
        *)  echo "FAIL $label: bad mode '$mode'" >&2; return 1 ;;
    esac
}
