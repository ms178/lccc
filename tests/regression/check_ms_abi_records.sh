#!/usr/bin/env bash
# Non-kernel C-record ABI interoperability, including FP-field records that
# must return through RAX rather than their SysV SSE carrier.
set -euo pipefail
CCC=${CCC:-./target/fastbuild/lccc}
GCC=${GCC:-gcc}
tmp=$(mktemp -d)
trap 'rm -rf -- "$tmp"' EXIT
python3 - "$tmp" <<'PY'
import pathlib,sys
root=pathlib.Path(sys.argv[1])
header=['#define MS __attribute__((ms_abi))\n']
lccc=[]; gcc=[]; checks=[]
for n in (*range(1,34),64,128):
    name=f'record{n}'
    header.append(f'typedef struct {{ unsigned char bytes[{n}]; }} {name};\n')
    for side,other in ((lccc,'gcc'),(gcc,'lccc')):
        own='lccc' if other=='gcc' else 'gcc'
        side.append(f'MS {name} {own}_make{n}(long seed) {{ {name} r; for(int i=0;i<{n};i++) r.bytes[i]=(unsigned char)(seed+i); return r; }}\n')
    header.append(f'extern MS {name} gcc_make{n}(long), lccc_make{n}(long);\n')
    lccc.append(f'long call_make{n}(long seed) {{ {name} r=gcc_make{n}(seed); long sum=0; for(int i=0;i<{n};i++)sum+=r.bytes[i]; return sum; }}\n')
    header.append(f'extern long call_make{n}(long);\n')
    checks.append(f'{{ {name} r=lccc_make{n}(seed); long sum=0; for(int i=0;i<{n};i++){{if(r.bytes[i]!=(unsigned char)(seed+i))return {n};sum+=r.bytes[i];}}if(call_make{n}(seed)!=sum)return {n}; }}\n')
    if n > 0:
        header.append(f'extern MS long gcc_take{n}(long,long,long,long,{name},long), lccc_take{n}(long,long,long,long,{name},long);\n')
        body=f'(long a,long b,long c,long d,{name} r,long tail) {{ r.bytes[0]^=77;long sum=a+2*b+3*c+4*d+tail;for(int i=0;i<{n};i++)sum+=r.bytes[i];return sum; }}\n'
        lccc.append('MS long lccc_take'+str(n)+body)
        gcc.append('MS long gcc_take'+str(n)+body)
        lccc.append(f'long call_take{n}(long seed){{{name} r=gcc_make{n}(seed);return gcc_take{n}(1,2,3,4,r,7);}}\n')
        header.append(f'extern long call_take{n}(long);\n')
        checks.append(f'{{ {name} r=gcc_make{n}(seed);long expected=gcc_take{n}(1,2,3,4,r,7);if(lccc_take{n}(1,2,3,4,r,7)!=expected||call_take{n}(seed)!=expected)return 1;for(int i=0;i<{n};i++)if(r.bytes[i]!=(unsigned char)(seed+i))return 2; }}\n')
    header.append(f'extern MS long gcc_reg{n}({name},long,long,long,long), lccc_reg{n}({name},long,long,long,long);\nextern long relay_reg{n}(long);\n')
    for side,own in ((lccc,'lccc'),(gcc,'gcc')):
        side.append(f'MS long {own}_reg{n}({name} r,long b,long c,long d,long e){{r.bytes[0]^=77;long sum=b+2*c+3*d+e;for(int i=0;i<{n};i++)sum+=r.bytes[i];return sum;}}\n')
    lccc.append(f'long relay_reg{n}(long seed){{{name} r=gcc_make{n}(seed);return gcc_reg{n}(r,2,3,4,5);}}\n')
    checks.append(f'{{{name} r=gcc_make{n}(seed);long expected=gcc_reg{n}(r,2,3,4,5);if(lccc_reg{n}(r,2,3,4,5)!=expected||relay_reg{n}(seed)!=expected)return 3;for(int i=0;i<{n};i++)if(r.bytes[i]!=(unsigned char)(seed+i))return 4;}}\n')
# Force the small-record *register* convention, not just stack overflow.
for name,field in (('f32','float'),('f64','double')):
    header.append(f'typedef struct{{{field} value;}} {name};\nextern MS {name} gcc_{name}({name}),lccc_{name}({name});\nextern double relay_{name}(double);\n')
    lccc.append(f'MS {name} lccc_{name}({name} x){{x.value+=2;return x;}}\ndouble relay_{name}(double x){{{name} a={{x}};return gcc_{name}(a).value;}}\n')
    gcc.append(f'MS {name} gcc_{name}({name} x){{x.value+=2;return x;}}\n')
    checks.append(f'{{{name} x={{3.25}};if(lccc_{name}(x).value!=5.25||relay_{name}(3.25)!=5.25)return 250;}}\n')
(root/'types.h').write_text(''.join(header))
(root/'lccc.c').write_text('#include "types.h"\n'+''.join(lccc))
(root/'oracle.c').write_text('#include "types.h"\n'+''.join(gcc)+'\nint main(void){for(long seed=-3;seed<260;seed+=17){'+''.join(checks)+'}return 0;}\n')
PY
"$GCC" -O2 -c "$tmp/oracle.c" -o "$tmp/oracle.o"
for opt in -O0 -O2 -Os; do
  "$CCC" "$opt" -c "$tmp/lccc.c" -o "$tmp/lccc.o"
  "$GCC" "$tmp/lccc.o" "$tmp/oracle.o" -o "$tmp/check"
  "$tmp/check"
  echo "PASS Microsoft record ABI $opt: sizes 1..33/64/128, GP/FP records, by-reference caller copies, hidden return, register/overflow parameters"
done
