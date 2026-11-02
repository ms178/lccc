from pathlib import Path
import argparse, shutil, subprocess, json
frozen = Path(__file__).resolve().parent
parser = argparse.ArgumentParser(description="Replay the frozen preprocessor counterexample without modifying evidence")
parser.add_argument('--out', type=Path, required=True)
args = parser.parse_args()
r = args.out.resolve()
r.mkdir(parents=True, exist_ok=True)
for name in ('before.rs', 'utils.rs'):
 shutil.copyfile(frozen / name, r / name)
rustc = shutil.which('rustc')
if not rustc:
 raise SystemExit('rustc is required')
s=(r/'before.rs').read_text()
s=s.replace("const BLUE_PAINT_MARKER: u8 = 0x01;", "const BLUE_PAINT_MARKER: u8 = 0x01;\nconst TOKEN_BOUNDARY: u8 = 0x04;")
s=s.replace('Self::is_ppnumber_suffix(result.as_bytes(), ident)', 'Self::is_ppnumber_suffix(&bytes[..start], ident)')
a=s.index('    fn append_with_paste_guard');b=s.index('    /// After expanding',a);block=s[a:b].replace("result.push(' ');",'result.push(TOKEN_BOUNDARY as char);');s=s[:a]+block+s[b:]
s=s.replace("(b'.', b'0'..=b'9') => true,", "(b'.', b'0'..=b'9') | (b'0'..=b'9', b'.') => true,")
s=s.replace('        self.rescan_pragma_operators(result, expanding)', '        self.rescan_pragma_operators(result.replace(TOKEN_BOUNDARY as char, " "), expanding)',1)
a=s.index('fn stringify_arg');s=s[:a]+s[a:].replace('if bytes[i] == BLUE_PAINT_MARKER {','if bytes[i] == BLUE_PAINT_MARKER || bytes[i] == TOKEN_BOUNDARY {',1)
(r/'after.rs').write_text(s)
head='''#![allow(dead_code)]
mod common {pub mod fx_hash {pub type FxHashMap<K,V> = std::collections::HashMap<K,V>;pub type FxHashSet<K> = std::collections::HashSet<K>;}}
#[path="utils.rs"] mod utils;
mod text_processing {#[derive(Clone,Debug)]pub enum LineResolver {Flat(usize)} impl LineResolver {pub fn line_at(&self,_:usize)->usize {match self {Self::Flat(x)=>*x}}}}
'''
tail='''
use std::io::BufRead;
fn main(){let mut t=macros::MacroTable::new();for line in std::io::stdin().lock().lines(){let s=line.unwrap();let a:Vec<_>=s.splitn(4,'\\t').collect(); match a[0] {"o"=>t.define(macros::MacroDef{name:a[1].into(),is_function_like:false,params:vec![],is_variadic:false,has_named_variadic:false,body:a[2].into()}),"f"=>t.define(macros::MacroDef{name:a[1].into(),is_function_like:true,params:a[2].split(',').map(str::to_string).collect(),is_variadic:false,has_named_variadic:false,body:a[3].into()}),"e"=>println!("{}",t.expand_line(a[1])),_=>panic!()}}}
'''
for variant in ['before','after']:
 (r/'main.rs').write_text(head+f'#[path="{variant}.rs"] mod macros;\n'+tail)
 subprocess.run([rustc,'--edition=2024','-C','opt-level=1',str(r/'main.rs'),'-o',str(r/variant)],check=True)
defs=[('o','MAJOR','','1'),('o','MINOR','','5'),('o','PATCH','','7'),('o','I','','13'),('o','L','','27'),('o','EMPTY','',''),('o','PLUS','','+'),('o','MINUS','','-'),('f','Q','x','#x'),('f','S','x','Q(x)'),('f','F','x','x.MINOR.PATCH'),('f','CAT','x,y','x ## y'),('f','XCAT','x,y','CAT(x,y)')]
probes=['S(MAJOR.MINOR.PATCH)','S(F(MAJOR))','S(1.I)','S(15.IF)','S(0x1p2)','S(10L)','S(MAJOR.2)','S(MAJOR + PLUS)','S(PLUS+)','S(MINUS-)','S(MAJOR.MINOR . PATCH)','S(XCAT(MAJOR,MINOR))']
for a in ['MAJOR','MINOR','PATCH','1','0x1p2','10L','I']:
 for sep in ['.',' . ', '+',' + ', '-', ' - ', '/', '*']:
  for b in ['MAJOR','MINOR','PATCH','I','L']:
   probes.append('S('+a+sep+b+')')
cmds='\n'.join(('o\t'+n+'\t'+body if kind=='o' else 'f\t'+n+'\t'+params+'\t'+body) for kind,n,params,body in defs)+'\n'+'\n'.join('e\t'+p for p in probes)+'\n'
c='\n'.join('#define '+n+('('+params+')' if kind=='f' else '')+' '+body for kind,n,params,body in defs)+'\n'+'\n'.join(probes)+'\n';(r/'probes.c').write_text(c)
ref=subprocess.check_output(['gcc-16','-E','-P',str(r/'probes.c')],text=True).splitlines();assert len(ref)==len(probes)
report={}
for v in ['before','after']:
 out=subprocess.check_output([str(r/v)],input=cmds,text=True).splitlines();assert len(out)==len(ref)
 bad=[{'source':p,'gcc':x,'actual':y} for p,x,y in zip(probes,ref,out) if x!=y];report[v]={'n':len(probes),'failures':bad};print(v,len(bad),'/',len(probes),bad[:6])
(r/'report.json').write_text(json.dumps(report,indent=2)+'\n')

assert len(report['before']['failures']) == 25, 'frozen red witness changed'
assert not report['after']['failures'], 'prototype still differs from GCC'
