from pathlib import Path
import argparse, shutil, subprocess, json, hashlib
frozen = Path(__file__).resolve().parent
parser = argparse.ArgumentParser(description="Replay the frozen unsafe late fuser without modifying evidence")
parser.add_argument('--out', type=Path, required=True)
args = parser.parse_args()
root = args.out.resolve()
root.mkdir(parents=True, exist_ok=True)
src = (frozen/'before-compare_branch.rs').read_text()
ty = (frozen/'before-types.rs').read_text()
rustc = shutil.which('rustc')
if not rustc:
 raise SystemExit('rustc is required')
f=src[src.index('pub(super) fn fuse_late_compare_bool_spills'):src.index('\n#[cfg(test)]',src.index('pub(super) fn fuse_late_compare_bool_spills'))].replace('pub(super)','pub').replace('crate::common::fx_hash::FxHashMap','std::collections::HashMap')
helpers=ty[ty.index('pub(super) fn parse_setcc'):ty.index('/// Check if two register names overlap')].replace('pub(super)','pub')
(root/'extract.rs').write_text(f+helpers+'\nfn main() {let p=std::env::args().nth(1).unwrap();let mut s=std::fs::read_to_string(p).unwrap();fuse_late_compare_bool_spills(&mut s);print!("{}",s);}\n')
subprocess.run([rustc,'-C','opt-level=1',str(root/'extract.rs'),'-o',str(root/'extract')],check=True)
prefix='movl $9, %eax\ncmpl %esi, %edi\nsetb %al\nmovzbl %al, %eax\nmovq %rax, -8(%rsp)\nmovsbq -8(%rsp), %rax\ntestq %rax, %rax\nje .Lzero\n'
cases={'live_rax':prefix+'ret\n.Lzero:\nret\n', 'live_test_flags':prefix+'movl $0, %eax\nsetbe %al\nret\n.Lzero:\nmovl $0, %eax\nsetbe %al\nret\n','live_stack_slot':'movq $7, -8(%rsp)\n'+prefix+'movq -8(%rsp), %rax\nret\n.Lzero:\nmovq -8(%rsp), %rax\nret\n','non_al_setcc':prefix.replace('movl $9, %eax','movl $0, %eax').replace('setb %al','setb %cl')+'movl $1, %eax\nret\n.Lzero:\nmovl $2, %eax\nret\n','non_stack_store':prefix.replace('-8(%rsp)','(%rdx)')+'movl $1, %eax\nret\n.Lzero:\nmovl $2, %eax\nret\n'}
harness='''#include <stdio.h>
#include <limits.h>
extern int orig(int,int,long*), transformed(int,int,long*);
int main(void){int v[]={INT_MIN,-2,-1,0,1,2,127,INT_MAX},n=0;for(unsigned i=0;i<8;i++)for(unsigned j=0;j<8;j++){long a=7,b=7;int x=orig(v[i],v[j],&a),y=transformed(v[i],v[j],&b);if(x!=y || a!=b)n++;}printf("%d\\n",n);return 0;}
''';(root/'driver.c').write_text(harness);rows=[]
for name,body in cases.items():
 wd=root/name;wd.mkdir(exist_ok=True)
 text='.text\n.globl orig\n.type orig,@function\norig:\n.cfi_startproc\n'+body+'.cfi_endproc\n.section .note.GNU-stack,"",@progbits\n';(wd/'orig.s').write_text(text)
 out=subprocess.check_output([str(root/'extract'),str(wd/'orig.s')],text=True).replace('orig','transformed');(wd/'transformed.s').write_text(out)
 cmd=['gcc-16','-O1',str(root/'driver.c'),str(wd/'orig.s'),str(wd/'transformed.s'),'-o',str(wd/'probe')];subprocess.run(cmd,check=True)
 q=subprocess.run(['valgrind','--quiet','--error-exitcode=99',str(wd/'probe')],capture_output=True,text=True,check=True)
 rows.append({'case':name,'pairs':64,'mismatches':int(q.stdout),'memcheck_exit':q.returncode,'stderr':q.stderr,'changed':out!=text.replace('orig','transformed'),'compiler_command':cmd})
r={'scope':'Extracted exact production late-fuser body; only visibility and hash-map type replaced; native original vs transformed GNU assembly. Not a full C-frontend reproduction.','source_sha256':hashlib.sha256(src.encode()).hexdigest(),'rows':rows};(root/'report.json').write_text(json.dumps(r,indent=2)+'\n');print(json.dumps(r,indent=2))

assert [r['mismatches'] for r in rows] == [64, 56, 64, 28, 64], 'frozen red witnesses changed'
