"""Diagnostic transport never changes validation outcomes or repository refs."""
import io
import json
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest import mock

import yaml
from scripts import export_ci_diagnostics as export

ROOT = Path(__file__).resolve().parents[2]


def run(identity, workflow=1, conclusion='failure', status='completed', pr=1094):
    return dict(id=identity, workflow_id=workflow, conclusion=conclusion,
                status=status, pull_requests=[dict(number=pr)])


class DiagnosticSelection(unittest.TestCase):
    def test_cancelled_run_keeps_failed_job_evidence(self):
        self.assertEqual(export.automatic_run_ids([run(8, conclusion='cancelled')], 1094), [8])

    def test_only_latest_run_per_workflow_and_at_most_two_workflows(self):
        self.assertEqual(export.automatic_run_ids(
            [run(9), run(8), run(7, 2), run(6, 3)], 1094), [9, 7])

    def test_api_order_does_not_choose_old_cancelled_retry(self):
        self.assertEqual(export.automatic_run_ids([
            run(208, conclusion='cancelled'), run(285, 2, conclusion='cancelled'),
            run(842), run(838, 2, conclusion='success')], 1094), [842, 285])

    def test_creation_time_precedes_id_tie_break(self):
        newer = dict(run(10), created_at='2026-11-04T11:00:00Z')
        older = dict(run(20), created_at='2026-11-04T10:00:00Z')
        self.assertEqual(export.automatic_run_ids([older, newer], 1094), [10])

    def test_never_selects_green_in_progress_or_other_pr(self):
        self.assertEqual(export.automatic_run_ids([
            run(9, conclusion='success'), run(8, status='in_progress'),
            run(7, pr=1095), run(6, conclusion='timed_out')], 1094), [6])

    def test_requires_exact_evidence_name_and_live_artifact(self):
        sha = 'a' * 40
        for prefix in ['kernel-61863-', 'intel-icx-provenance-']:
            self.assertTrue(export.retained_artifact(dict(name=prefix + sha, expired=False)))
            self.assertFalse(export.retained_artifact(dict(name=prefix + sha, expired=True)))
        for name in ['compiler-' + sha, 'kernel-61863-short',
                     'intel-icx-provenance-' + sha + '.zip', 'kernel-61863-../secret']:
            self.assertFalse(export.retained_artifact(dict(name=name, expired=False)))

    def test_fork_event_is_rejected_before_any_api_request(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'event.json'
            path.write_text(json.dumps(dict(number=1094, pull_request=dict(
                head=dict(repo=dict(full_name='other/repo'))))))
            with mock.patch.dict(export.os.environ, dict(
                    GITHUB_REPOSITORY='ms178/lccc', GITHUB_EVENT_PATH=str(path), SOURCE_RUN_IDS='')):
                with mock.patch.object(export, 'gh') as api:
                    with self.assertRaisesRegex(ValueError, 'same-repository'):
                        export.main()
                    api.assert_not_called()

    def test_explicit_incomplete_or_other_repository_runs_are_rejected(self):
        for status, repository in [('in_progress', 'ms178/lccc'), ('completed', 'other/repo')]:
            with self.subTest(status=status, repository=repository):
                with mock.patch.dict(export.os.environ, dict(
                        GITHUB_REPOSITORY='ms178/lccc', SOURCE_RUN_IDS='9')):
                    with mock.patch.object(export, 'gh', return_value=json.dumps(dict(
                            status=status, repository=dict(full_name=repository))).encode()):
                        with self.assertRaisesRegex(ValueError, 'completed runs of this repository'):
                            export.main()

    def export_run(self, jobs, logs):
        """Run the exporter over one fake run; return (exported names, notices)."""
        def api(path, payload=None, *, allow_binary=False):
            if path.endswith('/logs'):
                answer = logs[int(path.split('/')[-2])]
                if isinstance(answer, Exception):
                    raise answer
                return answer
            if '/jobs?' in path:
                return json.dumps(dict(jobs=jobs)).encode()
            if '/artifacts?' in path:
                return json.dumps(dict(artifacts=[])).encode()
            return json.dumps(dict(repository=dict(full_name='ms178/lccc'), status='completed',
                                   head_sha='abc', conclusion='cancelled')).encode()
        output = io.StringIO()
        with mock.patch.dict(export.os.environ, dict(
                GITHUB_REPOSITORY='ms178/lccc', SOURCE_RUN_IDS='42')), \
                mock.patch.object(export, 'gh', api), \
                mock.patch.object(export, 'export_file',
                                  side_effect=lambda repo, name, data: [f'file {name}']) as sent, \
                mock.patch('sys.stdout', output):
            export.main()
        notices = output.getvalue().replace('%0A', '\n')
        return [call.args[1] for call in sent.call_args_list], notices

    def test_timed_out_job_is_exported_although_github_reports_it_cancelled(self):
        """PR #1196: the lane that hit `timeout-minutes` was `cancelled`, and
        only the aggregate job that depends on it was `failure`."""
        jobs = [dict(id=7, conclusion='cancelled'), dict(id=8, conclusion='failure'),
                dict(id=9, conclusion='success'), dict(id=10, conclusion='skipped'),
                dict(id=11, conclusion='timed_out'), dict(id=12, conclusion=None)]
        names, notices = self.export_run(jobs, {7: b'timeout', 8: b'aggregate', 11: b'late'})
        self.assertEqual(names, ['run42-job7.log.gz', 'run42-job8.log.gz', 'run42-job11.log.gz'])
        self.assertNotIn('nolog', notices)

    def test_cancelled_job_without_a_log_is_named_and_does_not_stop_the_export(self):
        """A job a newer push cancelled while it was queued never ran a step."""
        for status in export.NO_LOG_STATUSES:
            with self.subTest(status=status):
                jobs = [dict(id=7, conclusion='cancelled'), dict(id=8, conclusion='failure')]
                names, notices = self.export_run(
                    jobs, {7: export.ApiError('gh: Not Found', status), 8: b'real failure'})
                self.assertEqual(names, ['run42-job8.log.gz'])
                self.assertIn('nolog run42-job7.log.gz cancelled', notices.splitlines())

    def test_missing_log_is_still_an_error_for_a_failed_job_or_any_other_status(self):
        for conclusion, status in [('failure', 404), ('failure', 410), ('cancelled', 403),
                                   ('cancelled', 500), ('timed_out', None)]:
            with self.subTest(conclusion=conclusion, status=status):
                with self.assertRaises(export.ApiError):
                    self.export_run([dict(id=7, conclusion=conclusion)],
                                    {7: export.ApiError('gh: refused', status)})

    def test_api_error_carries_the_http_status_gh_reported(self):
        for detail, status in [(b'gh: Not Found (HTTP 404)', 404), (b'HTTP 403: denied', 403),
                               (b'connection reset', None)]:
            error = subprocess.CalledProcessError(1, ['gh'], stderr=detail)
            with self.subTest(detail=detail), \
                    mock.patch.object(export.subprocess, 'run', side_effect=error):
                with self.assertRaises(export.ApiError) as caught:
                    export.gh('repos/ms178/lccc/actions/jobs/7/logs')
                self.assertEqual(caught.exception.status, status)

    def test_label_workflow_is_separate_and_does_not_replace_kernel_check(self):
        workflow = yaml.safe_load((ROOT / '.github/workflows/ci-diagnostics.yml').read_text())
        events = workflow.get('on', workflow.get(True))
        self.assertEqual(events['pull_request']['types'], ['labeled'])
        self.assertEqual(set(workflow['jobs']), {'diagnostic-transport'})
        job = workflow['jobs']['diagnostic-transport']
        self.assertIn('ci-diagnostic-transport', job['if'])
        self.assertIn('head.repo.full_name == github.repository', job['if'])
        self.assertEqual(job['permissions'], {'actions': 'read', 'contents': 'write'})
        self.assertFalse(job['steps'][0]['with']['persist-credentials'])
