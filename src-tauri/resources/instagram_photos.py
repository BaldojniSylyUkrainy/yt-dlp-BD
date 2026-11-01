"""Instagram photo support for BaldojnyiDownloader.

yt-dlp's Instagram extractor exposes only ``video_versions`` as downloadable
formats; photos stay in ``thumbnails``.  This override keeps the upstream
extractor (and every video it returns) intact and adds exactly one photo format
to a post or carousel slide only when that media item has no video at all.

Any unexpected upstream structure disables the photo path for that item and
returns the original yt-dlp result, so video downloads never depend on it.
"""
from yt_dlp.extractor.instagram import InstagramIE
from yt_dlp.utils import ExtractorError, int_or_none, url_or_none

PHOTO_FORMAT_ID = 'baldojnyi-photo'
PHOTO_FIELD = 'baldojnyi_photo'
PHOTO_ONLY_MARKER = 'BALDOJNYI_PHOTO_ONLY'
PHOTOS_SKIPPED_MARKER = 'BALDOJNYI_PHOTOS_SKIPPED'
_PHOTO_EXTENSIONS = {'jpg', 'jpeg', 'png', 'webp', 'heic', 'avif'}


def _photo_extension(url):
    path = url.split('?', 1)[0].split('#', 1)[0]
    extension = path.rsplit('.', 1)[-1].lower() if '.' in path.rsplit('/', 1)[-1] else ''
    if extension == 'jpeg':
        return 'jpg'
    return extension if extension in _PHOTO_EXTENSIONS else 'jpg'


def best_photo_format(product_media):
    """Return the largest still image of a media item that has no video."""
    if not isinstance(product_media, dict):
        return None
    if product_media.get('video_versions') or product_media.get('video_dash_manifest'):
        return None
    candidates = (product_media.get('image_versions2') or {}).get('candidates')
    if not isinstance(candidates, list):
        return None
    best = None
    best_area = -1
    for candidate in candidates:
        if not isinstance(candidate, dict):
            continue
        url = url_or_none(candidate.get('url'))
        if not url:
            continue
        width = int_or_none(candidate.get('width'))
        height = int_or_none(candidate.get('height'))
        area = (width or 0) * (height or 0)
        if area > best_area:
            best, best_area = (url, width, height), area
    if best is None:
        return None
    url, width, height = best
    return {
        'format_id': PHOTO_FORMAT_ID,
        'format_note': 'photo',
        'url': url,
        'ext': _photo_extension(url),
        'width': width or int_or_none(product_media.get('original_width')),
        'height': height or int_or_none(product_media.get('original_height')),
        'vcodec': 'none',
        'acodec': 'none',
    }


class _BaldojnyiInstagramIE(InstagramIE, plugin_name='baldojnyi'):
    _baldojnyi_skipped_photos = 0
    _baldojnyi_photo_only = False

    def _baldojnyi_skip_photos(self):
        try:
            return 'skip' in self._configuration_arg('baldojnyi_photos', ie_key='Instagram')
        except Exception:
            return False

    def _extract_product_media(self, product_media):
        info = super()._extract_product_media(product_media)
        if info.get('formats'):
            return info
        try:
            photo = best_photo_format(product_media)
        except Exception:
            return info
        if not photo:
            return info
        if self._baldojnyi_skip_photos():
            self._baldojnyi_skipped_photos += 1
            return {**info, PHOTO_FIELD: 'skipped'}
        return {**info, 'formats': [photo], PHOTO_FIELD: True}

    @staticmethod
    def _baldojnyi_photo_title(entry):
        title = entry.get('title')
        channel = entry.get('channel')
        if entry.get(PHOTO_FIELD) is True and channel and title == f'Video by {channel}':
            return {**entry, 'title': f'Photo by {channel}'}
        return entry

    def _extract_product(self, product_info, *args, **kwargs):
        result = super()._extract_product(product_info, *args, **kwargs)
        try:
            if result.get('_type') == 'playlist':
                entries = [self._baldojnyi_photo_title(entry) for entry in result.get('entries') or []]
                kept = [entry for entry in entries if entry.get(PHOTO_FIELD) != 'skipped']
                if entries and not kept:
                    self._baldojnyi_photo_only = True
                return {**result, 'entries': kept}
            if result.get(PHOTO_FIELD) == 'skipped':
                self._baldojnyi_photo_only = True
            return self._baldojnyi_photo_title(result)
        except Exception:
            return result

    @staticmethod
    def _baldojnyi_photo_only_error():
        return ExtractorError(
            f'{PHOTO_ONLY_MARKER}: this Instagram post contains only photos', expected=True)

    def _real_extract(self, url):
        self._baldojnyi_skipped_photos = 0
        self._baldojnyi_photo_only = False
        try:
            result = super()._real_extract(url)
        except ExtractorError as error:
            if self._baldojnyi_photo_only:
                raise self._baldojnyi_photo_only_error() from error
            raise
        if self._baldojnyi_photo_only:
            raise self._baldojnyi_photo_only_error()
        if self._baldojnyi_skipped_photos:
            self.report_warning(f'{PHOTOS_SKIPPED_MARKER}={self._baldojnyi_skipped_photos}')
        return result


# Keep the user-visible service name unchanged in yt-dlp metadata and history.
_BaldojnyiInstagramIE.IE_NAME = getattr(InstagramIE, 'IE_NAME', None) or 'Instagram'
