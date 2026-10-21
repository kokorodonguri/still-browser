import importlib.util
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location('compare', Path(__file__).with_name('compare_benchmarks.py'))
compare = importlib.util.module_from_spec(spec)
spec.loader.exec_module(compare)


class ComparisonChecks(unittest.TestCase):
    def sample(self):
        return dict(Comparable=True, OSVersion='test', Architecture='ARM64', TotalPhysicalKiB=100,
                    Phase='loaded', RequestedIntervalMs=1000, Samples=120,
                    RequestedDurationSeconds=120, DurationSeconds=120.1,
                    PrivateBytes=dict(Median=1000, Peak=2000), WorkingSetBytes=dict(Median=2000, Peak=3000))

    def test_missing_processes_are_not_savings(self):
        baseline = self.sample()
        candidate = self.sample()
        candidate.update(Comparable=False, PrivateBytes=dict(Median=500, Peak=600))
        with self.assertRaisesRegex(ValueError, 'Incomplete'):
            compare.compare(baseline, candidate)

    def test_mismatched_architectures_are_not_compared(self):
        candidate = self.sample()
        candidate['Architecture'] = 'x64'
        with self.assertRaisesRegex(ValueError, 'Architecture'):
            compare.compare(self.sample(), candidate)

    def test_complete_runs_compare_median_and_peak(self):
        candidate = self.sample()
        candidate['PrivateBytes'] = dict(Median=900, Peak=2200)
        result = compare.compare(self.sample(), candidate)
        self.assertAlmostEqual(result['PrivateBytes']['Median']['changePercent'], -10)
        self.assertEqual(result['PrivateBytes']['Peak']['change'], 200)


class ReporterChecks(unittest.TestCase):
    def test_finished_callback_does_not_imply_complete_reports(self):
        from benchmark_metrics import assess_report
        report = assess_report(dict(expected=[1, 2], after=[1, 2], error=None,
            reports=[dict(pid=1, path="heap/committed/unused-pages/dirty", amount=10)]))
        self.assertFalse(report['complete'])
        self.assertIsNone(report['dirtyBytes'])
        self.assertEqual(report['missingPids'], [2])

    def test_new_process_during_collection_is_accounted_for(self):
        from benchmark_metrics import assess_report
        report = assess_report(dict(expected=[1], after=[1, 2], error=None,
            reports=[dict(pid=1, path="heap/committed/unused-pages/dirty", amount=10)]))
        self.assertFalse(report['complete'])

    def test_only_dirty_pages_count_as_unreturned(self):
        from benchmark_metrics import assess_report, stats
        report = assess_report(dict(expected=[1], after=[1], error=None, reports=[
            dict(pid=1, path="heap/committed/unused-pages/dirty", amount=10),
            dict(pid=1, path="heap/committed/allocated", amount=999)]))
        self.assertTrue(report['complete'])
        self.assertEqual(report['dirtyBytes'], 10)
        self.assertEqual(stats([1, 100]), dict(median=50.5, peak=100, p95=100))


class SamplingChecks(unittest.TestCase):
    def sample(self, interval=1, end=1.1):
        return dict(sampling=dict(intervalSeconds=interval, samples=2, schedule='start-to-start', durationSeconds=end),
                    allocator=[dict(startSeconds=0, endSeconds=.1), dict(startSeconds=end-.1, endSeconds=end)])

    def test_interval_and_elapsed_must_match(self):
        for other in (self.sample(60, 60.1), self.sample(1, 60.1)):
            with self.assertRaises(ValueError):
                compare.check_sampling(self.sample(), other)

    def test_legacy_and_invalid_times_rejected(self):
        for value in (None, float('nan'), float('inf'), -1):
            other = self.sample(); other['sampling']['durationSeconds'] = value
            with self.assertRaises(ValueError):
                compare.check_sampling(self.sample(), other)
        with self.assertRaises(ValueError):
            compare.check_sampling(dict(allocator=[]), dict(allocator=[]))

    def test_small_scheduling_jitter_allowed(self):
        compare.check_sampling(self.sample(), self.sample(end=1.12))

    def test_scroll_requires_actual_movement_for_whole_workload(self):
        from benchmark_metrics import validate_scroll
        valid = dict(startY=0, scrollY=2400, movedFrames=60, frameIntervalsMs=[16]*60)
        validate_scroll(valid)
        for change in (dict(scrollY=0), dict(movedFrames=0), dict(scrollY=200, movedFrames=5)):
            with self.assertRaises(ValueError):
                validate_scroll(dict(valid, **change))
