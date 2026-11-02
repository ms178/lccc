import hashlib, json, os, shutil, statistics, subprocess, sys
from pathlib import Path
ROOT = Path('/home/user/lccc')
sys.path.insert(0, str(ROOT / 'scripts'))
import callgrind_ab as cg
out = ROOT / 'results/pr824-rebased/compiler-work'
out.mkdir(parents=True, exist_ok=True)
paths = {'candidate': ROOT / 'target/fastbuild/lccc',
         'main': Path('/home/user/.cache/lccc-main-fb02007/target/fastbuild/lccc')}
report = {'metric': 'simulated compiler work; NOT compilation wall-clock speedup',
          'baseline_commit': 'fb02007bd02d0eb30d8f956a2a5bd44f44ddaa23',
          'geometry': {'I1': '32768,8,64', 'D1': '32768,8,64', 'LL': '33554432,16,64'},
          'valgrind': cg.tool_identity('valgrind'), 'binaries': {}, 'results': {}, 'complete': False}
for name, path in paths.items():
    directory = out / ('aa' if name == 'candidate' else 'bb')
    directory.mkdir(exist_ok=True)
    copy = directory / 'compiler'
    shutil.copy2(path, copy)
    original_text = directory / 'original.text'
    stripped_text = directory / 'stripped.text'
    subprocess.run(['objcopy', '--dump-section', f'.text={original_text}', str(copy)], check=True)
    subprocess.run(['strip', '--strip-debug', str(copy)], check=True)
    subprocess.run(['objcopy', '--dump-section', f'.text={stripped_text}', str(copy)], check=True)
    assert original_text.read_bytes() == stripped_text.read_bytes(), 'strip changed executable text'
    report['binaries'][name] = {'original_sha256': cg.schema.file_hash(path),
                              'stripped_sha256': cg.schema.file_hash(copy),
                              'text_sha256': cg.schema.file_hash(original_text)}
    paths[name] = copy
for case, source in [('minmax', ROOT / 'tests/regression/minmax_shapes/minmax_shapes.c'),
                     ('libm', ROOT / 'tests/benchmark/programs/libm_round_family.c')]:
    rows = []
    for repetition in range(3):
        order = ['candidate', 'main'] if repetition % 2 == 0 else ['main', 'candidate']
        pair = {}
        for name in order:
            binary = paths[name]
            assembly = binary.parent / (case + '.s')
            args = [str(binary), '-O3', '-march=x86-64-v3', '-fwrapv', '-S', str(source), '-o', str(assembly)]
            subprocess.run(args, check=True, capture_output=True, timeout=60)
            native = assembly.read_bytes()
            profile = binary.parent / f'{case}-{repetition}.cg'
            command = ['valgrind', '--tool=callgrind', '--cache-sim=yes', '--branch-sim=yes', '--quiet',
                       '--I1=32768,8,64', '--D1=32768,8,64', '--LL=33554432,16,64',
                       f'--callgrind-out-file={profile}', *args]
            result = subprocess.run(command, capture_output=True, timeout=180)
            (binary.parent / f'{case}-{repetition}.stderr').write_bytes(result.stderr)
            assert result.returncode == 0, result.stderr.decode(errors='replace')[-2000:]
            assert assembly.read_bytes() == native, 'native/instrumented compiler assembly mismatch'
            pair[name] = {'events': cg.parse_summary(profile), 'command': command,
                          'profile_sha256': cg.schema.file_hash(profile),
                          'assembly_sha256': hashlib.sha256(native).hexdigest(),
                          'stderr_sha256': hashlib.sha256(result.stderr).hexdigest(),
                          'stderr_excerpt': result.stderr.decode(errors='replace')[-1500:]}
        rows.append({'order': order, 'arms': pair,
                     'ir_ratio': pair['candidate']['events']['Ir'] / pair['main']['events']['Ir']})
        print(case, repetition, rows[-1]['ir_ratio'], flush=True)
    report['results'][case] = {'source_sha256': cg.schema.file_hash(source), 'rounds': rows,
                               'median_ir_ratio': statistics.median(r['ir_ratio'] for r in rows)}
    (out / 'summary.json').write_text(json.dumps(report, indent=2) + '\n')
report['complete'] = True
(out / 'summary.json').write_text(json.dumps(report, indent=2) + '\n')
