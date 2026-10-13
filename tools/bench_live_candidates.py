"""Isolated local GGUF comparisons for candidate-native prompts; no Maru settings writes."""
import argparse
import hashlib
import json
import math
import os
import shutil
import socket
import subprocess
import time
import urllib.request
from pathlib import Path

PROFILES = ('maru', 'milmmt', 'hy-mt2', 'sub', 'tower', 'translategemma', 'translategemma-context', 'seed', 'exaone', 'konan', 'kanana')
MARU_SYSTEM = '''You translate Korean into natural, faithful English for a language learner who sees your translation next to the original.
Rules:
- Translate meaning and tone. Keep the politeness level audible in English where it is natural (please, sir, casual contractions).
- Fragments (menu items, buttons, titles, headlines) stay fragments.
- Names: use the glossary if given, otherwise standard romanization.
- Never add explanations, notes, quotes, labels or alternatives. Output only the English translation.
These lines are subtitles shown one at a time, each continuing the lines before it. Keep names, pronouns and tone consistent with the earlier lines. Subtitles are spoken language: keep them short and natural, never explain.
Note: the text was read from the screen by OCR and may contain small recognition errors; where the intended text is obvious, translate the intended text.'''


def request_for(profile, source, turns, alias='maru'):
    """Current reference is deliberately absent; context contains completed earlier turns only."""
    if profile not in PROFILES:
        raise ValueError(f'Unknown profile: {profile}')
    cap = min(len(source) * 2 + 40, 256)
    common = dict(stream=True, cache_prompt=True, temperature=0.0, top_k=1)
    if profile == 'milmmt':
        prompt = ''.join(f'Translate this from Korean to English:\nKorean: {ko}\nEnglish: {en}\n\n' for ko, en in turns)
        prompt += f'Translate this from Korean to English:\nKorean: {source}\nEnglish:'
        return '/completion', dict(common, prompt=prompt, n_predict=cap)
    if profile == 'seed':
        if turns:
            raise ValueError('Seed publisher explicitly excludes multi-round conversation')
        return '/completion', dict(common, prompt=f'Translate the following Korean sentence into English:\n{source} <en>', n_predict=cap)
    if profile.startswith('translategemma'):
        if turns and profile == 'translategemma':
            raise ValueError('TranslateGemma native single-source template has no dialogue context blocks')
        # Keep structured content: the native template expands language codes and instructions.
        def message(ko):
            return dict(role='user', content=[dict(type='text', source_lang_code='ko', target_lang_code='en', text=ko)])
        messages = []
        for ko, en in turns:
            messages.extend([message(ko), dict(role='assistant', content=en)])
        messages.append(message(source))
    elif profile == 'sub':
        system = 'TASK: Translate Korean subtitles into English.\nSTYLE: neutral.\nTranslate only CURRENT_SOURCE. PREVIOUS_SOURCE and PREVIOUS_TRANSLATION are context only. Preserve meaning, tone, slang, profanity, uncertainty, repetitions and incomplete speech. Return only the final translation without labels or commentary.'
        prompt = f'[CURRENT_SOURCE]\n{source}'
        if turns:
            ko = '\n'.join(ko for ko, _ in turns)
            en = '\n'.join(en for _, en in turns)
            prompt = f'[PREVIOUS_SOURCE]\n{ko}\n\n[PREVIOUS_TRANSLATION]\n{en}\n\n{prompt}'
        messages = [dict(role='system', content=system), dict(role='user', content=prompt)]
    elif profile == 'hy-mt2':
        if turns:
            context = ''.join(f'Korean: {ko}\nEnglish: {en}\n' for ko, en in turns)
            prompt = f'[Background Information]\n{context}\nPlease translate the following text into English, taking the provided background information into consideration.\n[Source Text]\n{source}'
        else:
            prompt = f'Translate the following text into English. Note that you should only output the translated result without any additional explanation:\n{source}'
        messages = [dict(role='user', content=prompt)]
        common.update(temperature=0.7, top_p=0.6, top_k=20, repeat_penalty=1.05)
    else:
        systems = {'maru': MARU_SYSTEM, 'exaone': 'You are EXAONE model from LG AI Research, a helpful assistant.', 'konan': 'You are a helpful assistant.'}
        messages = [dict(role='system', content=systems[profile])] if profile in systems else []
        def prompt(ko):
            if profile == 'tower':
                return f'Translate the following Korean source text to English:\nKorean: {ko}\nEnglish: '
            if profile == 'maru':
                return f'Translate into English:\n{ko}'
            return f'Translate the following Korean subtitle into English. Return only the English translation without explanations:\n{ko}'
        for ko, en in turns:
            messages.extend([dict(role='user', content=prompt(ko)), dict(role='assistant', content=en)])
        messages.append(dict(role='user', content=prompt(source)))
        if profile == 'maru':
            common.update(temperature=0.1)
            del common['top_k']
    template_args = {'enable_thinking': False}
    if profile.startswith('translategemma'):
        # llama.cpp's native adapter receives language codes through template kwargs.
        template_args.update(source_lang_code='ko', target_lang_code='en')
    if profile == 'kanana':
        template_args['thinking_mode'] = 'no_think'
    return '/v1/chat/completions', dict(common, model=alias, messages=messages, max_tokens=cap,
        chat_template_kwargs=template_args, stream_options={'include_usage': True})


def consume_event(value, plain):
    if not isinstance(value, dict):
        raise ValueError('Stream event must be a JSON object')
    if 'error' in value:
        raise ValueError(f"Server error: {value['error']}")
    if plain:
        if value.get('truncated') or value.get('stop_type') == 'limit':
            raise ValueError('Completion hit its output/context limit')
        return value.get('content', ''), bool(value.get('stop')), value.get('timings')
    choices = value.get('choices', [])
    if not choices:
        return '', False, value.get('timings') or value.get('usage')
    choice = choices[0]
    if choice.get('finish_reason') not in (None, 'stop'):
        raise ValueError(f"Chat ended with {choice['finish_reason']}")
    return choice.get('delta', {}).get('content') or '', False, value.get('timings') or value.get('usage')


def event_text(value, plain):
    if not isinstance(value, dict):
        raise ValueError('Stream event must be a JSON object')
    if plain:
        text = value.get('content', '')
    else:
        choices = value.get('choices', [])
        if not isinstance(choices, list) or (choices and not isinstance(choices[0], dict)):
            raise ValueError('Chat choices must be objects in a list')
        delta = (choices or [{}])[0].get('delta', {})
        if not isinstance(delta, dict):
            raise ValueError('Chat delta must be an object')
        text = delta.get('content') or ''
    if not isinstance(text, str):
        raise ValueError('Stream content must be text')
    return text


def measure(root, profile, source, turns, timeout=60, request_builder=None):
    route, body = (request_builder or request_for)(profile, source, turns)
    started_unix_ms = int(time.time() * 1000)
    started = time.perf_counter()
    first, text, error, terminal, timings = None, '', None, False, None
    try:
        request = urllib.request.Request(root + route, data=json.dumps(body, ensure_ascii=False).encode('utf-8'), headers={'Content-Type': 'application/json'})
        with urllib.request.urlopen(request, timeout=timeout) as response:
            while line := response.readline():
                if time.perf_counter() - started > timeout:
                    raise TimeoutError('Request exceeded its overall deadline')
                if not line.startswith(b'data:'):
                    continue
                payload = line[5:].strip()
                if payload == b'[DONE]':
                    terminal = True
                    break
                value = json.loads(payload.decode('utf-8'))
                # Preserve partial content even when the same event announces a limit/error.
                partial = event_text(value, route == '/completion')
                if partial.strip() and first is None:
                    first = (time.perf_counter() - started) * 1000
                text += partial
                _, done, info = consume_event(value, route == '/completion')
                timings = info or timings
                if done:
                    terminal = True
                    break
        if not terminal:
            raise ValueError('Stream ended without its final event')
        if not text.strip():
            raise ValueError('Model returned no translation')
    except (OSError, ValueError, TimeoutError) as exception:
        error = str(exception)
    translation = text.strip() if error is None else ''
    if profile in ('maru', 'hy-mt2') and len(translation) > 1 and translation[0] in '\"“「' and translation[-1] in '\"”」':
        translation = translation[1:-1].strip()
    return dict(started_unix_ms=started_unix_ms, first_text_ms=first, done_ms=(time.perf_counter()-started)*1000,
        translation=translation, streamed_text=text, error=error, timings=timings, route=route, request=body)


def occupied(port):
    with socket.socket() as probe:
        probe.settimeout(0.2)
        return probe.connect_ex(('127.0.0.1', port)) == 0


def advance(turns, case, measurement, mode, limit):
    answer = measurement['translation'] if mode == 'rolling' else case['reference']
    if answer and (mode != 'rolling' or not measurement['error']):
        turns.append((case['source'], answer))
    del turns[:max(0, len(turns)-limit)]


def hash_file(file):
    with Path(file).open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def stop_owned(process):
    if process.poll() is None:
        process.terminate()
    try:
        process.wait(timeout=10)
    except subprocess.TimeoutExpired:
        process.kill()
        process.wait(timeout=10)


def server_environment(cache):
    # Keep driver-generated files in test storage, including under a restricted filesystem.
    environment = os.environ.copy()
    environment['CUDA_CACHE_PATH'] = str(cache.resolve())
    return environment


def summarize(records):
    def quantile(values, fraction):
        values = sorted(values)
        return values[max(0, math.ceil(len(values)*fraction)-1)] if values else None
    result = []
    for name in ('baseline_before', 'candidate', 'baseline_after'):
        rows = [r['measurement'] for r in records if r['pass'] == name]
        good = [r for r in rows if not r['error']]
        result.append(dict(pass_name=name, n=len(rows), errors=len(rows)-len(good),
            first_p50=quantile([r['first_text_ms'] for r in good if r['first_text_ms'] is not None], .5),
            first_p95=quantile([r['first_text_ms'] for r in good if r['first_text_ms'] is not None], .95),
            done_p50=quantile([r['done_ms'] for r in good], .5), done_p95=quantile([r['done_ms'] for r in good], .95)))
    return result


def run(args):
    from prepare_live_benchmark import validate_cases
    cases = [json.loads(line) for line in args.file.read_text(encoding='utf-8-sig').splitlines() if line.strip()]
    validate_cases(cases)
    if args.runs < 1 or args.threads < 1 or args.context_lines < 0:
        raise ValueError('Runs and threads must be positive; context limit must be nonnegative')
    if args.profile in ('seed', 'translategemma') and args.context_lines:
        raise ValueError('This native profile accepts zero context only')
    for model in (args.baseline, args.candidate):
        if not model.is_file():
            raise ValueError(f'Model not found: {model}')
    if args.baseline.resolve() == args.candidate.resolve():
        raise ValueError('Baseline and candidate must be distinct')
    if any(occupied(port) for port in (args.port, 18080, 18090, 18095)):
        raise ValueError('Quit Maru and other model services before running the isolated comparison')
    args.out.mkdir(parents=True, exist_ok=False)
    cache = args.cuda_cache_path or args.out/'cuda-cache'
    cache.mkdir(parents=True, exist_ok=True)
    environment = server_environment(cache)
    inputs = {label: dict(file=str(file.resolve()), sha256=hash_file(file), bytes=file.stat().st_size) for label, file in
        [('server', args.server), ('baseline', args.baseline), ('candidate', args.candidate), ('corpus', args.file), ('tool', Path(__file__))]}
    records, passes, failure, gpu = [], [], None, None
    hidden = {'creationflags': subprocess.CREATE_NO_WINDOW} if hasattr(subprocess, 'CREATE_NO_WINDOW') else {}
    with (args.out/'records.jsonl').open('x', encoding='utf-8') as raw, (args.out/'gpu.csv').open('x') as telemetry:
        try:
            smi = shutil.which('nvidia-smi')
            if smi:
                gpu = subprocess.Popen([smi, '--query-gpu=timestamp,name,memory.used,memory.total,utilization.gpu,utilization.memory', '--format=csv', '-l', '1'], stdout=telemetry, stderr=subprocess.DEVNULL, **hidden)
            for name, model, profile in [('baseline_before', args.baseline, 'maru'), ('candidate', args.candidate, args.profile), ('baseline_after', args.baseline, 'maru')]:
                layers = args.gpu_layers if name == 'candidate' else 'all'
                context_size = 4096 if profile.startswith('translategemma') else 8192
                # Two slots match the existing Rust comparisons; sequential requests use one.
                command = [str(args.server), '-m', str(model.resolve()), '--host', '127.0.0.1', '--port', str(args.port), '-c', str(context_size), '-np', '2', '-ngl', layers, '-fa', 'on', '--cache-reuse', '256', '--reasoning', 'off', '-t', str(args.threads)]
                if name == 'candidate' and layers == '0':
                    command.extend(['--device', 'none', '--no-kv-offload', '--no-op-offload'])
                if profile.startswith('translategemma'):
                    command.extend(['--jinja', '--chat-template-kwargs', json.dumps({'source_lang_code':'ko', 'target_lang_code':'en'})])
                print(f'{args.out.name}: {name} loading {model.name}', flush=True)
                with (args.out/f'{name}-server.log').open('xb') as log:
                    process = subprocess.Popen(command, stdout=log, stderr=subprocess.STDOUT, env=environment, **hidden)
                    try:
                        start = time.monotonic()
                        root = f'http://127.0.0.1:{args.port}'
                        while True:
                            if process.poll() is not None:
                                raise ValueError(f'{name} server exited with {process.returncode}; inspect its log')
                            if time.monotonic()-start > 180:
                                raise TimeoutError(f'{name} model did not become ready within 180 seconds')
                            try:
                                with urllib.request.urlopen(root+'/health', timeout=2) as response:
                                    if response.status == 200:
                                        break
                            except OSError:
                                time.sleep(.25)
                        load_ms = (time.monotonic()-start)*1000
                        if name == 'candidate' and profile in ('sub', 'tower', 'translategemma', 'translategemma-context', 'exaone', 'konan', 'kanana'):
                            _, template_request = request_for(profile, '준비가 되면 시작하겠습니다.', [])
                            request = urllib.request.Request(root+'/apply-template', data=json.dumps(template_request, ensure_ascii=False).encode('utf-8'), headers={'Content-Type':'application/json'})
                            with urllib.request.urlopen(request, timeout=10) as response:
                                rendered = json.load(response)
                            (args.out/'native-template-check.json').write_text(json.dumps(rendered, ensure_ascii=False, indent=2), encoding='utf-8')
                            prompt = rendered.get('prompt', '')
                            if '준비가 되면 시작하겠습니다.' not in prompt:
                                raise ValueError('Native chat template lost the source; inspect native-template-check.json')
                            if profile.startswith('translategemma') and ('Korean (ko) to English (en) translator' not in prompt or '\n\n\n준비가 되면 시작하겠습니다.' not in prompt):
                                raise ValueError('TranslateGemma native template did not retain languages/source; inspect native-template-check.json')
                        warmup = measure(root, profile, '준비가 되면 시작하겠습니다.', [])
                        passes.append(dict(pass_name=name, model=str(model), profile=profile, args=command, load_ms=load_ms, warmup=warmup))
                        if warmup['error']:
                            raise ValueError(f"{name} warm-up failed: {warmup['error']}")
                        for repetition in range(1, args.runs+1):
                            scene, turns = None, []
                            for index, case in enumerate(cases, 1):
                                if scene != case['scene']:
                                    scene, turns = case['scene'], []
                                result = measure(root, profile, case['source'], turns)
                                row = dict(pass_=name, run=repetition, case=case, context=list(turns), profile=profile, measurement=result)
                                row['pass'] = row.pop('pass_')
                                raw.write(json.dumps(row, ensure_ascii=False)+'\n')
                                raw.flush()
                                records.append(row)
                                advance(turns, case, result, args.context_mode, args.context_lines)
                                if index % 24 == 0 or index == len(cases):
                                    print(f'{args.out.name}: {name} run {repetition}/{args.runs}, line {index}/{len(cases)}', flush=True)
                    finally:
                        stop_owned(process)
                deadline = time.monotonic()+10
                while occupied(args.port) and time.monotonic() < deadline:
                    time.sleep(.1)
                if occupied(args.port):
                    raise ValueError('Owned model server did not release its port')
                time.sleep(2)
        except (OSError, ValueError, TimeoutError) as exception:
            failure = str(exception)
        finally:
            if gpu and gpu.poll() is None:
                stop_owned(gpu)
    summary = summarize(records)
    report = dict(status='incomplete' if failure else 'completed_with_errors' if any(r['errors'] for r in summary) else 'completed',
        failure=failure, inputs=inputs, passes=passes, summaries=summary, runs=args.runs, context_mode=args.context_mode,
        context_lines=args.context_lines, game_state=args.game_state,
        cuda_environment={key:environment.get(key) for key in ('CUDA_CACHE_PATH','CUDA_CACHE_DISABLE','CUDA_CACHE_MAXSIZE','CUDA_FORCE_PTX_JIT')},
        limitation='Translation-only, sequential, warmed local GGUF comparison; no OCR, overlay or gameplay frame-time measurement. References are agent-created review aids, not gold. Native templates/greedy decoding are preserved except HY-MT2 uses Tencent sampling; generic models use an English-only subtitle instruction. MiLMMT/Tower/general chat history is an explicit experimental extension. Seed uses documented greedy, not recommended four-beam decoding, and a community quantization. TranslateGemma uses native structured content/Jinja and 2048 tokens per slot; its native profile has zero history, while translategemma-context explicitly experiments with completed structured user/assistant pairs. Sub uses published neutral subtitle blocks. HTTP stdlib transport differs from the Rust CLI; paired Gemma controls use the same transport. Exact requests and timings are retained; capped/incomplete/empty streams count as errors. CPU candidates use the requested thread budget and zero GPU layers; GPU baseline remains fully offloaded. Unspecified samplers are runtime defaults. No installed setting is changed.')
    (args.out/'report.json').write_text(json.dumps(report, ensure_ascii=False, indent=2)+'\n', encoding='utf-8')
    print(json.dumps({'out':str(args.out), 'status':report['status'], 'summaries':summary, 'failure':failure}), flush=True)
    return 1 if report['status'] != 'completed' else 0


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('baseline', 'candidate', 'server', 'file', 'out'):
        parser.add_argument('--'+name, type=Path, required=True)
    parser.add_argument('--profile', choices=PROFILES, required=True)
    parser.add_argument('-n', '--runs', type=int, default=3)
    parser.add_argument('--port', type=int, default=19180)
    parser.add_argument('--context-lines', type=int, default=0)
    parser.add_argument('--context-mode', choices=('fixed', 'rolling'), default='fixed')
    parser.add_argument('--gpu-layers', default='all')
    parser.add_argument('--threads', type=int, default=12)
    parser.add_argument('--game-state', default='unspecified')
    parser.add_argument('--cuda-cache-path', type=Path, help='Optional shared test cache; default is a private cache inside the new output folder')
    return run(parser.parse_args())


if __name__ == '__main__':
    raise SystemExit(main())
