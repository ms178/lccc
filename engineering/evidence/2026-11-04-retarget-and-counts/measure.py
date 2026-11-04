import re, subprocess, sys
def body(path):
    cur=None; res={}
    for line in open(path):
        line=line.rstrip('\n')
        m=re.match(r'^([A-Za-z_][\w.$]*):\s*(#.*|//.*)?$', line)
        if m and not line[0].isspace():
            cur=m.group(1)
            if cur not in res: res[cur]=0
            continue
        if cur is None: continue
        s=line.strip()
        if not s or s.startswith('.') or s.startswith('#') or s.startswith('//') or s.startswith(';'): continue
        res[cur]+=1
        if s.split()[0] in ('ret','retq'): cur=None
    return res
funcs=['np2','np2ll','highbit','fls','clz32','ctz32','clz64','ctz64','pc32','pc64','bsw64','bsw32','ffs32','rol32','shl64v']
drivers={'x86':'lccc','i686':'lccc-i686','arm':'lccc-arm','rv':'lccc-riscv'}
src='./probe.c'
res={}
for t,d in drivers.items():
    for kind in ('base','new'):
        out=f'/tmp/{kind}-{t}.s'
        r=subprocess.run([f'./{kind}/{d}','-O2','-S',src,'-o',out],capture_output=True,text=True)
        if r.returncode: print('FAIL',kind,t,r.stderr[:200]); sys.exit(1)
        res[(kind,t)]=body(out)
hdr=f"{'fn':10s}{'x86-64':>10s}{'i686':>12s}{'aarch64':>10s}{'riscv64':>12s}"
print(hdr); print('-'*len(hdr))
tb={};tn={}
for f in funcs:
    cells=''
    for t in ['x86','i686','arm','rv']:
        b=res[('base2',t)].get(f,0); n=res[('new',t)].get(f,0)
        tb[t]=tb.get(t,0)+b; tn[t]=tn.get(t,0)+n
        cells+=(f"{b}->{n}" if b!=n else f"{n} =").rjust(12 if t in('i686','rv') else 10)
    print(f'{f:10s}{cells}')
print('-'*len(hdr))
print(f"{'TOTAL':10s}"+''.join((f"{tb[t]}->{tn[t]}" if tb[t]!=tn[t] else f"{tn[t]} =").rjust(12 if t in('i686','rv') else 10) for t in ['x86','i686','arm','rv']))
