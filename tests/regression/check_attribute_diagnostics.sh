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
#   GCC name, ABI/layout changing  ->  silence by default (gcc implements
#                                      it); 'x' attribute ignored under an
#                                      explicit -Wattributes
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

# --- class 1: implemented -> silent -----------------------------------------
for a in packed aligned section used weak noinline noreturn const pure \
         always_inline noclone_but_unknown vector_size cleanup mode \
         visibility retained_section alias_ifunc; do
    case $a in
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

# --- class 3: GCC attribute we do not implement ----------------------------
# Measured on gcc-16.2.0: gcc *implements* every name below, so it is silent
# on a valid use and a default-on warning here would fire on correct programs
# (Windows-ABI headers, sqlite, ...) and would diverge from gcc's own output.
# lccc therefore keeps gcc's default silence and reports the inventory only
# under an explicit -Wattributes, which is what keeps a silently ignored
# ABI-changing attribute discoverable without inventing a diagnostic.
for a in ms_abi cdecl stdcall thiscall sseregparm ms_struct gcc_struct \
         interrupt musttail returns_twice target_version; do
    expect "silent-unimplemented:$a" EMPTY \
        "int f(void) __attribute__(($a));"
    expect "inventory:$a" "'$a' attribute ignored" \
        "int f(void) __attribute__(($a));" -Wattributes
    expect "inventory-off:$a" EMPTY \
        "int f(void) __attribute__(($a));" -Wattributes -Wno-attributes
done

# --- class 4: not a GCC attribute -> "attribute directive ignored" ---------
# `nodiscard`/`maybe_unused`/`unused_result` are C23 *standard* attributes:
# gcc-16.2 answers `__attribute__((nodiscard))` with the unknown-name wording
# (measured), so they belong here and not in the benign table.
for a in frobnicate nosuchthing quick_maths nodiscard maybe_unused unused_result; do
    expect "directive-ignored:$a" "'$a' attribute directive ignored" \
        "int f(void) __attribute__(($a));"
done

# --- class 5: the __name__ spelling is normalized in the message -----------
expect "spelling:normalized" "'ms_abi' attribute ignored" \
    'int f(void) __attribute__((__ms_abi__));' -Wattributes

# --- class 6: C23 [[...]] -------------------------------------------------
expect "c23:unknown" "'frobnicate' attribute ignored" \
    'int q [[frobnicate]];'
expect "c23:deprecated" EMPTY 'int q [[deprecated]];'

# --- the inventory diagnostic is tagged -Wattributes -----------------------
# -Werror=attributes is itself an explicit request, so it must promote the
# inventory warning to an error; -Wno-attributes after it silences the class.
printf '%s\n' 'int f(void) __attribute__((ms_abi));' >"$TMP/t.c"
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
