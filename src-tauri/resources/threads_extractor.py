import html
import json
import re

from yt_dlp.extractor.common import InfoExtractor
from yt_dlp.utils import ExtractorError, int_or_none, str_or_none, url_or_none


class BaldojnyiThreadsIE(InfoExtractor):
    IE_NAME = 'threads:baldojnyi'
    _VALID_URL = r'https?://(?:www\.)?threads\.(?:com|net)/(?:@[^/]+/(?:post|t)|share)/(?P<id>[^/?#&]+)'

    @staticmethod
    def _walk_json(value):
        if isinstance(value, dict):
            yield value
            for child in value.values():
                yield from BaldojnyiThreadsIE._walk_json(child)
        elif isinstance(value, list):
            for child in value:
                yield from BaldojnyiThreadsIE._walk_json(child)
        elif isinstance(value, str) and value[:1] in ('{', '['):
            try:
                decoded = json.loads(value)
            except (TypeError, ValueError):
                return
            yield from BaldojnyiThreadsIE._walk_json(decoded)

    def _post_from_webpage(self, webpage, post_id):
        script_bodies = re.findall(r'<script\b[^>]*>(.*?)</script>', webpage, flags=re.DOTALL | re.IGNORECASE)
        for script_body in script_bodies:
            if post_id not in script_body:
                continue
            try:
                payload = json.loads(script_body)
            except (TypeError, ValueError):
                try:
                    payload = json.loads(html.unescape(script_body))
                except (TypeError, ValueError):
                    continue
            for candidate in self._walk_json(payload):
                post = candidate.get('post') if isinstance(candidate.get('post'), dict) else candidate
                if post.get('code') == post_id and (
                    post.get('video_versions') or post.get('carousel_media')
                ):
                    return post
        return None

    @staticmethod
    def _first_thumbnail(media):
        candidates = (media.get('image_versions2') or {}).get('candidates') or []
        for candidate in candidates:
            thumbnail = url_or_none(candidate.get('url'))
            if thumbnail:
                return thumbnail
        return None

    def _media_entry(self, media, post_id, index, page_url, metadata):
        formats = []
        seen_urls = set()
        for version in media.get('video_versions') or []:
            media_url = url_or_none(version.get('url'))
            if not media_url or media_url in seen_urls:
                continue
            seen_urls.add(media_url)
            formats.append({
                'format_id': str_or_none(version.get('type')) or f'http-{index}',
                'url': media_url,
                'ext': 'mp4',
                'width': int_or_none(version.get('width')) or int_or_none(media.get('original_width')),
                'height': int_or_none(version.get('height')) or int_or_none(media.get('original_height')),
                'http_headers': {'Referer': page_url},
            })
        if not formats:
            return None
        title = metadata['title']
        if metadata['item_count'] > 1:
            title = f'{title} ({index + 1})'
        return {
            'id': post_id if metadata['item_count'] == 1 else f'{post_id}-{index + 1}',
            'title': title,
            'description': metadata['description'],
            'uploader': metadata['uploader'],
            'uploader_id': metadata['uploader_id'],
            'timestamp': metadata['timestamp'],
            'thumbnail': self._first_thumbnail(media) or metadata['thumbnail'],
            'webpage_url': page_url,
            'playlist_count': metadata['item_count'],
            'formats': formats,
        }

    def _real_extract(self, url):
        post_id = self._match_id(url)
        webpage, response = self._download_webpage_handle(
            url, post_id, headers={'Referer': 'https://www.threads.com/'})
        page_url = str(response.url)
        if 'error=invalid_post' in page_url:
            raise ExtractorError('Threads post is unavailable or was removed', expected=True)
        canonical_match = re.match(self._VALID_URL, page_url)
        if canonical_match and '/share/' not in page_url:
            post_id = canonical_match.group('id')
        post = self._post_from_webpage(webpage, post_id)

        if post is None:
            og_video = url_or_none(self._html_search_meta(
                ('og:video', 'og:video:url', 'twitter:player:stream'), webpage, default=None))
            if og_video:
                return {
                    'id': post_id,
                    'title': self._og_search_title(webpage, default=post_id),
                    'description': self._og_search_description(webpage, default=None),
                    'thumbnail': self._og_search_thumbnail(webpage, default=None),
                    'webpage_url': page_url,
                    'formats': [{
                        'format_id': 'http',
                        'url': og_video,
                        'ext': 'mp4',
                        'http_headers': {'Referer': page_url},
                    }],
                }
            if 'Barcelona404ErrorRoot' in webpage:
                raise ExtractorError('Threads post is unavailable or was removed', expected=True)
            self.raise_login_required(
                'Threads did not expose media to a guest session. Use --cookies-from-browser')

        media_items = post.get('carousel_media') or [post]
        video_items = [media for media in media_items if media.get('video_versions')]
        if not video_items:
            raise ExtractorError('This Threads post does not contain downloadable video', expected=True)

        user = post.get('user') or {}
        caption = post.get('caption') or {}
        description = str_or_none(caption.get('text')) or self._og_search_description(webpage, default=None)
        uploader_id = str_or_none(user.get('username'))
        uploader = str_or_none(user.get('full_name')) or uploader_id
        title = description or self._og_search_title(webpage, default=None) or f'Threads {post_id}'
        metadata = {
            'title': title,
            'description': description,
            'uploader': uploader,
            'uploader_id': uploader_id,
            'timestamp': int_or_none(post.get('taken_at')),
            'thumbnail': self._first_thumbnail(post),
            'item_count': len(video_items),
        }
        entries = [
            entry for index, media in enumerate(video_items)
            if (entry := self._media_entry(media, post_id, index, page_url, metadata))
        ]
        if not entries:
            raise ExtractorError('Threads returned video metadata without a usable media URL', expected=True)
        if len(entries) == 1 or self.get_param('noplaylist'):
            return entries[0]
        return self.playlist_result(entries, post_id, title, description)
