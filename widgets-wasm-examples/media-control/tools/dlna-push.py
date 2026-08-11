#!/usr/bin/env python3
# Copyright (C) 2026  Braiins Forge s.r.o.
#
# This program is free software: you can redistribute it and/or modify
# it under the terms of the GNU General Public License as published by
# the Free Software Foundation, either version 3 of the License, or
# (at your option) any later version.
#
# This program is distributed in the hope that it will be useful,
# but WITHOUT ANY WARRANTY; without even the implied warranty of
# MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
# GNU General Public License for more details.
#
# You should have received a copy of the GNU General Public License
# along with this program.  If not, see <https://www.gnu.org/licenses/>.
#
# Braiins Systems s.r.o. and Braiins Forge s.r.o. each reserve the right
# to grant any party a license to this program, or any part thereof,
# under any terms, and such a grant shall be considered distinct from
# the grant above.

"""Push media to a UPnP/DLNA renderer and control playback.

Can optionally spawn a built-in Python UPnP renderer that uses the system
audio player (afplay on macOS, mpv elsewhere).

Dependencies: Python 3.10+ standard library only.

Usage:
    # Spawn a local renderer (no media push)
    ./tools/dlna-push.py

    # Spawn a local renderer and push media to it
    ./tools/dlna-push.py https://stream.radioparadise.com/aac-320

    # Spawn on a specific port with a custom name
    ./tools/dlna-push.py --port 49494 --name "Deck Test" https://example.com/song.mp3

    # Push to an existing remote renderer
    ./tools/dlna-push.py 192.168.1.151:49494 https://stream.radioparadise.com/aac-320

    # Query current state
    ./tools/dlna-push.py 192.168.1.151:49494 --status
"""

import argparse
import shutil
import socket
import struct
import subprocess
import sys
import textwrap
import threading
import time
import xml.etree.ElementTree as ET
from html import escape as html_escape
from http.client import HTTPConnection
from http.server import HTTPServer, BaseHTTPRequestHandler
from typing import NoReturn
from urllib.parse import urlparse

# ── SOAP constants ───────────────────────────────────────────────

AV_TRANSPORT = 'urn:schemas-upnp-org:service:AVTransport:1'
RENDERING_CONTROL = 'urn:schemas-upnp-org:service:RenderingControl:1'

SOAP_ENVELOPE = textwrap.dedent("""\
    <?xml version="1.0" encoding="utf-8"?>
    <s:Envelope xmlns:s="http://schemas.xmlsoap.org/soap/envelope/"
                s:encodingStyle="http://schemas.xmlsoap.org/soap/encoding/">
      <s:Body>{body}</s:Body>
    </s:Envelope>""")

# gmrender-resurrect default paths
DEFAULT_AV_TRANSPORT_PATH = '/upnp/control/rendertransport1'
DEFAULT_RENDERING_CONTROL_PATH = '/upnp/control/rendercontrol1'

# ── DIDL-Lite metadata ───────────────────────────────────────────

DIDL_TEMPLATE = (
    '<DIDL-Lite xmlns="urn:schemas-upnp-org:metadata-1-0/DIDL-Lite/"'
    ' xmlns:dc="http://purl.org/dc/elements/1.1/"'
    ' xmlns:upnp="urn:schemas-upnp-org:metadata-1-0/upnp/">'
    '<item id="0" parentID="-1" restricted="1">'
    '<dc:title>{title}</dc:title>'
    '<dc:creator>{artist}</dc:creator>'
    '<upnp:artist>{artist}</upnp:artist>'
    '<upnp:album>{album}</upnp:album>'
    '<upnp:albumArtURI>{art_url}</upnp:albumArtURI>'
    '<upnp:class>object.item.audioItem.musicTrack</upnp:class>'
    '</item>'
    '</DIDL-Lite>'
)


def build_didl(
    *,
    title: str,
    artist: str,
    album: str,
    art_url: str,
) -> str:
    """Build DIDL-Lite XML metadata for a track."""
    return DIDL_TEMPLATE.format(
        title=html_escape(title),
        artist=html_escape(artist),
        album=html_escape(album),
        art_url=html_escape(art_url),
    )


# ── Example tracks ───────────────────────────────────────────────

EXAMPLE_TRACKS = [
    {
        'url': 'https://archive.org/download/NASASoundofSaturn/123163main_casskr1112203.mp3',
        'title': 'Sound of Saturn',
        'artist': 'NASA / Cassini',
        'album': 'NASA Sounds',
        'art_url': 'https://archive.org/services/img/NASASoundofSaturn',
    },
    {
        'url': (
            'https://archive.org/download/Kevin-MacLeod_Royalty-Free_2017_FullAlbum'
            '/Royalty%20Free/Kevin%20MacLeod%20-%2000%20-%20Achaidh%20Cheide.mp3'
        ),
        'title': 'Achaidh Cheide',
        'artist': 'Kevin MacLeod',
        'album': 'Royalty Free',
        'art_url': 'https://archive.org/services/img/Kevin-MacLeod_Royalty-Free_2017_FullAlbum',
    },
    {
        'url': (
            'https://archive.org/download/classical-music-mix-by-various-artists'
            '/01%20-%20Mozart%20-%20Overture%20The%20Magic%20Flute.mp3'
        ),
        'title': 'Overture — The Magic Flute',
        'artist': 'Mozart',
        'album': 'Classical Music Mix',
        'art_url': 'https://archive.org/services/img/classical-music-mix-by-various-artists',
    },
    {
        'url': 'https://ice1.somafm.com/groovesalad-128-mp3',
        'title': 'Groove Salad',
        'artist': 'SomaFM',
        'album': 'Internet Radio (continuous)',
        'art_url': 'https://somafm.com/img3/groovesalad-400.jpg',
    },
]


# ── Network helpers ───────────────────────────────────────────────


def get_local_ip() -> str:
    """Get the IP of the default route interface (what gmediarender binds to)."""
    with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as s:
        try:
            # Doesn't actually send anything — just triggers route lookup
            s.connect(('10.255.255.255', 1))
            return s.getsockname()[0]
        except OSError:
            return '127.0.0.1'


# ── SOAP client ──────────────────────────────────────────────────


class SoapClient:
    """Minimal UPnP SOAP client using only http.client."""

    def __init__(self, host: str, port: int) -> None:
        self.host = host
        self.port = port

    def _request(
        self,
        path: str,
        service: str,
        action: str,
        body_xml: str,
    ) -> str:
        envelope = SOAP_ENVELOPE.format(body=body_xml)
        conn = HTTPConnection(self.host, self.port, timeout=5)
        try:
            conn.request(
                'POST',
                path,
                body=envelope.encode(),
                headers={
                    'Content-Type': 'text/xml; charset="utf-8"',
                    'SOAPAction': f'"{service}#{action}"',
                },
            )
            resp = conn.getresponse()
            data = resp.read().decode(errors='replace')
            if resp.status >= 400:
                print(
                    f'SOAP {action} failed: HTTP {resp.status}\n{data}', file=sys.stderr
                )
                sys.exit(1)
            return data
        finally:
            conn.close()

    def av_transport(self, action: str, extra: str = '') -> str:
        body = (
            f'<u:{action} xmlns:u="{AV_TRANSPORT}">'
            f'<InstanceID>0</InstanceID>'
            f'{extra}'
            f'</u:{action}>'
        )
        return self._request(DEFAULT_AV_TRANSPORT_PATH, AV_TRANSPORT, action, body)

    def rendering_control(self, action: str, extra: str = '') -> str:
        body = (
            f'<u:{action} xmlns:u="{RENDERING_CONTROL}">'
            f'<InstanceID>0</InstanceID>'
            f'<Channel>Master</Channel>'
            f'{extra}'
            f'</u:{action}>'
        )
        return self._request(
            DEFAULT_RENDERING_CONTROL_PATH, RENDERING_CONTROL, action, body
        )

    def set_uri(self, uri: str, metadata: str = '') -> str:
        escaped_meta = html_escape(metadata) if metadata else ''
        return self.av_transport(
            'SetAVTransportURI',
            f'<CurrentURI>{html_escape(uri)}</CurrentURI>'
            f'<CurrentURIMetaData>{escaped_meta}</CurrentURIMetaData>',
        )

    def play(self) -> str:
        return self.av_transport('Play', '<Speed>1</Speed>')

    def pause(self) -> str:
        return self.av_transport('Pause')

    def stop(self) -> str:
        return self.av_transport('Stop')

    def get_transport_info(self) -> str:
        return self.av_transport('GetTransportInfo')

    def get_position_info(self) -> str:
        return self.av_transport('GetPositionInfo')

    def get_volume(self) -> str:
        return self.rendering_control('GetVolume')


# ── XML helpers ──────────────────────────────────────────────────


def xml_find_text(xml_str: str, local_name: str) -> str | None:
    """Find first element by local name (namespace-agnostic)."""
    try:
        root = ET.fromstring(xml_str)
    except ET.ParseError:
        return None
    for elem in root.iter():
        tag = elem.tag
        if '}' in tag:
            tag = tag.split('}', 1)[1]
        if tag == local_name:
            return elem.text
    return None


# ── Status display ───────────────────────────────────────────────


def print_status(client: SoapClient) -> None:
    transport = client.get_transport_info()
    position = client.get_position_info()
    volume = client.get_volume()

    state = xml_find_text(transport, 'CurrentTransportState') or '?'
    pos = xml_find_text(position, 'RelTime') or '0:00:00'
    dur = xml_find_text(position, 'TrackDuration') or '0:00:00'
    title = xml_find_text(position, 'title') or '-'
    artist = xml_find_text(position, 'artist') or '-'
    uri = xml_find_text(position, 'TrackURI') or '-'
    vol = xml_find_text(volume, 'CurrentVolume') or '?'

    print(f'State:    {state}')
    print(f'Track:    {title} — {artist}')
    print(f'Position: {pos} / {dur}')
    print(f'Volume:   {vol}%')
    print(f'URI:      {uri}')


# ── Built-in Python UPnP renderer (no external deps) ────────────


class RendererState:
    """Shared mutable state for the built-in renderer."""

    def __init__(self, name: str) -> None:
        self.name = name
        self.transport_state = 'STOPPED'
        self.current_uri = ''
        self.metadata = ''
        self.volume = 50
        self.mute = False
        self.play_start: float | None = None
        self.elapsed_at_pause: float = 0.0
        self.player_proc: subprocess.Popen[bytes] | None = None
        self._lock = threading.Lock()

    def set_uri(self, uri: str, metadata: str) -> None:
        with self._lock:
            self._stop_player()
            self.current_uri = uri
            self.metadata = metadata
            self.transport_state = 'STOPPED'
            self.play_start = None
            self.elapsed_at_pause = 0.0

    def play(self) -> None:
        with self._lock:
            if self.transport_state == 'PAUSED_PLAYBACK':
                self.play_start = time.monotonic() - self.elapsed_at_pause
            else:
                self.play_start = time.monotonic()
                self.elapsed_at_pause = 0.0
            self.transport_state = 'PLAYING'
            self._start_player()

    def pause(self) -> None:
        with self._lock:
            if self.play_start is not None:
                self.elapsed_at_pause = time.monotonic() - self.play_start
            self.transport_state = 'PAUSED_PLAYBACK'
            self._stop_player()

    def stop(self) -> None:
        with self._lock:
            self._stop_player()
            self.transport_state = 'STOPPED'
            self.play_start = None
            self.elapsed_at_pause = 0.0

    def get_elapsed(self) -> float:
        with self._lock:
            if self.transport_state == 'PLAYING' and self.play_start is not None:
                return time.monotonic() - self.play_start
            return self.elapsed_at_pause

    def _start_player(self) -> None:
        self._stop_player()
        if not self.current_uri:
            return
        uri = self.current_uri
        # Try players in order of preference for streaming URLs
        if shutil.which('ffplay'):
            self.player_proc = subprocess.Popen(
                ['ffplay', '-nodisp', '-autoexit', '-loglevel', 'quiet', uri],
                stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL,
            )
        elif shutil.which('mpv'):
            self.player_proc = subprocess.Popen(
                ['mpv', '--no-video', '--really-quiet', uri],
                stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL,
            )
        elif shutil.which('afplay'):
            # afplay only works with local files
            self.player_proc = subprocess.Popen(
                ['afplay', uri],
                stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL,
            )
        # else: no audio, just track state

    def _stop_player(self) -> None:
        if self.player_proc and self.player_proc.poll() is None:
            self.player_proc.terminate()
            self.player_proc = None


def _secs_to_hms(secs: float) -> str:
    s = int(secs)
    return f'{s // 3600}:{(s % 3600) // 60:02d}:{s % 60:02d}'


def _make_soap_response(action: str, service: str, body: str) -> bytes:
    xml = (
        '<?xml version="1.0" encoding="utf-8"?>'
        '<s:Envelope xmlns:s="http://schemas.xmlsoap.org/soap/envelope/"'
        ' s:encodingStyle="http://schemas.xmlsoap.org/soap/encoding/">'
        '<s:Body>'
        f'<u:{action}Response xmlns:u="{service}">'
        f'{body}'
        f'</u:{action}Response>'
        '</s:Body>'
        '</s:Envelope>'
    )
    return xml.encode()


class _UPnPHandler(BaseHTTPRequestHandler):
    """HTTP handler for UPnP SOAP requests."""

    renderer: RendererState  # set on class before serving

    def log_message(self, format: str, *args: object) -> None:  # noqa: A002
        print(f'  [renderer] {format % args}')

    def do_POST(self) -> None:
        length = int(self.headers.get('Content-Length', 0))
        body = self.rfile.read(length).decode(errors='replace')
        soap_action = self.headers.get('SOAPAction', '').strip('"')
        action = soap_action.rsplit('#', 1)[-1] if '#' in soap_action else ''
        rs = self.renderer
        print(f'  [renderer] ← {action} from {self.client_address[0]}')

        if self.path == DEFAULT_AV_TRANSPORT_PATH:
            resp = self._handle_av_transport(action, body, rs)
        elif self.path == DEFAULT_RENDERING_CONTROL_PATH:
            resp = self._handle_rendering_control(action, body, rs)
        else:
            self.send_error(404)
            return

        self.send_response(200)
        self.send_header('Content-Type', 'text/xml; charset="utf-8"')
        self.send_header('Content-Length', str(len(resp)))
        self.end_headers()
        self.wfile.write(resp)

    def _handle_av_transport(self, action: str, body: str, rs: RendererState) -> bytes:
        if action == 'SetAVTransportURI':
            uri = xml_find_text(body, 'CurrentURI') or ''
            meta = xml_find_text(body, 'CurrentURIMetaData') or ''
            rs.set_uri(uri, meta)
            print(f'  [renderer] SetAVTransportURI: {uri[:80]}')
            return _make_soap_response(action, AV_TRANSPORT, '')
        if action == 'Play':
            rs.play()
            print('  [renderer] Play')
            return _make_soap_response(action, AV_TRANSPORT, '')
        if action == 'Pause':
            rs.pause()
            print('  [renderer] Pause')
            return _make_soap_response(action, AV_TRANSPORT, '')
        if action == 'Stop':
            rs.stop()
            print('  [renderer] Stop')
            return _make_soap_response(action, AV_TRANSPORT, '')
        if action == 'GetTransportInfo':
            return _make_soap_response(
                action,
                AV_TRANSPORT,
                f'<CurrentTransportState>{rs.transport_state}</CurrentTransportState>'
                '<CurrentTransportStatus>OK</CurrentTransportStatus>'
                '<CurrentSpeed>1</CurrentSpeed>',
            )
        if action == 'GetPositionInfo':
            elapsed = _secs_to_hms(rs.get_elapsed())
            # Extract elapsed position from stored metadata
            return _make_soap_response(
                action,
                AV_TRANSPORT,
                f'<Track>1</Track>'
                f'<TrackDuration>0:00:00</TrackDuration>'
                f'<TrackMetaData>{html_escape(rs.metadata)}</TrackMetaData>'
                f'<TrackURI>{html_escape(rs.current_uri)}</TrackURI>'
                f'<RelTime>{elapsed}</RelTime>'
                f'<AbsTime>{elapsed}</AbsTime>',
            )
        return _make_soap_response(action, AV_TRANSPORT, '')

    def _handle_rendering_control(
        self, action: str, body: str, rs: RendererState
    ) -> bytes:
        if action == 'GetVolume':
            return _make_soap_response(
                action,
                RENDERING_CONTROL,
                f'<CurrentVolume>{rs.volume}</CurrentVolume>',
            )
        if action == 'SetVolume':
            vol = xml_find_text(body, 'DesiredVolume')
            if vol is not None:
                rs.volume = int(vol)
                print(f'  [renderer] SetVolume: {rs.volume}')
            return _make_soap_response(action, RENDERING_CONTROL, '')
        if action == 'GetMute':
            return _make_soap_response(
                action,
                RENDERING_CONTROL,
                f'<CurrentMute>{"1" if rs.mute else "0"}</CurrentMute>',
            )
        if action == 'SetMute':
            m = xml_find_text(body, 'DesiredMute')
            if m is not None:
                rs.mute = m == '1'
            return _make_soap_response(action, RENDERING_CONTROL, '')
        return _make_soap_response(action, RENDERING_CONTROL, '')

    def do_GET(self) -> None:
        # Minimal device description for UPnP probes
        if self.path in ('/', '/description.xml'):
            desc = (
                '<?xml version="1.0"?>'
                '<root xmlns="urn:schemas-upnp-org:device-1-0">'
                '<device>'
                f'<friendlyName>{self.renderer.name}</friendlyName>'
                '<deviceType>urn:schemas-upnp-org:device:MediaRenderer:1</deviceType>'
                '</device>'
                '</root>'
            ).encode()
            self.send_response(200)
            self.send_header('Content-Type', 'text/xml')
            self.send_header('Content-Length', str(len(desc)))
            self.end_headers()
            self.wfile.write(desc)
        else:
            self.send_error(404)


def _ssdp_listener(local_ip: str, port: int, stop_event: threading.Event) -> None:
    """Listen for SSDP M-SEARCH requests and respond with our renderer location."""
    SSDP_ADDR = '239.255.255.250'
    SSDP_PORT = 1900
    usn = f'uuid:python-renderer-{port}::urn:schemas-upnp-org:device:MediaRenderer:1'
    location = f'http://{local_ip}:{port}/description.xml'

    sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM, socket.IPPROTO_UDP)
    sock.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    if hasattr(socket, 'SO_REUSEPORT'):
        sock.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEPORT, 1)
    sock.bind(('', SSDP_PORT))
    mreq = struct.pack('4s4s', socket.inet_aton(SSDP_ADDR), socket.inet_aton(local_ip))
    sock.setsockopt(socket.IPPROTO_IP, socket.IP_ADD_MEMBERSHIP, mreq)
    sock.settimeout(1.0)

    response = (
        'HTTP/1.1 200 OK\r\n'
        'CACHE-CONTROL: max-age=1800\r\n'
        f'LOCATION: {location}\r\n'
        'ST: urn:schemas-upnp-org:device:MediaRenderer:1\r\n'
        f'USN: {usn}\r\n'
        'SERVER: Python UPnP Renderer\r\n'
        '\r\n'
    ).encode()

    try:
        while not stop_event.is_set():
            try:
                data, addr = sock.recvfrom(2048)
                msg = data.decode(errors='replace')
                if 'M-SEARCH' in msg and ('ssdp:all' in msg or 'MediaRenderer' in msg):
                    sock.sendto(response, addr)
            except TimeoutError:
                continue
            except OSError:
                if stop_event.is_set():
                    break
    finally:
        sock.close()


def _ssdp_announce(
    local_ip: str, port: int, name: str, stop_event: threading.Event
) -> None:
    """Broadcast SSDP alive notifications so UPnP control points can find us."""
    SSDP_ADDR = '239.255.255.250'
    SSDP_PORT = 1900
    usn = f'uuid:python-renderer-{port}::urn:schemas-upnp-org:device:MediaRenderer:1'
    location = f'http://{local_ip}:{port}/description.xml'
    notify = (
        'NOTIFY * HTTP/1.1\r\n'
        f'HOST: {SSDP_ADDR}:{SSDP_PORT}\r\n'
        'CACHE-CONTROL: max-age=1800\r\n'
        f'LOCATION: {location}\r\n'
        'NT: urn:schemas-upnp-org:device:MediaRenderer:1\r\n'
        'NTS: ssdp:alive\r\n'
        f'SERVER: Python UPnP Renderer\r\n'
        f'USN: {usn}\r\n'
        '\r\n'
    ).encode()

    sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM, socket.IPPROTO_UDP)
    sock.setsockopt(socket.IPPROTO_IP, socket.IP_MULTICAST_TTL, 4)
    try:
        # Send immediately, then every 30s
        while True:
            try:
                sock.sendto(notify, (SSDP_ADDR, SSDP_PORT))
            except OSError:
                pass
            if stop_event.wait(30):
                break
    finally:
        sock.close()


def spawn_builtin_renderer(
    port: int, name: str
) -> tuple[threading.Event, str, int, subprocess.Popen[bytes]]:
    """Start a pure-Python UPnP renderer on the given port.

    Returns (stop_event, host, port).
    """
    local_ip = get_local_ip()

    state = RendererState(name)
    _UPnPHandler.renderer = state

    server = HTTPServer(('0.0.0.0', port), _UPnPHandler)
    server_thread = threading.Thread(target=server.serve_forever, daemon=True)
    server_thread.start()

    stop_event = threading.Event()
    ssdp_thread = threading.Thread(
        target=_ssdp_announce, args=(local_ip, port, name, stop_event), daemon=True
    )
    ssdp_thread.start()
    listener_thread = threading.Thread(
        target=_ssdp_listener, args=(local_ip, port, stop_event), daemon=True
    )
    listener_thread.start()

    # Register via mDNS so WASM widgets can discover us
    mdns_proc = subprocess.Popen(
        ['dns-sd', '-R', name, '_upnp._tcp', 'local.', str(port)],
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    )

    print(f'Built-in UPnP renderer "{name}" on {local_ip}:{port}')
    return stop_event, local_ip, port, mdns_proc


# ── Main ─────────────────────────────────────────────────────────


def main() -> NoReturn | None:
    parser = argparse.ArgumentParser(
        description='Push media to a UPnP/DLNA renderer.',
        formatter_class=argparse.RawDescriptionHelpFormatter,
        epilog=textwrap.dedent("""\
            examples:
              %(prog)s                              # spawn renderer, no media
              %(prog)s --example 3                  # spawn + play radio stream
              %(prog)s --port 49494 --name "Deck Test"
              %(prog)s 192.168.1.151:49494 --status # query remote renderer

            example tracks (use with --example 0/1/2):
              0: NASA — Sound of Saturn (short, 73s)
              1: Kevin MacLeod — Achaidh Cheide (royalty free)
              2: Mozart — Overture, The Magic Flute (classical, public domain)
        """),
    )
    parser.add_argument(
        'target',
        nargs='?',
        help='host:port of the renderer (not needed with --spawn)',
    )
    parser.add_argument(
        'media_url',
        nargs='?',
        help='media URI to push to the renderer',
    )
    parser.add_argument(
        '--port',
        type=int,
        default=49_494,
        help='port for spawned renderer (default: 49494)',
    )
    parser.add_argument(
        '--name',
        default='Deck Test',
        help='friendly name for spawned renderer (default: "Deck Test")',
    )
    parser.add_argument(
        '--status',
        action='store_true',
        help='query and display current renderer state',
    )
    parser.add_argument(
        '--example',
        type=int,
        choices=range(len(EXAMPLE_TRACKS)),
        metavar='N',
        help=f'use built-in example track 0–{len(EXAMPLE_TRACKS) - 1} (implies media push)',
    )

    args = parser.parse_args()

    host = '127.0.0.1'
    port = args.port

    # Resolve example track
    track_meta: dict[str, str] | None = None
    if args.example is not None:
        track_meta = EXAMPLE_TRACKS[args.example]
        if not args.media_url:
            args.media_url = track_meta['url']

    # If first positional looks like a URL, treat it as media_url (not a host:port target)
    if args.target and '://' in args.target:
        if not args.media_url:
            args.media_url = args.target
        args.target = None

    stop_event: threading.Event | None = None
    mdns_proc: subprocess.Popen[bytes] | None = None

    if args.target:
        # Push to an existing remote renderer
        parsed = urlparse(f'http://{args.target}')
        host = parsed.hostname or args.target.split(':')[0]
        port = parsed.port or 49_494
    else:
        # Spawn a local built-in renderer
        stop_event, host, port, mdns_proc = spawn_builtin_renderer(args.port, args.name)

    client = SoapClient(host, port)

    if args.status:
        print_status(client)
        if stop_event:
            stop_event.set()
        return

    if args.media_url:
        # Build DIDL-Lite metadata if we have track info
        metadata = ''
        if track_meta:
            metadata = build_didl(
                title=track_meta['title'],
                artist=track_meta['artist'],
                album=track_meta['album'],
                art_url=track_meta['art_url'],
            )
            print(f'Track:    {track_meta["title"]} — {track_meta["artist"]}')
            print(f'Album:    {track_meta["album"]}')

        print(f'URI:      {args.media_url}')
        client.set_uri(args.media_url, metadata)

        print('Playing...')
        client.play()
        print('Done. Renderer should be playing.')

    if stop_event:
        _wait_for_builtin(stop_event, mdns_proc)


def _wait_for_builtin(
    stop_event: threading.Event, mdns_proc: subprocess.Popen[bytes] | None
) -> None:
    """Block until Ctrl+C, then signal the built-in renderer to stop."""
    print('\nBuilt-in renderer running. Press Ctrl+C to stop.')
    try:
        while not stop_event.wait(1.0):
            pass
    except KeyboardInterrupt:
        print('\nStopping built-in renderer...')
        stop_event.set()
        if mdns_proc:
            mdns_proc.terminate()


if __name__ == '__main__':
    main()
