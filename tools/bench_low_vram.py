"""Measure explicit local GGUF configurations, including per-process GPU allocations."""
import argparse
import json
import math
import os
import subprocess
import time
import urllib.request
from pathlib import Path

from bench_live_candidates import measure, occupied, request_for, stop_owned, hash_file


def quantile(values, fraction):
    if not values:
        return None
    values = sorted(values)
    index = (len(values) - 1) * fraction
    low, high = math.floor(index), math.ceil(index)
    return values[low] + (values[high] - values[low]) * (index - low)


def candidate_request(profile, source, turns):
    if profile != 'lfm':
        return request_for(profile, source, turns)
    if turns:
        raise ValueError('The native LFM comparison uses single-source translation')
    cap = min(len(source) * 2 + 40, 256)
    return '/v1/chat/completions', dict(model='maru', stream=True, temperature=.3, max_tokens=cap,
        messages=[dict(role='system', content='Translate the following text to English.'), dict(role='user', content=source)])


def resource_summary(samples, pids):
    rows = [p for s in samples for p in s['processes'] if p['target']['pid'] in pids]
    result = {}
    for pid in pids:
        own = [r for r in rows if r['target']['pid'] == pid]
        stats = {}
        for key in ('dedicated_gpu_bytes', 'shared_gpu_bytes', 'ram_bytes'):
            values = [r[key] for r in own if r[key] is not None]
            stats[key] = dict(samples=len(values), p50=quantile(values, .5), max=max(values) if values else None)
        result[str(pid)] = stats
    return result


def wait_ready(process, root):
    start = time.monotonic()
    while time.monotonic() - start < 180:
        if process.poll() is not None:
            raise ValueError(f'Server exited with {process.returncode}; inspect server.log')
        try:
            with urllib.request.urlopen(root + '/health', timeout=2) as response:
                if response.status == 200:
                    return (time.monotonic() - start) * 1000
        except OSError:
            time.sleep(.25)
    raise TimeoutError('Server did not load within 180 seconds')


def run(args):
    plan = json.loads(args.plan.read_text(encoding='utf-8'))
    cases = [json.loads(line) for line in args.file.read_text(encoding='utf-8-sig').splitlines() if line.strip()]
    if args.runs < 1 or not cases:
        raise ValueError('A nonempty corpus and positive run count are required')
    if any(occupied(p) for p in (args.port, 18080, 18090, 18095, 18096)):
        raise ValueError('Close Maru/model services before this isolated memory comparison')
    args.out.mkdir(parents=True, exist_ok=False)
    hidden = dict(creationflags=subprocess.CREATE_NO_WINDOW) if os.name == 'nt' else {}
    results = []
    for item in plan:
        folder = args.out / item['id']
        folder.mkdir()
        model = Path(item['model']).resolve()
        if not model.is_file():
            raise ValueError(f'Model is missing: {model}')
        command = [str(args.server.resolve()), '-m', str(model), '--host', '127.0.0.1', '--port', str(args.port), '--alias', 'maru'] + item['args']
        environment = os.environ.copy()
        cache = (args.out / 'cuda-cache').resolve()
        cache.mkdir(exist_ok=True)
        environment['CUDA_CACHE_PATH'] = str(cache)
        environment['CUDA_CACHE_DISABLE'] = '0'
        result = dict(id=item['id'], model=str(model), bytes=model.stat().st_size, model_sha256=hash_file(model),
            profile=item['profile'], args=command, runs=args.runs, context_lines=0, failure=None)
        print(f"Loading {item['id']}", flush=True)
        records = []
        process = telemetry = None
        with (folder / 'server.log').open('xb') as log, (folder / 'resources.jsonl').open('xb') as resource_log, (folder / 'records.jsonl').open('x', encoding='utf-8') as raw:
            try:
                process = subprocess.Popen(command, stdout=log, stderr=subprocess.STDOUT, env=environment, **hidden)
                result['pid'] = process.pid
                telemetry = subprocess.Popen([str(args.sampler.resolve()), str(process.pid)], stdout=resource_log, stderr=subprocess.DEVNULL, **hidden)
                root = f'http://127.0.0.1:{args.port}'
                result['load_ms'] = wait_ready(process, root)
                result['warmup'] = measure(root, item['profile'], '준비가 되면 시작하겠습니다.', [], request_builder=candidate_request)
                if result['warmup']['error']:
                    raise ValueError(result['warmup']['error'])
                time.sleep(2)
                for repetition in range(1, args.runs + 1):
                    for case in cases:
                        measurement = measure(root, item['profile'], case['source'], [], request_builder=candidate_request)
                        row = dict(run=repetition, case=case, measurement=measurement)
                        raw.write(json.dumps(row, ensure_ascii=False) + '\n')
                        raw.flush()
                        records.append(row)
                    print(f"{item['id']}: run {repetition}/{args.runs}", flush=True)
                time.sleep(2)
            except (OSError, ValueError, TimeoutError) as error:
                result['failure'] = str(error)
            finally:
                if telemetry is not None:
                    stop_owned(telemetry)
                if process is not None:
                    stop_owned(process)
        samples = [json.loads(line) for line in (folder / 'resources.jsonl').read_text(encoding='utf-8').splitlines() if line.strip()]
        result['resources'] = resource_summary(samples, [result['pid']]) if 'pid' in result else {}
        good = [r['measurement'] for r in records if r['measurement']['error'] is None]
        result['attempts'] = len(records)
        result['errors'] = len(records) - len(good)
        for metric in ('first_text_ms', 'done_ms'):
            values = [r[metric] for r in good if r[metric] is not None]
            result[metric] = dict(p50=quantile(values, .5), p95=quantile(values, .95), max=max(values) if values else None)
        (folder / 'report.json').write_text(json.dumps(result, indent=2) + '\n', encoding='utf-8')
        results.append(result)
        (args.out / 'summary.json').write_text(json.dumps(results, indent=2) + '\n', encoding='utf-8')
        print(json.dumps({k:result[k] for k in ('id', 'attempts', 'errors', 'failure', 'done_ms', 'resources')}), flush=True)
        time.sleep(1)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--plan', type=Path, required=True)
    parser.add_argument('--file', type=Path, required=True)
    parser.add_argument('--server', type=Path, required=True)
    parser.add_argument('--sampler', type=Path, required=True)
    parser.add_argument('--out', type=Path, required=True)
    parser.add_argument('--port', type=int, default=19188)
    parser.add_argument('-n', dest='runs', type=int, default=3)
    run(parser.parse_args())
