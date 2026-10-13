import unittest
from bench_low_vram import candidate_request, resource_summary


class LowVramTests(unittest.TestCase):
    def test_the_lfm_request_preserves_korean_and_uses_the_publishers_native_instruction(self):
        route, body = candidate_request('lfm', '부모 다음으로 중요하다.', [])
        self.assertEqual(route, '/v1/chat/completions')
        self.assertEqual(body['messages'], [{'role':'system', 'content':'Translate the following text to English.'}, {'role':'user', 'content':'부모 다음으로 중요하다.'}])
        self.assertEqual(body['temperature'], .3)
        with self.assertRaises(ValueError):
            candidate_request('lfm', '학교', [('어제', 'Yesterday')])

    def test_missing_gpu_counters_stay_missing_and_foreign_processes_are_excluded(self):
        def row(pid, value):
            return dict(target=dict(pid=pid), dedicated_gpu_bytes=value, shared_gpu_bytes=None, ram_bytes=123)
        summary = resource_summary([dict(processes=[row(1,None),row(2,999999)]), dict(processes=[row(1,100)])], [1])
        self.assertEqual(summary['1']['dedicated_gpu_bytes'], dict(samples=1, p50=100, max=100))
        self.assertEqual(summary['1']['shared_gpu_bytes'], dict(samples=0, p50=None, max=None))
        self.assertNotIn('2', summary)


if __name__ == '__main__':
    unittest.main()
