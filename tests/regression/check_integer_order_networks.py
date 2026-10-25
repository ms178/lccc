#!/usr/bin/env python3
"""Exact extremum observations and bit-reversal conjugation controls."""
import argparse
import os
from pathlib import Path
import tempfile
import re
import subprocess
from check_llvm_missed_zero_choice import run, body
from check_llvm_predicate_relations import gcc_oracle_identity
ROOT = Path(__file__).resolve().parents[2]
SOURCE = ROOT / 'tests/regression/integer_order_networks.c'


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--compiler',default=os.environ.get('CCC',str(ROOT/'target/fastbuild/lccc')))
    parser.add_argument('--gcc',default=os.environ.get('GCC','gcc-16'))
    parser.add_argument('--clang',default=os.environ.get('CLANG'))
    parser.add_argument('--arm',default=os.environ.get('CCC_ARM'))
    parser.add_argument('--allow-gcc16-snapshot',action='store_true')
    args=parser.parse_args()
    version=run([args.gcc,'-dumpfullversion']).decode().strip()
    normative=gcc_oracle_identity(version,args.allow_gcc16_snapshot)
    if args.clang and run([args.clang,'-dumpversion']).decode().strip()!='23.1.4':
        raise ValueError('exact Clang 23.1.4 required')
    env=os.environ|{'CCC_VERIFY_IR':'abort','CCC_VERIFY_REGALLOC':'1','CCC_VALIDATE_SSA':'1'}
    with tempfile.TemporaryDirectory() as tmp:
        directory=Path(tmp);expected=None
        for compiler in [args.gcc]+([args.clang] if args.clang else []):
            executable=directory/'oracle'
            run([compiler,'-O2','-Wall','-Wextra','-Werror',str(SOURCE),'-o',str(executable)])
            output=run([str(executable)])
            if expected is not None and output!=expected:raise ValueError('vendor disagreement')
            expected=output
        for flags in [[x] for x in ('-O0','-O1','-O2','-O3','-Os')]+[['-O2','-g'],['-O2','-m32']]:
            executable=directory/'native'
            run([args.compiler,*flags,str(SOURCE),'-o',str(executable)],env=env)
            if run([str(executable)])!=expected:raise ValueError(f'native mismatch: {flags}')
        executable=directory/'off'
        run([args.compiler,'-O2',str(SOURCE),'-o',str(executable)],env=env|{'CCC_DISABLE_PASSES':'bit_idioms'})
        if run([str(executable)])!=expected:raise ValueError('disabled control mismatch')
        if args.arm:
            path=directory/'arm.s'
            run([args.arm,'-O2','-DINTEGER_ORDER_ASSEMBLY','-S',str(SOURCE),'-o',str(path)],env=env)
            assembly=path.read_text()
            for name in ['reverse_left','reverse_right','reverse_not','reverse_or','reverse_and','reverse_andnot']:
                instructions=body(assembly,name)
                if any(re.match(r'rbit\b',line) for line in instructions):
                    diagnostic=subprocess.run([args.arm,'-O2','-DINTEGER_ORDER_ASSEMBLY','-S',str(SOURCE),'-o',str(path)],env=env|{'CCC_DUMP_EACH_PASS':'1','CCC_DUMP_FUNC':name+',reverse32'},capture_output=True,timeout=60)
                    stages=re.findall(r'==== IR ([^=]+) ====\n(.*?)==== END IR \1 ====',diagnostic.stderr.decode(errors='replace'),re.S)
                    selected='\n'.join(tag+'\n'+text for tag,text in stages if 'bit_reverse_ladders' in tag or 'bit_idioms' in tag)
                    raise ValueError(f'reversal survives in {name}: {instructions}; IR={selected[-24000:]}')
        print(f'PASS integer order networks: GCC {version}, normative={normative}, native modes, i386, verifiers, whole-bit_idioms-disabled control, ARM mechanism={bool(args.arm)}')


if __name__=='__main__':
    try:main()
    except Exception as error:
        detail=str(error).replace('%','%25').replace('\r','%0D').replace('\n','%0A')
        print(f'::error title=Integer order networks::{detail}')
        raise
