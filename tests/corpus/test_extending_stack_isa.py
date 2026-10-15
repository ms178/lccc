"""Architectural oracle for the extending-stack forwarding opcode/view contract."""
import platform
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

OPS=[('movb',1,'al','dl'),('movw',2,'ax','dx'),
     ('movl',4,'eax','edx'),('movq',8,'rax','rdx'),('movzbw',1,'al','dx'),('movsbw',1,'al','dx'),
     ('movzbl',1,'al','edx'),('movsbl',1,'al','edx'),
     ('movzbq',1,'al','rdx'),('movsbq',1,'al','rdx'),
     ('movzwl',2,'ax','edx'),('movswl',2,'ax','edx'),
     ('movzwq',2,'ax','rdx'),('movswq',2,'ax','rdx'),
     ('movslq',4,'eax','rdx')]

def fixtures():
    asm=['.text']; declarations=[]; calls=[]; count=0
    for sw,sm,sr in [(1,'movb','al'),(2,'movw','ax'),(4,'movl','eax'),(8,'movq','rax')]:
        for op,rw,low,dst in OPS:
            if rw>sw:continue
            for same in [False,True]:
                dest=dst.replace('d','a') if same else dst
                result='rax' if same else 'rdx'
                for tag in ['before','after']:
                    name=f'{tag}_{count}'
                    declarations.append(f'extern unsigned long long {name}(unsigned long long);')
                    load='8(%rsp)' if tag=='before' else '%'+low
                    asm += [f'.globl {name}',f'.type {name},@function',name+':','subq $16,%rsp',
                            'movq %rdi,%rax','movabsq $0xa5a5a5a5a5a5a5a5,%rdx',
                            f'{sm} %{sr},8(%rsp)',f'{op} {load},%{dest}',
                            f'movq %{result},%rax','addq $16,%rsp','ret']
                calls.append(f'if(before_{count}(x)!=after_{count}(x)) return {count+1};')
                count+=1
    asm+=['.section .note.GNU-stack,"",@progbits']
    source='\n'.join(declarations)+'''\nint main(int argc,char **argv) {
 (void)argv;
 unsigned limit=argc>1?1024:65536;
 for(unsigned i=0;i<limit;i++) {
  unsigned long long values[]={i,0xfedcba9800000000ULL|i,0x80000000ULL|i,0x7fff0000ULL|i,~(unsigned long long)i};
  for(unsigned v=0;v<5;v++) { unsigned long long x=values[v];
'''+ '\n'.join(calls)+'\n } } return 0; }\n'
    return '\n'.join(asm)+'\n',source,count

class ExtendingStackIsa(unittest.TestCase):
    @unittest.skipUnless(platform.machine()=='x86_64' and shutil.which('gcc-16'),'requires x86-64 GCC16 assembler driver')
    def test_native_and_memcheck_byte_word_dword_views(self):
        assembly,source,count=fixtures()
        self.assertEqual(count,96)
        with tempfile.TemporaryDirectory(prefix='stack-ext-isa-') as td:
            d=Path(td);(d/'pair.S').write_text(assembly);(d/'check.c').write_text(source)
            build=subprocess.run(['gcc-16','-O2',str(d/'check.c'),str(d/'pair.S'),'-o',str(d/'check')],capture_output=True,text=True,timeout=30)
            self.assertEqual(build.returncode,0,build.stderr)
            run=subprocess.run([str(d/'check')],capture_output=True,timeout=30)
            self.assertEqual(run.returncode,0,run.stderr)
            if shutil.which('valgrind'):
                run=subprocess.run(['valgrind','--quiet','--error-exitcode=97',str(d/'check'),'bounded'],capture_output=True,timeout=45)
                self.assertEqual(run.returncode,0,run.stderr)

if __name__=='__main__':unittest.main()
