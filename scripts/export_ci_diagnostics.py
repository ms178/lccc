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
import time
from urllib.parse import quote

PART_BYTES = 8 * 1024 * 1024
MAX_API_ATTEMPTS = 4
# Job conclusions whose logs are evidence. A job that exhausts
# `timeout-minutes` is reported as `cancelled`, not `failure`: on PR #1196 the
# AVX-512VL lane spent 1756 s of its 30-minute budget in one provisioning
# step, and an export keyed on `failure` carried only the six-second aggregate
# job that depends on it -- everything except the log that explained the red.
EVIDENCE_JOB_CONCLUSIONS = ('failure', 'cancelled', 'timed_out')
# "There is no log": never produced (the job was cancelled while still
# queued) or no longer retained.
NO_LOG_STATUSES = (404, 410)


class ApiError(RuntimeError):
    """A failed GitHub API request, with the HTTP status when gh reported one."""

    def __init__(self, message, status=None):
        super().__init__(message)
        self.status = status


def parse_run_ids(text):
    values = text.split(',')
    if not 1 <= len(values) <= 4 or any(not re.fullmatch(r'[1-9][0-9]{0,19}', x) for x in values):
        raise ValueError('expected one to four comma-separated positive run IDs')
    ids = list(map(int, values))
    if len(set(ids)) != len(ids):
        raise ValueError('duplicate run ID')
    return ids


def automatic_run_ids(runs, pr_number):
    """Pick at most two latest terminal non-green workflows for this PR.

    A cancelled workflow can contain a completed failed job; restricting the
    query to failure lost that evidence whenever a later push cancelled the
    remaining jobs. API page order can put an older cancelled retry first,
    so order explicitly by creation time with the numeric run ID as a tie-break.
    """
    selected = {}
    for run in sorted(runs, key=lambda row: (row.get('created_at', ''), row['id']), reverse=True):
        if (run.get('status') != 'completed'
                or run.get('conclusion') not in ('failure', 'cancelled', 'timed_out')
                or not any(pr.get('number') == pr_number for pr in run.get('pull_requests', []))):
            continue
        selected.setdefault(run['workflow_id'], run['id'])
        if len(selected) == 2:
            break
    return list(selected.values())


def retained_artifact(artifact):
    """Only the exact kernel/Intel evidence names, never arbitrary artifacts."""
    return (not artifact['expired'] and re.fullmatch(
        r'(?:kernel-61863|intel-icx-provenance)-[0-9a-f]{40}', artifact['name']) is not None)


def gh(path, payload=None, *, allow_binary=False):
    args = ['gh', 'api', path]
    if allow_binary:
        # Logs may contain ANSI escapes, and ZIP bytes are arbitrary. These
        # responses are captured, compressed and blob-exported, NEVER displayed
        # on a terminal. Keep gh's default protection for normal JSON requests.
        args.append('--allow-escape-sequences')
    if payload is not None:
        args += ['--method', 'POST', '--input', '-']
    # Reads and content-addressed blob creation are safe to replay after a
    # lost response. Never extend this retry policy to arbitrary mutating POSTs.
    replay_safe = payload is None or path.endswith('/git/blobs')
    for attempt in range(MAX_API_ATTEMPTS):
        try:
            return subprocess.run(args, input=payload, capture_output=True, check=True,
                                  timeout=180).stdout
        except (subprocess.CalledProcessError, subprocess.TimeoutExpired) as error:
            timed_out = isinstance(error, subprocess.TimeoutExpired)
            detail = (error.stderr or b'').decode('utf-8', errors='replace')
            transient = timed_out or re.search(r'HTTP (?:429|500|502|503|504)\b', detail)
            if replay_safe and transient and attempt + 1 < MAX_API_ATTEMPTS:
                time.sleep(2 ** attempt)
                continue
            # Preserve final failure and redact signed URLs / credentials.
            detail = re.sub(r'https?://\S+', '<URL redacted>', detail)
            for name in ('GH_TOKEN', 'GITHUB_TOKEN'):
                token = os.environ.get(name)
                if token:
                    detail = detail.replace(token, '<token redacted>')
            reason = 'timed out' if timed_out else f'failed ({error.returncode})'
            status = re.search(r'HTTP (\d{3})\b', detail)
            raise ApiError(f'GitHub API request {reason}: {detail[:1500]}',
                           int(status.group(1)) if status else None) from error


def job_log(repo, job):
    """The raw log of a terminal non-green job, or None when it has none.

    A job cancelled before a runner picked it up never ran a step, and the log
    endpoint answers 404 for it. That is absence of evidence, not a transport
    failure, so the export continues -- but only for a job that did not fail.
    A FAILED job always has a log; the same answer there is a real error, and
    so is every other status (403, 5xx) for any job.
    """
    try:
        return gh(f'repos/{repo}/actions/jobs/{int(job["id"])}/logs', allow_binary=True)
    except ApiError as error:
        if job['conclusion'] == 'failure' or error.status not in NO_LOG_STATUSES:
            raise
        return None



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
    requested = os.environ.get('SOURCE_RUN_IDS', '')
    if requested:
        run_ids = parse_run_ids(requested)
    else:
        with open(os.environ['GITHUB_EVENT_PATH'], encoding='utf-8') as stream:
            event = json.load(stream)
        pr = event['pull_request']
        if pr['head']['repo']['full_name'] != repo:
            raise ValueError('automatic export requires a same-repository PR')
        branch = quote(pr['head']['ref'], safe='')
        candidates = json.loads(gh(f'repos/{repo}/actions/runs?branch={branch}&status=completed&per_page=100'))
        run_ids = automatic_run_ids(candidates['workflow_runs'], event['number'])
        if not run_ids:
            print('No previous failed PR runs to export')
            return
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
            if job['conclusion'] not in EVIDENCE_JOB_CONCLUSIONS:
                continue
            name = f'run{run_id}-job{int(job["id"])}.log.gz'
            data = job_log(repo, job)
            if data is None:
                # Named in the manifest so a reader can tell "this job had no
                # log" from "the export never looked at this job".
                manifest.append(f'nolog {name} {job["conclusion"]}')
                continue
            manifest += export_file(repo, name, gzip.compress(data, mtime=0))
        for artifact in pages('artifacts', 'artifacts'):
            if not retained_artifact(artifact):
                continue
            data = gh(f'repos/{repo}/actions/artifacts/{int(artifact["id"])}/zip', allow_binary=True)
            manifest += export_file(repo, f'run{run_id}-artifact{artifact["id"]}.zip', data)
        for offset in range(0, len(manifest), 12):
            text = '\n'.join(manifest[offset:offset+12])
            escaped = text.replace('%', '%25').replace('\r', '%0D').replace('\n', '%0A')
            print(f'::notice title=CI diagnostic transport::{escaped}', flush=True)


def run_cli():
    try:
        main()
    except Exception as error:
        text = str(error).replace('%', '%25').replace('\r', '%0D').replace('\n', '%0A')
        print(f'::error title=CI diagnostic transport failed::{text}', flush=True)
        raise


if __name__ == '__main__':
    run_cli()
