"""Paired Callgrind compiler-work screen for the delimiter-index change."""
import hashlib
import json
from pathlib import Path
import shutil
import subprocess
import sys
sys.path.insert(0, '/home/user/lccc/scripts')
import callgrind_ab as cg
ROOT = Path('/home/user/lccc')
OUT = ROOT / 'results/pr824-next'
report = {'complete': False, 'metric': 'whole-compiler simulated Ir, NOT wall-clock speedup',
          'baseline_commit': '18cff8402337549f1d9cbe3a21e8e6c331708167',
          'candidate_commit': subprocess.check_output(['git','rev-parse','HEAD'],cwd=ROOT,text=True).strip(),
          'valgrind': cg.tool_identity('valgrind'), 'arms': {}, 'cases': {}}
for name, original, directory in [('candidate', ROOT/'target/fastbuild/lccc', OUT/'aa'),
                                   ('baseline', OUT/'bb/compiler', OUT/'bb')]:
    binary = directory/'compiler'
    original_hash = cg.schema.file_hash(original)
    if original != binary: shutil.copy2(original, binary)
    before, after = directory/'before.text', directory/'after.text'
    subprocess.run(['objcopy','--dump-section',f'.text={before}',str(binary)],check=True)
    subprocess.run(['strip','--strip-debug',str(binary)],check=True)
    subprocess.run(['objcopy','--dump-section',f'.text={after}',str(binary)],check=True)
    assert before.read_bytes() == after.read_bytes()
    report['arms'][name] = {'original_sha256':original_hash, 'stripped_sha256':cg.schema.file_hash(binary),
                            'text_sha256':cg.schema.file_hash(before),'binary':str(binary)}

def save(): (OUT/'index-ab.json').write_text(json.dumps(report,indent=2)+'\n')
save()
for number,(name,source) in enumerate([
    ('minmax', ROOT/'tests/regression/minmax_shapes/minmax_shapes.c'),
    ('libm', ROOT/'tests/benchmark/programs/libm_round_family.c'),
    ('chain-64', ROOT/'results/pr824-followup/latest-main-measurements/chain-64.c')]):
    entry = {'source_sha256':cg.schema.file_hash(source), 'rounds': []}
    for repetition in range(2):
        order=['candidate','baseline'] if (number+repetition)%2==0 else ['baseline','candidate']
        pair={'order':order,'arms':{}}
        outputs=[]
        for arm in order:
            binary=Path(report['arms'][arm]['binary']); assembly=binary.parent/(name+'.s')
            command=[str(binary),'-O3','-march=x86-64-v3','-fwrapv','-S',str(source),'-o',str(assembly)]
            native=cg._run(command,120)
            assert not native['failure'] and native['returncode']==0, cg.evidence(native)
            emitted=assembly.read_bytes(); outputs.append(emitted)
            profile=binary.parent/f'{name}-{repetition}.cg'
            argv=['valgrind','--tool=callgrind','--cache-sim=yes','--branch-sim=yes','--quiet',
                  '--I1=32768,8,64','--D1=32768,8,64','--LL=33554432,16,64',
                  f'--callgrind-out-file={profile}',*command]
            process=cg._run(argv,180)
            pair['arms'][arm]={'process':cg.evidence(process),'command':argv}
            if process['failure'] or process['returncode']!=0:
                report['failure']={'case':name,'round':repetition,'arm':arm,'process':cg.evidence(process)}
                entry['rounds'].append(pair); report['cases'][name]=entry; save()
                raise RuntimeError('bounded instrumentation failed; incomplete evidence retained')
            assert assembly.read_bytes()==emitted, 'native/instrumented assembly mismatch'
            pair['arms'][arm].update(events=cg.parse_summary(profile),profile_sha256=cg.schema.file_hash(profile),
                                     assembly_sha256=hashlib.sha256(emitted).hexdigest())
        assert outputs[0]==outputs[1], 'candidate/baseline assembly changed'
        pair['ir_candidate_over_baseline']=pair['arms']['candidate']['events']['Ir']/pair['arms']['baseline']['events']['Ir']
        entry['rounds'].append(pair);report['cases'][name]=entry;save()
        print(name,repetition,pair['ir_candidate_over_baseline'],flush=True)
report['complete']=True;save()
