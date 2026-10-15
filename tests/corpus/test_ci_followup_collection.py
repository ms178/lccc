"""Offline collection contracts; fixture observations are not hosted evidence."""
import importlib.util
from pathlib import Path
import unittest

ROOT=Path(__file__).resolve().parents[2]
spec=importlib.util.spec_from_file_location('ci_followup',ROOT/'scripts/collect_ci_followup.py')
m=importlib.util.module_from_spec(spec);spec.loader.exec_module(m)


class Collection(unittest.TestCase):
    def test_no_postmerge_observations_are_not_fabricated(self):
        routes=[]
        def get(route):
            routes.append(route)
            return {'merged_at':'2026-10-15T13:00:00Z'} if route.endswith('/834') else []
        result=m.collect('example/repo',834,get)
        self.assertFalse(result['complete']);self.assertEqual(result['eligible_merged_prs'],0)
        self.assertEqual(result['runs'],[]);self.assertFalse(any('/actions/' in route for route in routes))

    def test_attempt_recovery_is_a_signal_not_a_diagnosed_flake(self):
        def get(route):
            if route.endswith('/834'):return {'merged_at':'2026-10-15T13:00:00Z'}
            if '/pulls?' in route:
                return [{'number':835+i,'merged_at':f'2026-10-16T00:00:{i:02}Z','updated_at':'2026-10-16T01:00:00Z','base':{'ref':'main'},'head':{'sha':str(i)}} for i in range(20)]
            if '/actions/runs?' in route:
                sha=route.split('head_sha=')[1].split('&')[0]
                return {'workflow_runs':[{'id':int(sha),'path':'.github/workflows/ci.yml','head_sha':sha,'html_url':'https://example.invalid/run','run_attempt':2}]}
            if '/jobs?' in route:
                return {'jobs':[{'id':1,'name':'benchmark','status':'completed','conclusion':'success','started_at':'2026-10-16T00:00:00Z','completed_at':'2026-10-16T00:00:04Z','html_url':'https://example.invalid/job'}]}
            return {'status':'completed','conclusion':'failure' if route.endswith('/1') else 'success','run_started_at':'2026-10-16T00:00:00Z','updated_at':'2026-10-16T00:00:10Z'}
        result=m.collect('example/repo',834,get)
        self.assertTrue(result['complete']);self.assertEqual(len(result['prs']),20)
        self.assertTrue(all(r['rerun_recovery_signal'] for r in result['runs']))
        self.assertIn('root-cause',result['status'])
        self.assertEqual(result['runs'][0]['attempts'][0]['jobs'][0]['duration_s'],4)
