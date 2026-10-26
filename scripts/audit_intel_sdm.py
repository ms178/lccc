#!/usr/bin/env python3
"""Collect edition-anchored primary SDM sections, not semantic sign-off.

Run on a network-enabled host. The PDF remains outside Git. Notices make selected
primary pages available through the GitHub Checks API when artifact/log redirects
are inaccessible. A section's presence is never a claim it has been reviewed.
"""
import argparse
import hashlib
import os
import json
from pathlib import Path
import re
import urllib.request

URL = 'https://cdrdv2-public.intel.com/929349/325462-093-sdm-vol-1-2abcd-3abcd-4.pdf'
INSTRUCTIONS = re.compile(r'^(?:DIVP[SD]|SQRTP[SD]|MINP[SD]|MAXP[SD]|MOV[AU]P[SD]|MOVDQU|MOVDQA|MOVSD|MOVSS|CVT(?:TP[SD]2DQ|P[SD]2DQ|DQ2P[SD])|VZEROUPPER|VZEROALL|PSHUFB|PTEST|DPPS|DPPD|ROUNDPS|ROUNDPD|PCMP(?:E|I)STR[IM])(?:[^A-Z]|$)')
ARCHITECTURE = re.compile(r'MXCSR|DATA TYPES|FLOATING.POINT EXCEPTION|MEMORY ORDERING|INSTRUCTION ORDERING|RIP.RELATIVE|DATA ALIGNMENT', re.I)


def sections(toc):
    selected = []
    for index, (level, title, page) in enumerate(toc):
        instruction = bool(INSTRUCTIONS.match(title.strip()))
        if not instruction and not ARCHITECTURE.search(title):
            continue
        stop = next((p - 1 for lev, _, p in toc[index + 1:] if lev <= level and p > page), page + 2)
        selected.append({'title': title, 'pdf_page_first': page,
                         'pdf_page_last': min(stop, page + (15 if instruction else 3)),
                         'outline_section_last': stop,
                         'excerpt_only': stop > page + (15 if instruction else 3),
                         'kind': 'instruction' if instruction else 'architecture',
                         'status': 'extracted-unreviewed'})
    return selected


def notice(title, message):
    def escape(s):
        return s.replace('%', '%25').replace('\r', '%0D').replace('\n', '%0A')
    print(f'::notice title={escape(title)}::{escape(message)}', flush=True)


def publish_check(manifest, toc, files):
    # Use the Checks API directly: runner notices truncate messages and retain
    # only ten annotations per step. Never put a token or signed URL in output.
    token = os.environ['GH_TOKEN']
    base = 'https://api.github.com/repos/' + os.environ['GITHUB_REPOSITORY'] + '/check-runs'
    def request(url, payload, method):
        req = urllib.request.Request(url, json.dumps(payload).encode(), method=method,
            headers={'Authorization': 'Bearer ' + token, 'Accept': 'application/vnd.github+json',
                     'X-GitHub-Api-Version': '2022-11-28', 'Content-Type': 'application/json'})
        with urllib.request.urlopen(req, timeout=60) as response:
            return json.load(response)
    result = request(base, {'name': 'Intel SDM source pages (unreviewed)',
        'head_sha': os.environ['GITHUB_SHA'], 'status': 'in_progress',
        'output': {'title': 'Primary source acquisition, not semantic sign-off',
                   'summary': 'Edition 093 source hash and selected pages. PDF remains outside Git.',
                   'text': json.dumps(manifest)}}, 'POST')
    annotations = [('intel-sdm/source-manifest.json', json.dumps(manifest, ensure_ascii=False))]
    for offset in range(0, len(toc), 200):
        annotations.append(('intel-sdm/outline-' + str(offset) + '.json', json.dumps(toc[offset:offset + 200], ensure_ascii=False)))
    for path in files:
        data = json.loads(path.read_text())
        for page in data['pages']:
            annotations.append((f'intel-sdm/page-{page["pdf_page"]}.json',
                                json.dumps({'title': data['section']['title'], **page}, ensure_ascii=False)))
    for offset in range(0, len(annotations), 50):
        batch = [{'path': path, 'start_line': 1, 'end_line': 1,
                  'annotation_level': 'notice', 'message': text}
                 for path, text in annotations[offset:offset + 50]]
        request(base + '/' + str(result['id']), {'output': {
            'title': 'Primary source acquisition, not semantic sign-off',
            'summary': 'Edition/hash-anchored source pages; review remains separate.',
            'annotations': batch}}, 'PATCH')
    request(base + '/' + str(result['id']), {'status': 'completed', 'conclusion': 'success', 'output': {
        'title': 'Primary source acquisition, not semantic sign-off',
        'summary': 'Edition/hash-anchored pages; semantic review remains separate.',
        'text': json.dumps(manifest, ensure_ascii=False)}}, 'PATCH')
    print('SDM source check ID:', result['id'], flush=True)


def self_test():
    toc = [[1, 'Chapter 4 Data Types', 91], [2, '4.1 Fundamental Data Types', 91],
           [1, 'DIVPD—Divide', 100], [1, 'DIVPS—Divide', 103],
           [1, 'Other instruction', 110], [1, 'MAXPD—Maximum', 120],
           [1, 'Other instruction', 150]]
    rows = sections(toc)
    assert [r['title'] for r in rows] == [r[1] for r in toc if r[1] != 'Other instruction']
    assert rows[2]['pdf_page_first'] == 100 and rows[2]['pdf_page_last'] == 102
    assert not rows[2]['excerpt_only']
    assert rows[-1]['excerpt_only'] and rows[-1]['pdf_page_last'] == 135
    assert all(r['status'] == 'extracted-unreviewed' for r in rows)
    print('PASS: SDM selection boundaries, page caps and unreviewed status')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path)
    parser.add_argument('--self-test', action='store_true')
    parser.add_argument('--publish-check', action='store_true')
    parser.add_argument('--pdf', type=Path)
    parser.add_argument('--sha256', help='require the independently reviewed PDF hash')
    parser.add_argument('--notices', action='store_true')
    args = parser.parse_args()
    if args.self_test:
        self_test()
        return
    if args.output is None:
        parser.error('--output is required for acquisition')
    args.output.mkdir(parents=True, exist_ok=True)
    pdf = args.pdf or args.output / 'intel-sdm-093.pdf'
    if not args.pdf:
        with urllib.request.urlopen(URL, timeout=120) as response:
            data = response.read(100_000_001)
        if len(data) > 100_000_000 or not data.startswith(b'%PDF-'):
            raise ValueError('unexpected primary document response')
        pdf.write_bytes(data)
    digest = hashlib.sha256(pdf.read_bytes()).hexdigest()
    if args.sha256 and digest != args.sha256:
        raise ValueError('SDM source hash mismatch')
    import pymupdf
    document = pymupdf.open(pdf)
    toc = document.get_toc()
    rows = sections(toc)
    if len(rows) < 20:
        raise ValueError(f'vacuous SDM section selection: {len(rows)}')
    manifest = {'schema': 1, 'url': URL, 'requested_url': 'https://cdrdv2.intel.com/v1/dl/getContent/671200',
                'edition': '325462-093', 'sha256': digest, 'pages': len(document),
                'outline_entries': len(toc), 'extractor': pymupdf.__version__,
                'status': 'source acquisition only; review and compiler evidence remain separate',
                'sections': rows}
    (args.output / 'source-manifest.json').write_text(json.dumps(manifest, indent=2) + '\n')
    (args.output / 'outline.json').write_text(json.dumps(toc, indent=2) + '\n')
    if args.notices:
        notice('intel-sdm:source', json.dumps(manifest))
    for index, row in enumerate(rows):
        pages = []
        for number in range(row['pdf_page_first'], min(row['pdf_page_last'], len(document)) + 1):
            text = document[number - 1].get_text()
            pages.append({'pdf_page': number, 'text': text,
                          'text_sha256': hashlib.sha256(text.encode()).hexdigest()})
        (args.output / f'section-{index:03d}.json').write_text(json.dumps({'section': row, 'pages': pages}, indent=2) + '\n')
        if args.notices:
            for page in pages:
                notice(f'intel-sdm:{index:03d}:page-{page["pdf_page"]}',
                       json.dumps({'title': row['title'], **page}))
    if args.publish_check:
        publish_check(manifest, toc, sorted(args.output.glob('section-*.json')))
    print(f'Extracted {len(rows)} unreviewed sections from {len(document)} pages; SHA256 {digest}')


if __name__ == '__main__':
    main()
