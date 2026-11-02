"""Narrow the timed-out diamond screen; never retry the same request."""
import ast
import hashlib
import json
from pathlib import Path
import sys
sys.path.insert(0, '/home/user/lccc/scripts')
import callgrind_ab as cg

ROOT = Path('/home/user/lccc')
OUT = ROOT / 'results/pr824-followup/latest-main-measurements'
path = OUT / 'summary.json'
report = json.loads(path.read_text())
# Preserve the failed 128-diamond attempt from the exact bounded runner log.
log = Path('/home/user/latest-main-callgrind.log').read_text()
error = next(line.partition('AssertionError: ')[2] for line in log.splitlines() if line.startswith('AssertionError: '))
report['compiler-work']['failures'] = {'diamond-128': {
    'arm': 'main', 'process': ast.literal_eval(error), 'candidate_attempted': False,
    'usable_counters': False, 'note': 'baseline timed out at 180s; empty profile, never ranked'}}
report['compiler-work']['complete'] = False

def save(): path.write_text(json.dumps(report, indent=2) + '\n')
save()
compilers = {name: OUT / suffix / 'compiler' for name, suffix in [('candidate', 'aa'), ('main', 'bb')]}
for arm, compiler in compilers.items():
    assert cg.schema.file_hash(compiler) == report['compiler-work']['arms'][arm]['stripped_sha256']

for number, (family, count) in enumerate([('chain', 16), ('diamond', 8), ('diamond', 16)]):
    name = f'{family}-{count}'
    source = OUT / (name + '.c')
    body = []
    for i in range(count):
        if family == 'chain': body.append(f'x=(x+(unsigned long)p[i+{i}])^{i+1}u;')
        else: body.append(f'if(p[i+{i}]&1u) x+=p[i+{i+1}]; else x^=p[i+{i+2}];')
    source.write_text('__attribute__((noinline)) unsigned long kernel(const unsigned *p,unsigned long n) { unsigned long x=0;\n'
                      'for(unsigned long i=0;i<n;++i) {\n' + '\n'.join(body) + '\n} return x; }\n'
                      'int printf(const char*,...); int main(void) { unsigned p[1024]; '
                      'for(unsigned i=0;i<1024;++i) p[i]=i*2654435761u; '
                      'printf("%lu\\n",kernel(p,16)); return 0; }\n')
    order = ['candidate', 'main'] if number % 2 == 0 else ['main', 'candidate']
    pair = {'source_sha256': cg.schema.file_hash(source), 'order': order, 'arms': {}}
    for arm in order:
        compiler = compilers[arm]
        assembly = compiler.parent / (name + '.s')
        command = [str(compiler), '-O3', '-march=x86-64-v3', '-fwrapv', '-S', str(source), '-o', str(assembly)]
        native = cg._run(command, 120)
        assert not native['failure'] and native['returncode'] == 0
        emitted = assembly.read_bytes()
        profile = compiler.parent / (name + '.cg')
        full = ['valgrind', '--tool=callgrind', '--cache-sim=yes', '--branch-sim=yes', '--quiet',
                '--I1=32768,8,64', '--D1=32768,8,64', '--LL=33554432,16,64',
                f'--callgrind-out-file={profile}', *command]
        instrumented = cg._run(full, 180)
        assert not instrumented['failure'] and instrumented['returncode'] == 0, cg.evidence(instrumented)
        assert assembly.read_bytes() == emitted
        pair['arms'][arm] = {'events': cg.parse_summary(profile), 'command': full,
                             'process': cg.evidence(instrumented), 'profile_sha256': cg.schema.file_hash(profile),
                             'assembly_sha256': hashlib.sha256(emitted).hexdigest()}
    outputs = []
    for arm, compiler in [('candidate', ROOT / 'target/fastbuild/lccc'),
                          ('main', Path('/home/user/.cache/lccc-main-3ff36e8/target/fastbuild/lccc')),
                          ('gcc12', Path('/usr/bin/gcc'))]:
        binary = OUT / ('synthetic-' + arm)
        assert cg.compile_program(str(compiler), '-O3 -march=x86-64-v3 -fwrapv', source, binary) is None
        run = cg._run([str(binary)], 60)
        assert not run['failure'] and run['returncode'] == 0
        outputs.append(run['stdout'])
        pair['arms'].setdefault(arm, {})['native_correctness'] = cg.evidence(run)
    assert len(set(outputs)) == 1
    pair['ir_candidate_over_main'] = pair['arms']['candidate']['events']['Ir'] / pair['arms']['main']['events']['Ir']
    report['compiler-work']['cases'][name] = pair
    print(name, pair['ir_candidate_over_main'], flush=True)
    save()
report['compiler-work']['bounded_scaling_complete'] = True
report['compiler-work']['bounded_scaling_sizes'] = {'chain': [16, 32, 64, 128], 'diamond': [8, 16, 32, 64]}
report['note'] = 'All selected bounded scaling points passed; larger diamond-128 baseline timeout remains explicit. Overall complete remains false.'
save()
