import re, subprocess, sys, os

def body_counts(path):
    cur = None
    res = {}
    order = []
    for line in open(path):
        line = line.rstrip('\n')
        m = re.match(r'^([A-Za-z_][\w.$]*):\s*(#.*|//.*)?$', line)
        if m and not line[0].isspace():
            cur = m.group(1)
            if cur not in res:
                res[cur] = []
                order.append(cur)
            continue
        if cur is None:
            continue
        s = line.strip()
        if not s or s.startswith('.') or s.startswith('#') or s.startswith('//') or s.startswith(';'):
            continue
        res[cur].append(s)
        if s.split()[0] in ('ret', 'retq'):
            cur = None
    return {k: len(v) for k, v in res.items()}

def run(driver, src, out):
    r = subprocess.run([driver, '-O2', '-S', src, '-o', out], capture_output=True, text=True)
    if r.returncode != 0:
        return None, r.stderr.strip().splitlines()[:3]
    return body_counts(out), None

if __name__ == '__main__':
    src = sys.argv[1]
    base = sys.argv[2] if len(sys.argv) > 2 else None
    new = sys.argv[3] if len(sys.argv) > 3 else None
    targets = ['', '-i686', '-arm', '-riscv']
    funcs = None
    rows = []
    for t in targets:
        name = 'x86' if t == '' else t[1:]
        nb = os.path.join(new, 'lccc' + t)
        bb = os.path.join(base, 'lccc' + t)
        n, err = run(nb, src, '/tmp/_n.s')
        if n is None:
            print(f'{name}: NEW FAILED {err}'); continue
        b, err2 = run(bb, src, '/tmp/_b.s')
        if funcs is None:
            funcs = [f for f in n if not f.startswith('_')]
        for f in funcs:
            bv = (b or {}).get(f)
            nv = n.get(f)
            rows.append((f, name, bv, nv))
    # print table: rows grouped by function
    hdr = f"{'function':24s}" + ''.join(f'{t:>18s}' for t in ['x86-64', 'i686', 'aarch64', 'riscv64'])
    print(hdr); print('-' * len(hdr))
    for f in funcs:
        cells = ''
        for t in ['x86', 'i686', 'arm', 'riscv']:
            r = [x for x in rows if x[0] == f and x[1] == t]
            if not r:
                cells += f'{"-":>18s}'; continue
            _, _, bv, nv = r[0]
            if bv is None:
                cells += f'{str(nv) + " (new)":>18s}'
            elif bv == nv:
                cells += f'{str(nv) + " =" :>18s}'
            else:
                cells += f'{f"{bv} -> {nv}":>18s}'
        print(f'{f:24s}{cells}')
