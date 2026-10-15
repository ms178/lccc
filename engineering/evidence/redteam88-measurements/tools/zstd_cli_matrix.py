"""Bounded real-CLI roundtrips, dictionaries, patching and malformed-frame rejection.
All successes require byte equality; decoder exits must agree with GCC16.
This is correctness evidence, not a performance benchmark.
"""
import argparse, ast, hashlib, json, re, subprocess
from pathlib import Path
p=argparse.ArgumentParser(description=__doc__)
p.add_argument('--cli',action='append',required=True,help='name=path; gcc required')
p.add_argument('--inputs',type=Path,required=True);p.add_argument('--source',type=Path,required=True)
p.add_argument('--out',type=Path,required=True);a=p.parse_args()
clis=dict(x.split('=',1) for x in a.cli)
if 'gcc' not in clis or len(clis)!=len(a.cli):p.error('unique arms and gcc required')
a.out.mkdir(parents=True,exist_ok=True)
r={'clis':{n:{'path':v,'sha256':hashlib.sha256(Path(v).read_bytes()).hexdigest()} for n,v in clis.items()},'complete':False,'metric':'CLI byte-exact correctness, NOT runtime','cases':[],'failures':[]}
def save():
 tmp=a.out/'report.tmp';tmp.write_text(json.dumps(r,indent=2)+'\n');tmp.replace(a.out/'report.json')
def run(arm,args,data=None):
 cmd=[clis[arm],'-q',*args]
 q=subprocess.run(cmd,input=data,capture_output=True,timeout=90)
 r['cases'].append({'arm':arm,'command':cmd,'returncode':q.returncode,'stdout_bytes':len(q.stdout),'stdout_sha256':hashlib.sha256(q.stdout).hexdigest(),'stderr':q.stderr.decode(errors='replace')[-1200:]})
 return q
def check(key,callback):
 try:callback()
 except Exception as e:r['failures'].append({'case':key,'error':str(e)})
 save()
def decode_all(frame,plain,opts=()):
 for arm in clis:
  q=run(arm,['-d','-c',*opts],frame)
  if q.returncode or q.stdout!=plain:raise ValueError('decode mismatch '+arm)
def roundtrip(plain,opts,decoder_opts=()):
 if '--ultra' in opts or any(x.startswith('--patch-from=') for x in opts):opts=[*opts,'--stream-size='+str(len(plain))]
 for arm in clis:
  q=run(arm,['-c',*opts],plain)
  if q.returncode:raise ValueError('compression failed '+arm)
  decode_all(q.stdout,plain,decoder_opts)
profiles=[['-1'],['-3'],['-9'],['-19'],['--ultra','-22'],['--fast=5'],['-T1'],['-T2'],['-T4'],['--long=20'],['--no-check'],['--check'],['--no-content-size'],['--rsyncable','-T2'],['--adapt=min=1,max=9','-T2'],['--target-compressed-block-size=1024'],['--format=gzip'],['--format=xz'],['--format=lzma'],['--format=lz4']]
for src in sorted(a.inputs.iterdir()):
 if not src.is_file():continue
 plain=src.read_bytes()
 for opts in profiles:check(src.name+'/'+str(opts),lambda:roundtrip(plain,opts))
 for opts in [['--stream-size='+str(len(plain))],['--size-hint='+str(len(plain))]]:check(src.name+'/'+str(opts),lambda:roundtrip(plain,opts))
# Zero/one byte and boundaries around word, SIMD, block and frame sizes.
for size in [0,1,7,8,15,16,31,32,63,64,127,128,255,256,1023,1024,65535,65536,131071,131072,131073]:
 plain=(bytes(range(256))*((size+255)//256))[:size]
 check('boundary/'+str(size),lambda:roundtrip(plain,['-3','--check']))
base=(a.inputs/'source').read_bytes();modified=bytearray(base)
for i in range(0,len(modified),4096):modified[i]^=0x5a
ref=a.out/'patch-base';ref.write_bytes(base)
check('patch-from',lambda:roundtrip(bytes(modified),['--patch-from='+str(ref)],['--patch-from='+str(ref)]))
# GCC-trained and candidate-trained dictionaries must both interoperate.
samples=a.out/'samples';samples.mkdir(exist_ok=True);names=[]
for i in range(64):
 f=samples/str(i);f.write_bytes(base[i*4096:(i+1)*4096]);names.append(str(f))
for trainer in clis:
 def dictionary_case():
  dictionary=a.out/('dict-'+trainer)
  q=run(trainer,['--train','--maxdict=8192','--dictID=178','-f','-o',str(dictionary),*names])
  if q.returncode or not dictionary.is_file():raise ValueError('dictionary training failure')
  roundtrip(base,['-3','-D',str(dictionary)],['-D',str(dictionary)])
  roundtrip(base,['-3','--no-dictID','-D',str(dictionary)],['-D',str(dictionary)])
 check('dictionary/'+trainer,dictionary_case)
frame=run('gcc',['-c','--check','--long=20'],base)
if frame.returncode:r['failures'].append({'case':'invalid-fixture','error':'GCC compression failed'})
else:
 corrupt=bytearray(frame.stdout);corrupt[-1]^=1
 for tag,blob in [('checksum',bytes(corrupt)),('truncated',frame.stdout[:-5])]:
  def reject():
   for arm in clis:
    q=run(arm,['-d','-c'],blob)
    if q.returncode==0:raise ValueError('accepted malformed frame '+arm)
  check(tag,reject)
 def memory_limit():
  for arm in clis:
   if run(arm,['-d','-c','-M1K'],frame.stdout).returncode==0:raise ValueError('memory ceiling ignored '+arm)
 check('memory-limit',memory_limit)
# Pinned upstream golden successes and malformed inputs.
for folder,valid in [('golden-decompression',True),('golden-decompression-errors',False)]:
 for path in sorted((a.source/'tests'/folder).glob('*.zst')):
  def golden():
   blob=path.read_bytes();ref=run('gcc',['-d','-c'],blob)
   if (ref.returncode==0)!=valid:raise ValueError('GCC fixture expectation failed')
   for arm in clis:
    got=run(arm,['-d','-c'],blob)
    if (got.returncode==0)!=valid or (valid and got.stdout!=ref.stdout):raise ValueError('golden mismatch '+arm)
  check(folder+'/'+path.name,golden)
# The upstream legacy fixture includes v0.4, outside this recipe's default
# ZSTD_LEGACY_LEVEL=5. Test the v0.5/v0.6/v0.7/v0.8 frames independently.
legacy=(a.source/'tests/legacy.c').read_text()
def literal(name,encoding):
 body=legacy.split('const char* const '+name+' =',1)[1].split(';',1)[0]
 return ''.join(ast.literal_eval(x) for x in re.findall(r'"(?:[^"\\]|\\.)*"',body)).encode(encoding)
compressed=literal('COMPRESSED','latin1');expected=literal('EXPECTED','utf-8')
assert len(compressed)==917 and len(expected)%5==0
part=expected[:len(expected)//5];assert part*5==expected
offsets=[compressed.find(bytes([x,0xb5,0x2f,0xfd])) for x in range(0x24,0x29)]
assert offsets[0]==0 and offsets==sorted(set(offsets)) and min(offsets)==0
r['legacy_configuration']={'minimum':5,'excluded_fixture':'v0.4.3 is below configured support'}
for i in range(1,5):
 blob=compressed[offsets[i]:offsets[i+1] if i<4 else len(compressed)]
 check('legacy/v0.'+str(i+4),lambda:decode_all(blob,part))
for arm in clis:
 def version_case():
  q=run(arm,['--version'])
  if q.returncode or b'1.5.7' not in q.stdout+q.stderr:raise ValueError('wrong version string '+arm)
 check('version/'+arm,version_case)
r['traversal_finished']=True
r['complete']=not r['failures'];save();print(json.dumps({'complete':r['complete'],'commands':len(r['cases']),'failures':r['failures']},indent=2))
raise SystemExit(not r['complete'])
