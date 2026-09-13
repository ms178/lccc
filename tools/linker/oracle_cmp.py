#!/usr/bin/env python3
"""Static code-quality comparison: lccc-i686 vs gcc -m32 -O2 per benchmark TU."""
import os, re, subprocess, sys, glob
REPO="/home/user/lccc"; L=os.path.join(REPO,"target/fastbuild/lccc-i686")
INSN=re.compile(r"^\s+[a-z][a-z0-9]*\b")
def count(path):
    n=0
    for l in open(path,errors="replace"):
        if INSN.match(l): n+=1
    return n
rows=[]
for src in sorted(glob.glob(os.path.join(REPO,"tests/benchmark/programs/*.c"))):
    a="/var/tmp/oc_l.s"; b="/var/tmp/oc_g.s"
    r1=subprocess.run([L,"-O2","-S","-o",a,src],capture_output=True,text=True)
    r2=subprocess.run(["gcc","-m32","-O2","-S","-o",b,src],capture_output=True,text=True)
    if r1.returncode!=0 or r2.returncode!=0: continue
    nl=count(a); ng=count(b)
    rows.append((os.path.basename(src)[:-2], nl, ng, nl-ng, (nl/ng if ng else 0)))
rows.sort(key=lambda r:-r[4])
tl=sum(r[1] for r in rows); tg=sum(r[2] for r in rows)
print(f"{'benchmark':32s} {'lccc':>7s} {'gcc-m32':>8s} {'delta':>7s} {'ratio':>7s}")
for r in rows[:16]:
    print(f"{r[0]:32s} {r[1]:7d} {r[2]:8d} {r[3]:+7d} {r[4]:7.3f}")
print(f"{'...':32s}")
for r in rows[-6:]:
    print(f"{r[0]:32s} {r[1]:7d} {r[2]:8d} {r[3]:+7d} {r[4]:7.3f}")
print(f"\nTOTAL over {len(rows)} benchmarks: lccc={tl} gcc={tg}  ratio={tl/tg:.4f}  ({'lccc bigger' if tl>tg else 'lccc smaller'} by {abs(tl-tg)})")
worse=[r for r in rows if r[4]>1.05]; better=[r for r in rows if r[4]<0.95]
print(f"  >5% worse than gcc: {len(worse)}   >5% better: {len(better)}   within 5%: {len(rows)-len(worse)-len(better)}")
