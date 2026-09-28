# BDK-880 widget pause — hardware test

Proves on real devices that widgets are stopped before a firmware image lands in `/tmp`, stay stopped until the flash
reboots the device, and come back when the upgrade fails. One lane per upgrade owner:

| Lane   | Device                                                     | Upgrade owner                      | Driver                                  |
| ------ | ---------------------------------------------------------- | ---------------------------------- | --------------------------------------- |
| Deck   | BMC100 `10.37.50.130` (VPN only)                           | the BMC application (self-managed) | `nix run .#deck -- e2e-grpc-sysupgrade` |
| BMM101 | `10.0.0.148` (LAN) on 09-24, `10.37.50.239` (VPN) on 09-28 | Boser (`boser_managed`)            | `nix run .#deck -- boser-upgrade-e2e`   |

Both devices download the image to `/tmp/firmware.tar` (bmc: `bmc/src/startup.rs` `UPGRADE_IMAGE_PATH`; Boser: same
path, and it leaves the file behind after a failed sysupgrade).

## Observation

The harnesses prove the upgrade; they cannot see widgets. A device-side sampler does, streamed to the host over a plain
SSH session (not through the harness tunnel), one line per second (busybox has no sub-second sleep):

```text
<uptime_s> <bmc-wasm-thin processes> <firmware_tar_bytes|-> <bmc_pid|-> <tmp_used_kB> <bmc-wasm-host processes>
```

Widgets are the `bmc-wasm-thin` processes; `check.sh` judges column 2 only. `bmc-wasm-host` is the shared renderer the
compositor starts from `/etc/bmc_system.json`; the pause does not stop it, so it is counted apart (the first version
counted it and failed every run on it). Scripts: `.tmp/BDK-880/hw/sampler.sh` (device) and `.tmp/BDK-880/hw/check.sh`
(host verdict); the 09-28 versions are copied in the appendix.

```sh
ssh root@DEVICE sh -s < .tmp/BDK-880/hw/sampler.sh | tee .tmp/BDK-880/hw/<run>/sampler.log
.tmp/BDK-880/hw/check.sh .tmp/BDK-880/hw/<run>/sampler.log stopped-until-reboot     # success run
.tmp/BDK-880/hw/check.sh .tmp/BDK-880/hw/<run>/sampler.log restarted-after-failure  # failure run
```

## Pass criteria

- **C1** widgets run in the first sample. Preflight removes any stale `/tmp/firmware.tar` first.
- **C2** no sample in which `firmware.tar` appeared or grew shows a live widget.
- **C3** the first download sample already shows zero widgets (the stop precedes the download).
- **C4a** (success run) no widget comes back between the stop and the end of the log (the reboot drops SSH).
- **C4b** (failure run) widgets come back after the last download growth, within 120 s.
- After a success run the rebooted device runs widgets again on the new firmware.
- On the BMM101, the bmc log shows the Boser upgrade state stream was consumed (the pause is driven by it, not by a
  local run).

## Scenarios per lane

1. **Failure** — start the upgrade, and once `firmware.tar` grows past ~1 MB kill the reverse-tunnel forward serving the
   image. The run ends `FAILED`; expect `restarted-after-failure`. If the download finishes before the kill lands, the
   run becomes the success scenario; record it as such and retry the failure scenario afterwards.
2. **Success** — the full upgrade to the newer image; expect `stopped-until-reboot`, then widgets back after boot.

## Setup constraints

- Start the sampler only after the harness's own bmc restart ("Restart bmc with index override" on the Deck), or
  `check.sh` mistakes that restart for the pause.

- A LAN download of the BMM101 image passes the 1 MB failure window in under a second, below the 1 Hz sampler: throttle
  it through `bmm101/throttle.py` (host relay, e.g. 256 KiB/s) and point the tunnel's firmware port at it.

- This branch's `e2e-grpc-sysupgrade` has no `--serve-ip` and serves on the host's NetBird address;
  `deck/tunnel-loop.sh` adds a device-side nft redirect from that address to the reverse tunnel.

- Deploy the branch's `core` (`nix run .#deck -- deploy --device IP --packages core`, unsandboxed) and verify
  `readlink -f /proc/$(pidof bmc-openwrt)/exe` equals the deployed store path; the deploy exits 0 on a failed
  registration. On the BMM101 run `/etc/init.d/boser restart` once afterwards.

- Host ports 8080–8083 belong to a concurrent session's lane: Deck lane uses 8180–8183, BMM101 lane 8190–8192. Both
  devices are reached by `ssh -R` reverse tunnels (the Deck is VPN-only; the host firewall blocks the BMM101), so the
  harnesses run with `--serve-ip 127.0.0.1`. Do not touch `10.37.50.118` or `10.0.0.129`.

- Snapshot and restore per lane: `/etc/bos_version`, `/etc/nix-upgrade/servers.json`, `/etc/nix/nix.conf`,
  `/etc/bmc/config.json`, auto-upgrade settings, the Boser init script, profile generation.

## Images

- Deck: `2026-09-24-0-b7626daa-26.10.1-plus-nightly` (bos-main `fbo/e2e-0924/master-b`, bmc-main master 8070847843),
  `/home/fbw/p/bmc-main/.tmp/e2e-0924/ci-images/B/…/feeds/firmware_…26.10.1…tar`. The harness anchors
  `/etc/bos_version`, so any newer Nix-era image works.
- BMM101: `2026-09-22-0-8cc4d7bd-26.10.1-plus-nightly` (`firmware-bcb101`, BDK-787 lineage),
  `.worktrees/BDK-796-mr1/bmc-main/.tmp/BDK-796-787/ci-images/bmm101/B/…`. Boser orders by release number only, so it
  must be later than the running 26.10.

## Baseline (2026-09-28 15:52)

- Deck `.130`: firmware `2026-09-24-0-b7626daa-26.10.1-plus-nightly`, bmc `lgwxv9zp…` (the 09-24 branch build, gen 9), 7
  thin + 1 host widget processes, `servers.json` = factory only, auto-upgrade disabled.
- BMM101 `.239`: firmware `2026-09-23-0-fc3e8f68-26.10-plus-nightly`, bmc `f8s7qk01…` (master, gen 3), 6 thin + 1 host,
  procd Boser on `127.0.0.1:8088`, `servers.json` = factory only. Snapshots in `.tmp/BDK-880/hw2/*/baseline/`.

## Baseline (2026-09-24 12:30)

- Deck `.130`: firmware `2026-09-11-0-ff0d18a2-26.09`, bmc `p7cnpjc0…`, 8 widget processes, `servers.json` = factory
  only (rewritten 11:54 today by an unknown session), 100 MB `/tmp` tmpfs.
- BMM101 `.148`: firmware `2026-09-23-0-fc3e8f68-26.10-plus-nightly`, bmc `f8s7qk01…`, 7 widget processes, Boser honours
  `BOS_INDEX_URL`, proxy marker present, 100 MB `/tmp` tmpfs.

## Runs

### 2026-09-24, branch `8ce747d0b9`

Evidence, scripts and exact commands: `.tmp/BDK-880/hw/{deck,bmm101}/` (`report.md` per lane). Verdicts are thin-only
(`check-thin-only.txt`); the host-counting `check.txt` fails every run on `bmc-wasm-host` alone.

| Lane   | Run                           | Harness                              | check.sh                                         | bmc.log                                                                      |
| ------ | ----------------------------- | ------------------------------------ | ------------------------------------------------ | ---------------------------------------------------------------------------- |
| Deck   | fail-1, tunnel cut at 1.08 MB | FAILED `Failed to download firmware` | PASS, back 1 s after the last growth             | last widget gone 0.3 ms before the download; respawn 50 ms after the failure |
| Deck   | success-1                     | PASS, 26.09 → 26.10.1                | PASS                                             | no spawn before the reboot                                                   |
| BMM101 | fail-1, unthrottled           | FAILED                               | not judgeable, the cycle fit between two samples | stop before Boser's start 204, respawn 1.1 s later                           |
| BMM101 | fail-2, 256 KiB/s             | FAILED                               | PASS, back 1 s after the last growth             | respawn 25 ms after the tunnel kill                                          |
| BMM101 | success-1                     | rebooted **without flashing** (rc 1) | PASS                                             | cause unknown and outside the pause; `sysupgrade -T` of the image passes     |
| BMM101 | success-2, retry              | PASS, 26.10 → 26.10.1                | PASS                                             | widgets spawned after boot                                                   |

The Deck samples carried the host in column 2; `deck/*/sampler.thin.log` is the upgrading bmc pid's window minus it.

End state: Deck on `2026-09-24-0-b7626daa-26.10.1-plus-nightly`, BMM101 on `2026-09-22-0-8cc4d7bd-26.10.1-plus-nightly`,
both on the branch bmc `lgwxv9zp…` with widgets running; `servers.json`, `nix.conf`, config and auto-upgrade as at
baseline. A rerun needs images newer than those.

### 2026-09-28, branch `415190e859` (MR !586 head after review)

Evidence: `.tmp/BDK-880/hw2/{deck,bmm}/<run>/` (`sampler.log`, `harness.log` or `runner-<run>.log`, `watcher.log`,
`device-after.txt`), scripts in `.tmp/BDK-880/hw2/`. Both devices reached over the VPN through `ssh -R` reverse tunnels;
the BMM lane's firmware port went through `throttle.py` (256 KiB/s for the failure run, 1 MiB/s for the success run, the
VPN capped it near 100 KiB/s anyway). Deployed `core` = bmc `72wxykj5…` (Deck gen 10, BMM gen 4).

Two changes to the method. The sampler now waits for the first widget process before its first line
(`sampler-when-widgets.sh`), and on the Deck it is started by `sampler-on-ready.sh` when the harness log reports "gRPC
ready" after the index-override restart, so the restart never masquerades as the pause. `check.sh` is unchanged and
judges `bmc-wasm-thin` only.

| Lane | Run                               | Harness                                                   | check.sh                             | bmc.log (UTC)                                                                                                                             |
| ---- | --------------------------------- | --------------------------------------------------------- | ------------------------------------ | ----------------------------------------------------------------------------------------------------------------------------------------- |
| Deck | fail-1, tunnel cut at 1.12 MB     | FAILED `Failed to download firmware`, rc 1                | PASS, back 1 s after the last growth | StartUpgrade 14:01:57.7417, SIGTERM ×7 .7422–.7433, last exit .7540, download .7547, failed 14:02:00.7495, respawn .7543                  |
| Deck | success-1, same image re-anchored | PASS rc 0, boot id 25af046c… → ffa81146…                  | PASS                                 | pre-reboot `bmc.log` not retained across the flash; the harness stream carries the phases                                                 |
| BMM  | fail-1, 256 KiB/s, cut at 1.32 MB | FAILED/FIRMWARE/DOWNLOADING rc 1                          | PASS, back 1 s after the last growth | Boser start 14:09:41.339, SIGTERM ×6 .344–.351, last exit .366, respawn 14:09:55.07 after Boser's Failed                                  |
| BMM  | success-1, 1 MiB/s                | PASS rc 0, 26.10 → 26.10.1, boot id 40d99517… → 756d5a6c… | PASS, 559 samples                    | Boser start 14:13:20.614, SIGTERM ×6 .618–.638, last exit .6386, no spawn until 14:24:07 on the new boot, right after the success overlay |

Both failure runs left no `/tmp/firmware.tar` behind: bmc's download writer and Boser's file uploader both remove a
partial file on a transport error. Neither removes the image after a **failed flash** (bmc has no removal on that path,
Boser's `cleanup` has no caller), so widgets then restart beside the tarball; that is outside this ticket.

End state: BMM on `2026-09-22-0-8cc4d7bd-26.10.1-plus-nightly`, bmc branch build gen 4, 6 widget processes, procd Boser
without `BOS_INDEX_URL`, `servers.json`, `nix.conf` and `config.json` restored byte-identical to the 09-28 baseline,
`init.d/boser` is the flashed image's. Deck on `2026-09-24-0-b7626daa-26.10.1-plus-nightly` (re-flashed), bmc branch
build gen 10, 7 widget processes, `servers.json`, `nix.conf` and `config.json` byte-identical to the 09-28 baseline, no
nft table, no `BMC_INDEX_URL` in the init script.

## Appendix: scripts as run on 2026-09-28

### `sampler-when-widgets.sh` (device side)

```sh
#!/bin/sh
# Device side: wait until at least one widget process runs, then sample once per second.
while [ "$(pgrep -f 'bmc-wasm-[t]hin' | wc -l)" -lt 1 ]; do sleep 0.2; done
while :; do
  set -- $(cat /proc/uptime)
  up=$1
  t=$(pgrep -f 'bmc-wasm-[t]hin' | wc -l)
  h=$(pgrep -f 'bmc-wasm-[h]ost' | wc -l)
  fw=$(ls -ln /tmp/firmware.tar 2>/dev/null | awk '{print $5}')
  bmc=$(pidof bmc-openwrt)
  tmp=$(df -k /tmp | awk 'NR==2{print $3}')
  echo "$up $t ${fw:--} ${bmc:--} $tmp $h"
  sleep 1
done
```

### `check.sh` (host verdict)

```sh
#!/bin/sh
# check.sh <sampler.log> <stopped-until-reboot|restarted-after-failure>
# Verdict on the widget-pause contract from the device-side samples:
#  C1  widgets run in the first sample (preflight removed any stale firmware.tar)
#  C2  no sample in which firmware.tar appeared or grew shows a live widget
#  C3  the first firmware.tar sample already has no widget
#  C4a success: no widget comes back after the stop before the log ends (reboot)
#  C4b failure: a widget comes back after the stop, after the last growth, within 120 s of it
log=$1; expect=$2
awk -v expect="$expect" '
NF<4 {next}
{ up=$1; w=$2; fw=$3; n++ }
n==1 { c1=(w>0); first_up=up; prev_fw=fw }
n>1 && fw!="-" && fw!=prev_fw {
  if (!grow_first_up) { grow_first_up=up; grow_first_w=w }
  grow_last_up=up
  if (w>0) { c2_bad++; if (!bad_line) bad_line=$0 }
}
w==0 && !stop_up { stop_up=up }
stop_up && w>0 && !back_up { back_up=up }
{ prev_fw=fw }
END {
  printf "samples=%d first_up=%s stop_up=%s grow_first_up=%s grow_last_up=%s back_up=%s\n", n, first_up, stop_up, grow_first_up, grow_last_up, back_up
  ok=1
  if (!c1) { print "FAIL C1 widgets not running at start"; ok=0 } else print "PASS C1 widgets running at start"
  if (!grow_first_up) { print "FAIL no firmware download observed"; ok=0 }
  else {
    if (c2_bad) { printf "FAIL C2 %d growth samples with live widgets, first: %s\n", c2_bad, bad_line; ok=0 } else print "PASS C2 no widget ran while firmware.tar grew"
    if (grow_first_w!=0) { print "FAIL C3 widgets alive at the first download sample"; ok=0 } else printf "PASS C3 widgets gone %.0f s before the download appeared\n", grow_first_up-stop_up
  }
  if (expect=="stopped-until-reboot") {
    if (back_up) { printf "FAIL C4a widgets came back at %s before the reboot\n", back_up; ok=0 } else print "PASS C4a widgets stayed stopped until the log ended"
  } else if (!back_up || back_up<grow_last_up || back_up-grow_last_up>120) { print "FAIL C4b widgets not back within 120 s after the download ended"; ok=0 }
  else printf "PASS C4b widgets back %.0f s after the last download growth\n", back_up-grow_last_up
  print (ok ? "VERDICT PASS" : "VERDICT FAIL")
  exit !ok
}' "$log"
```

### `deck/sampler-on-ready.sh`

```sh
#!/bin/sh
# sampler-on-ready.sh HARNESS_LOG RUNDIR: once the harness reports bmc's gRPC ready after its
# index-override restart, stream the device sampler (which itself waits for the first widget).
E2=/home/fbw/p/bmc-main/.worktrees/BDK-880/bmc-main/.tmp/BDK-880/hw2
log=$1; run=$2
while ! grep -q 'gRPC ready' "$log" 2>/dev/null; do sleep 0.5; done
echo "$(date +%T.%N | cut -c1-12) harness reported gRPC ready; starting sampler" >> "$run/trigger.log"
ssh -o BatchMode=yes root@10.37.50.130 sh -s < "$E2/sampler-when-widgets.sh" > "$run/sampler.log" 2>>"$run/trigger.log"
echo "$(date +%T.%N | cut -c1-12) sampler ended rc=$?" >> "$run/trigger.log"
```

### `deck/tunnel-loop.sh`

```sh
#!/bin/sh
# Reverse tunnel loop for the Deck lane; exits when tunnel.stop exists.
# The harness serves on this host's VPN address, which the Deck cannot reach;
# a device-side nft DNAT redirects that address:8180-8182 into the tunnel.
D=$(dirname "$0")
H=100.248.107.7
while [ ! -e "$D/tunnel.stop" ]; do
  ssh -o ConnectTimeout=5 root@10.37.50.130 "nft list table ip bdk880 >/dev/null 2>&1 || { nft add table ip bdk880 && nft add chain ip bdk880 out '{ type nat hook output priority -100; }' && nft add rule ip bdk880 out ip daddr $H tcp dport '{ 8180, 8181, 8182 }' dnat to 127.0.0.1 && echo dnat-installed; }" 2>/dev/null
  echo "$(date -Is) tunnel up"
  ssh -N -o ExitOnForwardFailure=yes -o ServerAliveInterval=5 -o ServerAliveCountMax=3 -o ConnectTimeout=10 \
    -R 8180:$H:8180 -R 8181:$H:8181 -R 8182:$H:8182 root@10.37.50.130 &
  echo $! > "$D/tunnel.ssh.pid"
  wait $!
  echo "$(date -Is) tunnel exited rc=$?"
  sleep 2
done
echo "$(date -Is) loop stopped"
```

### `deck/kill-watcher.sh`

```bash
#!/usr/bin/env bash
# Polls /tmp/firmware.tar on the Deck (~0.5 s) over a multiplexed SSH master
# separate from the tunnel; past the threshold it stops the tunnel loop and
# kills the tunnel ssh so the image download breaks mid-stream.
D=$(cd "$(dirname "$0")" && pwd)
THRESH=${THRESH:-1048576}
CP=/tmp/claude-1002/bdk880-deck-cm
mkdir -p /tmp/claude-1002
ssh -o ControlMaster=yes -o ControlPath=$CP -o ControlPersist=600 -fN root@10.37.50.130
while :; do
  s=$(ssh -o ControlPath=$CP root@10.37.50.130 'ls -ln /tmp/firmware.tar 2>/dev/null' | awk '{print $5}')
  echo "$(date +%T.%N | cut -c1-12) size=${s:--}"
  if [ -n "$s" ] && [ "$s" -gt "$THRESH" ]; then
    touch "$D/tunnel.stop"
    pid=$(cat "$D/tunnel.ssh.pid")
    kill "$pid" && echo "$(date +%T.%N | cut -c1-12) KILLED tunnel ssh pid=$pid at size=$s"
    for i in 1 2 3 4 5 6; do
      s=$(ssh -o ControlPath=$CP root@10.37.50.130 'ls -ln /tmp/firmware.tar 2>/dev/null' | awk '{print $5}')
      echo "$(date +%T.%N | cut -c1-12) after-kill size=${s:--}"; sleep 0.5
    done
    break
  fi
  sleep 0.5
done
ssh -o ControlPath=$CP -O exit root@10.37.50.130 2>/dev/null
```

### `bmm/tunnel-loop.sh`

```sh
#!/bin/sh
# Reverse-forward the BMM101 lane's rig ports (8190 cache, 8191 index, 8192 firmware) into the device.
while true; do
  ssh -N -o BatchMode=yes -o ExitOnForwardFailure=yes -o ServerAliveInterval=5 -o ServerAliveCountMax=3 \
    -R 8190:127.0.0.1:8190 -R 8191:127.0.0.1:8191 -R 8192:127.0.0.1:${FW_TARGET_PORT:-8192} root@10.37.50.239 &
  echo $! > tunnel-ssh.pid
  echo "$(date +%T.%N) tunnel ssh pid $!"
  wait $!
  echo "$(date +%T.%N) tunnel exited rc=$?, retrying in 3 s"
  sleep 3
done
```

### `bmm/watcher.sh`

```sh
#!/bin/sh
# watcher.sh RUNDIR: poll /tmp/firmware.tar size every ~0.5 s; past 1 MiB kill the tunnel loop and its ssh.
E=/home/fbw/p/bmc-main/.worktrees/BDK-880/bmc-main/.tmp/BDK-880/hw2/bmm
R=$1; CTL=/tmp/claude-1002/bdk880-bmm-ctl.sock
mkdir -p /tmp/claude-1002
ssh -M -S $CTL -o ControlPersist=no -f -N root@10.37.50.239 2>/dev/null
killed=
while :; do
  sz=$(ssh -S $CTL root@10.37.50.239 'ls -ln /tmp/firmware.tar 2>/dev/null' 2>/dev/null | awk '{print $5}')
  echo "$(date +%T.%N) ${sz:--}"
  if [ -z "$killed" ] && [ "${sz:-0}" -gt 1048576 ]; then
    kill "$(cat $E/tunnel-loop.pid)"; kill "$(cat $E/tunnel-ssh.pid)"
    echo "$(date +%T.%N) KILLED tunnel loop $(cat $E/tunnel-loop.pid) and ssh $(cat $E/tunnel-ssh.pid) at size $sz"
    killed=$(date +%s)
  fi
  [ -n "$killed" ] && [ $(( $(date +%s) - killed )) -gt 60 ] && break
  sleep 0.5
done
ssh -S $CTL -O exit root@10.37.50.239 2>/dev/null
echo "$(date +%T.%N) watcher done"
```

### `bmm/throttle.py`

```python
#!/usr/bin/env python3
"""throttle.py LISTEN_PORT TARGET_PORT RATE_BPS: TCP relay on 127.0.0.1 that caps the
target->client direction at RATE_BPS, so the device-side sampler (1 Hz) can see the
firmware download grow. Rate is re-read from throttle.rate (if present) per connection."""
import os, socket, sys, threading, time

listen_port, target_port, default_rate = int(sys.argv[1]), int(sys.argv[2]), int(sys.argv[3])
rate_file = os.path.join(os.path.dirname(os.path.abspath(__file__)), "throttle.rate")

def rate():
    try:
        return int(open(rate_file).read().strip())
    except Exception:
        return default_rate

def pipe(src, dst, bps):
    start, sent = time.monotonic(), 0
    try:
        while True:
            data = src.recv(16384)
            if not data:
                break
            dst.sendall(data)
            sent += len(data)
            if bps:
                ahead = sent / bps - (time.monotonic() - start)
                if ahead > 0:
                    time.sleep(ahead)
    except OSError:
        pass
    finally:
        for s in (src, dst):
            try:
                s.shutdown(socket.SHUT_RDWR)
            except OSError:
                pass

def handle(client):
    try:
        upstream = socket.create_connection(("127.0.0.1", target_port))
    except OSError:
        client.close()
        return
    bps = rate()
    print(f"{time.strftime('%T')} conn rate={bps}", flush=True)
    threading.Thread(target=pipe, args=(client, upstream, 0), daemon=True).start()
    threading.Thread(target=pipe, args=(upstream, client, bps), daemon=True).start()

srv = socket.socket()
srv.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
srv.bind(("127.0.0.1", listen_port))
srv.listen(16)
while True:
    c, _ = srv.accept()
    handle(c)
```

### `bmm/start-runner.sh`

```sh
#!/bin/sh
# usage: start-runner.sh NAME [runner args...]; answer by appending lines to in-NAME.txt
E=/home/fbw/p/bmc-main/.worktrees/BDK-880/bmc-main/.tmp/BDK-880/hw2/bmm
name=$1; shift
: > "$E/in-$name.txt"
cd /home/fbw/p/bmc-main/.worktrees/BDK-880/bmc-main || exit 1
setsid nohup sh -c "tail -f -n +1 '$E/in-$name.txt' | nix run .#deck -- boser-upgrade-e2e --device 10.37.50.239 --ssh 10.37.50.239 --serve-ip 127.0.0.1 --port 8190 --index-port 8191 --firmware-port 8192 --running-version 2026-09-23-0-fc3e8f68-26.10-plus-nightly $*; echo RUNNER_RC=\$?" > "$E/runner-$name.log" 2>&1 < /dev/null &
echo "runner started, log $E/runner-$name.log"
```
