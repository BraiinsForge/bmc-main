#!/usr/bin/env python3
# Copyright (C) 2026  Braiins Systems s.r.o.
#
# Synthesize the fleet-management capture fixtures.
#
# Unlike most widgets, fleet-management has no single API to stub: it builds
# its view from mDNS discovery plus per-device telemetry fetches. `just
# wasm::record` records that from real miners on the network; we have no
# multi-model fleet to record, so the fixtures are hand-authored here from the
# adapter contracts in `src/families/{bos,ubos,bitaxe}.rs`.
#
# The fleet is fourteen reachable devices across four model groups, sized to
# exercise the breakdown table's column spacing (3-digit power, large group
# hashrate, two-digit counts):
#   -  2x BOS    "Braiins Mini Miner BMM 101"
#   -  1x uBOS   "BMM Adapter W5500"
#   - 10x Bitaxe "NerdQAxe++"  (4.4 TH/s, 70 W each -> 44 TH/s, 700 W)
#   -  1x Bitaxe with no resolvable model -> the synthetic "Unknown" group
#
# The same fleet drives every declared size: `full`/`large` render the table,
# while `medium`/`small` fall back to the summary-only layout (viewports below
# the table's 638x480 box). `large`/`medium`/`small` share one static layout
# timeline (the capture binary sets the viewport); `full` additionally replays
# a device-lifecycle timeline (mid-flight removal, non-response) with several
# captures — see the lifecycle section below.
#
# Browse ids are deterministic from `init()` registration order:
# 1 = BOS, 2 = uBOS, 3 = Bitaxe. A family snapshots its device list when its
# first device is discovered, so a family's later devices are only polled on
# the next 30s pass; the capture is therefore taken after the second pass
# (at_ms 31000) so every device has telemetry. The fetch interceptor replays
# the last response for repeat polls, so one stub per URL suffices for the
# layout sizes; `full` instead supplies a NerdQAxe++ stub per pass so survivor
# telemetry changes between captures.
#
# Regenerate with:  python3 capture/fixtures.py   (run from the widget dir)
# Then refresh baselines on a GPU host:  just wasm::update-baselines fleet-management

import gzip
import json
import pathlib

HERE = pathlib.Path(__file__).resolve().parent
OUT_DIR = HERE / 'fixtures'

CAPTURE_AT_MS = 31_000

BROWSE_BOS = 1
BROWSE_UBOS = 2
BROWSE_BITAXE = 3

discovery = []
fetches = []


def mdns(browse_id, name, host, port, txt=None):
    payload = {
        'service_type': name.split('.', 1)[1] if '.' in name else name,
        'name': name,
        'host': host,
        'port': port,
        'txt': txt or {},
    }
    discovery.append(
        {'type': 'mdns_found', 'browse_id': browse_id, 'data': json.dumps(payload)}
    )


def fetch(method, url, body):
    fetches.append(
        {
            'type': 'fetch',
            'method': method,
            'url': url,
            'status': 200,
            'body': {'json': body},
        }
    )


def bos(host, name, token, watt, degree_c):
    base = f'http://{host}:80/api/v1'
    mdns(BROWSE_BOS, name, host, 80)
    fetch('POST', f'{base}/auth/login', {'token': token})
    fetch(
        'GET',
        f'{base}/miner/stats',
        {
            'miner_stats': {
                'real_hashrate': {'last_1m': {'gigahash_per_second': 1000.0}}
            },
            'power_stats': {'approximated_consumption': {'watt': watt}},
        },
    )
    fetch(
        'GET',
        f'{base}/miner/hw/hashboards',
        {
            'hashboards': [
                {
                    'highest_chip_temp': {'temperature': {'degree_c': degree_c}},
                    'chip_type': 'BM1370',
                    'chips_count': 76,
                },
            ],
        },
    )
    fetch(
        'GET',
        f'{base}/miner/details',
        {
            'bosminer_uptime_s': 187_020,
            'platform': 8,
            'miner_identity': {'miner_model': 'Braiins Mini Miner BMM 101'},
        },
    )


def ubos(host, name, hashrate_hs, power_mw, temp):
    mdns(BROWSE_UBOS, name, host, 8080)
    fetch(
        'GET',
        f'http://{host}:8080/api/info',
        {
            'hashrate': hashrate_hs,
            'power_out_mw': power_mw,
            'temperature': temp,
            'uptime': 90_000,
            'name': 'BMM Adapter W5500',
        },
    )


def bitaxe(host, name, hashrate_ghs, power, temp, model_fields):
    mdns(BROWSE_BITAXE, name, host, 80)
    body = {
        'hashRate': hashrate_ghs,
        'power': power,
        'temp': temp,
        'uptimeSeconds': 50_000,
    }
    body.update(model_fields)
    fetch('GET', f'http://{host}:80/api/system/info', body)


bos('10.0.0.10', 'bmm-101-a._http._tcp.local.', 'tok-a', 33.0, 60.0)
bos('10.0.0.11', 'bmm-101-b._http._tcp.local.', 'tok-b', 33.0, 64.0)

ubos('10.0.0.20', 'w5500._ubos._tcp.local.', 1.0e12, 30_000.0, 58.0)

# Ten identical NerdQAxe++ units: 4.4 TH/s and 70 W each.
for i in range(10):
    bitaxe(
        f'10.0.0.{30 + i}',
        f'nerdqaxe-{i}._http._tcp.local.',
        4400.0,
        70.0,
        55.0,
        {'deviceModel': 'NerdQAxe++', 'ASICModel': 'BM1370', 'asicCount': 4},
    )

# A Bitaxe with no model fields -> falls into the "Unknown" group.
bitaxe('10.0.0.40', 'axe-unknown._http._tcp.local.', 1500.0, 22.0, 50.0, {})


def header():
    return {
        'time': '2026-06-05T12:00:00+00:00',
        'initial_params': {
            'fleet_name': 'My Fleet',
            'bos_password': 'root',
            'ubos_username': 'root',
            'ubos_password': 'root',
            'model_whitelist': '[]',
            'model_blacklist': '[]',
            'bos_enabled': True,
            'ubos_enabled': True,
            'axeos_enabled': True,
            'bos_hosts': '[]',
            'ubos_hosts': '[]',
            'axeos_hosts': '[]',
        },
    }


def seed_events(stubs):
    # The loader requires at_ms to be monotonically non-decreasing. Fetch stubs
    # only seed the interceptor (their at_ms is never matched), so they and the
    # discovery events all sit at at_ms 0 ahead of the timed capture/removal.
    for event in stubs:
        yield {'at_ms': 0, **event}
    for event in discovery:
        yield {'at_ms': 0, **event}


def layout_lines():
    yield header()
    yield from seed_events(fetches)
    yield {'at_ms': CAPTURE_AT_MS, 'type': 'capture'}


# ── Lifecycle timeline (the `full` fixture) ──────────────────────────
#
# `full` replays device-lifecycle edge cases the layout sizes do not. The only
# capture assertion is a baseline pixel diff, so a stall is visible only if
# surviving devices' telemetry CHANGES across passes: a healthy fleet renders
# the new values, a stalled one renders the frozen old ones.
#
# Bitaxe is the no-auth family whose opening kick is a telemetry fetch — the
# path stranded by a mid-pass removal. The NerdQAxe++ units report a value set
# per pass (pass 2 = current values, so frame_0000 still matches the layout
# baseline). `nerdqaxe-0` heads the family cursor and is removed mid-flight;
# `nerdqaxe-1` goes non-responsive on the last pass.

NERDQAXE_HOSTS = [f'10.0.0.{30 + i}' for i in range(10)]
NERDQAXE_MODEL = {'deviceModel': 'NerdQAxe++', 'ASICModel': 'BM1370', 'asicCount': 4}


def bitaxe_stub(host, hashrate_ghs, power, temp, status=200):
    body = {
        'hashRate': hashrate_ghs,
        'power': power,
        'temp': temp,
        'uptimeSeconds': 50_000,
    }
    body.update(NERDQAXE_MODEL)
    return {
        'type': 'fetch',
        'method': 'GET',
        'url': f'http://{host}:80/api/system/info',
        'status': status,
        'body': {'json': body},
    }


def nerdqaxe_sequence(host):
    # One stub per poll; the host reuses the last for further polls. Polls map
    # to passes 2/3/4 = frame_0000/_0001/_0002. Pass-2 values equal the layout
    # fixture's so frame_0000 stays byte-identical to the layout baseline.
    pass2 = bitaxe_stub(host, 4400.0, 70.0, 55.0)
    pass3 = bitaxe_stub(host, 5000.0, 75.0, 56.0)
    if host == '10.0.0.30':
        # Removed before pass 3 fires; only ever polled with the pass-2 value.
        return [pass2]
    if host == '10.0.0.31':
        # Non-responsive (HTTP 500) on pass 4: the rest of the fleet must keep
        # updating around it.
        return [pass2, pass3, bitaxe_stub(host, 0.0, 0.0, 0.0, status=500)]
    return [pass2, pass3, bitaxe_stub(host, 5500.0, 80.0, 57.0)]


def is_nerdqaxe(stub):
    return any(stub['url'].startswith(f'http://{h}:80/') for h in NERDQAXE_HOSTS)


# Reuse the BOS/uBOS/unknown-Bitaxe stubs verbatim; swap the ten NerdQAxe++
# single stubs for their per-pass sequences.
lifecycle_fetches = [s for s in fetches if not is_nerdqaxe(s)]
for nerdqaxe_host in NERDQAXE_HOSTS:
    lifecycle_fetches.extend(nerdqaxe_sequence(nerdqaxe_host))


def lifecycle_lines():
    yield header()
    yield from seed_events(lifecycle_fetches)
    yield {'at_ms': CAPTURE_AT_MS, 'type': 'capture'}  # frame_0000: initial fleet
    yield {
        'at_ms': 40_000,
        'type': 'mdns_removed',
        'browse_id': BROWSE_BITAXE,
        'data': 'nerdqaxe-0._http._tcp.local.',
    }
    yield {'at_ms': 75_000, 'type': 'capture'}  # frame_0001: pass after the removal
    yield {'at_ms': 105_000, 'type': 'capture'}  # frame_0002: pass after non-response


def main():
    OUT_DIR.mkdir(parents=True, exist_ok=True)
    # `full` carries the lifecycle timeline; the smaller sizes keep the static
    # layout timeline. mtime=0 keeps the gzip byte-stable across regenerations.
    bodies = {
        'full': lifecycle_lines,
        'large': layout_lines,
        'medium': layout_lines,
        'small': layout_lines,
    }
    for size, build in bodies.items():
        body = ''.join(json.dumps(line) + '\n' for line in build()).encode('utf-8')
        with gzip.GzipFile(OUT_DIR / f'{size}.jsonl.gz', 'wb', mtime=0) as fh:
            fh.write(body)
    print('wrote ' + ', '.join(f'{size}.jsonl.gz' for size in bodies) + f' in {OUT_DIR}')


if __name__ == '__main__':
    main()
