#!/usr/bin/env bash
# A call to a function whose DEFINITION carries __attribute__((noreturn)) is a
# block terminator (linux-6.18.55 init/main.c: rest_init() is defined
# `__noreturn` and called from start_kernel()). The pre-pass used to record
# only declarations, so the call fell through into following code: lccc
# emitted a dead `mfence` + epilogue after rest_init() and objtool reported
# "call to rest_init() ... missing __noreturn". Expected shape after the call:
# `ud2`, with no trailing code in the same function.
set -euo pipefail
CCC=${CCC:-./target/fastbuild/lccc}
tmp=${TMPDIR:-/tmp}/lccc-noreturn-tail.$$
trap 'rm -rf "$tmp"' EXIT
mkdir -p "$tmp"

cat >"$tmp/t.c" <<'C'
#define __noreturn __attribute__((__noreturn__))
extern void ext_work(void);
static __attribute__((noinline)) void __noreturn rest_init(void)
{
	for (;;)
		ext_work();
}
void start_k(void)
{
	ext_work();
	rest_init();
	__asm__ volatile("mfence" ::: "memory");
	ext_work();
}
C

"$CCC" -O2 -fno-pic -mcmodel=kernel -mno-red-zone -fno-asynchronous-unwind-tables \
    -fcf-protection=none -S -o "$tmp/t.s" "$tmp/t.c"

body=$(awk '$0=="start_k:"{on=1;next} on&&/^\s*\.size/{exit} on{print}' "$tmp/t.s")
insns=$(printf '%s\n' "$body" | grep -Ev '^\s*(#|$)' | sed 's/^\s*//')
expected=$'subq $8, %rsp\ncall ext_work\ncall rest_init\nud2'
if [ "$insns" != "$expected" ]; then
    echo "FAIL: start_k body is not 'call rest_init; ud2' terminated:" >&2
    printf '%s\n' "$insns" >&2
    exit 1
fi
echo "check_noreturn_definition_tail: ok"
