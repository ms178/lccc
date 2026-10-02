"""Structured GCC/Clang/LCCC/ICC text diagnostics and occurrence verification.

Source/caret/include-stack echoes never enter the message stream. Literal
Clang directives use substring matching; only -re directives splice {{ERE}}
fragments. Regex support is deliberately an ASCII POSIX-ERE subset, not an
unqualified reimplementation of Clang's SourceManager/verifier.
"""
from __future__ import annotations

from collections import deque
from pathlib import Path
import re

ANSI = re.compile(r'\x1b\[[0-?]*[ -/]*[@-~]')
LOCATED = re.compile(r'^(?P<file>.*?):(?P<line>\d+)(?::(?P<column>\d+))?:\s*'
                     r'(?P<kind>fatal error|error|warning|note|remark):\s*(?P<message>.*)$')
ICC = re.compile(r'^(?P<file>.+?)\((?P<line>\d+)\):\s*'
                 r'(?P<kind>error|warning|remark|note)(?:\s+#\d+)?:\s*(?P<message>.*)$')
DRIVER = re.compile(r'^(?:[^\s:]+:\s*)?(?P<kind>fatal error|error|warning|note|remark):\s*(?P<message>.*)$')


def parse(stderr: str, *, compiler_family="custom", returncode=None) -> list[dict]:
    records = []
    for raw in ANSI.sub('', stderr).splitlines():
        # Leading whitespace belongs to context/source excerpts, not a
        # diagnostic header. In particular do not search *inside* a line.
        if raw[:1].isspace():
            continue
        m = LOCATED.match(raw) or ICC.match(raw) or DRIVER.match(raw)
        if not m:
            continue
        g = m.groupdict()
        records.append(dict(index=len(records), kind='error' if g['kind']=='fatal error' else g['kind'],
                            file=g.get('file'), line=int(g['line']) if g.get('line') else None,
                            column=int(g['column']) if g.get('column') else None,
                            message=g['message'], raw=raw))
    # LCCC's driver repeats the error count after its located frontend
    # diagnostics. This is a summary, not a second language diagnostic. Only
    # an explicitly identified LCCC profile, rc=1 and an exact count match may
    # omit it. Unknown/wrong-count/mixed driver errors remain infrastructure.
    if compiler_family == 'lccc' and returncode == 1:
        errors = [d for d in records if d['kind'] == 'error']
        summaries = [d for d in errors if d['file'] is None]
        located = [d for d in errors if d['file'] is not None]
        if len(summaries) == 1 and located:
            message = summaries[0]['message']
            count = (re.fullmatch(r'(\d+) error\(s\) during semantic analysis', message)
                     or re.fullmatch(r'.+: (\d+) (?:parse|frontend) error\(s\)', message)
                     or re.fullmatch(r'(\d+) preprocessor error\(s\) in .+', message)
                     or re.fullmatch(r'(\d+) error\(s\) \(warnings promoted by -Werror\)', message))
            if (summaries[0]['raw'].startswith('ccc: error: ') and count
                    and int(count[1]) == len(located)):
                records = [d for d in records if d is not summaries[0]]
    return records


def template_regex(text: str) -> re.Pattern:
    """Compile literal chunks and {{ERE}} chunks, rejecting dialect drift."""
    if len(text)>2048:
        raise ValueError('regex template resource budget exceeded')
    pieces, pos, unbounded = [], 0, 0
    while True:
        start = text.find('{{', pos)
        if start < 0:
            pieces.append(re.escape(text[pos:])); break
        end = text.find('}}', start + 2)
        if end < 0:
            raise ValueError('unclosed regex fragment')
        pieces.append(re.escape(text[pos:start]))
        ere = text[start+2:end]
        if not ere.isascii() or re.search(r'\\[A-Za-z0-9]',ere):
            raise ValueError('regex character/escape outside ASCII POSIX-ERE subset')
        if re.search(r'(?:[*+?]|\})[+*?]',ere):
            raise ValueError('stacked/possessive regex quantifier unsupported')
        # Python-specific operators would not mean the same thing to LLVM's
        # POSIX ERE engine. Fail closed; never silently translate those.
        if re.search(r'\(\?|\\(?:[1-9dDsSwWbBAZpPkK])|\[\[:|\*\?|\+\?|\?\?', ere):
            raise ValueError('regex outside supported POSIX-ERE subset')
        # Avoid catastrophic backtracking while retaining useful single-fragment
        # patterns such as {{.*}} and {{[0-9]+}}. This subset is explicit.
        if ere.count('(')>64:
            raise ValueError('regex nesting resource budget exceeded')
        if re.search(r'(?<!\\)\)[*+?{]', ere):
            raise ValueError('quantified regex groups outside safe subset')
        for bound in re.findall(r'\{(\d+)(?:,(\d*))?\}',ere):
            if any(v and int(v)>1024 for v in bound):
                raise ValueError('regex repetition budget exceeded')
        unbounded += len(re.findall(r'(?<!\\)[*+]',ere))
        if unbounded>2:
            raise ValueError('regex unbounded-quantifier budget exceeded')
        pieces.append('(?:' + ere + ')')
        pos = end + 2
    try:
        return re.compile(''.join(pieces), re.ASCII)
    except (re.error,RecursionError,OverflowError) as exc:
        raise ValueError(f'invalid regex fragment: {exc}') from exc


def same_file(actual: str | None, wanted: str, source: Path) -> bool:
    if actual is None:
        return False
    if wanted == '$SOURCE':
        # Both absolute paths and source.name are valid compiler spellings.
        # Do not accept another directory just because its basename matches.
        return (actual == str(source) or actual == source.name or
                Path(actual).absolute() == source.absolute())
    return actual == wanted


def verify(expectations: list[dict], observed: list[dict], *, source: Path,
           ignore_unexpected: list[str] = (), _local_regex: bool = False) -> dict:
    if not _local_regex and any(e['regex_form'] for e in expectations):
        return _bounded_regex_verification(expectations,observed,source,ignore_unexpected)
    result = dict(ok=False, matches=[], missing=[], unexpected=[], detail='')
    if (any(len(diag['message']) > 8192 for diag in observed)
            or len(observed) > 512 or len(observed) * len(expectations) > 65536
            or len(expectations) > 4096):
        result['detail'] = 'diagnostic matching resource budget exceeded'
        return result
    no_diag = [e for e in expectations if e['kind']=='no-diagnostics']
    if no_diag:
        if len(no_diag) != len(expectations):
            result['detail'] = 'no-diagnostics conflicts with other active expectations'
        elif observed:
            result['unexpected'] = [d['index'] for d in observed]
            result['detail'] = 'expected no diagnostics'
        else:
            result['ok'] = True
        return result
    patterns = {}
    for e in expectations:
        if e.get('unsupported_reason'):
            result['detail'] = f"unsupported expectation {e['id']}: {e['unsupported_reason']}"
            return result
        if e['regex_form']:
            try:
                patterns[e['id']] = template_regex(e['message'])
            except ValueError as exc:
                result['detail'] = f"{e['id']}: {exc}"; return result
    def accepts(e, d):
        loc = e['location']
        if e['kind'] != d['kind']:
            return False
        if not loc['any_file'] and not same_file(d['file'], loc['file'], source):
            return False
        if loc['line'] is not None and loc['line'] != d['line']:
            return False
        return (patterns[e['id']].search(d['message']) is not None if e['regex_form']
                else e['message'] in d['message'])
    n = len(observed)
    candidates = [[i for i, diag in enumerate(observed) if accepts(e, diag)]
                  for e in expectations]
    # Capacitated one-to-one matching. Mandatory slots first; subsequent
    # augmenting paths keep every mandatory slot occupied. This handles
    # overlapping templates without order-dependent greedy false failures.
    slots, owners, assignment = [], {}, {}
    needed = {i for i, diag in enumerate(observed) if diag['kind'] not in ignore_unexpected}
    needed_matched = 0
    def fill(slot):
        nonlocal needed_matched
        q, prev_slot, prev_diag = deque([slot]), {slot: None}, {}
        end = None
        while q and end is None:
            s = q.popleft()
            for diag in candidates[slots[s]]:
                if diag in prev_diag:
                    continue
                prev_diag[diag] = s
                old = owners.get(diag)
                if old is None:
                    end = diag; break
                if old not in prev_slot:
                    prev_slot[old] = diag; q.append(old)
        if end is None:
            return False
        if end in needed:
            needed_matched += 1
        diag = end
        while True:
            s = prev_diag[diag]
            prior = assignment.get(s)
            assignment[s], owners[diag] = diag, s
            if prior is None:
                break
            diag = prior
        return True
    if n > 4096 or len(expectations) > 4096:
        result['detail'] = 'diagnostic verifier resource budget exceeded'; return result
    for ei, e in enumerate(expectations):
        minimum = e['count_min']
        if minimum > n:
            result['missing'].append(dict(id=e['id'], count=minimum))
            result['detail'] = 'insufficient diagnostics for required count'; return result
        for _ in range(minimum):
            s = len(slots); slots.append(ei)
            if not fill(s):
                result['missing'].append(dict(id=e['id'], count=1))
                result['detail'] = 'no one-to-one assignment satisfies required occurrences'
                return result
    for ei, e in enumerate(expectations):
        if needed_matched == len(needed):
            break
        # Mandatory slots have already been satisfied. Ignored kinds need
        # no optional occurrences; a future optional slot cannot help them.
        if e['kind'] in ignore_unexpected:
            continue
        upper = min(n, e['count_max'] if e['count_max'] is not None else n,
                    len(candidates[ei]))
        for _ in range(max(0, upper - e['count_min'])):
            if needed_matched == len(needed):
                break
            slot = len(slots)
            slots.append(ei)
            if not fill(slot):
                # Identical copies have identical adjacency: if this copy
                # cannot augment a maximum matching, adding another cannot.
                # Subsequent distinct left vertices still extend that maximum
                # matching by the usual augmenting-path invariant.
                slots.pop()
                break
    result['matches'] = sorted((dict(expectation=expectations[slots[s]]['id'],
                                     diagnostic=observed[diag]['index'])
                                 for s, diag in assignment.items()),
                                key=lambda m: (m['expectation'], m['diagnostic']))
    result['unexpected'] = [diag['index'] for i, diag in enumerate(observed)
                            if i not in owners and diag['kind'] not in ignore_unexpected]
    result['ok'] = not result['unexpected']
    if not result['ok']:
        result['detail'] = 'unmatched diagnostics under strict policy'
    return result

# Anchored so an echoed C comment/string containing these words is not an ICE.
CRASH_HEADER = re.compile(r"^(?:[^\s:]+:\s*)?(?:internal compiler error|internal error:|"
                          r"segmentation fault|signal SIG|panic:|thread '.+'(?: \([0-9]+\))? panicked at)",
                          re.IGNORECASE|re.MULTILINE)


def classify(returncode,stderr,*,failure=None,compiler_family="custom",parsed=None):
    """Never turn panic/wrapper/driver/transport failures into C rejection.

    A normal rejection needs rc=1 and at least one located error header.
    Unknown nonzero codes and driver-only errors remain infrastructure ERROR.
    The text is the decoded diagnostic channel, not echoed source substrings.
    """
    if failure=='timeout':return 'TIMEOUT'
    if failure:return 'ERROR'
    if returncode is None:return 'ERROR'
    if returncode<0 or returncode>=128 or CRASH_HEADER.search(ANSI.sub('',stderr)):return 'CRASH'
    if '\ufffd' in stderr:return 'ERROR'
    headers = parse(stderr, compiler_family=compiler_family, returncode=returncode) if parsed is None else parsed
    errors=[d for d in headers if d['kind']=='error']
    if returncode==0:return 'ERROR' if errors else 'ACCEPT'
    if returncode==1 and errors and all(d['file'] is not None for d in errors):return 'REJECT'
    return 'ERROR'


REGEX_TIMEOUT=0.75


def _bounded_regex_verification(expectations,observed,source,ignore_unexpected):
    """Isolate CPython backtracking, not just compiler time/output, per verify.

    One worker per regex-bearing invocation, NOT one subprocess per edge. A
    timeout/output budget is infrastructure ERROR at the runner boundary, not
    a semantic XFAIL. Plain literal verification stays in-process.
    """
    import sys
    import tempfile
    from . import process,schema
    failure=dict(ok=False,matches=[],missing=[],unexpected=[],detail='regex matching resource budget/worker failure')
    payload=dict(expectations=expectations,observed=observed,source=str(source),ignore_unexpected=list(ignore_unexpected))
    data=schema.dumps(payload).encode()
    if len(data)>2*1024*1024:return failure
    with tempfile.TemporaryDirectory(prefix='lccc-diagnostic-verify-') as td:
        path=Path(td)/'input.json';path.write_bytes(data)
        try:
            r=process.run([sys.executable,'-m','tools.corpus.diagnostics','--verify-worker',str(path)],REGEX_TIMEOUT,
                          env={'PYTHONPATH':str(Path(__file__).resolve().parents[2])})
            if r['failure'] or r['returncode']!=0:return failure
            return schema.loads(r['stdout'].decode('utf-8'))
        except (OSError,ValueError):return failure


if __name__=='__main__':
    import sys
    from . import schema
    if len(sys.argv)!=3 or sys.argv[1]!='--verify-worker':raise SystemExit(2)
    payload=schema.loads(Path(sys.argv[2]).read_text())
    result=verify(payload['expectations'],payload['observed'],source=Path(payload['source']),
                  ignore_unexpected=payload['ignore_unexpected'],_local_regex=True)
    print(schema.dumps(result),end='')
