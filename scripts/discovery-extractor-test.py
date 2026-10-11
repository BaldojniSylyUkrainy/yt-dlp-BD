"""Pure fixture tests; no network and no installed yt-dlp required."""
import importlib.util
from pathlib import Path
import sys
import types
import unittest
sys.dont_write_bytecode = True

for name in ('yt_dlp', 'yt_dlp.extractor', 'yt_dlp.extractor.common', 'yt_dlp.utils'):
    sys.modules[name] = types.ModuleType(name)
sys.modules['yt_dlp.extractor.common'].InfoExtractor = type('InfoExtractor', (), {})
sys.modules['yt_dlp.utils'].ExtractorError = RuntimeError
path = Path(__file__).resolve().parents[1] / 'src-tauri/resources/discovery_extractor.py'
spec = importlib.util.spec_from_file_location('discovery', path)
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


class DiscoveryTests(unittest.TestCase):
    def test_real_resolution_answers_are_checked_not_only_hostname(self):
        def answers(ip):
            return [(2, 1, 6, '', (ip, 443))]
        for ip in ['127.0.0.1', '10.0.0.1', '169.254.169.254', '::1', '::ffff:127.0.0.1', '224.0.0.1', '100.64.0.1']:
            with self.assertRaises(OSError):
                module.guarded_resolver(lambda *a: answers(ip))('public.example', 443)
        good = answers('1.1.1.1')
        self.assertIs(module.guarded_resolver(lambda *a: good)('public.example', 443), good)
        with self.assertRaises(OSError):
            module.guarded_resolver(lambda *a: good + answers('192.168.1.1'))('mixed.example', 443)

    def test_redirect_or_rebinding_lookup_is_rechecked(self):
        addresses = iter(['1.1.1.1', '127.0.0.1'])
        resolver = module.guarded_resolver(lambda *a: [(2, 1, 6, '', (next(addresses), 443))])
        resolver('changing.example', 443)
        with self.assertRaises(OSError):
            resolver('changing.example', 443)

    def test_relative_filenames_without_leading_slash(self):
        page = '<source src="movie.mp4"><source src="../media/master.m3u8">'
        self.assertEqual(module.candidate_urls(page, 'https://example.com/html/video'), ['https://example.com/html/movie.mp4', 'https://example.com/media/master.m3u8'])

    def test_relative_signed_sources_and_deduplication(self):
        page = '<video src="/file.mp4?a=1&amp;b=2"></video><script>{"src":"https:\\/\\/cdn.example.com/master.m3u8"}</script>'
        self.assertEqual(module.candidate_urls(page + page, 'https://example.com/post'), ['https://example.com/file.mp4?a=1&b=2', 'https://cdn.example.com/master.m3u8'])

    def test_private_and_credential_urls_are_not_offered(self):
        page = ' '.join('"' + u + '"' for u in ['http://127.0.0.1/v.mp4', 'http://10.1.1.1/v.mp4', 'https://u:p@site.example/v.mp4', 'http://localhost/v.mp4'])
        self.assertEqual(module.candidate_urls(page, 'https://example.com'), [])

    def test_candidate_count_is_bounded(self):
        page = ' '.join(f'"https://example.com/{i}.mp4"' for i in range(100))
        self.assertEqual(len(module.candidate_urls(page, 'https://example.com')), 30)


if __name__ == '__main__':
    unittest.main()
