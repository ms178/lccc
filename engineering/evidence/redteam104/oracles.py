import sys,json,pathlib,subprocess,hashlib
repo=pathlib.Path('/home/user/lccc');r=pathlib.Path('/home/user/.cache/redteam104/oracles');r.mkdir(parents=True,exist_ok=True);source=repo/'tests/regression/bmi2_saturating_mask.c';sys.path.insert(0,str(repo/'scripts'));import godbolt
text=source.read_text().replace('int main(void) {','int main(void) {\n printf("VERSION %s\\n", __VERSION__);\n#ifdef __INTEL_LLVM_COMPILER\n printf("INTEL_LLVM=%ld\\n", (long)__INTEL_LLVM_COMPILER);\n#endif\n');rows=[]
for compiler in ['gcc16.2','clang23.1','icx']:
 rec={'compiler':compiler,'source_sha256':hashlib.sha256(text.encode()).hexdigest(),'flags':'-std=gnu17 -O2 -march=x86-64-v3 -Werror'}
 try:
  data=godbolt.compile_on_godbolt(compiler,text,rec['flags'],execute=True,raise_on_error=True,timeout=90);(r/(compiler+'.json')).write_text(json.dumps(data,indent=2));ex=data.get('execResult',{});rec.update(execution=ex,metadata=godbolt.compiler_metadata(godbolt.resolve_compiler(compiler)),passed=ex.get('code')==0 and ex.get('didExecute') is True)
 except Exception as e:rec.update(passed=False,error=str(e))
 rows.append(rec);(r/'index.json').write_text(json.dumps(rows,indent=2));print(compiler,rec,flush=True)
assert all(x['passed'] for x in rows)
