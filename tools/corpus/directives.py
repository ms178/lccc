"""C comments, locations and explicit Clang RUN invocation declarations.

RUN commands are authoritative for imported Clang tests. EDG directives remain
separate inventory (or outcome-only fallback without RUNs); they are NEVER
positionally zipped to Clang prefixes. Unsupported syntax/conditional verifier
semantics are retained as reasons, not transformed into unqualified PASS.
"""
from __future__ import annotations

import json
import re
import shlex
from . import diagnostics,schema

ANNOTATION=re.compile(r'\b(?P<prefix>[A-Za-z0-9_]+(?:-[A-Za-z0-9_]+)*?)-'
                      r'(?P<kind>no-diagnostics|warning|error|note|remark)(?P<re>-re)?\b'
                      r'(?:\s*@(?P<loc>[^\s{]+))?(?:\s+(?P<count>\d+(?:-\d+|\+)?))?')
DIRECTIVE=re.compile(r'^\s*//([a-z_]+):\s*(.*)$')
RUN=re.compile(r'^\s*//\s*RUN:\s*(.*)$')
CONTROL=re.compile(r'^\s*//\s*(REQUIRES|UNSUPPORTED|XFAIL):\s*(.*)$')
LINE=re.compile(r'^\s*#\s*(?:line\s+)?(\d+)\s*("(?:[^"\\]|\\.)*")?(?:\s+\d+)*\s*$')
KNOWN_FLAGS={'-pedantic','-pedantic-errors','-w','-ffreestanding','-fno-builtin',
             '-fms-extensions','-fms-compatibility','-fblocks','-fno-blocks',
             '-fno-signed-char','-fsigned-char','-funsigned-char','-fshort-wchar',
             '-fshort-enums','-fno-common','-fcommon','-fno-diagnostics-show-option',
             '-fdiagnostics-show-option','-m32','-m64','-P','-dM','-dD','-undef',
             '-Werror','-Wall','-Wextra','-fheinous-gnu-extensions'}


def comments_and_code(text):
    """Equal-length comment/code masks, honoring quotes and physical newlines."""
    comments=list(text);code=list(text);i=0;quote=None;block=False;line_comment=False
    while i<len(text):
        c=text[i]
        if line_comment:
            code[i]='\n' if c=='\n' else ' '
            if c=='\n' and (i==0 or text[i-1]!='\\'):line_comment=False
            i+=1;continue
        if block:
            code[i]='\n' if c=='\n' else ' '
            if text[i:i+2]=='*/':
                code[i:i+2]=[' ',' '];i+=2;block=False
            else:i+=1
            continue
        if quote:
            comments[i]='\n' if c=='\n' else ' '
            if c=='\\' and i+1<len(text):
                comments[i+1]='\n' if text[i+1]=='\n' else ' ';i+=2;continue
            if c==quote:quote=None
            i+=1;continue
        if text[i:i+2] in ('//','/*'):
            line_comment=text[i:i+2]=='//';block=not line_comment
            code[i:i+2]=[' ',' '];i+=2;continue
        comments[i]='\n' if c=='\n' else ' '
        if c in ('"',"'"):quote=c
        i+=1
    return ''.join(comments),''.join(code)


def message_at(line,pos):
    while pos<len(line) and line[pos].isspace():pos+=1
    if line[pos:pos+2]!='{{':return '',pos,'missing {{message}}'
    start=pos+2;i=start;depth=1
    while i<len(line)-1:
        if line[i:i+2]=='{{':depth+=1;i+=2
        elif line[i:i+2]=='}}':
            depth-=1
            if not depth:return line[start:i],i+2,None
            i+=2
        else:i+=1
    return line[start:],len(line),'unclosed {{message}}'


def parse_text(text,*,prefix_lines=0):
    comments,code=comments_and_code(text)
    comment_lines=comments.splitlines();code_lines=code.splitlines();raw_lines=text.splitlines()
    # Physical copy locations -> presumed diagnostics, including #line. The
    # attribution prefix is fixed by the declared transform, not guessed +3.
    locations={};logical=prefix_lines+1;filename='$SOURCE';line_control_error=None
    for n,line in enumerate(code_lines,1):
        locations[n]=dict(file=filename,line=logical,any_file=False)
        m=LINE.match(line)
        if m:
            logical=int(m[1])
            if logical<0:line_control_error='invalid #line location'
            if re.search(r'\"\s+\d+',line):line_control_error='GNU line-marker stack/flags need SourceManager'
            if m[2]:
                try:filename=json.loads(m[2])
                except ValueError:line_control_error='unsupported #line filename escape'
            continue
        if re.match(r'^\s*#\s*line\b',line):line_control_error='nonliteral #line requires SourceManager'
        logical+=1
    edg={};runs=[];controls=[];header=True;pending=None
    for raw,comm,code_line in zip(raw_lines,comment_lines,code_lines):
        if header:
            m=DIRECTIVE.match(comm)
            if m:edg[m[1]]=m[2].strip()
            if code_line.strip():header=False
        rm=RUN.match(comm)
        if rm:
            command=rm[1].strip()
            pending=(pending+' '+command) if pending else command
            if pending.endswith('\\'):pending=pending[:-1].rstrip()
            else:runs.append(pending);pending=None
        cm=CONTROL.match(comm)
        if cm:controls.append((cm[1],cm[2]))
    if pending:runs.append(pending+' <UNTERMINATED-RUN>')
    expected=[];conditional_depth=0;continuation=False
    for n,(comm,code_line,raw) in enumerate(zip(comment_lines,code_lines,raw_lines),1):
        if re.match(r'^\s*#\s*(if|ifdef|ifndef)\b',code_line):conditional_depth+=1
        elif re.match(r'^\s*#\s*endif\b',code_line):conditional_depth=max(0,conditional_depth-1)
        pos=0
        while True:
            m=ANNOTATION.search(comm,pos)
            if not m:break
            kind=m['kind'];unsupported=[]
            if kind=='no-diagnostics':
                msg='';end=m.end()
                if m['loc'] or m['count'] or m['re']:unsupported.append('no-diagnostics marker cannot carry location/count/regex')
            else:
                msg,end,error=message_at(comm,m.end())
                if error:unsupported.append(error)
            pos=max(end,m.end())
            lo,hi=schema.count(m['count']);loc=m['loc'] or '';target=n
            location=dict(locations.get(n,dict(file='$SOURCE',line=None,any_file=False)))
            if re.fullmatch(r'[+-]\d+',loc):target=n+int(loc)
            elif loc.isdigit():target=int(loc)
            elif loc=='*':location['line']=None
            elif loc=='*:*':location=dict(file='$SOURCE',line=None,any_file=True)
            elif loc.startswith('#'):unsupported.append('label location needs SourceManager: '+loc)
            elif loc:
                fm=re.fullmatch(r'(.+):(\d+|\*)',loc)
                if fm:location=dict(file=fm[1],line=None if fm[2]=='*' else int(fm[2]),any_file=False)
                else:unsupported.append('unsupported diagnostic location: '+loc)
            if not loc or loc.isdigit() or re.fullmatch(r'[+-]\d+',loc):
                if target not in locations:unsupported.append('diagnostic physical line out of range')
                else:location=dict(locations[target])
            if conditional_depth:unsupported.append('conditional diagnostic requires frontend preprocessing')
            if continuation or raw.rstrip().endswith('\\'):unsupported.append('line-spliced diagnostic comment needs SourceManager')
            if line_control_error:unsupported.append(line_control_error)
            if loc=='*' and len({v['file'] for v in locations.values()})>1:
                unsupported.append('any-line across #line files needs SourceManager')
            if m['re']:
                try:diagnostics.template_regex(msg)
                except ValueError as exc:unsupported.append(str(exc))
            expected.append(dict(id=f'd{len(expected):04d}',prefix=m['prefix'],kind=kind,msg=msg,
                                 regex_form=bool(m['re']),count_min=lo,count_max=hi,
                                 annotation_line=n,location=location,unsupported=list(dict.fromkeys(unsupported))))
        continuation=raw.rstrip().endswith('\\') and '//' in comm
    return dict(edg=edg,runs=runs,controls=controls,expected=expected,
                conditional_unterminated=conditional_depth!=0,
                has_includes=bool(re.search(r'^\s*#\s*(include|include_next|import)\b',code,re.M)))


def clang_invocation(command,number,file_id,parsed):
    inv=dict(id=file_id+f'#run-{number:04d}',model='clang-run',command=command,
             language='c',phase='frontend',flags=[],target=None,prefixes=[],verify=False,
             expect='accept',xfail=False,expected_ids=[],ignore_unexpected=[],unsupported=[],capabilities=[])
    unsupported=inv['unsupported']
    try:
        lexer=shlex.shlex(command,posix=True,punctuation_chars='|&;<>');lexer.whitespace_split=True;lexer.commenters=''
        tokens=list(lexer)
    except ValueError as exc:
        unsupported.append('RUN tokenization: '+str(exc));return inv
    negative=False
    if tokens and tokens[0]=='not':negative=True;tokens=tokens[1:]
    if not tokens or tokens[0] not in ('%clang_cc1','%clang'):
        unsupported.append('unsupported RUN compiler/substitution');return inv
    cc1=tokens.pop(0)=='%clang_cc1'
    if not cc1:inv['phase']='link'
    if any(t in ('|','&&',';','>','>>','<','||','&','>&') for t in tokens):
        inv['phase']='filecheck' if any('FileCheck' in t for t in tokens) else 'execute'
        unsupported.append('RUN pipeline/redirection/stage not implemented')
    i=0;source_count=0
    while i<len(tokens):
        token=tokens[i];i+=1
        if token=='%s':source_count+=1;continue
        if token=='-verify' or token.startswith('-verify='):
            inv['verify']=True
            if inv['prefixes']:unsupported.append('repeated -verify prefix policy needs native verifier adjudication')
            prefixes=token.partition('=')[2].split(',') if '=' in token else ['expected']
            if not all(re.fullmatch(r'[A-Za-z][A-Za-z0-9_-]*',p) for p in prefixes):unsupported.append('invalid verifier prefix')
            inv['prefixes'].extend(p for p in prefixes if p);continue
        if token=='-verify-ignore-unexpected' or token.startswith('-verify-ignore-unexpected='):
            kinds=token.partition('=')[2].split(',') if '=' in token else ['warning','error','note','remark']
            if not set(kinds)<=schema.KINDS-{'no-diagnostics'}:unsupported.append('invalid ignored diagnostic kind')
            inv['ignore_unexpected'].extend(k for k in kinds if k in schema.KINDS-{'no-diagnostics'});continue
        if token=='-fsyntax-only':inv['phase']='frontend';continue
        if token=='-E':inv['phase']='preprocess';continue
        if token in ('-S','-c','-emit-llvm','-emit-pch','-emit-obj'):
            if inv['phase'] not in ('filecheck','execute'):inv['phase']='codegen'
            unsupported.append('code generation/output oracle not implemented');continue
        if token in ('-x','-triple','-target','--target','-D','-U','-I','-include','-isystem','-o'):
            if i>=len(tokens):unsupported.append('missing option operand: '+token);continue
            operand=tokens[i];i+=1
            if token=='-x':
                if operand not in ('c','c++'):
                    inv['language']='unsupported';unsupported.append('unsupported language: '+operand)
                else:inv['language']=operand
            elif token in ('-triple','-target','--target'):
                inv['target']=operand;inv['flags'].append('--target='+operand);inv['capabilities'].append('clang-driver-target')
            elif token=='-o':
                if operand!='-':unsupported.append('output-file stage not implemented')
            else:
                if '%' in operand:unsupported.append('unresolved operand substitution: '+operand)
                inv['flags'].extend([token,operand])
                if token in ('-I','-include','-isystem'):inv['capabilities'].append('external-inputs')
            continue
        if token.startswith('--target='):
            inv['target']=token.split('=',1)[1];inv['flags'].append(token);inv['capabilities'].append('clang-driver-target');continue
        if token.startswith(('-std=','-D','-U','-I')):
            inv['flags'].append(token)
            if token.startswith('-I'):inv['capabilities'].append('external-inputs')
            continue
        if token in KNOWN_FLAGS or token.startswith('-W') or re.fullmatch(r'-fgnuc-version=\d+(?:\.\d+)*',token):
            inv['flags'].append(token)
            # Do not certify a flag an immature driver might silently ignore.
            if token not in {'-Werror','-Wall','-Wextra','-pedantic','-pedantic-errors','-w','-P','-dM','-dD','-undef'}:
                inv['capabilities'].append('flag:'+token)
            continue
        unsupported.append('untranslated RUN token: '+token)
    if source_count!=1:unsupported.append('RUN requires exactly one %s input')
    if negative and inv['verify']:unsupported.append('not with -verify tests verifier failure, not a C rejection')
    active=[e for e in parsed['expected'] if e['prefix'] in inv['prefixes']] if inv['verify'] else []
    inv['expected_ids']=[e['id'] for e in active]
    inv['expect']='reject' if negative or any(e['kind']=='error' and e['count_min']>0 for e in active) else 'accept'
    if inv['verify'] and not active:unsupported.append('active verifier prefixes have no declarations')
    if (inv['verify'] and inv['expect']=='accept' and
            any(e['kind']=='error' and e['count_min']==0 and (e['count_max'] is None or e['count_max']>0) for e in active)):
        unsupported.append('optional-error verifier outcome needs native verifier adjudication')
    if any(e['kind']=='no-diagnostics' for e in active) and any(e['kind']!='no-diagnostics' for e in active):
        unsupported.append('conflicting no-diagnostics declaration')
    for e in active:unsupported.extend(e['unsupported'])
    if parsed['conditional_unterminated']:unsupported.append('unterminated preprocessor condition')
    if parsed.get('has_includes'):unsupported.append('source include graph not resolved/pinned')
    for name,value in parsed['controls']:
        if name=='XFAIL' and value=='*':inv['xfail']=True
        else:unsupported.append('lit '+name+' constraint not evaluated: '+value)
    if any('FileCheck' in t for t in tokens):inv['phase']='filecheck'
    elif any(t in ('&&',';','||') for t in tokens):inv['phase']='execute'
    if inv['phase'] not in ('frontend','preprocess'):unsupported.append('unsupported execution phase: '+inv['phase'])
    inv['unsupported']=list(dict.fromkeys(unsupported));inv['capabilities']=list(dict.fromkeys(inv['capabilities']))
    inv['prefixes']=list(dict.fromkeys(inv['prefixes']));inv['ignore_unexpected']=list(dict.fromkeys(inv['ignore_unexpected']))
    return inv


def build_invocations(file_id,parsed,translate):
    if parsed['runs']:
        return [clang_invocation(cmd,n,file_id,parsed) for n,cmd in enumerate(parsed['runs'])]
    # A single opaque unsupported source plan preserves inventory without
    # inventing an options/cases Cartesian product. It is not executable
    # coverage and never borrows Clang annotations. EDG cardinalities remain
    # facts in parsed['edg']; only actual Clang RUNs define executable plans.
    edg=parsed['edg'];shared=edg.get('options_all','');options=edg.get('options','')
    flags,arch,unknown=translate(shared+' '+options)
    kind=edg.get('type','cp');phase='frontend' if kind in ('fp','fn') else 'codegen'
    reason=['EDG-only execution/diagnostic association not implemented']
    if unknown:reason.append('untranslated EDG options: '+', '.join(unknown))
    if edg.get('cases') or ';' in options:reason.append('EDG cases/type overrides not implemented')
    if arch:reason.append('EDG target mapping not implemented')
    if kind not in ('fp','fn'):reason.append('EDG phase/catastrophic/skip semantics not implemented')
    return [dict(id=file_id+'#edg-0000',model='edg-options',command=shared+' '+options or '(default)',
        language='c++' if re.search(r'--c\+\+|--cpp|--cp\b',shared+' '+options) else 'c',phase=phase,
        flags=flags,target=arch[0] if arch else None,prefixes=[],verify=False,expect='reject' if kind=='fn' else 'accept',
        xfail=False,expected_ids=[],ignore_unexpected=[],unsupported=reason,capabilities=[])]
