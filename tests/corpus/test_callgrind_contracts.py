#!/usr/bin/env python3
"""Pure/mocked Callgrind orchestration; no compiler/program/Valgrind runs."""
import hashlib
import importlib.util
from pathlib import Path
import os
import sys
import tempfile
import unittest
from unittest import mock

REPO=Path(__file__).resolve().parents[2];sys.path.insert(0,str(REPO))
def load():
    spec=importlib.util.spec_from_file_location('cg_contract',REPO/'scripts/callgrind_ab.py')
    module=importlib.util.module_from_spec(spec);spec.loader.exec_module(module);return module
cg=load()
from tools.corpus import process,schema


def ev(ir=100):return dict(Ir=ir,I1mr=1,D1mr=2,D1mw=3,ILmr=4,DLmr=5,DLmw=6,Bcm=7,Bim=8)
def result(rc=0,stdout=b'ok\n',failure=None):
    r=process.normalize((rc,stdout,b''));r['failure']=failure;return r


class CallgrindContract(unittest.TestCase):
    def test_import_no_discovery(self):
        with mock.patch.object(process,'run',side_effect=AssertionError('eager discovery')):load()
    def test_no_builtin_shadow(self):self.assertFalse(hasattr(cg,'compile'));self.assertTrue(callable(cg.compile_program))
    def test_lazy_gcc_include(self):
        cg.gcc_include.cache_clear()
        with tempfile.TemporaryDirectory() as td,mock.patch.object(cg,'_run',return_value=result(stdout=(td+'\n').encode())) as run:
            self.assertEqual(cg.gcc_include('fake-gcc'),'-I'+td);self.assertEqual(run.call_count,1)
            self.assertEqual(cg.gcc_include('fake-gcc'),'-I'+td);self.assertEqual(run.call_count,1)
    def test_invalid_include_path(self):
        cg.gcc_include.cache_clear()
        with mock.patch.object(cg,'_run',return_value=result(stdout=b'/definitely/not/headers\n')):
            with self.assertRaises(ValueError):cg.gcc_include('fake-include')
    def test_geometry_fixed_and_visible_override(self):
        with mock.patch.dict(os.environ,{'LCCC_CG_D1':'65536,8,64'}):
            model,override=cg.geometry();self.assertEqual(model['I1'],'32768,8,64');self.assertEqual(model['LL'],'33554432,16,64')
            self.assertEqual(override['LCCC_CG_D1'],model['D1'])
    def test_invalid_geometry_rejected(self):
        for value in ('0,8,64','32768,8,63','abc','32769,8,64'):
            with mock.patch.dict(os.environ,{'LCCC_CG_I1':value}),self.assertRaises(ValueError):cg.geometry()
    def test_zero_Ir_both_sides(self):
        for a,b in ((ev(0),ev()),(ev(),ev(0))):
            with self.assertRaises(ValueError):cg.compare_events(a,b)
    def test_missing_negative_bool_counters_rejected(self):
        for events in ({'Ir':1},dict(ev(),D1mw=-1),dict(ev(),Ir=True)):
            with self.assertRaises(ValueError):cg.validate_events(events)
    def test_write_misses_included(self):
        m=cg.compare_events(ev(),ev(90));self.assertEqual(m['d1_misses'],[5,5]);self.assertEqual(m['ll_misses'],[15,15]);self.assertEqual(m['ir_ratio'],.9)
    def test_summary_strict_cardinality(self):
        with tempfile.TemporaryDirectory() as td:
            p=Path(td)/'counts';p.write_text('events: '+ ' '.join(ev())+'\nsummary: '+ ' '.join(map(str,ev().values()))+'\n')
            self.assertEqual(cg.parse_summary(p),ev())
            for text in ('events: Ir Ir\nsummary: 1 2\n','events: Ir\nsummary: 1 2\n','events: Ir\nsummary: x\n','events: Ir\nsummary: 0\n'):
                p.write_text(text)
                with self.assertRaises(ValueError):cg.parse_summary(p)
    def test_correctness_exit_AND_stdout(self):
        self.assertIsNone(cg.same_output(result(),result()))
        for other in (result(rc=-11),result(rc=127),result(stdout=b'different'),result(failure='timeout')):
            self.assertIsNotNone(cg.same_output(result(),other))
    def test_compile_quoted_options_and_no_stale_binary(self):
        with tempfile.TemporaryDirectory() as td:
            out=Path(td)/'program';out.write_text('stale');out.chmod(0o755)
            with mock.patch.object(cg,'_run',return_value=result(rc=1)) as run:
                self.assertIsNotNone(cg.compile_program('fake-cc','-O2 -DNAME="a b"','source.c',out,include='-Ifake'))
                self.assertIn('-DNAME=a b',run.call_args.args[0]);self.assertFalse(out.exists())
    def test_zero_exit_without_binary_is_failure(self):
        with tempfile.TemporaryDirectory() as td,mock.patch.object(cg,'_run',return_value=result()):
            self.assertIsNotNone(cg.compile_program('fake-cc','-O2','source.c',Path(td)/'missing',include='-Ifake'))
    def test_instrumented_output_checked_and_stale_counts_removed(self):
        with tempfile.TemporaryDirectory() as td:
            path=Path(td);(path/'program.cg').write_text('stale')
            with mock.patch.object(cg,'_run',return_value=result(stdout=b'wrong')):
                events,error,info=cg.callgrind(path/'program',path,expected_stdout=b'ok\n')
                self.assertIsNone(events);self.assertIn('stdout',error);self.assertFalse((path/'program.cg').exists())
                self.assertIn('process',info)


class MainContract(unittest.TestCase):
    def mock_measurement(self,outroot,events):
        def compiler(cmd,opt,src,out,**kw):out.write_bytes(b'dummy-not-a-compiler-output');out.chmod(0o755)
        with mock.patch.object(cg,'compile_program',side_effect=compiler),mock.patch.object(cg,'tool_identity',return_value={'version':'mock','sha256':'a'*64}),mock.patch.object(cg,'gcc_include',return_value='-Ifake'),mock.patch.object(cg,'_run',return_value=result()),mock.patch.object(cg,'callgrind',return_value=(events,None,{'mock':True})):
            return cg.main(['mock-mine','mock-ref','-O2','fib','--out-root',str(outroot)])
    def test_success_manifest_equal_paths_geometry_and_hashes(self):
        with tempfile.TemporaryDirectory() as td:
            root=Path(td);self.assertEqual(self.mock_measurement(root,ev()),0)
            doc=schema.loads((root/'manifest.json').read_text());self.assertEqual(doc['comparable'],1)
            self.assertEqual(len(doc['paths']['mine']),len(doc['paths']['ref']));self.assertEqual(Path(doc['paths']['mine']).name,'aa');self.assertEqual(Path(doc['paths']['ref']).name,'bb')
            self.assertTrue(doc['results']['fib']['correctness']['mine']['stdout_sha256'])
            self.assertIn('not PMU',doc['model'])
    def test_zero_Ir_failure_manifest_not_division_or_log_error(self):
        with tempfile.TemporaryDirectory() as td:
            root=Path(td);self.assertEqual(self.mock_measurement(root,ev(0)),2)
            doc=schema.loads((root/'manifest.json').read_text());self.assertEqual(doc['comparable'],0);self.assertIn('fib',doc['failures']);self.assertNotIn('geomean_ir_mine_over_ref',doc)
    def test_partial_failure_returns_failure_not_success(self):
        with tempfile.TemporaryDirectory() as td,mock.patch.object(cg,'_measure',return_value=1) as measure,mock.patch.object(cg,'geometry',return_value=({'I1':cg.CG_I1,'D1':cg.CG_D1,'LL':cg.CG_LL},{})):
            self.assertEqual(cg.main(['mock-a','mock-b','-O2','fib','--out-root',td]),1)
    def test_setup_error_manifest_observable(self):
        with tempfile.TemporaryDirectory() as td,mock.patch.object(cg,'gcc_include',side_effect=ValueError('missing GCC')):
            self.assertEqual(cg.main(['mock-a','mock-b','-O2','fib','--out-root',td]),2)
            self.assertIn('setup',schema.loads((Path(td)/'manifest.json').read_text())['failures'])
    def test_heavy_is_used_opt_in(self):
        with tempfile.TemporaryDirectory() as td,mock.patch.object(cg,'_measure',return_value=2) as measure:
            cg.main(['mock-a','mock-b','-O2','--heavy','--out-root',td]);self.assertTrue(cg.HEAVY<=set(measure.call_args.args[1]))

class ArgumentVectorContract(unittest.TestCase):
    """The documented CLI parses identically on every supported CPython.

    `main` rewrites the third positional (the `MINE REF "OPT [OPT...]"` option
    string) into `--opt=VALUE`, which puts an OPTIONAL in front of the `bench`
    positional.  Plain `parse_args` consumes positionals in the contiguous
    groups the optionals split them into, and `nargs='*'` is satisfied by an
    EMPTY group -- so CPython 3.12.3, which is what Ubuntu 24.04 ships and
    therefore what hosted CI runs, binds `bench` to nothing and rejects every
    bench name as `unrecognized arguments: fib`.  Later 3.12 patches and 3.13
    group them correctly.  That single version split is why PR #730 was green on
    every developer host and red on the runner (4 errors, step "Verify remaining
    fast local contracts"), and `parse_intermixed_args` is what closes it.

    These drive the REAL `main`, never a replica parser: a replica keeps passing
    after the script's parser changes shape, which is how the defect shipped in
    the first place.  Run on an unpatched 3.12 they fail loudly if anyone
    reverts to `parse_args`.
    """

    def manifest_for(self,argv):
        """Run the real `main` over `argv` and return the manifest it wrote.

        `--out-root` is appended unless `argv` already carries it; the literal
        token OUTROOT anywhere in `argv` expands to the temporary directory, so
        a case can place bench names on either side of that option's value.
        """
        def compiler(cmd,opt,src,out,**kw):
            out.write_bytes(b'dummy-not-a-compiler-output');out.chmod(0o755)
        with tempfile.TemporaryDirectory() as td:
            argv=[td if a=='OUTROOT' else a for a in argv]
            if '--out-root' not in argv:argv=argv+['--out-root',td]
            with mock.patch.object(cg,'compile_program',side_effect=compiler),\
                 mock.patch.object(cg,'tool_identity',return_value={'version':'mock','sha256':'a'*64}),\
                 mock.patch.object(cg,'gcc_include',return_value='-Ifake'),\
                 mock.patch.object(cg,'_run',return_value=result()),\
                 mock.patch.object(cg,'callgrind',return_value=(ev(),None,{'mock':True})):
                self.assertEqual(cg.main(argv),0)
            return schema.loads((Path(td)/'manifest.json').read_text())

    def test_bench_names_after_the_option_string(self):
        # The exact argv shape that was red on the runner.
        doc=self.manifest_for(['mock-mine','mock-ref','-O2','fib'])
        self.assertEqual(doc['opt'],'-O2')
        self.assertEqual(sorted(doc['results']),['fib'])

    def test_bench_names_trailing_another_options_value(self):
        doc=self.manifest_for(['mock-mine','mock-ref','-O2','--out-root','OUTROOT','fib'])
        self.assertEqual(doc['opt'],'-O2')
        self.assertEqual(sorted(doc['results']),['fib'])

    def test_bench_names_after_a_flag_option(self):
        doc=self.manifest_for(['mock-mine','mock-ref','-O2','--heavy','fib'])
        self.assertEqual(sorted(doc['results']),['fib'])
        self.assertTrue(doc['heavy_opt_in'] is False)  # 'fib' is not a heavy bench

    def test_multi_token_option_string_with_several_benches(self):
        doc=self.manifest_for(['mock-mine','mock-ref','-O2 -march=x86-64-v3','fib','nbody'])
        self.assertEqual(doc['opt'],'-O2 -march=x86-64-v3')
        self.assertEqual(doc['opt_tokens'],['-O2','-march=x86-64-v3'])
        self.assertEqual(sorted(doc['results']),['fib','nbody'])

    def test_explicit_opt_option_is_not_rewritten(self):
        doc=self.manifest_for(['mock-mine','mock-ref','--opt=-O3','fib'])
        self.assertEqual(doc['opt'],'-O3')
        self.assertEqual(sorted(doc['results']),['fib'])

    def test_bare_double_dash_keeps_its_meaning(self):
        # Hand-rolled argv reordering hoists the `--` away and argparse then
        # reads the following token as an unknown option; the stdlib honours it.
        # `--out-root` must precede the separator: after it, everything is a
        # positional by definition, which is what `--` means.
        doc=self.manifest_for(['mock-mine','mock-ref','-O2','--out-root','OUTROOT','--','fib'])
        self.assertEqual(sorted(doc['results']),['fib'])

    def test_missing_option_string_is_reported_not_swallowed(self):
        # `mine ref --out-root X` omits the required option string; rewriting
        # argv[2] into `--opt=--out-root` would hide that behind a bogus value.
        with mock.patch.object(sys,'argv',['callgrind_ab.py']),self.assertRaises(SystemExit) as caught:
            cg.main(['mock-mine','mock-ref','--out-root','/tmp/lccc-cg-contract'])
        self.assertEqual(caught.exception.code,2)


class InterspersedCliRatchetTests(unittest.TestCase):
    """A `nargs='*'` positional parsed by plain `parse_args` is a latent red CI.

    argparse consumes positionals in the contiguous groups the optionals split
    them into, and `nargs='*'` is satisfied by an EMPTY group -- so on CPython
    3.12.3, which Ubuntu 24.04 ships and hosted CI therefore runs, one optional
    typed before the star positional makes argparse reject every trailing
    positional as `unrecognized arguments`.  Later 3.12 patches and 3.13 group
    them correctly, which is precisely how PR #730 stayed green on every
    developer host and turned the Test Suite red on the runner.

    `parse_intermixed_args` is the fix, and it is safe for every parser here:
    it refuses only subparsers, REMAINDER and mixed mutually-exclusive groups,
    and this scan asserts none of them are present.  The scripts in KNOWN
    predate the fix and are all invoked optionals-first (verified against
    .github/workflows/ci.yml), so they are recorded as a ratchet that may only
    shrink -- the same shape check_env_test_hygiene.sh uses for its
    environment-read budget.  A NEW offender fails here instead of in CI.
    """

    KNOWN=frozenset((
        'scripts/asmdiff.py','scripts/census_ab.py','scripts/check_conditionals_family.py',
        'scripts/comdat_registration_bench.py','scripts/distill_evex_opcodes.py','scripts/encdiff.py',
        'scripts/gen_encoding_sweep.py','scripts/i686_alu_redteam.py','scripts/peephole_trace_bisect.py',
        'scripts/perf_ab.py','scripts/ra_quality_census.py','scripts/stack_census.py',
        'scripts/tight_loop_oracle.py','scripts/x86_gcc_torture.py','tests/stress/reduce_ice.py',
    ))

    def offenders(self):
        import ast
        found=set()
        for path in sorted(REPO.rglob('*.py')):
            rel=path.relative_to(REPO).as_posix()
            if rel.startswith('tests/corpus/clang-c/') or '/clang-c/' in rel:continue
            try:tree=ast.parse(path.read_text(),rel)
            except SyntaxError:continue
            star=False;plain=False;incompatible=False
            for node in ast.walk(tree):
                if not (isinstance(node,ast.Call) and isinstance(node.func,ast.Attribute)):continue
                if node.func.attr=='add_argument':
                    for kw in node.keywords:
                        if kw.arg!='nargs':continue
                        if isinstance(kw.value,ast.Constant) and kw.value.value=='*':star=True
                        if isinstance(kw.value,ast.Attribute) and kw.value.attr in ('REMAINDER','PARSER'):incompatible=True
                    if any(isinstance(a,ast.Constant) and isinstance(a.value,str) and a.value=='*subparsers*' for a in node.args):
                        incompatible=True
                if node.func.attr=='add_subparsers':incompatible=True
                if node.func.attr in ('parse_args','parse_known_args'):plain=True
            if star and plain and not incompatible:found.add(rel)
        return found

    def test_no_new_star_positional_parsed_by_plain_parse_args(self):
        found=self.offenders()
        new=sorted(found-self.KNOWN)
        self.assertEqual(new,[],
            'these parse a nargs="*" positional with plain parse_args; use '
            'parse_intermixed_args so an optional before the positional cannot '
            'starve it on CPython 3.12.3 (the hosted runner): '+', '.join(new))

    def test_known_ratchet_has_not_grown_and_records_progress(self):
        found=self.offenders()
        self.assertLessEqual(len(found),len(self.KNOWN),
            'the interspersed-CLI ratchet grew; fix the new offender rather than raising it')
        paid=sorted(self.KNOWN-found)
        if paid:  # informational: shrink KNOWN when an offender is fixed
            self.assertTrue(paid)

    def test_no_offender_uses_a_feature_parse_intermixed_args_refuses(self):
        # The remedy must stay applicable: REMAINDER/PARSER/subparsers would make
        # parse_intermixed_args raise TypeError, so the ratchet would be a trap.
        import ast
        for rel in sorted(self.offenders()):
            tree=ast.parse((REPO/rel).read_text(),rel)
            for node in ast.walk(tree):
                if isinstance(node,ast.Call) and isinstance(node.func,ast.Attribute):
                    self.assertNotEqual(node.func.attr,'add_subparsers',rel)
                    for kw in node.keywords:
                        if kw.arg=='nargs' and isinstance(kw.value,ast.Attribute):
                            self.assertNotIn(kw.value.attr,('REMAINDER','PARSER'),rel)


if __name__=='__main__':unittest.main()
