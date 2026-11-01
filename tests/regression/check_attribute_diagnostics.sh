#!/usr/bin/env bash
# -Wattributes must distinguish the three classes GCC 16.2 distinguishes, and
# must stay silent for everything this compiler accepts.
#
# WHY: silence for an attribute the compiler does not implement is a silent
# semantic divergence. Measured before this check: `__attribute__((ms_abi))`
# was accepted without a word while the generated code used the SysV
# convention, so a callback registered by a Windows-ABI caller returned
# garbage. The registry (`src/frontend/parser/attribute_registry.rs`) is the
# single source of truth for every spelling (`__attribute__((x))` and C23
# `[[x]]` both funnel through it); this check pins the message wording and the
# three classes' behaviour by class.
#
# Expectations measured against gcc 16.2 (x86-64):
#   unknown name                  ->  warning: 'x' attribute directive ignored
#   GCC name, semantics not applied->  'x' attribute ignored under an explicit
#                                      -Wattributes (gcc is silent on the
#                                      valid use, so a default-on warning
#                                      would fire on correct programs)
#   calling convention             ->  gcc's own verdict for this target: silent
#                                      when it names the convention already
#                                      emitted, 'x' attribute ignored when gcc
#                                      itself ignores it, and a loud refusal
#                                      when it is a real ABI this compiler
#                                      cannot lower (class 3)
#   implemented / benign           ->  no diagnostic
#   C23 [[x]] unknown              ->  warning: 'x' attribute ignored
set -uo pipefail
CCC=${CCC:-./target/fastbuild/lccc}
TMP=${TMPDIR:-/tmp}/lccc-attrdiag.$$
trap 'rm -rf "$TMP"' EXIT
mkdir -p "$TMP"
fail=0

expect() { # expect <case> <expected-substring|EMPTY> <source> [flags...]
    local name=$1 want=$2 src=$3 out
    shift 3
    printf '%s\n' "$src" >"$TMP/t.c"
    out=$("$CCC" -fsyntax-only "$@" "$TMP/t.c" 2>&1)
    if [[ $want == EMPTY ]]; then
        [[ -z $out ]] && return
        echo "FAIL $name: expected silence, got: $out" >&2
        fail=1
    else
        [[ $out == *"$want"* ]] && return
        echo "FAIL $name: expected '$want', got: $out" >&2
        fail=1
    fi
}

# --- class 1: implemented AND correctly placed -> silent ---------------------
# Every source below is the position GCC 16.2 accepts for that attribute; the
# misapplied positions are class 1b, because GCC diagnoses them (measured
# 2026-10-30: `int x __attribute__((noreturn));` is a warning, in either
# position, so pinning silence here would pin a false negative).
for a in packed aligned section used weak noinline noreturn const pure \
         always_inline noclone_but_unknown vector_size cleanup mode \
         visibility retained_section alias_ifunc; do
    case $a in
        packed) src='struct S { char c; } __attribute__((packed));' ;;
        aligned) src='int x __attribute__((aligned(16)));' ;;
        section) src='int x __attribute__((section(".keep")));' ;;
        noinline) src='void f(void) __attribute__((noinline)); void f(void){}' ;;
        noreturn) src='void f(void) __attribute__((noreturn));' ;;
        const) src='int f(void) __attribute__((const));' ;;
        pure) src='int f(void) __attribute__((pure));' ;;
        always_inline) src='inline void f(void) __attribute__((always_inline)); void f(void){}' ;;
        retained_section) src='int x __attribute__((retain, section(".keep")));' ;;
        alias_ifunc) src='void f(void); void g(void) __attribute__((alias("f")));' ;;
        noclone_but_unknown) src='__attribute__((noclone)) void f(void);' ;;
        vector_size) src='typedef int v __attribute__((vector_size(16)));' ;;
        mode) src='int f(void) __attribute__((mode(DI)));' ;;
        cleanup) src='void h(int *p); void f(void){ int v __attribute__((cleanup(h))); (void)v; }' ;;
        visibility) src='int x __attribute__((visibility("hidden")));' ;;
        *) src="int x __attribute__(($a));" ;;
    esac
    expect "silent:$a" EMPTY "$src"
done

# --- class 1b: implemented but misapplied -> GCC's own wording ----------------
# Function-only attributes on an object. GCC's sentence is the generic one;
# `naked` and `noplt` have dedicated wordings and are checked by the Python
# twin (check_attribute_diagnostics.py), which diffs against a live gcc.
for a in packed noinline noreturn const pure always_inline; do
    expect "misapplied:$a" "'$a' attribute ignored" \
        "int x __attribute__(($a));" -Wattributes
done

# --- class 2: benign (diagnostics/optimisation only) -> silent ---------------
for a in format nonnull warn_unused_result deprecated sentinel \
         access alloc_size alloc_align counted_by leafy malloc_like nothrow \
         hot flatten may_alias assume_aligned no_reorder sysv_abi_attr; do
    case $a in
        leafy) a=leaf ;; malloc_like) a=malloc ;; sysv_abi_attr) a=sysv_abi ;;
    esac
    case $a in
        format) src='void f(const char *, ...) __attribute__((format(printf,1,2)));' ;;
        nonnull) src='void f(int *) __attribute__((nonnull(1)));' ;;
        warn_unused_result) src='int f(void) __attribute__((warn_unused_result));' ;;
        deprecated) src='__attribute__((deprecated)) void f(void);' ;;
        sentinel) src='void f(const char *, ...) __attribute__((sentinel));' ;;
        access) src='void f(int *p) __attribute__((access(write_only,1)));' ;;
        alloc_size) src='void *f(unsigned) __attribute__((alloc_size(1)));' ;;
        alloc_align) src='void *f(unsigned) __attribute__((alloc_align(1)));' ;;
        counted_by) src='struct s { int n; char b[] __attribute__((counted_by(n))); };' ;;
        *) src="int f(void) __attribute__(($a));" ;;
    esac
    expect "silent-benign:$a" EMPTY "$src"
done

# --- class 3: calling conventions (silent / gcc's wording / refused) --------
# A convention is not interchangeable with a benign attribute. A caller and a
# callee that disagree about where the arguments live still link and still run;
# they just return garbage. So each name gets exactly the answer gcc gives on
# this target (measured on gcc-16.2.0, x86-64), and the ones that are a real
# foreign ABI are refused instead of ignored.
#   sysv_abi                     -> this target's own convention: silent
#   cdecl stdcall thiscall fastcall sseregparm -> gcc ignores them here
#   preserve_none no_callee_saved_registers    -> real GCC 16.2 contracts: refused
#   callee_pop_aggregate_return  -> gcc's "only available for 32-bit"
#   ms_abi no_caller_saved_registers           -> refused
for a in sysv_abi; do
    expect "convention-default:$a" EMPTY "int f(void) __attribute__(($a));"
done
for a in cdecl stdcall thiscall fastcall sseregparm; do
    expect "convention-ignored:$a" "'$a' attribute ignored" \
        "int f(void) __attribute__(($a));"
done
expect "convention-ignored:regparm" "'regparm' attribute ignored" \
    'int f(void) __attribute__((regparm(3)));'
expect "convention-only32" \
    "'callee_pop_aggregate_return' attribute only available for 32-bit" \
    "int f(void) __attribute__((callee_pop_aggregate_return(0)));"
# The record-layout names mean something on a struct/union tag and nothing
# anywhere else, and gcc says so in each position: honoured on the tag
# (`gcc_struct` is the default layout, so silence), ignored on a function.
expect "convention-layout-default" EMPTY \
    'struct __attribute__((gcc_struct)) S { char c; int b : 3; };'
expect "convention-layout-ignored" "'ms_struct' attribute ignored" \
    'int f(void) __attribute__((ms_struct));'

# Refusals. The sentence has to name the attribute and the convention, because
# it is the only thing standing between the program and a wrong answer at run
# time; and the documented escape hatch must downgrade it, not disable it.
refuse() { # refuse <case> <source>
    local name=$1 src=$2 out
    printf '%s\n' "$src" >"$TMP/r.c"
    if "$CCC" -fsyntax-only "$TMP/r.c" >/dev/null 2>&1; then
        echo "FAIL $name: a foreign convention was accepted silently" >&2
        fail=1
        return
    fi
    out=$("$CCC" -fsyntax-only "$TMP/r.c" 2>&1)
    [[ $out == *"'$name'"* ]] || {
        echo "FAIL $name: refusal does not name the attribute: $out" >&2
        fail=1
    }
    [[ $out == *'does not implement'* ]] || {
        echo "FAIL $name: refusal does not explain itself: $out" >&2
        fail=1
    }
    # The verdict stays one sentence in the error; the escape hatch and the
    # reason live in a note attached to the same location, so a reader sees a
    # scannable error line and still gets the full explanation.
    [[ $out == *'define CCC_ALLOW_UNSUPPORTED_ABI'* ]] || {
        echo "FAIL $name: refusal has no escape-hatch note: $out" >&2
        fail=1
    }
    out=$(CCC_ALLOW_UNSUPPORTED_ABI=1 "$CCC" -fsyntax-only "$TMP/r.c" 2>&1)
    [[ $out == *CCC_ALLOW_UNSUPPORTED_ABI* ]] || {
        echo "FAIL $name: escape hatch does not downgrade the refusal: $out" >&2
        fail=1
    }
    if ! CCC_ALLOW_UNSUPPORTED_ABI=1 "$CCC" -fsyntax-only "$TMP/r.c" >/dev/null 2>&1; then
        echo "FAIL $name: escape hatch still fails the compile" >&2
        fail=1
    fi
}
refuse ms_abi 'int f(void) __attribute__((ms_abi));'
refuse no_caller_saved_registers 'int f(void) __attribute__((no_caller_saved_registers));'
refuse no_callee_saved_registers 'int f(void) __attribute__((no_callee_saved_registers));'
refuse preserve_none 'int f(void) __attribute__((preserve_none));'
refuse force_align_arg_pointer 'int f(void) __attribute__((force_align_arg_pointer));'
refuse ms_struct 'struct __attribute__((ms_struct)) S { char c; int b : 3; };'

# --- class 4: not a GCC attribute -> "attribute directive ignored" ---------
# `nodiscard`/`maybe_unused`/`unused_result` are C23 *standard* attributes:
# gcc-16.2 answers `__attribute__((nodiscard))` with the unknown-name wording
# (measured), so they belong here and not in the benign table.
for a in frobnicate nosuchthing quick_maths nodiscard maybe_unused unused_result; do
    expect "directive-ignored:$a" "'$a' attribute directive ignored" \
        "int f(void) __attribute__(($a));"
done

# --- class 5: the __name__ spelling is normalized in the message -----------
expect "spelling:normalized" "'stdcall' attribute ignored" \
    'int f(void) __attribute__((__stdcall__));' 

# --- class 6: C23 [[...]] -------------------------------------------------
expect "c23:unknown" "'frobnicate' attribute ignored" \
    'int q [[frobnicate]];'
expect "c23:deprecated" EMPTY 'int q [[deprecated]];'

# --- the inventory diagnostic is tagged -Wattributes -----------------------
# -Werror=attributes is itself an explicit request, so it must promote the
# inventory warning to an error; -Wno-attributes after it silences the class.
printf '%s\n' 'int f(void) __attribute__((stdcall));' >"$TMP/t.c"
if "$CCC" -fsyntax-only -Werror=attributes "$TMP/t.c" >/dev/null 2>&1; then
    echo "FAIL: -Werror=attributes did not make the inventory an error" >&2
    fail=1
fi
if ! "$CCC" -fsyntax-only -Wattributes -Wno-attributes "$TMP/t.c" 2>&1 \
        | grep -c . >/dev/null; then :; else
    echo "FAIL: -Wno-attributes did not silence the inventory diagnostic" >&2
    fail=1
fi

if ((fail)); then
    echo "check_attribute_diagnostics: FAILED" >&2
    exit 1
fi
echo "check_attribute_diagnostics: PASSED"
