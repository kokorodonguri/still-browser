#!/usr/bin/env python3
"""Compare matching complete runs; never turn missing measurements into savings."""
import argparse
import json
from pathlib import Path


def delta(before, after):
    if before is None or after is None:
        return None
    return {'baseline': before, 'candidate': after, 'change': after - before,
            'changePercent': (after / before - 1) * 100 if before else None}


def compare(before, after):
    if 'summaryMs' in before:
        if not before.get('comparable') or not after.get('comparable'):
            raise ValueError('Incomplete browser run; comparison refused')
        for key in ('platform', 'machine', 'headless', 'autoUnload', 'fixture', 'urls'):
            if before.get(key) != after.get(key):
                raise ValueError(f'Mismatched browser workload: {key}')
        if len(before['operations']) != len(after['operations']) or len(before['allocator']) != len(after['allocator']):
            raise ValueError('Mismatched sample counts')
        result = {'operationMs': {}}
        for operation, stats in before['summaryMs'].items():
            result['operationMs'][operation] = {k: delta(stats[k], after['summaryMs'][operation][k]) for k in ('median', 'p95', 'peak')}
        result['dirtyBytes'] = {k: delta(before['dirtyBytes'][k], after['dirtyBytes'][k]) for k in ('median', 'peak')}
        result['scrollFrameMs'] = {k: delta(before['scrollFrameMs'][k], after['scrollFrameMs'][k]) for k in ('median', 'p95', 'peak')}
        return result
    if not before.get('Comparable') or not after.get('Comparable'):
        raise ValueError('Incomplete memory run; comparison refused')
    for key in ('OSVersion', 'Architecture', 'TotalPhysicalKiB', 'Phase', 'RequestedIntervalMs', 'Samples'):
        if before.get(key) != after.get(key):
            raise ValueError(f'Mismatched memory workload: {key}')
    return {metric: {k: delta(before[metric][k], after[metric][k]) for k in ('Median', 'Peak')}
            for metric in ('PrivateBytes', 'WorkingSetBytes')}


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('baseline', type=Path)
    parser.add_argument('candidate', type=Path)
    args = parser.parse_args()
    try:
        print(json.dumps(compare(json.loads(args.baseline.read_text(encoding='utf-8-sig')),
                                 json.loads(args.candidate.read_text(encoding='utf-8-sig'))), indent=2))
    except ValueError as error:
        parser.error(str(error))
