#!/usr/bin/env python3
"""Export existing CI evidence through GitHub content-addressed blobs.

Diagnostic transport only: this never builds a compiler or kernel, changes a
branch, or converts a failed check into a pass. Useful in GitHub-only sandboxes
which cannot reach Actions' signed log/artifact storage URLs.
"""
import base64
import gzip
import hashlib
import json
import os
import re
import subprocess

PART_BYTES = 8 * 1024 * 1024


def parse_run_ids(text):
    values = text.split(',')
    if not 1 <= len(values) <= 4 or any(not re.fullmatch(r'[1-9][0-9]{0,19}', x) for x in values):
        raise ValueError('expected one to four comma-separated positive run IDs')
    ids = list(map(int, values))
    if len(set(ids)) != len(ids):
        raise ValueError('duplicate run ID')
    return ids


def gh(path, payload=None):
    args = ['gh', 'api', path]
    if payload is not None:
        args += ['--method', 'POST', '--input', '-']
    return subprocess.run(args, input=payload, capture_output=True, check=True,
                          timeout=180).stdout


def export_file(repo, name, data):
    rows = [f'file {name} {len(data)} {hashlib.sha256(data).hexdigest()}']
    for index, start in enumerate(range(0, len(data), PART_BYTES)):
        part = data[start:start + PART_BYTES]
        payload = json.dumps({'encoding': 'base64',
                              'content': base64.b64encode(part).decode('ascii')}).encode()
        response = json.loads(gh(f'repos/{repo}/git/blobs', payload))
        expected = hashlib.sha1(b'blob ' + str(len(part)).encode() + b'\0' + part).hexdigest()
        if response.get('sha') != expected:
            raise RuntimeError('GitHub blob identity does not match exported bytes')
        rows.append(f'part {name} {index} {expected} {hashlib.sha256(part).hexdigest()}')
    return rows


def main():
    repo = os.environ['GITHUB_REPOSITORY']
    if not re.fullmatch(r'[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+', repo):
        raise ValueError('invalid repository')
    run_ids = parse_run_ids(os.environ['SOURCE_RUN_IDS'])
    for run_id in run_ids:
        run = json.loads(gh(f'repos/{repo}/actions/runs/{run_id}'))
        if run['repository']['full_name'] != repo or run['status'] != 'completed':
            raise ValueError('only completed runs of this repository can be exported')
        # --paginate is required: a run can have more than thirty jobs/artifacts.
        def pages(resource, key):
            page = 1
            while True:
                result = json.loads(gh(f'repos/{repo}/actions/runs/{run_id}/{resource}?per_page=100&page={page}'))
                yield from result[key]
                if len(result[key]) < 100:
                    break
                page += 1
        manifest = [f'run {run_id} {run["head_sha"]} {run["conclusion"]}']
        for job in pages('jobs', 'jobs'):
            if job['conclusion'] != 'failure':
                continue
            data = gh(f'repos/{repo}/actions/jobs/{int(job["id"])}/logs')
            manifest += export_file(repo, f'run{run_id}-job{job["id"]}.log.gz',
                                    gzip.compress(data, mtime=0))
        for artifact in pages('artifacts', 'artifacts'):
            if artifact['expired']:
                continue
            data = gh(f'repos/{repo}/actions/artifacts/{int(artifact["id"])}/zip')
            manifest += export_file(repo, f'run{run_id}-artifact{artifact["id"]}.zip', data)
        for offset in range(0, len(manifest), 12):
            text = '\n'.join(manifest[offset:offset+12])
            escaped = text.replace('%', '%25').replace('\r', '%0D').replace('\n', '%0A')
            print(f'::notice title=CI diagnostic transport::{escaped}', flush=True)


if __name__ == '__main__':
    main()
