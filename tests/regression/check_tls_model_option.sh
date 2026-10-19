#!/usr/bin/env bash
# Compilation-wide and per-symbol ELF TLS model contract. Dynamic models use
# the GNU2 TLS descriptor ABI on x86-64: its resolver preserves allocator homes,
# supports dlopen after threads exist, and remains linker-relaxable.
set -euo pipefail
CCC=${CCC:-target/fastbuild/lccc}
GCC=${GCC:-gcc}
READELF=${READELF:-readelf}
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

cat >"$tmp/models.c" <<'C'
static __thread int local_tls;
extern __thread int external_tls;
int *local_addr(void) { return &local_tls; }
int *external_addr(void) { return &external_tls; }
C

compile_relocs() {
    local model=$1 output=$2
    "$CCC" -O2 -fPIC -ftls-model="$model" -c "$tmp/models.c" -o "$output"
    "$READELF" -rW "$output"
}

"$CCC" -O2 -fPIC -c "$tmp/models.c" -o "$tmp/auto-pic.o"
auto_pic=$("$READELF" -rW "$tmp/auto-pic.o")
[[ $(grep -c 'R_X86_64_GOTPC32_TLSDESC' <<<"$auto_pic") -eq 2 ]]
"$CCC" -O2 -fPIE -c "$tmp/models.c" -o "$tmp/auto-pie.o"
auto_pie=$("$READELF" -rW "$tmp/auto-pie.o")
grep -Eq 'R_X86_64_TPOFF32.* local_tls ' <<<"$auto_pie"
grep -Eq 'R_X86_64_GOTTPOFF.* external_tls ' <<<"$auto_pie"

le=$(compile_relocs local-exec "$tmp/le.o")
ie=$(compile_relocs initial-exec "$tmp/ie.o")
ld=$(compile_relocs local-dynamic "$tmp/ld.o")
gd=$(compile_relocs global-dynamic "$tmp/gd.o")
grep -q 'R_X86_64_TPOFF32' <<<"$le"
! grep -Eq 'GOTTPOFF|TLSDESC' <<<"$le"
grep -q 'R_X86_64_GOTTPOFF' <<<"$ie"
! grep -Eq 'TPOFF32|TLSDESC' <<<"$ie"
for relocs in "$ld" "$gd"; do
    grep -q 'R_X86_64_GOTPC32_TLSDESC' <<<"$relocs"
    grep -q 'R_X86_64_TLSDESC_CALL' <<<"$relocs"
    ! grep -Eq 'R_X86_64_(TPOFF32|GOTTPOFF)' <<<"$relocs"
done
"$CCC" -O2 -fPIC -ftls-model=global-dynamic -S "$tmp/models.c" -o "$tmp/gd.s"
# A frame-less leaf enters with rsp == 8 (mod 16); the real descriptor call
# must retain the minimal alignment frame through the late frame compactor.
sed -n '/^local_addr:/,/^\.size local_addr/p' "$tmp/gd.s" >"$tmp/gd-fn.s"
grep -q 'subq \$8, %rsp' "$tmp/gd-fn.s"
grep -q 'call .*@TLSCALL' "$tmp/gd-fn.s"
! grep -q 'lccc_tls_call' "$tmp/gd.s"

# A source attribute overrides the command line independently for every symbol.
cat >"$tmp/attrs.c" <<'C'
extern __thread int a __attribute__((tls_model("local-exec")));
extern __thread int b __attribute__((tls_model("initial-exec")));
extern __thread int c __attribute__((tls_model("local-dynamic")));
extern __thread int d __attribute__((tls_model("global-dynamic")));
int *fa(void){return &a;} int *fb(void){return &b;}
int *fc(void){return &c;} int *fd(void){return &d;}
C
"$CCC" -O2 -fPIC -ftls-model=global-dynamic -c "$tmp/attrs.c" -o "$tmp/attrs.o"
attrs=$("$READELF" -rW "$tmp/attrs.o")
grep -Eq 'R_X86_64_TPOFF32.* a ' <<<"$attrs"
grep -Eq 'R_X86_64_GOTTPOFF.* b ' <<<"$attrs"
grep -Eq 'R_X86_64_GOTPC32_TLSDESC.* c ' <<<"$attrs"
grep -Eq 'R_X86_64_GOTPC32_TLSDESC.* d ' <<<"$attrs"

# An explicit model on a later redeclaration supersedes an earlier explicit
# model, matching GCC. Test both directions so first-declaration wins cannot
# accidentally satisfy the contract.
cat >"$tmp/redecl.c" <<'C'
extern __thread int a __attribute__((tls_model("initial-exec")));
extern __thread int a __attribute__((tls_model("local-exec")));
extern __thread int b __attribute__((tls_model("local-exec")));
extern __thread int b __attribute__((tls_model("initial-exec")));
int *fa(void) { return &a; }
int *fb(void) { return &b; }
C
"$CCC" -O2 -fPIC -c "$tmp/redecl.c" -o "$tmp/redecl.o"
redecl=$("$READELF" -rW "$tmp/redecl.o")
grep -Eq 'R_X86_64_TPOFF32.* a ' <<<"$redecl"
grep -Eq 'R_X86_64_GOTTPOFF.* b ' <<<"$redecl"

cat >"$tmp/bad-attr.c" <<'C'
extern __thread int bad __attribute__((tls_model("bogus")));
C
if "$CCC" -c "$tmp/bad-attr.c" -o "$tmp/bad-attr.o" 2>"$tmp/bad-attr.err"; then
    echo 'FAIL: unknown declaration TLS model was accepted' >&2
    exit 1
fi
grep -q "unknown TLS model 'bogus'" "$tmp/bad-attr.err"

for bad in bogus ''; do
    if "$CCC" -c -ftls-model="$bad" "$tmp/models.c" -o "$tmp/bad.o" \
        >"$tmp/bad.out" 2>"$tmp/bad.err"; then
        echo "FAIL: unknown TLS model '$bad' was accepted" >&2
        exit 1
    fi
    grep -q "unknown TLS model '$bad'" "$tmp/bad.err"
done

# Runtime proof that the default full-PIC model is genuinely dynamic. The DSO
# is loaded only after another thread exists; Initial-Exec is not a substitute.
cat >"$tmp/dso.c" <<'C'
__thread int tls_value = 40;
int bump_tls(void) { return ++tls_value; }
int *addr_tls(void) { return &tls_value; }
C
cat >"$tmp/host.c" <<'C'
#include <dlfcn.h>
#include <pthread.h>
#include <stdio.h>
static pthread_barrier_t gate;
static int (*bump)(void); static int *(*addr)(void);
static void *worker(void *unused) {
    (void)unused; pthread_barrier_wait(&gate); pthread_barrier_wait(&gate);
    int *p = addr(); int a = bump(), b = bump();
    return (void *)(long)(a == 41 && b == 42 && *p == 42);
}
int main(int argc, char **argv) {
    pthread_t thread; void *result = 0; pthread_barrier_init(&gate, 0, 2);
    pthread_create(&thread, 0, worker, 0); pthread_barrier_wait(&gate);
    void *h = dlopen(argv[1], RTLD_NOW | RTLD_LOCAL); if (!h) return 2;
    *(void **)(&bump) = dlsym(h, "bump_tls");
    *(void **)(&addr) = dlsym(h, "addr_tls");
    pthread_barrier_wait(&gate);
    int *p = addr(); int a = bump(), b = bump(); pthread_join(thread, &result);
    if (a != 41 || b != 42 || *p != 42 || result != (void *)1) return 3;
    puts("dynamic TLS runtime: PASS"); return 0;
}
C
"$CCC" -O2 -fPIC -c "$tmp/dso.c" -o "$tmp/dso.o"
grep -q 'R_X86_64_GOTPC32_TLSDESC' < <("$READELF" -rW "$tmp/dso.o")
"$GCC" -shared "$tmp/dso.o" -o "$tmp/dso.so"
"$GCC" -O2 "$tmp/host.c" -ldl -pthread -o "$tmp/host"
timeout 30 "$tmp/host" "$tmp/dso.so"

# i386 uses the compact GNU2 descriptor sequence too. Run when multilib is usable;
# absence is an explicit skip rather than silently weakening x86-64 coverage.
if printf 'int main(void){return 0;}\n' | "$GCC" -m32 -x c - -o "$tmp/probe32" \
    >/dev/null 2>&1; then
    for spec in \
        'local-exec:R_386_TLS_LE' \
        'initial-exec:R_386_TLS_GOTIE' \
        'local-dynamic:R_386_TLS_GOTDESC' \
        'global-dynamic:R_386_TLS_GOTDESC'; do
        model=${spec%%:*}; reloc=${spec#*:}
        "$CCC" -m32 -O2 -fPIC -ftls-model="$model" -c "$tmp/models.c" \
            -o "$tmp/$model.32.o"
        relocs32=$("$READELF" -rW "$tmp/$model.32.o")
        grep -q "$reloc" <<<"$relocs32"
        if [[ $model == *dynamic ]]; then
            grep -q 'R_386_TLS_DESC_CALL' <<<"$relocs32"
        fi
    done
    cat >"$tmp/nonpic-dynamic.32.c" <<'C'
__thread unsigned value = 0x55667788u;
int main(void) { return value != 0x55667788u; }
C
    "$CCC" -m32 -O2 -fno-pic -no-pie -ftls-model=global-dynamic \
        -S "$tmp/nonpic-dynamic.32.c" -o "$tmp/nonpic-dynamic.32.s"
    grep -q 'call __x86.get_pc_thunk.bx' "$tmp/nonpic-dynamic.32.s"
    grep -q '@TLSCALL' "$tmp/nonpic-dynamic.32.s"
    "$CCC" -m32 -O2 -fno-pic -no-pie -ftls-model=global-dynamic \
        "$tmp/nonpic-dynamic.32.c" -o "$tmp/nonpic-dynamic.32"
    timeout 30 "$tmp/nonpic-dynamic.32"

    "$CCC" -m32 -O2 -fPIC -c "$tmp/dso.c" -o "$tmp/dso.32.o"
    "$GCC" -m32 -shared "$tmp/dso.32.o" -o "$tmp/dso.32.so"
    "$GCC" -m32 -O2 "$tmp/host.c" -ldl -pthread -o "$tmp/host.32"
    timeout 30 "$tmp/host.32" "$tmp/dso.32.so"
else
    echo 'SKIP: i386 TLS model objects/runtime (host multilib unavailable)'
fi

echo 'TLS model option: PASS'
