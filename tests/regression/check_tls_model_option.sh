#!/usr/bin/env bash
# ELF TLS model contract: -ftls-model=, -mtls-dialect=, the tls_model
# attribute, and the dynamic models at run time (GCC 16.2 parity, measured):
#
#  * The model of a symbol is max(default, -ftls-model= floor, attribute) in
#    the order global-dynamic < local-dynamic < initial-exec < local-exec.
#    The default is local-exec/initial-exec in executable code (binds locally
#    or not) and local-dynamic/global-dynamic in -fPIC code. So the option is
#    a FLOOR: an executable-local definition stays local-exec even under
#    -ftls-model=global-dynamic. glibc compiles its local TLS with
#    initial-exec and links the object into libc.so later, so the floor must
#    reach separately compiled PIC objects: a Local-Exec R_X86_64_TPOFF32
#    there makes every ELF linker reject the shared-library link.
#  * Dynamic models are emitted as TLS descriptors (x86-64/i386 `gnu2`,
#    AArch64 `desc`) or la.tls.gd + __tls_get_addr (RISC-V `trad`); they are
#    never approximated by a static model, which would assume the object is
#    loaded with the executable. The runtime legs dlopen a DSO after a second
#    thread exists, which only a genuinely dynamic access survives.
#  * A later explicit tls_model on a redeclaration replaces an earlier one.
#  * Diagnostics use GCC's wording (each one measured against gcc-16).
set -euo pipefail
CCC=${CCC:-target/fastbuild/lccc}
GCC=${GCC:-gcc}
READELF=${READELF:-readelf}
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

fail() { echo "FAIL: $*" >&2; exit 1; }
has() { grep -qx "$2" <<<"$1"; }

cat >"$tmp/tls.c" <<'EOF'
static __thread int local_tls;
int *address(void) { return &local_tls; }
int read_tls(void) { return local_tls; }
void write_tls(int value) { local_tls = value; }
EOF
cat >"$tmp/models.c" <<'EOF'
static __thread int local_tls;
extern __thread int external_tls;
int *local_addr(void) { return &local_tls; }
int *external_addr(void) { return &external_tls; }
EOF

# relocs <src> <name> <flags...>: relocation types of <src>.c.
relocs() {
    local src=$1 name=$2; shift 2
    "$CCC" -O2 "$@" -c "$tmp/$src.c" -o "$tmp/$name.o"
    "$READELF" -rW "$tmp/$name.o" | awk '/R_X86_64_|R_386_/ {print $3}' | sort -u
}
# symrelocs <src> <flags...>: "TYPE SYMBOL" pairs of <src>.c's .text relocations.
symrelocs() {
    local src=$1; shift
    "$CCC" -O2 "$@" -c "$tmp/$src.c" -o "$tmp/$src.o"
    "$READELF" -rW "$tmp/$src.o" |
        awk '/^Relocation section/ {t = ($3 == "'"'"'.rela.text'"'"'")} t && /R_X86_64_/ {print $3, $5}' |
        sort -u
}

# --- x86-64 defaults: executable code (LE local, IE extern), -fPIC (dynamic).
r=$(symrelocs models -fPIE)
has "$r" "R_X86_64_TPOFF32 local_tls" || fail "-fPIE: local_tls not local-exec ($r)"
has "$r" "R_X86_64_GOTTPOFF external_tls" || fail "-fPIE: external_tls not initial-exec ($r)"
r=$(symrelocs models -fPIC)
for v in local_tls external_tls; do
    has "$r" "R_X86_64_GOTPC32_TLSDESC $v" || fail "-fPIC: $v has no descriptor ($r)"
done

# --- The floor, per model, on a static and an extern symbol (-fPIC).
r=$(relocs models le -fPIC -ftls-model=local-exec)
has "$r" R_X86_64_TPOFF32 || fail "-ftls-model=local-exec: no TPOFF32 ($r)"
grep -Eq 'GOTTPOFF|TLSDESC' <<<"$r" && fail "-ftls-model=local-exec: weaker model left ($r)"
r=$(relocs models ie -fPIC -ftls-model=initial-exec)
has "$r" R_X86_64_GOTTPOFF || fail "-ftls-model=initial-exec: no GOTTPOFF ($r)"
grep -Eq 'TPOFF32|TLSDESC' <<<"$r" && fail "-ftls-model=initial-exec: wrong model ($r)"
for flags in "-fPIC" "-fPIC -ftls-model=global-dynamic" "-fPIC -ftls-model=local-dynamic" \
             "-fPIC -mtls-dialect=gnu2"; do
    # shellcheck disable=SC2086 # word splitting of the flag list is intended
    r=$(relocs tls dyn $flags)
    has "$r" R_X86_64_GOTPC32_TLSDESC || fail "$flags: no GOTPC32_TLSDESC ($r)"
    has "$r" R_X86_64_TLSDESC_CALL || fail "$flags: no TLSDESC_CALL ($r)"
    if has "$r" R_X86_64_TPOFF32 || has "$r" R_X86_64_GOTTPOFF; then
        fail "$flags: dynamic-model object uses a static TLS relocation ($r)"
    fi
done
r=$(relocs tls floor -fPIE -ftls-model=global-dynamic)
has "$r" R_X86_64_TPOFF32 || fail "-fPIE -ftls-model=global-dynamic lowered local-exec ($r)"
has "$r" R_X86_64_GOTPC32_TLSDESC && fail "-fPIE floor produced a descriptor ($r)"
r=$(relocs tls last -fPIC -ftls-model=local-exec -ftls-model=initial-exec)
has "$r" R_X86_64_GOTTPOFF || fail "last -ftls-model did not win ($r)"

# A frame-less leaf enters with rsp == 8 (mod 16); the descriptor call must
# keep the minimal alignment frame through the late frame compactor, and the
# optimizer's opaque descriptor pseudo must not reach the output.
"$CCC" -O2 -fPIC -ftls-model=global-dynamic -S "$tmp/models.c" -o "$tmp/gd.s"
sed -n '/^local_addr:/,/^\.size local_addr/p' "$tmp/gd.s" >"$tmp/gd-fn.s"
grep -q 'subq \$8, %rsp' "$tmp/gd-fn.s" || fail "descriptor call in an unaligned leaf frame"
grep -q 'call .*@TLSCALL' "$tmp/gd-fn.s" || fail "no descriptor call in local_addr"
grep -q 'lccc_tls_call' "$tmp/gd.s" && fail "descriptor pseudo leaked into the assembly"

# --- Attributes, per symbol, against the floor (GCC 16.2: a LE, b IE, c/d
# descriptors under -fPIC -ftls-model=global-dynamic).
cat >"$tmp/attrs.c" <<'EOF'
extern __thread int a __attribute__((tls_model("local-exec")));
extern __thread int b __attribute__((tls_model("initial-exec")));
extern __thread int c __attribute__((tls_model("local-dynamic")));
extern __thread int d __attribute__((tls_model("global-dynamic")));
int *fa(void){return &a;} int *fb(void){return &b;}
int *fc(void){return &c;} int *fd(void){return &d;}
EOF
r=$(symrelocs attrs -fPIC -ftls-model=global-dynamic)
has "$r" "R_X86_64_TPOFF32 a" || fail "attribute local-exec on a ($r)"
has "$r" "R_X86_64_GOTTPOFF b" || fail "attribute initial-exec on b ($r)"
has "$r" "R_X86_64_GOTPC32_TLSDESC c" || fail "attribute local-dynamic on c ($r)"
has "$r" "R_X86_64_GOTPC32_TLSDESC d" || fail "attribute global-dynamic on d ($r)"

# The LAST declaration in source order decides, at file and block scope: a
# later explicit model replaces an earlier one (both directions, so "first
# declaration wins" cannot pass by accident), and a later plain declaration
# drops back to the default (GCC 16.2: global-dynamic under -fPIC).
cat >"$tmp/redecl.c" <<'EOF'
extern __thread int a __attribute__((tls_model("initial-exec")));
extern __thread int a __attribute__((tls_model("local-exec")));
extern __thread int b __attribute__((tls_model("local-exec")));
extern __thread int b __attribute__((tls_model("initial-exec")));
extern __thread int p __attribute__((tls_model("initial-exec")));
extern __thread int p;
__thread int q __attribute__((tls_model("initial-exec")));
extern __thread int q;
extern __thread int s __attribute__((tls_model("initial-exec")));
extern __thread int k;
int *fa(void) { return &a; }
int *fb(void) { return &b; }
int *fp(void) { return &p; }
int *fq(void) { return &q; }
int *fs(void) { extern __thread int s; return &s; }
int *fk(void) { extern __thread int k __attribute__((tls_model("initial-exec"))); return &k; }
EOF
r=$(symrelocs redecl -fPIC)
has "$r" "R_X86_64_TPOFF32 a" || fail "redeclaration: later local-exec on a lost ($r)"
has "$r" "R_X86_64_GOTTPOFF b" || fail "redeclaration: later initial-exec on b lost ($r)"
for v in p q s; do
    has "$r" "R_X86_64_GOTPC32_TLSDESC $v" || fail "redeclaration: plain last declaration of $v kept a model ($r)"
done
has "$r" "R_X86_64_GOTTPOFF k" || fail "redeclaration: block-scope model on k lost ($r)"

# The attribute raises one variable (-fPIC: the static ie_a goes local-dynamic
# -> IE), and a leading visibility attribute covers every declarator (-fPIE:
# hidden extern h1 AND h2 bind locally -> LE; a lost attribute on h2 shows up
# as GOTTPOFF). A plain definition after an annotated extern declaration is
# the last declaration: default model (-fPIC global-dynamic, -fPIE local-exec).
cat >"$tmp/attr.c" <<'EOF'
static __thread int ie_a __attribute__((tls_model("initial-exec")));
__attribute__((visibility("hidden"))) extern __thread int h1, h2;
extern __thread int merged __attribute__((tls_model("initial-exec")));
__thread int merged;
void set(int v) { ie_a = v; merged = v; }
int get(void) { return ie_a + h1 + h2; }
EOF
r=$(symrelocs attr -fPIC)
has "$r" "R_X86_64_GOTTPOFF ie_a" || fail "-fPIC: tls_model(\"initial-exec\") ignored ($r)"
has "$r" "R_X86_64_GOTPC32_TLSDESC merged" || fail "-fPIC: plain definition kept the model ($r)"
for v in h1 h2; do
    has "$r" "R_X86_64_GOTPC32_TLSDESC $v" || fail "-fPIC: hidden $v not local-dynamic ($r)"
done
r=$(symrelocs attr -fPIE)
for v in ie_a h1 h2 merged; do
    has "$r" "R_X86_64_TPOFF32 $v" || fail "-fPIE: $v not local-exec ($r)"
done
grep -q GOTTPOFF <<<"$r" && fail "-fPIE: per-declarator visibility lost ($r)"

# --- Runtime: the default -fPIC model is genuinely dynamic. The DSO is loaded
# only after another thread exists; Initial-Exec is not a substitute.
cat >"$tmp/dso.c" <<'EOF'
__thread int tls_value = 40;
int bump_tls(void) { return ++tls_value; }
int *addr_tls(void) { return &tls_value; }
EOF
cat >"$tmp/host.c" <<'EOF'
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
    (void)argc;
    pthread_create(&thread, 0, worker, 0); pthread_barrier_wait(&gate);
    void *h = dlopen(argv[1], RTLD_NOW | RTLD_LOCAL); if (!h) return 2;
    *(void **)(&bump) = dlsym(h, "bump_tls");
    *(void **)(&addr) = dlsym(h, "addr_tls");
    pthread_barrier_wait(&gate);
    int *p = addr(); int a = bump(), b = bump(); pthread_join(thread, &result);
    if (a != 41 || b != 42 || *p != 42 || result != (void *)1) return 3;
    return 0;
}
EOF
"$CCC" -O2 -fPIC -c "$tmp/dso.c" -o "$tmp/dso.o"
has "$("$READELF" -rW "$tmp/dso.o" | awk '/R_X86_64_/ {print $3}' | sort -u)" \
    R_X86_64_GOTPC32_TLSDESC || fail "DSO object has no descriptor"
"$GCC" -shared "$tmp/dso.o" -o "$tmp/dso.so"
"$GCC" -O2 "$tmp/host.c" -ldl -pthread -o "$tmp/host"
timeout 30 "$tmp/host" "$tmp/dso.so" || fail "x86-64 dlopen-after-thread TLS run (rc=$?)"

# --- i386: the same contract with the i386 relocation names. The runtime
# legs need a usable multilib; its absence is an explicit skip.
if "$CCC" -m32 -fsyntax-only "$tmp/tls.c" 2>/dev/null; then
    for spec in local-exec:R_386_TLS_LE initial-exec:R_386_TLS_GOTIE \
                local-dynamic:R_386_TLS_GOTDESC global-dynamic:R_386_TLS_GOTDESC; do
        model=${spec%%:*}; reloc=${spec#*:}
        r=$(relocs models "$model.32" -m32 -fPIC -ftls-model="$model")
        has "$r" "$reloc" || fail "-m32 -fPIC -ftls-model=$model: no $reloc ($r)"
        if [[ $model == *dynamic ]]; then
            has "$r" R_386_TLS_DESC_CALL || fail "-m32 $model: no R_386_TLS_DESC_CALL ($r)"
        fi
    done
    r=$(relocs tls d32 -m32 -fPIC)
    has "$r" R_386_TLS_GOTDESC || fail "-m32 -fPIC: no R_386_TLS_GOTDESC ($r)"
    # Non-PIC executable code: the floor never lowers the local-exec default
    # of a definition (GCC 16.2: `%gs:value@ntpoff`), nor initial-exec of an
    # extern (`e@indntpoff`, R_386_TLS_IE: the absolute GOT-slot address).
    cat >"$tmp/np32.c" <<'EOF'
__thread unsigned value = 0x55667788u;
extern __thread int e;
int get_e(void) { return e; }
int main(void) { return value != 0x55667788u; }
EOF
    "$CCC" -m32 -O2 -fno-pic -ftls-model=global-dynamic -c "$tmp/np32.c" -o "$tmp/np32.o"
    r=$("$READELF" -rW "$tmp/np32.o" | awk '/R_386_/ {print $3, $5}' | sort -u)
    has "$r" "R_386_TLS_LE value" || fail "-m32 -fno-pic GD floor: value not local-exec ($r)"
    has "$r" "R_386_TLS_IE e" || fail "-m32 -fno-pic GD floor: e not initial-exec ($r)"
    grep -Eq 'GOTDESC|DESC_CALL' <<<"$r" && fail "-m32 -fno-pic: descriptor emitted ($r)"
    if printf 'int main(void){return 0;}\n' | "$GCC" -m32 -x c - -o "$tmp/probe32" \
        >/dev/null 2>&1; then
        sed -i '/extern __thread int e;/d; /get_e/d' "$tmp/np32.c"
        "$CCC" -m32 -O2 -fno-pic -no-pie -ftls-model=global-dynamic \
            "$tmp/np32.c" -o "$tmp/np32"
        timeout 30 "$tmp/np32" || fail "-m32 -fno-pic local-exec run (rc=$?)"
        "$CCC" -m32 -O2 -fPIC -c "$tmp/dso.c" -o "$tmp/dso.32.o"
        "$GCC" -m32 -shared "$tmp/dso.32.o" -o "$tmp/dso.32.so"
        "$GCC" -m32 -O2 "$tmp/host.c" -ldl -pthread -o "$tmp/host.32"
        timeout 30 "$tmp/host.32" "$tmp/dso.32.so" || fail "i386 dlopen-after-thread TLS run (rc=$?)"
        # -O2 direct %gs: folds (omitted TLS roots): IE loads and LE loads/
        # stores of every narrow width, incl. a byte store whose value may sit
        # in %esi/%edi (no low-byte form: staged through %eax), in PIE and
        # non-PIC builds. Self-checking; GCC 16.2 validates the expectations.
        # The folded functions must never materialize the thread pointer.
        cat >"$tmp/fold_def.c" <<'EOF'
__thread int e = 41;
__thread signed char ec = -3;
__thread unsigned short eh = 65000;
EOF
        cat >"$tmp/fold_main.c" <<'EOF'
extern __thread int e;
extern __thread signed char ec;
extern __thread unsigned short eh;
static __thread int s = 7;
static __thread signed char sc;
static __thread unsigned char sb;
static __thread unsigned short sh;
__attribute__((noinline)) int opaque(int x) { __asm__("" : "+r"(x)); return x; }
__attribute__((noinline)) int ie_sum(void) { return e + ec + eh; }
__attribute__((noinline)) void le_set(int v) {
    s = v; sc = (signed char)v; sb = (unsigned char)(v + 100); sh = (unsigned short)(v * 1000);
}
__attribute__((noinline)) int le_get(void) { return s + sc + sb + sh; }
int main(void) {
    if (ie_sum() != 65038) return 1;
    if (le_get() != 7) return 2;
    le_set(opaque(-56));
    if (le_get() != -56 - 56 + 44 + 9536) return 3;
    e = opaque(5);
    if (ie_sum() != 5 - 3 + 65000) return 4;
    return 0;
}
EOF
        "$GCC" -m32 -O2 "$tmp/fold_main.c" "$tmp/fold_def.c" -o "$tmp/fold.gcc"
        timeout 30 "$tmp/fold.gcc" || fail "i386 fold leg: GCC reference run (rc=$?)"
        for pic in "" "-fno-pic -no-pie"; do
            # shellcheck disable=SC2086 # $pic is a deliberate flag list
            "$CCC" -m32 -O2 $pic "$tmp/fold_main.c" "$tmp/fold_def.c" -o "$tmp/fold"
            timeout 30 "$tmp/fold" || fail "i386 -O2 ${pic:-PIE} TLS fold run (rc=$?)"
            # shellcheck disable=SC2086
            "$CCC" -m32 -O2 $pic -S "$tmp/fold_main.c" -o "$tmp/fold.s"
            tp=$(awk '/^(ie_sum|le_set|le_get):/{f=1} f&&/%gs:0,/{print} f&&/^[[:space:]]*ret/{f=0}' "$tmp/fold.s")
            [ -z "$tp" ] || fail "i386 -O2 ${pic:-PIE}: folded TLS access materialized %gs:0: $tp"
            grep -q 'movl e@\(GOTNTPOFF(%ebx)\|INDNTPOFF\), %' "$tmp/fold.s" \
                || fail "i386 -O2 ${pic:-PIE}: no initial-exec direct load"
            grep -q '%gs:sb@NTPOFF$' "$tmp/fold.s" || fail "i386 -O2 ${pic:-PIE}: no LE byte store"
        done
    else
        echo "SKIP: i386 TLS runtime legs (host multilib unavailable)"
    fi
fi

# --- AArch64 / RISC-V (lccc's own assemblers; binutils readelf names the
# relocations of any ELF machine). One static, one extern TLS variable, and an
# extern non-TLS object whose -fPIC address must come from the GOT.
cat >"$tmp/x.c" <<'EOF'
static __thread int loc;
extern __thread int ext;
extern int ext_obj;
int sum(void) { return loc + ext; }
int *obj(void) { return &ext_obj; }
EOF
# xrelocs <ccc> <flags...>: relocation types of x.c compiled by <ccc>.
xrelocs() {
    local cc=$1; shift
    "$cc" -O2 "$@" -c "$tmp/x.c" -o "$tmp/x.o"
    "$READELF" -rW "$tmp/x.o" | awk '/R_AARCH64_|R_RISCV_/ {print $3}' | sort -u
}
none_of() { # none_of <relocs> <what> <ERE>
    if grep -qE "$3" <<<"$1"; then fail "$2: unexpected $(grep -E "$3" <<<"$1" | tr '\n' ' ')"; fi
}
arm=$(dirname "$CCC")/lccc-arm
if [ -x "$arm" ]; then
    r=$(xrelocs "$arm" -fPIC)
    for t in R_AARCH64_TLSDESC_ADR_PAGE21 R_AARCH64_TLSDESC_LD64_LO12 \
             R_AARCH64_TLSDESC_ADD_LO12 R_AARCH64_TLSDESC_CALL R_AARCH64_LD64_GOT_LO12_NC; do
        has "$r" $t || fail "aarch64 -fPIC: no $t ($r)"
    done
    none_of "$r" "aarch64 -fPIC" 'TLSLE|TLSIE'
    r=$(xrelocs "$arm" -fPIE)
    for t in R_AARCH64_TLSLE_ADD_TPREL_HI12 R_AARCH64_TLSLE_ADD_TPREL_LO12_NC \
             R_AARCH64_TLSIE_ADR_GOTTPREL_PAGE21 R_AARCH64_TLSIE_LD64_GOTTPREL_LO12_NC; do
        has "$r" $t || fail "aarch64 -fPIE: no $t ($r)"
    done
    none_of "$r" "aarch64 -fPIE" 'TLSDESC'
    r=$(xrelocs "$arm" -fPIC -ftls-model=initial-exec)
    has "$r" R_AARCH64_TLSIE_ADR_GOTTPREL_PAGE21 || fail "aarch64 IE floor: no GOTTPREL ($r)"
    none_of "$r" "aarch64 -fPIC IE floor" 'TLSDESC|TLSLE'
    "$arm" -mtls-dialect=desc -fsyntax-only "$tmp/x.c" || fail "aarch64 rejected its native dialect"
else
    echo "SKIP: aarch64 legs ($arm not built)"
fi
rv=$(dirname "$CCC")/lccc-riscv
if [ -x "$rv" ]; then
    r=$(xrelocs "$rv" -fPIC)
    for t in R_RISCV_TLS_GD_HI20 R_RISCV_CALL_PLT R_RISCV_GOT_HI20; do
        has "$r" $t || fail "riscv -fPIC: no $t ($r)"
    done
    none_of "$r" "riscv -fPIC" 'TPREL|TLS_GOT_HI20|R_RISCV_PCREL_HI20'
    r=$(xrelocs "$rv" -fPIE)
    for t in R_RISCV_TPREL_HI20 R_RISCV_TPREL_ADD R_RISCV_TPREL_LO12_I R_RISCV_TLS_GOT_HI20; do
        has "$r" $t || fail "riscv -fPIE: no $t ($r)"
    done
    none_of "$r" "riscv -fPIE" 'TLS_GD_HI20'
    r=$(xrelocs "$rv" -fPIC -ftls-model=initial-exec)
    has "$r" R_RISCV_TLS_GOT_HI20 || fail "riscv IE floor: no TLS_GOT_HI20 ($r)"
    none_of "$r" "riscv -fPIC IE floor" 'TLS_GD_HI20|TPREL'
    "$rv" -mtls-dialect=trad -fsyntax-only "$tmp/x.c" || fail "riscv rejected its native dialect"
else
    echo "SKIP: riscv legs ($rv not built)"
fi

# --- Diagnostics (GCC 16.2 text). expect_diag <status> <fragment> <src> <flags...>
expect_diag() {
    local status=$1 want=$2 src=$3; shift 3
    local rc=0
    "$CCC" "$@" -c "$tmp/$src" -o "$tmp/diag.o" >"$tmp/diag.out" 2>"$tmp/diag.txt" || rc=$?
    if [ "$status" = error ] && [ "$rc" -eq 0 ]; then fail "$* $src was accepted"; fi
    if [ "$status" = warning ] && [ "$rc" -ne 0 ]; then fail "$* $src failed: $(cat "$tmp/diag.txt")"; fi
    grep -qF -- "$want" "$tmp/diag.txt" || fail "$* $src: expected \"$want\", got: $(cat "$tmp/diag.txt")"
}
expect_diag error "unknown TLS model 'bogus'; valid arguments to '-ftls-model=' are: global-dynamic initial-exec local-dynamic local-exec" tls.c -ftls-model=bogus
expect_diag error "missing argument to '-ftls-model='" tls.c -ftls-model=
expect_diag error "unrecognized argument in option '-mtls-dialect=foo'" tls.c -mtls-dialect=foo
expect_diag error "unrecognized argument in option '-mtls-dialect=desc'" tls.c -mtls-dialect=desc
# -m32 after the option still selects the i386 value set.
expect_diag error "unrecognized argument in option '-mtls-dialect=trad'" tls.c -mtls-dialect=trad -m32
cat >"$tmp/bad-attr.c" <<'EOF'
extern __thread int bad __attribute__((tls_model("bogus")));
EOF
expect_diag error "'tls_model' argument must be one of 'local-exec', 'initial-exec', 'local-dynamic', or 'global-dynamic'" bad-attr.c
cat >"$tmp/nonstr-attr.c" <<'EOF'
__thread int x __attribute__((tls_model(1)));
EOF
expect_diag error "'tls_model' argument not a string" nonstr-attr.c
cat >"$tmp/ignored-attr.c" <<'EOF'
int y __attribute__((tls_model("initial-exec")));
int f(void) __attribute__((tls_model("initial-exec")));
EOF
expect_diag warning "'tls_model' attribute ignored because 'y' does not have thread storage duration [-Wattributes]" ignored-attr.c
expect_diag warning "'tls_model' attribute ignored because 'f' is not a variable [-Wattributes]" ignored-attr.c
"$CCC" -Wno-attributes -c "$tmp/ignored-attr.c" -o "$tmp/diag.o" 2>"$tmp/diag.txt"
[ -s "$tmp/diag.txt" ] && fail "-Wno-attributes did not silence: $(cat "$tmp/diag.txt")"
# -Werror promotes it; -Wno-error=attributes demotes it back in either order
# (GCC 16.2: the per-warning request wins over the global -Werror).
"$CCC" -Werror -c "$tmp/ignored-attr.c" -o "$tmp/diag.o" 2>"$tmp/diag.txt" \
    && fail "-Werror: the attribute warning did not fail the build"
grep -q '\[-Werror=attributes\]$' "$tmp/diag.txt" || fail "-Werror: $(cat "$tmp/diag.txt")"
for order in "-Werror -Wno-error=attributes" "-Wno-error=attributes -Werror"; do
    # shellcheck disable=SC2086 # $order is a deliberate flag list
    "$CCC" $order -c "$tmp/ignored-attr.c" -o "$tmp/diag.o" 2>"$tmp/diag.txt" \
        || fail "$order: still an error: $(cat "$tmp/diag.txt")"
    grep -q 'warning: .*\[-Wattributes\]$' "$tmp/diag.txt" || fail "$order: warning lost"
done
echo 'TLS model option: PASS'
