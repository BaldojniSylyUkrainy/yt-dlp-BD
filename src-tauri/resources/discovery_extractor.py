"""Opt-in fallback only; never shadows a native yt-dlp site extractor."""
import html
import ipaddress
import json
import re
import socket
from urllib.parse import urljoin, urlparse

from yt_dlp.extractor.common import InfoExtractor
from yt_dlp.utils import ExtractorError


def public_address(value):
    try:
        ip = ipaddress.ip_address(value.split('%')[0])
        mapped = getattr(ip, 'ipv4_mapped', None)
        return ip.is_global and not ip.is_multicast and (mapped is None or public_address(str(mapped)))
    except ValueError:
        return False


def guarded_resolver(resolve):
    def lookup(*args, **kwargs):
        addresses = resolve(*args, **kwargs)
        if not addresses or any(not public_address(address[4][0]) for address in addresses):
            raise OSError('Discovery blocked a local or special network address')
        # The exact checked numeric addresses are used by the socket connection.
        return addresses
    return lookup


def install_network_guard(ydl):
    """Fail closed before any discovery request; scoped to this child process.

    Redirects, manifests, fragments and encryption keys share Urllib's transport.
    No curl/native/external downloader may bypass the checked socket resolver.
    Unsupported future yt-dlp APIs raise here, before a page is fetched.
    """
    if getattr(ydl, '_baldoj_guarded', False):
        return
    from yt_dlp.networking._urllib import UrllibRH
    from yt_dlp.downloader.external import ExternalFD
    from yt_dlp.utils import DownloadError

    class PublicUrllibRH(UrllibRH):
        _SUPPORTED_URL_SCHEMES = ('http', 'https')

    def deny_external(*args, **kwargs):
        raise DownloadError('Discovery refuses an unguarded external media downloader')

    # Do not inherit environment proxies or impersonation transports. Browser
    # cookies explicitly requested by the user stay in this process only.
    ydl.params['proxy'] = ''
    ydl.params['impersonate'] = None
    ydl.params['hls_prefer_native'] = True
    ydl.params['external_downloader'] = {}
    # Postprocessors only receive local files; never let a media container make
    # FFmpeg/ffprobe open an embedded network URL outside this transport.
    ydl.params['postprocessor_args'] = {'ffmpeg_i': ['-protocol_whitelist', 'file,pipe'], 'ffprobe_i': ['-protocol_whitelist', 'file,pipe']}
    ydl.__dict__.pop('proxies', None)
    previous = ydl.__dict__.pop('_request_director', None)
    if previous:
        previous.close()
    director = ydl.build_request_director([PublicUrllibRH], set())
    socket.getaddrinfo = guarded_resolver(socket.getaddrinfo)
    ExternalFD.real_download = deny_external
    ydl.__dict__['_request_director'] = director
    ydl._baldoj_guarded = True


class BaldojnyiSourceIE(InfoExtractor):
    IE_NAME = 'source:baldojnyi'
    _VALID_URL = r'baldoj-source:(?P<id>https?://.+)'

    def _real_extract(self, url):
        install_network_guard(self._downloader)
        return self.url_result(self._match_id(url), ie='Generic')


def candidate_urls(webpage, page_url):
    text = html.unescape(webpage[:4 * 1024 * 1024]).replace(r'\/', '/')
    found = []
    for match in re.finditer(r'''["']([^"'<>\s]+?\.(?:mp4|m3u8|mpd)(?:\?[^"'<>\s]*)?)["']''', text, re.I):
        url = urljoin(page_url, match.group(1))
        parsed = urlparse(url)
        host = parsed.hostname or ''
        if parsed.scheme not in ('http', 'https') or parsed.username or parsed.password:
            continue
        if host == 'localhost' or host.endswith(('.localhost', '.local')):
            continue
        try:
            if not ipaddress.ip_address(host).is_global:
                continue
        except ValueError:
            pass
        if url not in found:
            found.append(url)
        if len(found) >= 30:
            break
    return found


class BaldojnyiDiscoveryIE(InfoExtractor):
    IE_NAME = 'discovery:baldojnyi'
    _VALID_URL = r'baldoj-discover:(?P<id>https?://.+)'

    def _real_extract(self, url):
        install_network_guard(self._downloader)
        page_url = self._match_id(url)
        webpage = self._download_webpage(page_url, 'discovery')
        candidates = candidate_urls(webpage, page_url)
        if not candidates:
            raise ExtractorError('Automatic discovery found no media; try browser discovery', expected=True)
        self.to_screen('__YTDLP_CANDIDATES__' + json.dumps({'pageUrl': page_url, 'urls': candidates}))
        raise ExtractorError('Media sources found; choose the required video in the app', expected=True)
