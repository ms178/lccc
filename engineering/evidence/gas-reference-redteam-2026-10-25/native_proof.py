"""Independent native C/GAS evidence; run from the repository root.

Uses the existing Godbolt local compile helper, not a remote CE claim.
The baseline is an explicit saved distiller script (never fetched implicitly).
"""
import argparse
import sys,itertools,subprocess,json,hashlib,importlib.util
from pathlib import Path
sys.path.insert(0,'scripts');import godbolt,gas_encoding_crossref as gas
parser=argparse.ArgumentParser(description=__doc__)
parser.add_argument('--out',type=Path,required=True)
parser.add_argument('--baseline',type=Path,required=True)
parser.add_argument('--gcc',default='gcc-16')
parser.add_argument('--clang',default='clang-23')
parser.add_argument('--lccc',default='target/fastbuild/lccc')
parser.add_argument('--assembler',default='as')
parser.add_argument('--objdump',default='objdump')
args=parser.parse_args()
out=args.out.resolve();out.mkdir(parents=True,exist_ok=True)
ops=['+','*','<<','|']
expr=[f'{a} {x} {b} {y} {c}' for a,b,c in itertools.product(range(4),repeat=3) for x,y in itertools.product(ops,repeat=2)]
source='#include <stdio.h>\nint main(void){\n'+''.join(f'printf("%llu\\n",(unsigned long long)({e}));\n' for e in expr)+'return 0;}\n'
p=out/'expressions.c';p.write_text(source);expected=''.join(f'{gas.eval_expr(e)}\n' for e in expr).encode();records={}
for name,cc in [('gcc',args.gcc),('clang',args.clang),('lccc',args.lccc)]:
 flags='-O2 -fuse-ld=mold'
 if name=='lccc':flags='-O2'
 lines=godbolt._compile_local(cc,p,flags)
 (out/(name+'.s')).write_text('\n'.join(lines)+'\n')
 binary=out/(name+'-expressions')
 cmd=[cc,*flags.split(),str(p),'-o',str(binary)]
 r=subprocess.run(cmd,capture_output=True,check=True)
 r=subprocess.run([str(binary)],capture_output=True,check=True);assert r.stdout==expected,name
 records[name]={'command':cmd,'version':subprocess.check_output([cc,'--version'],text=True).splitlines()[0],'stdout_sha256':hashlib.sha256(r.stdout).hexdigest(),'assembly_sha256':hashlib.sha256((out/(name+'.s')).read_bytes()).hexdigest(),'exit_code':r.returncode}
oldspec=importlib.util.spec_from_file_location('baseline',args.baseline);old=importlib.util.module_from_spec(oldspec);oldspec.loader.exec_module(old)
wrong=sum(old.eval_expr(e)!=gas.eval_expr(e) for e in expr)
records['expressions']={'count':len(expr),'old_disagreements':wrong,'source_sha256':hashlib.sha256(p.read_bytes()).hexdigest()}
# Query actual latest GAS, not the distilled table. Rejecting a forced VEX
# form and invented mnemonic is independent of the Python model.
as_=args.assembler;objdump=args.objdump;cases={
 'evex_fpclass':('vfpclassps $1, %xmm0, %k1',True,'62 f3 7d 08 66 c8 01'),
 'evex_blend':('vpblendmb %xmm0, %xmm1, %xmm2',True,'62 f2 75 08 66 d0'),
 'bmi_andn':('andn %rdx, %rcx, %rax',True,'c4 e2 f0 f2 c2'),
 'vex_sqrt':('vsqrtps %xmm0, %xmm1',True,'c5 f8 51 c8'),
 'forced_vex_fpclass':('{vex} vfpclassps $1, %xmm0, %k1',False,None),
 'forced_vex_blend':('{vex} vpblendmb %xmm0, %xmm1, %xmm2',False,None),
 **{n:(f'{n} %rdx, %rcx, %rax' if n in ('vandn','vshrx') else f'{n} %rdx, %rax',False,None) for n in ('vandn','vblsi','vblsr','vblsmsk','vshrx')},
}
records['gas']={'version':subprocess.check_output([as_,'--version'],text=True).splitlines()[0],'cases':{}}
for name,(asm,ok,hex_) in cases.items():
 src=out/(name+'.s');obj=out/(name+'.o');src.write_text('.text\n'+asm+'\n');obj.unlink(missing_ok=True)
 r=subprocess.run([as_,'--64',str(src),'-o',str(obj)],capture_output=True,text=True)
 assert (r.returncode==0)==ok,(name,r.stderr)
 dis=subprocess.check_output([objdump,'-d',str(obj)],text=True) if ok else ''
 if hex_:assert hex_ in dis.replace('\t',' '),(name,dis)
 records['gas']['cases'][name]={'source':asm,'exit_code':r.returncode,'expected_success':ok,'diagnostic':r.stderr,'disassembly':dis}
(out/'native-proof.json').write_text(json.dumps(records,indent=2)+'\n');print(len(expr),'native expression cases; baseline wrong:',wrong,'; GAS:',len(cases),'positive/refusal cases')
