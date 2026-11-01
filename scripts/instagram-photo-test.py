"""Pure fixture tests for the Instagram photo plugin; no network or yt-dlp needed.

The stub mirrors the upstream InstagramIE contract the plugin relies on:
_extract_product_media() exposes only video formats, _extract_product() wraps
carousels in a playlist, and _real_extract() raises when a single post has no
formats.  scripts/instagram-photo-integration-test.py runs the same plugin
against a real yt-dlp checkout.
"""
import importlib.util
from pathlib import Path
import sys
import types
import unittest

sys.dont_write_bytecode = True


class ExtractorError(Exception):
    def __init__(self, msg, expected=False):
        super().__init__(msg)
        self.expected = expected


def int_or_none(value):
    try:
        return int(value)
    except (TypeError, ValueError):
        return None


def url_or_none(value):
    return value if isinstance(value, str) and value.startswith(('http://', 'https://')) else None


class StubInstagramIE:
    IE_NAME = 'Instagram'
    extractor_args = {}
    warnings = []
    product = None

    def __init_subclass__(cls, *, plugin_name=None, **kwargs):
        super().__init_subclass__(**kwargs)

    def _configuration_arg(self, key, default=None, *, ie_key=None):
        return self.extractor_args.get((ie_key, key), [])

    def report_warning(self, message):
        self.warnings.append(message)

    def _extract_product_media(self, product_media):
        formats = [{'url': v['url'], 'format_id': str(v.get('type'))}
                   for v in product_media.get('video_versions') or []]
        return {'id': product_media.get('pk'), 'formats': formats, 'thumbnails': []}

    def _extract_product(self, product_info, video_id=None, get_comments=True):
        info = {'id': video_id, 'title': f"Video by {product_info['user']}", 'channel': product_info['user']}
        if product_info.get('carousel_media'):
            return {'_type': 'playlist', **info, 'title': f"Post by {product_info['user']}",
                    'entries': [{**info, **self._extract_product_media(m)} for m in product_info['carousel_media']]}
        return {**info, **self._extract_product_media(product_info)}

    def _real_extract(self, url):
        info = self._extract_product(self.product, video_id='abc', get_comments=False)
        if info.get('_type') != 'playlist' and not info.get('formats'):
            raise ExtractorError('There is no video in this post', expected=True)
        return info


modules = {name: types.ModuleType(name) for name in (
    'yt_dlp', 'yt_dlp.extractor', 'yt_dlp.extractor.instagram', 'yt_dlp.utils')}
modules['yt_dlp.extractor.instagram'].InstagramIE = StubInstagramIE
modules['yt_dlp.utils'].ExtractorError = ExtractorError
modules['yt_dlp.utils'].int_or_none = int_or_none
modules['yt_dlp.utils'].url_or_none = url_or_none
sys.modules.update(modules)
path = Path(__file__).resolve().parents[1] / 'src-tauri/resources/instagram_photos.py'
spec = importlib.util.spec_from_file_location('instagram_photos', path)
plugin = importlib.util.module_from_spec(spec)
spec.loader.exec_module(plugin)


def photo(pk, *candidates):
    return {'pk': pk, 'image_versions2': {'candidates': [
        {'url': url, 'width': w, 'height': h} for url, w, h in candidates]}}


def video(pk):
    return {**photo(pk, ('https://cdn.example/poster.jpg', 1080, 1920)),
            'video_versions': [{'url': f'https://cdn.example/{pk}.mp4', 'type': 101}]}


def extract(product, skip=False):
    ie = plugin._BaldojnyiInstagramIE()
    ie.product = {'user': 'author', **product}
    ie.warnings = []
    ie.extractor_args = {('Instagram', 'baldojnyi_photos'): ['skip']} if skip else {}
    return ie, ie._real_extract('https://www.instagram.com/p/abc/')


class BestPhotoFormatTests(unittest.TestCase):
    def test_largest_candidate_wins_regardless_of_order(self):
        fmt = plugin.best_photo_format(photo('1', ('https://cdn.example/s.jpg?x=1', 320, 400),
                                             ('https://cdn.example/l.jpg?x=1', 1080, 1350),
                                             ('https://cdn.example/m.jpg?x=1', 640, 800)))
        self.assertEqual(fmt['url'], 'https://cdn.example/l.jpg?x=1')
        self.assertEqual((fmt['width'], fmt['height'], fmt['ext']), (1080, 1350, 'jpg'))
        self.assertEqual(fmt['format_id'], 'baldojnyi-photo')

    def test_media_with_any_video_never_gets_a_photo_format(self):
        self.assertIsNone(plugin.best_photo_format(video('1')))
        self.assertIsNone(plugin.best_photo_format({**photo('1', ('https://cdn.example/a.jpg', 1, 1)),
                                                    'video_dash_manifest': '<MPD/>'}))

    def test_malformed_or_unsafe_candidates_are_ignored(self):
        self.assertIsNone(plugin.best_photo_format(None))
        self.assertIsNone(plugin.best_photo_format({'image_versions2': {'candidates': 'nope'}}))
        self.assertIsNone(plugin.best_photo_format(photo('1', ('javascript:alert(1)', 99, 99))))
        fmt = plugin.best_photo_format({'image_versions2': {'candidates': [
            None, {'url': 'file:///etc/passwd', 'width': 5000, 'height': 5000},
            {'url': 'https://cdn.example/ok.webp', 'width': 'x'}]}})
        self.assertEqual((fmt['url'], fmt['ext']), ('https://cdn.example/ok.webp', 'webp'))

    def test_extension_falls_back_to_jpg(self):
        for url, ext in [('https://cdn.example/a.jpeg?b=c.png', 'jpg'), ('https://cdn.example/a.heic', 'heic'),
                         ('https://cdn.example/v/t51/12345', 'jpg'), ('https://cdn.example/a.exe', 'jpg')]:
            self.assertEqual(plugin.best_photo_format(photo('1', (url, 1, 1)))['ext'], ext, url)


class ExtractorOverrideTests(unittest.TestCase):
    def test_single_photo_post_becomes_downloadable(self):
        _, info = extract(photo('1', ('https://cdn.example/a.jpg', 1080, 1080)))
        self.assertEqual([f['format_id'] for f in info['formats']], ['baldojnyi-photo'])
        self.assertIs(info['baldojnyi_photo'], True)
        self.assertEqual(info['title'], 'Photo by author')

    def test_video_post_is_returned_unchanged(self):
        expected = StubInstagramIE()
        expected.product = {'user': 'author', **video('1')}
        _, info = extract(video('1'))
        self.assertEqual(info, expected._real_extract('x'))
        self.assertNotIn('baldojnyi_photo', info)

    def test_mixed_carousel_keeps_slide_order_and_types(self):
        _, info = extract({'carousel_media': [video('1'), photo('2', ('https://cdn.example/2.jpg', 9, 9)), video('3')]})
        entries = info['entries']
        self.assertEqual([e['id'] for e in entries], ['1', '2', '3'])
        self.assertEqual([e['formats'][0]['format_id'] for e in entries], ['101', 'baldojnyi-photo', '101'])
        self.assertEqual([e['title'] for e in entries], ['Video by author', 'Photo by author', 'Video by author'])

    def test_audio_skip_mode_drops_photos_and_warns(self):
        ie, info = extract({'carousel_media': [video('1'), photo('2', ('https://cdn.example/2.jpg', 9, 9))]}, skip=True)
        self.assertEqual([e['id'] for e in info['entries']], ['1'])
        self.assertEqual(ie.warnings, ['BALDOJNYI_PHOTOS_SKIPPED=1'])

    def test_audio_skip_mode_on_photo_only_posts_raises_marker(self):
        for product in (photo('1', ('https://cdn.example/1.jpg', 9, 9)),
                        {'carousel_media': [photo('1', ('https://cdn.example/1.jpg', 9, 9)),
                                            photo('2', ('https://cdn.example/2.jpg', 9, 9))]}):
            with self.assertRaises(ExtractorError) as caught:
                extract(product, skip=True)
            self.assertTrue(str(caught.exception).startswith('BALDOJNYI_PHOTO_ONLY'))
            self.assertTrue(caught.exception.expected)

    def test_other_errors_are_not_masked(self):
        class Broken(plugin._BaldojnyiInstagramIE):
            def _extract_product(self, *args, **kwargs):
                raise ExtractorError('login required')
        ie = Broken()
        ie.extractor_args = {('Instagram', 'baldojnyi_photos'): ['skip']}
        with self.assertRaisesRegex(ExtractorError, 'login required'):
            ie._real_extract('x')

    def test_media_without_any_photo_or_video_still_fails_upstream_way(self):
        with self.assertRaisesRegex(ExtractorError, 'There is no video in this post'):
            extract({'pk': '1'})

    def test_service_name_stays_instagram(self):
        self.assertEqual(plugin._BaldojnyiInstagramIE.IE_NAME, 'Instagram')


if __name__ == '__main__':
    unittest.main(verbosity=2)
