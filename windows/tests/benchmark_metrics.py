"""Pure aggregation: missing child reports never become zero-byte savings."""
import math
import statistics


def stats(values):
    if not values:
        return None
    ordered = sorted(values)
    return {'median': statistics.median(values), 'peak': max(values),
            'p95': ordered[min(len(ordered) - 1, math.ceil(len(ordered) * .95) - 1)]}



def assess_report(report):
    dirty = [r for r in report['reports'] if r['path'] == 'heap/committed/unused-pages/dirty']
    received = {r['pid'] for r in dirty}
    expected = set(report['expected']) | set(report['after'])
    report['missingPids'] = sorted(expected - received)
    report['complete'] = bool(expected) and not report['error'] and not report['missingPids'] and None not in received
    report['dirtyBytes'] = sum(r['amount'] for r in dirty) if report['complete'] else None
    return report
