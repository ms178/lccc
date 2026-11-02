#!/usr/bin/env bash
# RA-01: distinguish PIE from full PIC, and omit only GlobalAddr roots whose
# every use can be reconstructed safely. The kill switch must restore the old
# fully-GOT executable lowering for controlled workload measurements.
set -euo pipefail
CCC=${CCC:-./target/release/lccc}
dir=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
src="$dir/global_addr_remat.c"
td=$(mktemp -d)
trap 'rm -rf "$td"' EXIT

"$CCC" -O2 -fPIE -S "$src" -o "$td/pie.s"
CCC_NO_GLOBAL_ADDR_REMAT=1 "$CCC" -O2 -fPIE -S "$src" -o "$td/control.s"
"$CCC" -O2 -fPIC -S "$src" -o "$td/pic.s"
cat >"$td/extern_fn_addr.c" <<'EOF'
typedef int (*fn_ptr)(int);
extern int ra01_external_function(int);
fn_ptr external_function_address(void) { return ra01_external_function; }
EOF
"$CCC" -O2 -fPIE -S "$td/extern_fn_addr.c" -o "$td/extern_fn_addr.s"

body=$(sed -n '/^global_addr_probe:/,/^\.size global_addr_probe/p' "$td/pie.s")
grep -q 'leaq ra01_bytes(%rip)' <<<"$body" || {
    echo "PIE probe did not rematerialize the byte-array base"; exit 1;
}
grep -q 'ra01_index(%rip)' <<<"$body" || {
    echo "PIE probe did not use a direct index load"; exit 1;
}
if grep -Eq 'ra01_(bytes|index|scalar)@GOTPCREL' <<<"$body"; then
    echo "ordinary PIE executable data still uses GOTPCREL"; exit 1
fi
# Escaping an address is deliberately outside the rematerialization audit, but
# PIE still materializes that root with a legal direct LEA.
escape=$(sed -n '/^global_addr_escape:/,/^\.size global_addr_escape/p' "$td/pie.s")
grep -q 'leaq ra01_scalar(%rip)' <<<"$escape" || {
    echo "escaping PIE global address lost its direct materialization"; exit 1;
}
# Weak externs can resolve to zero and therefore must retain a GOT slot in PIE.
grep -q 'ra01_missing@GOTPCREL' "$td/pie.s" || {
    echo "weak PIE extern lost required GOT indirection"; exit 1;
}
# Function addresses have no data-copy-relocation equivalent and therefore
# remain GOT-indirect when the declaration is external and interposable.
grep -q 'ra01_external_function@GOTPCREL' "$td/extern_fn_addr.s" || {
    echo "PIE extern function address lost required GOT indirection"; exit 1;
}
# Full PIC keeps all default-visibility definitions interposable.
for sym in ra01_bytes ra01_index ra01_scalar; do
    grep -q "${sym}@GOTPCREL" "$td/pic.s" || {
        echo "full PIC emitted direct access for interposable $sym"; exit 1;
    }
done
# One switch controls both direct PIE folding and root rematerialization.
for sym in ra01_bytes ra01_index ra01_scalar; do
    grep -q "${sym}@GOTPCREL" "$td/control.s" || {
        echo "kill-switch control did not restore GOT access for $sym"; exit 1;
    }
done

"$CCC" -dM -E "$src" >"$td/default.macros"
"$CCC" -fPIC -dM -E "$src" >"$td/pic.macros"
"$CCC" -fno-pic -dM -E "$src" >"$td/nopic.macros"
grep -q '^#define __PIE__ 2$' "$td/default.macros" || {
    echo "default PIE mode does not define __PIE__"; exit 1;
}
if grep -q '^#define __PIE__' "$td/pic.macros"; then
    echo "full PIC incorrectly defines __PIE__"; exit 1
fi
if grep -Eq '^#define __(PIC|PIE)__' "$td/nopic.macros"; then
    echo "-fno-pic left PIC/PIE macros enabled"; exit 1
fi
