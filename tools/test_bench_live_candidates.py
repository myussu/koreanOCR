"""Checks that candidate comparisons cannot quietly produce misleading successes."""
import io
import json
import os
import unittest
from pathlib import Path
from unittest.mock import patch

import bench_live_candidates as bench


class CandidateBenchTests(unittest.TestCase):
    def test_the_owned_server_gets_a_test_cache_without_changing_the_parent_environment(self):
        with patch.dict(os.environ, {'CUDA_CACHE_PATH':'parent-cache'}):
            environment = bench.server_environment(Path('target/test-cuda-cache'))
            self.assertEqual(environment['CUDA_CACHE_PATH'], str(Path('target/test-cuda-cache').resolve()))
            self.assertEqual(os.environ['CUDA_CACHE_PATH'], 'parent-cache')
    def test_seed_keeps_the_target_tag_and_refuses_a_chat_history(self):
        route, body = bench.request_for('seed', '가지 마세요.', [])
        self.assertEqual(route, '/completion')
        self.assertEqual(body['prompt'], 'Translate the following Korean sentence into English:\n가지 마세요. <en>')
        self.assertNotIn('messages', body)
        with self.assertRaisesRegex(ValueError, 'multi-round'):
            bench.request_for('seed', '가지 마세요.', [('네.', 'Yes.')])

    def test_translategemma_retains_language_codes_in_native_structured_content(self):
        _, body = bench.request_for('translategemma', '안녕.', [])
        self.assertEqual(body['messages'], [{'role':'user', 'content':[{'type':'text', 'source_lang_code':'ko', 'target_lang_code':'en', 'text':'안녕.'}]}])
        self.assertEqual(body['chat_template_kwargs']['source_lang_code'], 'ko')
        self.assertEqual(body['chat_template_kwargs']['target_lang_code'], 'en')
        with self.assertRaisesRegex(ValueError, 'single-source'):
            bench.request_for('translategemma', '안녕.', [('네.', 'Yes.')])

    def test_tower_context_is_completed_pairs_and_the_current_source_has_no_answer(self):
        _, body = bench.request_for('tower', '위로?', [('이걸 타면 올라갈 수 있어요.', 'We can ride this up.')])
        self.assertEqual([r['role'] for r in body['messages']], ['user','assistant','user'])
        self.assertEqual(body['messages'][-1]['content'], 'Translate the following Korean source text to English:\nKorean: 위로?\nEnglish: ')

    def test_translategemma_context_is_an_explicit_profile_with_completed_structured_turns(self):
        _, body = bench.request_for('translategemma-context', '위로?', [('이걸 타면 올라갈 수 있어요.', 'We can ride this up.')])
        self.assertEqual([r['role'] for r in body['messages']], ['user','assistant','user'])
        self.assertEqual(body['messages'][-1]['content'][0]['text'], '위로?')
        self.assertEqual(body['messages'][1]['content'], 'We can ride this up.')

    def test_subtitle_blocks_keep_previous_sources_and_answers_aligned(self):
        _, body = bench.request_for('sub', '위로?', [('올라갈 수 있어요.', 'We can go up.'), ('네.', 'Yes.')])
        self.assertEqual(body['messages'][1]['content'], '[PREVIOUS_SOURCE]\n올라갈 수 있어요.\n네.\n\n[PREVIOUS_TRANSLATION]\nWe can go up.\nYes.\n\n[CURRENT_SOURCE]\n위로?')

    def test_hy_mt2_context_stays_background_and_uses_the_publisher_sampler(self):
        _, body = bench.request_for('hy-mt2', '위로?', [('이걸 타면 올라갈 수 있어요.', 'We can ride this up.')])
        self.assertIn('[Source Text]\n위로?', body['messages'][0]['content'])
        self.assertEqual((body['temperature'], body['top_p'], body['top_k'], body['repeat_penalty']), (.7,.6,20,1.05))

    def test_kanana_receives_its_own_no_think_flag_rather_than_relying_on_another_models_flag(self):
        _, body = bench.request_for('kanana', '안녕.', [])
        self.assertEqual(body['chat_template_kwargs']['thinking_mode'], 'no_think')

    def test_rolling_history_skips_failed_outputs_while_fixed_history_uses_only_the_completed_case(self):
        turns = []
        case = {'source':'가지 마세요.', 'reference':'Do not go.'}
        failed = {'translation':'', 'error':'timeout'}
        bench.advance(turns, case, failed, 'rolling', 4)
        self.assertEqual(turns, [])
        bench.advance(turns, case, failed, 'fixed', 4)
        self.assertEqual(turns, [('가지 마세요.', 'Do not go.')])
        bench.advance(turns, case, {'translation':'Stay.', 'error':None}, 'rolling', 0)
        self.assertEqual(turns, [])

    def measured(self, stream, profile='tower'):
        with patch.object(bench.urllib.request, 'urlopen', return_value=io.BytesIO(stream.encode('utf-8'))):
            return bench.measure('http://127.0.0.1:1', profile, '안녕.', [])

    def test_unicode_text_and_unwanted_labels_survive_for_quality_review(self):
        result = self.measured('data: '+json.dumps({'choices':[{'delta':{'content':'English: 안녕'}, 'finish_reason':None}]}, ensure_ascii=False)+'\n\ndata: [DONE]\n\n')
        self.assertIsNone(result['error'])
        self.assertEqual(result['translation'], 'English: 안녕')
        self.assertIsNotNone(result['first_text_ms'])

    def test_capped_and_incomplete_streams_preserve_partial_text_but_are_not_successes(self):
        for tail in ('', 'data: {"choices":[{"delta":{},"finish_reason":"length"}]}\n\ndata: [DONE]\n'):
            result = self.measured('data: {"choices":[{"delta":{"content":"partial"}}]}\n\n'+tail)
            self.assertIsNotNone(result['error'])
            self.assertEqual(result['streamed_text'], 'partial')
            self.assertEqual(result['translation'], '')
        result = self.measured('data: {"content":"partial","stop":true,"stop_type":"limit"}\n', 'seed')
        self.assertIsNotNone(result['error'])
        self.assertEqual(result['streamed_text'], 'partial')

    def test_invalid_and_empty_streams_are_errors(self):
        for stream in ('data: invalid-json\n', 'data: {"error":"failed"}\n', 'data: [DONE]\n', 'data: []\n', 'data: {"choices":{}}\n', 'data: {"choices":[{"delta":{"content":{}}}]}\n'):
            self.assertIsNotNone(self.measured(stream)['error'])

    def test_failed_latencies_do_not_enter_success_percentiles(self):
        rows = [{'pass':'candidate','measurement':{'error':None,'first_text_ms':5,'done_ms':10}}, {'pass':'candidate','measurement':{'error':'cap','first_text_ms':20,'done_ms':200}}]
        candidate = bench.summarize(rows)[1]
        self.assertEqual((candidate['n'], candidate['errors'], candidate['done_p50']), (2,1,10))


if __name__ == '__main__':
    unittest.main()
