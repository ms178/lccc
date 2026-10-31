"""SV-01 reproducer matrix: 7 interleaved-access loop shapes x -O1/-O2/-O3 compared with gcc -O0.
Usage: LCCC=/path/to/lccc python3 tests/simd/aos_pair_shapes.py   (writes scratch files in the cwd)
Expected today: a_swapadd and c_cmul crash (rc 139) at -O2/-O3; everything else ok."""
import subprocess, os
import sys
L = os.environ.get("LCCC", "lccc")
T = r'''#include <stdio.h>
#define N 64
static float x[3*N+8], y[3*N+8], o[3*N+8];
__attribute__((noinline)) void k(void){ for(int i=0;i<N;i++){ BODY } }
int main(void){ for(int i=0;i<3*N;i++){x[i]=i*0.5f;y[i]=i*0.25f+1;} k(); double s=0; for(int i=0;i<3*N;i++) s+=o[i]*(i%7+1); printf("%.3f\n",s); return 0; }
'''
bodies = {
 "a_swapadd": "o[2*i]=x[2*i]+y[2*i+1]; o[2*i+1]=x[2*i+1]+y[2*i];",
 "b_mul2": "o[2*i]=x[2*i]*y[2*i]; o[2*i+1]=x[2*i+1]*y[2*i+1];",
 "c_cmul": "float ar=x[2*i],ai=x[2*i+1],br=y[2*i],bi=y[2*i+1]; o[2*i]=ar*br-ai*bi; o[2*i+1]=ar*bi+ai*br;",
 "d_split": "float ar=x[2*i],ai=x[2*i+1],br=y[2*i],bi=y[2*i+1]; o[2*i]=ar*br; o[2*i+1]=ai*bi;",
 "e_stride3": "o[3*i]=x[3*i]+y[3*i+2]; o[3*i+1]=x[3*i+1]; o[3*i+2]=y[3*i];",
 "f_copy2": "o[2*i]=x[2*i]; o[2*i+1]=x[2*i+1];",
 "g_add_aos": "o[2*i]=x[2*i]+y[2*i]; o[2*i+1]=x[2*i+1]+y[2*i+1];",
}
for name, body in bodies.items():
    src = T.replace("BODY", body)
    open(name + ".c", "w").write(src)
    subprocess.run(f"gcc -O0 {name}.c -o g_{name}", shell=True, capture_output=True)
    ref = subprocess.run(f"./g_{name}", shell=True, capture_output=True, text=True).stdout.strip()
    res = []
    for o in ("-O1", "-O2", "-O3"):
        subprocess.run(f"{L} {o} {name}.c -o l_{name}", shell=True, capture_output=True)
        p = subprocess.run(f"./l_{name}", shell=True, capture_output=True, text=True)
        got = p.stdout.strip()
        res.append(f"{o}:{'ok' if got == ref else 'BAD(rc=%d %s)' % (p.returncode, got[:12])}")
    print(f"{name:12s} " + "  ".join(res))
