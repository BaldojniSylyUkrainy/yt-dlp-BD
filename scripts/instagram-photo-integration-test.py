"""End-to-end check of the Instagram photo plugin against a real yt-dlp checkout.

No Instagram network access is used: a test-only plugin replaces the network
part of the upstream extractor with fixture post data, and a local HTTP server
serves the media.  Everything else (plugin override loading, lazy extractors,
format selection, extractor-args, filenames and --print templates) is real
yt-dlp.

Usage:
    YT_DLP_SOURCE=/path/to/yt-dlp-checkout python scripts/instagram-photo-integration-test.py

Skips when no yt-dlp source is given.  FFmpeg is needed to create fixtures.
"""
import functools
import http.server
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import threading
import unittest

sys.dont_write_bytecode = True
ROOT = Path(__file__).resolve().parents[1]
PLUGIN = ROOT / 'src-tauri/resources/instagram_photos.py'
YT_DLP_SOURCE = os.environ.get('YT_DLP_SOURCE')

# Mirrors src-tauri/src/lib.rs. The Rust tests assert the same strings.
BEST_SELECTOR = 'bestvideo+bestaudio/best/baldojnyi-photo'
VIDEO_SELECTOR = BEST_SELECTOR  # quality limits are a separate `-S res:N` sort
AUDIO_SELECTOR = 'bestaudio/best'
AFTER_MOVE = ('after_move:__YTDLP_FILE__%(.{filepath,duration,height,fps,vcodec,acodec,'
              'webpage_url,title,thumbnail,uploader,extractor,baldojnyi_photo})j')

FIXTURE_PLUGIN = r'''
import json, os
import yt_dlp.extractor.instagram as instagram

original = instagram.InstagramIE
while getattr(original, '__wrapped__', None):
    original = original.__wrapped__


def fake_real_extract(self, url):
    video_id = self._match_valid_url(url).group('id')
    product_info = json.loads(os.environ['BALDOJ_IG_FIXTURE'])
    # Same tail as upstream InstagramIE._real_extract after the network part.
    info_dict = self._extract_product(product_info, video_id=video_id, get_comments=False)
    if info_dict.get('_type') != 'playlist' and not info_dict.get('formats'):
        self.raise_no_formats('There is no video in this post', expected=True)
    return info_dict


original._real_extract = fake_real_extract
original._real_initialize = lambda self: None  # upstream fetches a guest token here
'''


def media(pk, base, *, video=False, photo=True):
    item = {'pk': str(pk), 'id': f'{pk}_1', 'original_width': 1080, 'original_height': 1350}
    if photo:
        item['image_versions2'] = {'candidates': [
            {'url': f'{base}/photo-{pk}.jpg?stp=small', 'width': 320, 'height': 400},
            {'url': f'{base}/photo-{pk}.jpg?stp=full', 'width': 1080, 'height': 1350},
            {'url': f'{base}/photo-{pk}.jpg?stp=mid', 'width': 640, 'height': 800},
        ]}
    if video:
        item['video_versions'] = [{'url': f'{base}/video.mp4', 'width': 640, 'height': 360, 'type': 101}]
        item['video_duration'] = 1.0
        item['has_audio'] = True
    return item


def post(base, items):
    info = {
        'pk': '3000000000000000001',
        'code': 'DeRf1RKjLc7',
        'user': {'username': 'baldoj_test', 'full_name': 'Baldoj Test', 'pk': '42'},
        'caption': {'text': 'fixture post'},
        'taken_at': 1760000000,
    }
    if len(items) == 1:
        info.update(items[0])
        info['pk'] = '3000000000000000001'
    else:
        info['carousel_media'] = items
    return info


@unittest.skipUnless(YT_DLP_SOURCE, 'set YT_DLP_SOURCE to a yt-dlp checkout')
class InstagramPhotoIntegration(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.work = Path(tempfile.mkdtemp(prefix='baldoj-ig-'))
        served = cls.work / 'served'
        served.mkdir()
        for pk in range(1, 6):
            subprocess.run(['ffmpeg', '-v', 'error', '-f', 'lavfi', '-i', f'color=c=red:s=108x135',
                            '-frames:v', '1', str(served / f'photo-{pk}.jpg')], check=True)
        subprocess.run(['ffmpeg', '-v', 'error', '-f', 'lavfi', '-i', 'testsrc=s=640x360:d=1',
                        '-f', 'lavfi', '-i', 'sine=d=1', '-c:v', 'libx264', '-pix_fmt', 'yuv420p',
                        '-c:a', 'aac', '-shortest', str(served / 'video.mp4')], check=True)
        handler = functools.partial(QuietHandler, directory=str(served))
        cls.server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), handler)
        threading.Thread(target=cls.server.serve_forever, daemon=True).start()
        cls.base = f'http://127.0.0.1:{cls.server.server_address[1]}'
        app_plugins = cls.work / 'app-plugins/baldojnyi/yt_dlp_plugins/extractor'
        app_plugins.mkdir(parents=True)
        shutil.copy(PLUGIN, app_plugins / 'instagram_baldojnyi.py')
        fixture_plugins = cls.work / 'fixture-plugins/fixture/yt_dlp_plugins/extractor'
        fixture_plugins.mkdir(parents=True)
        (fixture_plugins / 'zz_fixture.py').write_text(FIXTURE_PLUGIN)

    @classmethod
    def tearDownClass(cls):
        cls.server.shutdown()
        shutil.rmtree(cls.work, ignore_errors=True)

    def run_app_command(self, items, selector, *, audio=False, lazy=True, plugin=True):
        out = Path(tempfile.mkdtemp(dir=self.work))
        args = [sys.executable, '-m', 'yt_dlp', '--ignore-config', '--encoding', 'UTF-8',
                '--no-simulate', '--newline', '--no-colors', '--progress',
                '-o', '%(title).140B [%(id).40B].%(ext)s', '-P', str(out)]
        if plugin:
            args += ['--plugin-dirs', str(self.work / 'app-plugins')]
        args += ['--plugin-dirs', str(self.work / 'fixture-plugins'),
                 '--no-playlist', '-f', selector, '--print', AFTER_MOVE]
        if audio:
            args += ['--extractor-args', 'instagram:baldojnyi_photos=skip']
        else:
            args += ['--merge-output-format', 'mkv']
        args += ['-P', f'temp:{out / "tmp"}', 'https://www.instagram.com/p/DeRf1RKjLc7/?img_index=1']
        env = {**os.environ, 'PYTHONPATH': YT_DLP_SOURCE,
               'BALDOJ_IG_FIXTURE': json.dumps(post(self.base, items))}
        if not lazy:
            env['YTDLP_NO_LAZY_EXTRACTORS'] = '1'
        result = subprocess.run(args, env=env, capture_output=True, text=True, timeout=120)
        files = [json.loads(line[len('__YTDLP_FILE__'):]) for line in result.stdout.splitlines()
                 if line.startswith('__YTDLP_FILE__')]
        return result, files

    def assert_ok(self, result):
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertNotIn('Error while importing module', result.stderr)

    def test_single_photo_downloads_largest_image_with_or_without_lazy_extractors(self):
        for lazy in (True, False):
            with self.subTest(lazy=lazy):
                result, files = self.run_app_command([media(1, self.base)], VIDEO_SELECTOR, lazy=lazy)
                self.assert_ok(result)
                self.assertEqual(len(files), 1)
                self.assertTrue(files[0]['filepath'].endswith('.jpg'))
                self.assertTrue(files[0]['baldojnyi_photo'])
                self.assertEqual(files[0]['extractor'], 'Instagram')
                self.assertIn('Photo by baldoj_test', Path(files[0]['filepath']).name)
                self.assertGreater(Path(files[0]['filepath']).stat().st_size, 0)

    def test_photo_carousel_downloads_every_slide(self):
        result, files = self.run_app_command([media(n, self.base) for n in (1, 2, 3)], BEST_SELECTOR)
        self.assert_ok(result)
        self.assertEqual(len(files), 3)
        self.assertEqual(len({f['filepath'] for f in files}), 3)
        self.assertTrue(all(f['baldojnyi_photo'] and f['filepath'].endswith('.jpg') for f in files))

    def test_mixed_carousel_keeps_order_and_video_path_untouched(self):
        items = [media(1, self.base, video=True), media(2, self.base), media(3, self.base, video=True)]
        result, files = self.run_app_command(items, VIDEO_SELECTOR)
        self.assert_ok(result)
        self.assertEqual([Path(f['filepath']).suffix for f in files], ['.mp4', '.jpg', '.mp4'])
        self.assertEqual([bool(f.get('baldojnyi_photo')) for f in files], [False, True, False])
        self.assertNotEqual(files[0].get('vcodec'), 'none')
        self.assertEqual(len({f['filepath'] for f in files}), 3)

    def test_video_post_is_identical_with_and_without_plugin(self):
        with_plugin = self.run_app_command([media(1, self.base, video=True)], VIDEO_SELECTOR)
        without_plugin = self.run_app_command([media(1, self.base, video=True)], VIDEO_SELECTOR, plugin=False)
        for result, files in (with_plugin, without_plugin):
            self.assert_ok(result)
            self.assertEqual(len(files), 1)
        strip = lambda f: {k: v for k, v in f.items() if k != 'filepath'}
        self.assertEqual(strip(with_plugin[1][0]), strip(without_plugin[1][0]))
        self.assertEqual(Path(with_plugin[1][0]['filepath']).name, Path(without_plugin[1][0]['filepath']).name)

    def test_without_plugin_photos_still_fail_as_upstream(self):
        result, files = self.run_app_command([media(1, self.base)], VIDEO_SELECTOR, plugin=False)
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(files, [])

    def test_audio_mode_skips_photos_in_mixed_carousel_and_reports_it(self):
        items = [media(1, self.base, video=True), media(2, self.base), media(3, self.base)]
        result, files = self.run_app_command(items, AUDIO_SELECTOR, audio=True)
        self.assert_ok(result)
        self.assertEqual(len(files), 1)
        self.assertFalse(files[0].get('baldojnyi_photo'))
        self.assertIn('BALDOJNYI_PHOTOS_SKIPPED=2', result.stderr)

    def test_audio_mode_on_photo_only_posts_fails_with_marker(self):
        for items in ([media(1, self.base)], [media(1, self.base), media(2, self.base)]):
            with self.subTest(count=len(items)):
                result, files = self.run_app_command(items, AUDIO_SELECTOR, audio=True)
                self.assertNotEqual(result.returncode, 0)
                self.assertEqual(files, [])
                self.assertIn('BALDOJNYI_PHOTO_ONLY', result.stderr)
                self.assertNotIn('no video formats', result.stderr.lower())


class QuietHandler(http.server.SimpleHTTPRequestHandler):
    def log_message(self, *args):
        pass


if __name__ == '__main__':
    unittest.main(verbosity=2)
