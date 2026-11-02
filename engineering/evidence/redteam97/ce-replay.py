import sys,json,pathlib,hashlib,time
repo=pathlib.Path('/home/user/lccc');sys.path.insert(0,str(repo/'scripts'));import godbolt
out=repo/'engineering/evidence/redteam97/ce';out.mkdir(exist_ok=True)
fixtures=repo/'tests/regression/fixtures/peephole-padding-effects';evidence=out.parent
records=[]
for family,driver,original,badexit in [('vector','vector-driver.c','vector.s',1),('padding','driver.c','caller.s',13)]:
 for stage,assembly,expected in [('original',fixtures/original,0),('before',evidence/f'{family}-before.s',badexit)]:
  c=(fixtures/driver).read_text().replace('int main(void) {','int main(void) {\n printf("%s\\n", __VERSION__);\n#ifdef __INTEL_LLVM_COMPILER\n printf("INTEL_LLVM=%ld\\n", (long)__INTEL_LLVM_COMPILER);\n#endif\n')
  source='#include <stdio.h>\n__asm__('+json.dumps(assembly.read_text().replace(".LBB", ".Ls97_witness_bb"))+');\n'+c
  (out/f'{family}-{stage}.c').write_text(source)
  for compiler in ['gcc16.2','clang23.1','icx']:
   name=f'{family}-{stage}-{compiler}';record={'family':family,'stage':stage,'compiler':compiler,'expected_exit':expected,'source_sha256':hashlib.sha256(source.encode()).hexdigest(),'time':time.time(),'flags':'-O2 -Werror','assembly_adaptation':'namespace .LBB labels to .Ls97_witness_bb to avoid compiler-generated debug-label collisions'}
   try:
    data=godbolt.compile_on_godbolt(compiler,source,'-O2 -Werror',execute=True,raise_on_error=True,timeout=90)
    (out/f'{name}.json').write_text(json.dumps(data,indent=2));ex=data.get('execResult',{});record.update(execution=ex,metadata=godbolt.compiler_metadata(godbolt.resolve_compiler(compiler)),pass_result=ex.get('code')==expected and ex.get('didExecute') is True)
   except Exception as exc:record.update(error=str(exc),pass_result=False)
   records.append(record);(out/'index.json').write_text(json.dumps(records,indent=2));print(name,json.dumps(record),flush=True)
