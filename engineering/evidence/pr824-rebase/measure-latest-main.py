"""Fresh, bounded, correctness-gated current/main simulated-work screen."""
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys

ROOT = Path('/home/user/lccc')
sys.path.insert(0, str(ROOT / 'scripts'))
import callgrind_ab as cg
import callgrind_corpus as corpus

OUT = ROOT / 'results/pr824-followup/latest-main-measurements'
OUT.mkdir(parents=True, exist_ok=True)
MAIN = Path('/home/user/.cache/lccc-main-3ff36e8/target/fastbuild/lccc')
CANDIDATE = ROOT / 'target/fastbuild/lccc'
FLAGS = '-O2 -march=x86-64-v3 -fwrapv'
report = {'complete': False, 'baseline_commit': '3ff36e8592b0d0e26645bf37d6685395b1a6eca5',
          'candidate_commit': subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip(),
          'metric': 'whole-program/whole-compiler simulated Ir; NOT cycles or native speedup',
          'valgrind': cg.tool_identity('valgrind'), 'gcc': cg.tool_identity('gcc'),
          'compiler_hashes': {name: cg.schema.file_hash(path) for name, path in [('main', MAIN), ('candidate', CANDIDATE)]}}

def save():
    (OUT / 'summary.json').write_text(json.dumps(report, indent=2) + '\n')

save()
cases = 'nbody,spectral_norm,double_reduction,ring_fifo,libm_round_family,byte_load_reassembly,zlib_ng_adler32,linux_find_bit_scaled,lz4_match_extend'
for name, ref in [('main', MAIN), ('gcc12', Path('/usr/bin/gcc'))]:
    directory = OUT / ('runtime-' + name)
    rc = corpus.main(['--mine', str(CANDIDATE), '--ref', str(ref), '--only', cases, '--out', str(directory)])
    assert rc == 0, ('runtime', name, rc)
    index = json.loads((directory / 'index.json').read_text())
    assert index['complete'] and not index['failures'] and len(index['results']) == 9
    # Keep every actual counter and correctness/tool identity in a compact aggregate.
    for row in index['results'].values():
        row['full_manifest'] = json.loads(Path(row['manifest']).read_text())
    report['runtime-' + name] = index
    save()

# The actual checked-in unsigned-map kernel, with its original correctness
# battery retained, followed by enough repeated work to expose guarded-path cost.
map_source = OUT / 'map-work.c'
map_source.write_text('''#define main regression_main
#include "/home/user/lccc/tests/regression/map_i64_two_lane.c"
#undef main
int main(void) {
 if (regression_main()) return 1;
 for (unsigned long r=0;r<20000;++r) sub64(out,a,b,N);
 uint64_t sum=0;
 for (unsigned long i=0;i<N;++i) {
  if(out[i]!=a[i]-b[i]) return 2;
  sum += out[i];
 }
 printf("map-work %llu\\n",(unsigned long long)sum);
 return 0;
}
''')
report['map'] = {'source_sha256': cg.schema.file_hash(map_source), 'flags': FLAGS, 'arms': {}}
expected = None
for name, compiler, suffix in [('candidate', CANDIDATE, 'aa'), ('main', MAIN, 'bb'), ('gcc12', Path('/usr/bin/gcc'), 'cc')]:
    directory = OUT / ('map-' + suffix)
    directory.mkdir(exist_ok=True)
    binary = directory / 'map-work'
    assert cg.compile_program(str(compiler), FLAGS, map_source, binary) is None
    process = cg._run([str(binary)], 60)
    assert not process['failure'] and process['returncode'] == 0
    if expected is None: expected = process['stdout']
    assert process['stdout'] == expected, ('map parity', name)
    events, error, instrumentation = cg.callgrind(binary, directory, expected_stdout=expected, timeout=120)
    assert error is None, error
    report['map']['arms'][name] = {'events': events, 'correctness': cg.evidence(process),
                                  'instrumentation': instrumentation, 'binary_sha256': cg.schema.file_hash(binary)}
    save()

# Strip debug sections only, checking executable text byte-for-byte before
# instrumenting the compilers. Alternating arm order removes ordering preference.
compilers = {}
report['compiler-work'] = {'arms': {}, 'cases': {}}
for name, original in [('candidate', CANDIDATE), ('main', MAIN)]:
    directory = OUT / ('aa' if name == 'candidate' else 'bb')
    directory.mkdir(exist_ok=True)
    binary = directory / 'compiler'
    shutil.copy2(original, binary)
    before, after = directory / 'before.text', directory / 'after.text'
    subprocess.run(['objcopy', '--dump-section', f'.text={before}', str(binary)], check=True)
    subprocess.run(['strip', '--strip-debug', str(binary)], check=True)
    subprocess.run(['objcopy', '--dump-section', f'.text={after}', str(binary)], check=True)
    assert before.read_bytes() == after.read_bytes()
    report['compiler-work']['arms'][name] = {'original_sha256': cg.schema.file_hash(original),
                                            'stripped_sha256': cg.schema.file_hash(binary),
                                            'text_sha256': cg.schema.file_hash(before)}
    compilers[name] = binary

sources = [('minmax', ROOT / 'tests/regression/minmax_shapes/minmax_shapes.c'),
           ('libm', ROOT / 'tests/benchmark/programs/libm_round_family.c')]
for family in ('chain', 'diamond'):
    for count in (32, 64, 128):
        source = OUT / f'{family}-{count}.c'
        body = []
        for i in range(count):
            if family == 'chain':
                body.append(f'x=(x+(unsigned long)p[i+{i}])^{i+1}u;')
            else:
                body.append(f'if(p[i+{i}]&1u) x+=p[i+{i+1}]; else x^=p[i+{i+2}];')
        source.write_text('__attribute__((noinline)) unsigned long kernel(const unsigned *p,unsigned long n) { unsigned long x=0;\n'
                          'for(unsigned long i=0;i<n;++i) {\n' + '\n'.join(body) + '\n} return x; }\n'
                          'int printf(const char*,...); int main(void) { unsigned p[1024]; '
                          'for(unsigned i=0;i<1024;++i) p[i]=i*2654435761u; '
                          'printf("%lu\\n",kernel(p,16)); return 0; }\n')
        sources.append((f'{family}-{count}', source))

for number, (name, source) in enumerate(sources):
    order = ['candidate', 'main'] if number % 2 == 0 else ['main', 'candidate']
    pair = {'source_sha256': cg.schema.file_hash(source), 'order': order, 'arms': {}}
    for arm in order:
        compiler = compilers[arm]
        assembly = compiler.parent / (name + '.s')
        command = [str(compiler), '-O3', '-march=x86-64-v3', '-fwrapv', '-S', str(source), '-o', str(assembly)]
        native = cg._run(command, 120)
        assert not native['failure'] and native['returncode'] == 0, cg.evidence(native)
        emitted = assembly.read_bytes()
        profile = compiler.parent / (name + '.cg')
        full = ['valgrind', '--tool=callgrind', '--cache-sim=yes', '--branch-sim=yes', '--quiet',
                '--I1=32768,8,64', '--D1=32768,8,64', '--LL=33554432,16,64',
                f'--callgrind-out-file={profile}', *command]
        instrumented = cg._run(full, 180)
        assert not instrumented['failure'] and instrumented['returncode'] == 0, cg.evidence(instrumented)
        assert assembly.read_bytes() == emitted, 'native/instrumented assembly mismatch'
        pair['arms'][arm] = {'events': cg.parse_summary(profile), 'command': full,
                             'process': cg.evidence(instrumented), 'profile_sha256': cg.schema.file_hash(profile),
                             'assembly_sha256': hashlib.sha256(emitted).hexdigest()}
    pair['ir_candidate_over_main'] = pair['arms']['candidate']['events']['Ir'] / pair['arms']['main']['events']['Ir']
    if name.startswith(('chain-', 'diamond-')):
        outputs = []
        for arm, compiler in [('candidate', CANDIDATE), ('main', MAIN), ('gcc12', Path('/usr/bin/gcc'))]:
            binary = OUT / ('synthetic-' + arm)
            error = cg.compile_program(str(compiler), '-O3 -march=x86-64-v3 -fwrapv', source, binary)
            assert error is None, error
            run = cg._run([str(binary)], 60)
            assert not run['failure'] and run['returncode'] == 0
            outputs.append(run['stdout'])
            pair['arms'].setdefault(arm, {})['native_correctness'] = cg.evidence(run)
        assert len(set(outputs)) == 1, 'synthetic native output mismatch'
    report['compiler-work']['cases'][name] = pair
    print('compiler-work', name, pair['ir_candidate_over_main'], flush=True)
    save()
report['complete'] = True
save()
print('Fresh current/main screen: COMPLETE; all native/instrumented parity checks passed', flush=True)
