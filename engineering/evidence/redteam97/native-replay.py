import pathlib,subprocess,json,hashlib
r=pathlib.Path('/home/user/lccc');e=r/'engineering/evidence/redteam97';out=e/'native-tools';out.mkdir(exist_ok=True);work=pathlib.Path('/home/user/.cache/redteam94/native97');work.mkdir(exist_ok=True)
rows=[]
for family,source,driver,badexit in [('padding','caller.s','driver.c',13),('vector','vector.s','vector-driver.c',1)]:
 for stage,asm,expected in [('original',r/'tests/regression/fixtures/peephole-padding-effects'/source,0),('before',e/f'{family}-before.s',badexit)]:
  binary=work/f'{family}-{stage}';cmd=['gcc','-O2','-Werror','-no-pie',str(asm),str(r/'tests/regression/fixtures/peephole-padding-effects'/driver),'-o',str(binary)]
  build=subprocess.run(cmd,capture_output=True,text=True);assert build.returncode==0 and not build.stderr,build.stderr
  for tool in ['memcheck','callgrind']:
   log=out/f'{family}-{stage}-{tool}.txt';cmd2=['valgrind','--tool='+tool,'--error-exitcode=99','--log-file='+str(log)]
   if tool=='callgrind':cmd2+=['--callgrind-out-file='+str(out/f'{family}-{stage}.callgrind')]
   cmd2+=[str(binary)];p=subprocess.run(cmd2,capture_output=True,text=True)
   row={'family':family,'stage':stage,'tool':tool,'command':cmd2,'build':cmd,'binary_sha256':hashlib.sha256(binary.read_bytes()).hexdigest(),'expected_exit':expected,'exit':p.returncode,'stdout':p.stdout,'stderr':p.stderr,'pass':p.returncode==expected};rows.append(row);assert row['pass'],row
(out/'index.json').write_text(json.dumps(rows,indent=2));print('native tool controls',len(rows),'PASS')
