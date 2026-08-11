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

"""Drive a real Deck to prove a credential reaches the wire only where it is allowed.

What allows it is the account's own pin where it has one, the credential type's
otherwise. An account pin replaces the type's rather than narrowing it.

The runtime tests decide the same rules in isolation. This one answers what they
cannot: whether the secret survives the whole chain — secret store, coordinator,
wayland, wasm host — and still stops at the right hosts.

The destination is a server this run owns, so the evidence is what the device
actually sent us rather than what it says it sent.

The carrier is `params-demo`, which fetches whatever its `string_uri` param holds.
Its manifest declares one slot per credential type the firmware knows,
plus a second pinned slot.

That matters: a slot has to be declared for resolution to authorise it,
so a widget declaring none cannot carry a credential at all.

Bindings are written straight into the config, past the gRPC validation that
would refuse a slot the manifest never declared. That is not a shortcut around
the test: a hand-edited config is a state the device has to survive, and one case
here is exactly that.
"""

import json
import re
import tempfile
import uuid
from collections.abc import Mapping
from dataclasses import dataclass
from datetime import UTC, datetime
from pathlib import Path
from typing import Literal

from bmc_tui import catalog, console
from bmc_tui.device import Device, RemotePath
from bmc_tui.server import Request, ServerHandle, View, ViewConfig, server
from bmc_tui.stage import Abort, entrypoint, require

# Two logs, because two components decide. The host reports that a slot had no
# secret; only the compositor knows whether it was never bound or was withheld.
HOST_LOG = RemotePath("/var/log/bmc/run-bmc-wasm-host-sdk-v0.log")
COMPOSITOR_LOG = RemotePath("/var/log/bmc/bmc.log")

CONFIG = RemotePath("/etc/bmc/config.json")
SECRETS = RemotePath("/etc/bmc/secrets.json")
CONFIG_BACKUP = RemotePath("/etc/bmc/config.json.credential-egress-bak")
SECRETS_BACKUP = RemotePath("/etc/bmc/secrets.json.credential-egress-bak")

WIDGET_TYPE_ID = "550e8400-e29b-41d4-a716-446655440300"

# Repo-relative: the pushed config carries this widget's own param defaults, and
# reading them beats restating fourteen values that the manifest already holds.
MANIFEST = Path("widgets-wasm-examples/params-demo/manifest.json")

# 1x1 PNG. The widget has to receive something decodable so a decoder error
# cannot be mistaken for the fetch never arriving.
PIXEL_PNG = bytes.fromhex(
    "89504e470d0a1a0a0000000d49484452000000010000000108060000001f15c4"
    "890000000d4944415478da63f8cfc0500f0002870180a1a4fa6b0000000049454e44ae426082"
)

Expect = Literal[
    "deliver", "deliver-no-follow", "refuse-pin", "refuse-no-secret", "refuse-withheld"
]

# The expectations judged from what arrived rather than from a refusal line.
# Only the rest need a slot of their own, since a refusal names a slot, not a URL.
_ARRIVAL_JUDGED = ("deliver", "deliver-no-follow")

# Where the `redirect` case's 302 points. A request here means the client
# followed a redirect on a secret-bearing request, which it must not.
LANDED_PATH = "/landed.png"

# `bmc_wasm_sdk::net::DEFAULT_FETCH_TIMEOUT`, which `params-demo` takes
# by passing no timeout of its own. A fetch is resolved or failed by then.
_SDK_FETCH_TIMEOUT_SECS = 10

_REFUSAL = re.compile(r"refusing fetch: (.+?)(?:\s{2,}|$)")
_PIN_REFUSAL = "destination is outside the credential's egress pin"
_NO_SECRET_REFUSAL = "no secret available for credential slot"
_WITHHELD = "withholding a credential the installed manifest no longer authorises"


@dataclass(frozen=True, slots=True)
class Case:
    name: str
    expect: Expect
    # The slot the placeholder names, and the field on it.
    # Distinct among the refusal-judged cases, so a refusal naming a slot
    # belongs to just one of them.
    slot: str
    field: str
    # None binds nothing, which is the refusal an operator causes rather than one
    # the pin or the manifest does.
    account_type: str | None
    # A URL shape other than "our server, secret in the query". `{placeholder}`
    # is this slot's reference and `{base}` our server's root.
    url_form: str | None = None
    # A secret other than the run's canary, for a case whose point is what the
    # value does to the URL once substituted. `{authority}` is our host:port.
    secret_form: str | None = None
    # The account's own pin, replacing the type's wherever it is non-empty.
    # `{authority}` becomes our host:port, so a case can pin the same server
    # it will be judged against.
    allow_hosts_form: tuple[str, ...] = ()

    @property
    def path(self) -> str:
        return f"/{self.name}.png"

    def url(self, base: str) -> str:
        # No spaces inside the braces: the URL travels through a log line that
        # is split on whitespace, and `substitute` trims either way.
        placeholder = f"{{{{credential.{self.slot}.{self.field}}}}}"
        if self.url_form is not None:
            return self.url_form.format(base=base, placeholder=placeholder)
        return f"{base}{self.path}?v={placeholder}"

    def secret(self, canary: str, authority: str) -> str:
        if self.secret_form is None:
            return canary
        return self.secret_form.format(authority=authority, path=self.path)

    def allow_hosts(self, authority: str) -> list[str]:
        return [entry.format(authority=authority) for entry in self.allow_hosts_form]


CASES: tuple[Case, ...] = (
    Case("permitted", "deliver", "weather", "token", "generic-token"),
    Case("pinned", "refuse-pin", "pool", "token", "braiins-pool"),
    Case("unbound", "refuse-no-secret", "media", "password", None),
    # `api` is not in params-demo's manifest, so resolution must withhold the
    # binding even though the config names one and the account exists.
    Case("undeclared", "refuse-withheld", "api", "token", "generic-token"),
    # Shares `media` with `unbound`: this instance binds an account, that one
    # does not, so the secretless refusal still names exactly one of them.
    Case("redirect", "deliver-no-follow", "media", "password", "generic-userpass"),
    # The template names the pinned host, so a pin reading it would approve. The
    # token then ends the authority early and aims the request at us instead —
    # judged by whether it arrives, since a leak here is a request we can see.
    Case(
        "reshaped",
        "refuse-pin",
        "pool_backup",
        "token",
        "braiins-pool",
        # http, not https: the reshaped request has to reach our plain server,
        # or it would fail the handshake and pass this case for the wrong reason.
        url_form="http://{placeholder}@api.braiins.com/x",
        secret_form="{authority}{path}?v=",
    ),
    # On a type that pins nothing of its own, the account pins the very server
    # it is about to reach: the pin is honoured, not merely tolerated.
    Case(
        "account_pin",
        "deliver",
        "weather",
        "token",
        "generic-token",
        allow_hosts_form=("{authority}",),
    ),
    # The same type, pinned elsewhere. Where the type alone would have allowed
    # this request, the account's pin refuses it.
    Case(
        "account_pin_denied",
        "refuse-pin",
        "weather",
        "token",
        "generic-token",
        allow_hosts_form=("api.example.invalid",),
    ),
    # The account pin replaces the type's rather than narrowing it: `braiins-pool`
    # admits only its own API, yet this reaches us because the account says so.
    # Only the store can express it — the API refuses a list on a pinned type.
    Case(
        "account_pin_replaces",
        "deliver",
        "pool_backup",
        "token",
        "braiins-pool",
        allow_hosts_form=("{authority}",),
    ),
)


@dataclass
class Outcome:
    case: Case
    served: Request | None = None
    refusal: str = ""
    withheld: str = ""
    followed: Request | None = None

    @property
    def failed(self) -> bool:
        if self.case.expect == "deliver":
            return self.served is None
        if self.case.expect == "deliver-no-follow":
            # Both halves matter: no arrival proves nothing, and reaching
            # the redirect target is the secret going where we never sent it.
            return self.served is None or self.followed is not None
        if self.served is not None:
            return True
        if self.case.expect == "refuse-pin":
            return _PIN_REFUSAL not in self.refusal
        if self.case.expect == "refuse-withheld":
            # The compositor's decision is the evidence. The host's refusal is
            # only its consequence, and reads the same as a never-bound slot.
            return not self.withheld
        return _NO_SECRET_REFUSAL not in self.refusal


@dataclass
class CheckCredentialEgress:
    device: str  # IP or host of the target Deck
    dwell_seconds: int = 15  # scene cycling duration in the pushed config
    keep_config: bool = False  # leave the test config and accounts on the device
    restore: bool = False  # put the backed-up config and accounts back, then exit

    def run(self) -> None:
        dev = Device(self.device)
        console.header("Credential egress")
        dev.print()
        catalog.ensure_device_reachable(dev)

        if self.restore:
            _restore(dev)
            return

        require(self.dwell_seconds > 0, "--dwell-seconds must be positive")
        require(MANIFEST.is_file(), f"{MANIFEST} not found — run from the repository root")

        # Per run, so a hit in the log is this run's and a stale line
        # from an earlier one cannot pass or fail the leak check.
        secret = f"canary-{uuid.uuid4().hex}"
        accounts = {c.name: str(uuid.uuid4()) for c in CASES if c.account_type is not None}

        seen: list[Request] = []
        with server(_views(seen), reachable_from=dev.host) as assets:
            base_url = _device_facing(assets)
            console.kv("serving", base_url)
            console.kv("cases", str(len(CASES)))

            _backup(dev)
            _push(dev, base_url, secret, accounts, self.dwell_seconds)

            with (
                dev.log_window(HOST_LOG) as host_window,
                dev.log_window(COMPOSITOR_LOG) as compositor_window,
            ):
                catalog.restart_compositor(dev)
                _settle(len(CASES))

        outcomes = _collect(host_window.text, compositor_window.text, seen)
        _report(outcomes)

        if not self.keep_config:
            _restore(dev)

        _judge(outcomes, host_window.text, secret)
        console.ok("every secret travelled only where its account, or its type, allows")


def _views(seen: list[Request]) -> dict[str, ViewConfig]:
    """A recording view per case: report the request, then answer with an image."""

    def record(request: Request) -> bytes:
        seen.append(request)
        return PIXEL_PNG

    def bounce(request: Request) -> View:
        seen.append(request)
        # The secret rides the query, so it reaches the target only when
        # the client follows. Relative — ureq resolves it against the request.
        sent = request.query.get("v", [""])[0]
        return View(response=b"", status=302, headers={"Location": f"{LANDED_PATH}?v={sent}"})

    views: dict[str, ViewConfig] = {
        case.path: bounce if case.expect == "deliver-no-follow" else record for case in CASES
    }
    views[LANDED_PATH] = record
    return views


def _device_facing(assets: ServerHandle) -> str:
    """The bind the device can reach; loopback only ever answers ourselves."""
    lan = [bind for bind in assets.binds if "127.0.0.1" not in bind]
    require(bool(lan), "no routable address to serve from — the device cannot reach us")
    return lan[0]


def _default_params() -> dict[str, object]:
    """`params-demo`'s own manifest defaults.

    It reads most of its params as required, so the pushed config carries them
    verbatim and varies only the URL. Params without a default are optional.
    """
    manifest = json.loads(MANIFEST.read_text(encoding="utf-8"))
    return {
        key: spec["default_value"]
        for key, spec in manifest["params"].items()
        if "default_value" in spec
    }


def _account(account_id: str, case: Case, secret: str, authority: str) -> dict[str, object]:
    account: dict[str, object] = {
        "id": account_id,
        "type_id": case.account_type,
        "name": f"Egress test ({case.name})",
        "field_values": {case.field: case.secret(secret, authority)},
        "created_at": datetime.now(UTC).isoformat().replace("+00:00", "Z"),
    }
    # Omitted rather than written empty, matching what the firmware serialises,
    # so an account without a pin exercises the absent-key path.
    if pin := case.allow_hosts(authority):
        account["allow_hosts"] = pin
    return account


def _scene(
    case: Case,
    base_url: str,
    defaults: Mapping[str, object],
    account_id: str | None,
) -> dict[str, object]:
    widget: dict[str, object] = {
        "id": str(uuid.uuid4()),
        "row": 0,
        "col": 0,
        "placement": "fullscreen",
        "widget_type_id": WIDGET_TYPE_ID,
        "viewport_shape": "rectangular",
        "params": {**defaults, "string_uri": case.url(base_url)},
    }
    if account_id is not None:
        widget["credential_bindings"] = {case.slot: account_id}
    return {
        "id": str(uuid.uuid4()),
        "enabled": True,
        "kind": "fullscreen",
        "widgets": [widget],
    }


def _push(
    dev: Device,
    base_url: str,
    secret: str,
    accounts: dict[str, str],
    dwell: int,
) -> None:
    console.header("Push test config")
    defaults = _default_params()
    config = {
        "version": 2,
        "scenes": [_scene(c, base_url, defaults, accounts.get(c.name)) for c in CASES],
        "scene_cycling": {
            "automatic_cycling_enabled": True,
            "automatic_cycling_default_duration": f"{dwell}s",
            "transition": "slide",
        },
    }
    authority = base_url.split("://", 1)[-1]
    stored = [_account(accounts[c.name], c, secret, authority) for c in CASES if c.name in accounts]
    _push_json(dev, config, CONFIG)
    _push_json(dev, {"version": 1, "accounts": stored}, SECRETS)
    # The store writes its own file at 0600; a pushed one would otherwise
    # sit world-readable and misrepresent what the device does.
    dev.run(f"chmod 600 {SECRETS}")
    console.ok(f"{len(CASES)} scenes and {len(stored)} accounts written")


def _push_json(dev: Device, document: Mapping[str, object], remote: RemotePath) -> None:
    """Stream rather than inline: a config this size does not survive an ssh command line."""
    with tempfile.NamedTemporaryFile("w", suffix=".json", delete=False) as handle:
        json.dump(document, handle, indent=2)
        staged = Path(handle.name)
    try:
        dev.push(staged, remote)
    finally:
        staged.unlink(missing_ok=True)


def _backup(dev: Device) -> None:
    dev.run(f"[ -f {CONFIG_BACKUP} ] || cp {CONFIG} {CONFIG_BACKUP}")
    # A device that never saved an account has no store, and that absence is
    # itself state to put back. Tested rather than tolerated: swallowing a
    # failed copy would leave the restore deleting accounts it never saved.
    dev.run(f"[ -f {SECRETS_BACKUP} ] || [ ! -f {SECRETS} ] || cp {SECRETS} {SECRETS_BACKUP}")
    console.kv("backup", f"{CONFIG_BACKUP}, {SECRETS_BACKUP}")


def _restore(dev: Device) -> None:
    if dev.read(f"[ -f {CONFIG_BACKUP} ] && echo yes || echo no").strip() != "yes":
        console.warn(f"no backup at {CONFIG_BACKUP} — leaving the device alone")
        return
    dev.run(f"cp {CONFIG_BACKUP} {CONFIG} && rm -f {CONFIG_BACKUP}")
    # A device with no accounts before the run must have none after it,
    # so a missing backup means remove rather than keep.
    dev.run(
        f"if [ -f {SECRETS_BACKUP} ]; then cp {SECRETS_BACKUP} {SECRETS} && "
        f"rm -f {SECRETS_BACKUP}; else rm -f {SECRETS}; fi"
    )
    catalog.restart_compositor(dev)
    console.ok("previous config and accounts restored")


def _settle(scenes: int) -> None:
    """Wait for the fetches, which do not queue behind the scene cycling.

    Widgets for every scene spawn when the compositor starts, so all of them
    fetch at once rather than as their scene comes up — one fetch's worth of
    waiting covers the whole corpus. Twice the SDK's 10s default, so even a
    fetch that times out has reached the log before it is read.
    """
    console.countdown(f"waiting for {scenes} fetches", 2 * _SDK_FETCH_TIMEOUT_SECS)


def _collect(host_log: str, compositor_log: str, seen: list[Request]) -> list[Outcome]:
    console.header("Collect results")
    by_path = {case.path: Outcome(case=case) for case in CASES}
    for request in seen:
        outcome = by_path.get(request.path)
        if outcome is not None and outcome.served is None:
            outcome.served = request

    refusals = [
        match.group(1)
        for match in (_REFUSAL.search(line) for line in host_log.splitlines())
        if match
    ]
    withheld = [line for line in compositor_log.splitlines() if _WITHHELD in line]

    landed = next((r for r in seen if r.path == LANDED_PATH), None)
    for outcome in by_path.values():
        case = outcome.case
        if case.expect == "deliver-no-follow":
            outcome.followed = landed
        if case.expect in _ARRIVAL_JUDGED:
            continue
        # A refusal names no URL, so it is attributed by the slot it names —
        # which is why each case owns one.
        quoted = f'"{case.slot}"'
        wanted = _PIN_REFUSAL if case.expect == "refuse-pin" else _NO_SECRET_REFUSAL
        outcome.refusal = next((r for r in refusals if wanted in r and quoted in r), "")
        outcome.withheld = next((line for line in withheld if quoted in line), "")

    return [by_path[case.path] for case in CASES]


def _judge(outcomes: list[Outcome], host_log: str, secret: str) -> None:
    """Order matters: a run where nothing fetched refuses every case for free."""
    served = next(o.served for o in outcomes if o.case.expect == "deliver")
    if served is None:
        raise Abort(
            "no request arrived for the permitted case — nothing was proven about the refusals"
        )

    # Equality, not "contains the secret": it also rules out the placeholder
    # arriving as written, which is the other way this can go wrong.
    require(
        served.query.get("v") == [secret],
        f"the permitted request carried {served.query.get('v')}, not the resolved secret",
    )
    require(
        secret not in host_log,
        "the secret appeared in the wasm-host log; only the placeholder form may be logged",
    )

    broken = [o for o in outcomes if o.failed]
    if broken:
        names = ", ".join(o.case.name for o in broken)
        raise Abort(f"{len(broken)} case(s) failed on the device: {names}")


def _arrival_verdict(out: Outcome) -> str:
    if out.served is None:
        return "NOT DELIVERED"

    if out.followed is not None:
        return "FOLLOWED — the secret was replayed to the redirect target"

    return "delivered" if out.case.expect == "deliver" else "delivered, redirect not followed"


def _verdict(out: Outcome) -> str:
    if out.case.expect in _ARRIVAL_JUDGED:
        return _arrival_verdict(out)

    if out.served is not None:
        return "SENT — the secret left for a host its type forbids"

    if out.case.expect == "refuse-withheld":
        return "withheld by the manifest check" if out.withheld else "NOT WITHHELD"
    return f"refused: {out.refusal}" if out.refusal else "NO REFUSAL RECORDED"


def _report(outcomes: list[Outcome]) -> None:
    for out in outcomes:
        line = _verdict(out)
        if out.failed:
            console.warn(f"{out.case.name}: {line}")
        else:
            console.kv(out.case.name, line)


@entrypoint
def main(args: CheckCredentialEgress) -> None:
    args.run()


if __name__ == "__main__":
    main()
