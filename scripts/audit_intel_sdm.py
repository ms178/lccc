#!/usr/bin/env python3
"""Collect edition-anchored primary SDM sections, not semantic sign-off.

Run on a network-enabled host. The PDF remains outside Git. Notices make selected
primary pages available through the GitHub Checks API when artifact/log redirects
are inaccessible. A section's presence is never a claim it has been reviewed.
"""
import argparse
import hashlib
import json
from pathlib import Path
import re
import urllib.request

URL = 'https://cdrdv2-public.intel.com/929349/325462-093-sdm-vol-1-2abcd-3abcd-4.pdf'
INSTRUCTIONS = re.compile(r'^(?:DIVP[SD]|SQRTP[SD]|MINP[SD]|MAXP[SD]|MOV[AU]P[SD]|MOVDQU|MOVDQA|MOVSD|MOVSS|CVT(?:TP[SD]2DQ|P[SD]2DQ|DQ2P[SD])|VZEROUPPER|VZEROALL|PSHUFB|PTEST|DPPS|DPPD|ROUNDPS|ROUNDPD|PCMP(?:E|I)STR[IM])(?:[^A-Z]|$)')
ARCHITECTURE = re.compile(r'MXCSR|DATA TYPES|FLOATING.POINT EXCEPTION|MEMORY ORDERING|INSTRUCTION ORDERING|RIP.RELATIVE|DATA ALIGNMENT', re.I)


def sections(toc):
    selected = []
    architecture = 0
    for index, (level, title, page) in enumerate(toc):
        instruction = bool(INSTRUCTIONS.match(title.strip()))
        if not instruction and (architecture >= 16 or not ARCHITECTURE.search(title)):
            continue
        if not instruction:
            architecture += 1
        stop = next((p - 1 for lev, _, p in toc[index + 1:] if lev <= level and p > page), page + 2)
        selected.append({'title': title, 'pdf_page_first': page,
                         'pdf_page_last': min(stop, page + 3),
                         'kind': 'instruction' if instruction else 'architecture',
                         'status': 'extracted-unreviewed'})
    return selected


def notice(title, message):
    def escape(s):
        return s.replace('%', '%25').replace('\r', '%0D').replace('\n', '%0A')
    print(f'::notice title={escape(title)}::{escape(message)}', flush=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--pdf', type=Path)
    parser.add_argument('--sha256', help='require the independently reviewed PDF hash')
    parser.add_argument('--notices', action='store_true')
    args = parser.parse_args()
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
    print(f'Extracted {len(rows)} unreviewed sections from {len(document)} pages; SHA256 {digest}')


if __name__ == '__main__':
    main()
