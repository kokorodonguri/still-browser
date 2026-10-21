#!/usr/bin/env python3
"""Compare matching complete runs; never turn missing measurements into savings."""
import argparse
import json
import math
from pathlib import Path


def delta(before, after):
    if before is None or after is None:
        return None
    return {'baseline': before, 'candidate': after, 'change': after - before,
            'changePercent': (after / before - 1) * 100 if before else None}


def number(value):
    if isinstance(value, bool) or not isinstance(value, (int, float)) or not math.isfinite(value) or value < 0:
        raise ValueError('Missing or invalid sampling time; comparison refused')
    return value


def close_time(before, after):
    before, after = number(before), number(after)
    # Small scheduling jitter is allowed, never a different recovery window.
    if abs(before - after) > max(0.25, min(before, after) * 0.05):
        raise ValueError('Mismatched elapsed sampling time')


def check_sampling(before, after):
    for run in (before, after):
        config = run.get('sampling', {})
        number(config.get('intervalSeconds'))
        duration = number(config.get('durationSeconds'))
        if config.get('schedule') != 'start-to-start' or config.get('samples') != len(run['allocator']):
            raise ValueError('Missing or invalid sampling conditions')
        previous = 0
        for sample in run['allocator']:
            start, end = number(sample.get('startSeconds')), number(sample.get('endSeconds'))
            if start < previous or end < start or end > duration:
                raise ValueError('Invalid sample chronology')
            previous = end
    for key in ('intervalSeconds', 'samples', 'schedule'):
        if before['sampling'][key] != after['sampling'][key]:
            raise ValueError(f'Mismatched sampling conditions: {key}')
    close_time(before['sampling']['durationSeconds'], after['sampling']['durationSeconds'])
    for a, b in zip(before['allocator'], after['allocator']):
        close_time(a['startSeconds'], b['startSeconds'])
        close_time(a['endSeconds'], b['endSeconds'])


def compare(before, after):
    if 'summaryMs' in before:
        if not before.get('comparable') or not after.get('comparable'):
            raise ValueError('Incomplete browser run; comparison refused')
        for key in ('platform', 'machine', 'headless', 'autoUnload', 'fixture', 'urls'):
            if before.get(key) != after.get(key):
                raise ValueError(f'Mismatched browser workload: {key}')
        if len(before['operations']) != len(after['operations']) or len(before['allocator']) != len(after['allocator']):
            raise ValueError('Mismatched sample counts')
        check_sampling(before, after)
        result = {'operationMs': {}}
        for operation, stats in before['summaryMs'].items():
            result['operationMs'][operation] = {k: delta(stats[k], after['summaryMs'][operation][k]) for k in ('median', 'p95', 'peak')}
        result['dirtyBytes'] = {k: delta(before['dirtyBytes'][k], after['dirtyBytes'][k]) for k in ('median', 'peak')}
        result['scrollFrameMs'] = {k: delta(before['scrollFrameMs'][k], after['scrollFrameMs'][k]) for k in ('median', 'p95', 'peak')}
        return result
    if not before.get('Comparable') or not after.get('Comparable'):
        raise ValueError('Incomplete memory run; comparison refused')
    number(before.get('RequestedIntervalMs'))
    number(after.get('RequestedIntervalMs'))
    number(before.get('RequestedDurationSeconds'))
    number(after.get('RequestedDurationSeconds'))
    close_time(before.get('DurationSeconds'), after.get('DurationSeconds'))
    for key in ('OSVersion', 'Architecture', 'TotalPhysicalKiB', 'Phase', 'RequestedIntervalMs', 'RequestedDurationSeconds', 'Samples'):
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
