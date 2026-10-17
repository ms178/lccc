#!/usr/bin/env python3
"""Rebuild saved fresh-entry requests and prove timed ELF identity.

Only byte-identical complete executables can retain old runtime measurements.
Recompile the common driver, replay each exact source/macro request, perform the
verified ELF entry rename, and relink. Changed executables require new timings.
The original build and raw samples are never overwritten.
"""
import argparse
import json
from pathlib import Path
import sys
from benchmark_native_batched import ROOT, bench, execute, object_identity, publish, rename_entry, sha


def output_command(command, output, compiler=None):
    command = list(command)
    if command.count('-o') != 1 or command.index('-o') + 1 >= len(command):
        raise ValueError('ambiguous or absent recorded output')
    command[command.index('-o') + 1] = str(output)
    if compiler is not None:
        command[0] = str(compiler)
    return command


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--build', type=Path, required=True)
    ap.add_argument('--compiler', type=Path, required=True)
    ap.add_argument('--out', type=Path, required=True)
    ap.add_argument('--arm', default='lccc')
    args = ap.parse_args()
    root, out = args.build.resolve().parent, args.out.resolve()
    if root.is_relative_to(out) or out.is_relative_to(root):
        ap.error('replay output must be disjoint from the saved build')
    prior = json.loads(args.build.read_text())
    identity = prior['identity']
    if not prior.get('complete') or args.arm not in identity['compilers']:
        ap.error('complete original build and recorded arm required')
    if (identity['driver_sha256'] != sha(ROOT/'tests/benchmark/batched_driver.c')
            or identity['object_protocol_sha256'] != sha(ROOT/'scripts/benchmark_elf_identity.py')):
        ap.error('driver or ELF entry protocol changed')
    compiler = args.compiler.resolve()
    out.mkdir(parents=True, exist_ok=True)
    result = dict(complete=False, original_build_sha256=sha(args.build),
                  original_identity=identity, compiler=dict(path=str(compiler),sha256=sha(compiler)),
                  metric='complete timed-ELF byte identity, not new runtime samples', checks=[], changed=[])
    known = {b.name:b for b in bench.BENCHMARKS}
    try:
        command = json.loads((root/'driver.json').read_text())['command']
        if str(ROOT/'tests/benchmark/batched_driver.c') not in command:
            raise ValueError('unrecognized common driver request')
        execute(output_command(command,out/'driver.o'),out/'driver.json')
        if sha(out/'driver.o') != sha(root/'driver.o'):
            raise ValueError('common driver object changed')
        for name, source_record in identity['sources'].items():
            if name not in known:
                raise ValueError('unknown saved workload: '+name)
            source = bench.source_for(known[name])
            if sha(source) != source_record['sha256']:
                raise ValueError('source changed: '+name)
            old = root/name/args.arm
            if sha(old/'program') != prior['binaries'][name][args.arm]:
                raise ValueError('saved timed executable changed: '+name)
            new = out/name;new.mkdir(exist_ok=True)
            command = json.loads((old/'original.json').read_text())['command']
            if str(source) not in command or '-S' not in command:
                raise ValueError('unrecognized source compilation: '+name)
            execute(output_command(command,new/'original.s',compiler),new/'original.json')
            if identity.get('object_mode', 'assembly') == 'direct':
                build = json.loads((old/'assemble.json').read_text())['command']
                if str(source) not in build or '-c' not in build or '-S' in build:
                    raise ValueError('unrecognized direct object compilation: '+name)
                execute(output_command(build,new/'original.o',compiler),new/'assemble.json')
            else:
                execute([compiler,'-c',new/'original.s','-o',new/'original.o'],new/'assemble.json')
            original = (new/'original.o').read_bytes()
            renamed = rename_entry(original)
            if object_identity(original,'main') != object_identity(renamed,'benchmark_entry'):
                raise ValueError('entry rename changed object: '+name)
            (new/'work.o').write_bytes(renamed)
            link = json.loads((old/'link.json').read_text())['command']
            for previous, replacement in [(root/'driver.o',out/'driver.o'),(old/'work.o',new/'work.o')]:
                if link.count(str(previous)) != 1:
                    raise ValueError('unrecognized recorded link')
                link[link.index(str(previous))] = str(replacement)
            execute(output_command(link,new/'program'),new/'link.json')
            expected, actual = prior['binaries'][name][args.arm], sha(new/'program')
            row = dict(name=name, original_program_sha256=expected, replayed_program_sha256=actual,
                       original_assembly_sha256=sha(old/'original.s'), replayed_assembly_sha256=sha(new/'original.s'),
                       identical=expected==actual)
            result['checks'].append(row)
            if not row['identical']:result['changed'].append(name)
            publish(out/'replay.json',result)
        result['complete'] = True
    finally:
        publish(out/'replay.json',result)
    print(f"{len(result['checks'])} complete ELF comparisons; changed: {result['changed']}")
    return int(bool(result['changed']))


if __name__ == '__main__':
    sys.exit(main())
