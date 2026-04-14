"""CLI entry points for bmc-virt.

Provides subcommands for interacting with a running VM:
  bmc-virt ssh <command>
  bmc-virt pull <remote> <local>
  bmc-virt push <local> <remote>
  bmc-virt events [--raw]
  bmc-virt wait <event>
  bmc-virt exec <command> [--cmd <shell_cmd>]
"""

import argparse
import sys
from collections.abc import Sequence

from rich.console import Console

from bmc_virt import ui
from bmc_virt.events import Event
from bmc_virt.ssh import Ssh
from bmc_virt.vm import VM

_console = Console(stderr=True)
_out = Console()


def main(argv: Sequence[str] | None = None) -> None:
    """CLI entry point."""
    parser = argparse.ArgumentParser(prog="bmc-virt", description="BMC virtual machine harness")
    parser.add_argument("--host", default="localhost", help="VM host (default: localhost)")
    parser.add_argument("--event-port", type=int, default=5920, help="Event daemon port")
    parser.add_argument("--ssh-port", type=int, default=2222, help="SSH port")

    sub = parser.add_subparsers(dest="command", required=True)

    # ssh
    p_ssh = sub.add_parser("ssh", help="Run a command on the VM via SSH")
    p_ssh.add_argument("cmd", nargs=argparse.REMAINDER, help="Command to run")

    # pull
    p_pull = sub.add_parser("pull", help="Copy file from VM to host")
    p_pull.add_argument("remote", help="Remote path on VM")
    p_pull.add_argument("local", help="Local destination path")

    # push
    p_push = sub.add_parser("push", help="Copy file from host to VM")
    p_push.add_argument("local", help="Local source path")
    p_push.add_argument("remote", help="Remote path on VM")

    # events
    p_events = sub.add_parser("events", help="Stream events from the VM")
    p_events.add_argument("--raw", action="store_true", help="Output raw JSONL")

    # wait
    p_wait = sub.add_parser("wait", help="Wait for an event")
    p_wait.add_argument("event", help="Event name (e.g. app.ready)")
    p_wait.add_argument("--timeout", type=float, default=60, help="Timeout in seconds")

    # exec
    p_exec = sub.add_parser("exec", help="Execute a shell command on the VM via event daemon")
    p_exec.add_argument("cmd", nargs=argparse.REMAINDER, help="Shell command to run")
    p_exec.add_argument("--timeout", type=float, default=30, help="Timeout in seconds")

    args = parser.parse_args(argv)

    if args.command == "ssh":
        _do_ssh(args)
    elif args.command == "pull":
        _do_pull(args)
    elif args.command == "push":
        _do_push(args)
    elif args.command == "events":
        _do_events(args)
    elif args.command == "wait":
        _do_wait(args)
    elif args.command == "exec":
        _do_exec(args)


def _do_ssh(args: argparse.Namespace) -> None:
    ssh = Ssh(host=args.host, port=args.ssh_port)
    cmd_str = " ".join(args.cmd)
    result = ssh.run(cmd_str, check=False)
    if result.stdout:
        sys.stdout.write(result.stdout)
    if result.stderr:
        sys.stderr.write(result.stderr)
    sys.exit(result.returncode)


def _do_pull(args: argparse.Namespace) -> None:
    ssh = Ssh(host=args.host, port=args.ssh_port)
    ssh.pull(remote=args.remote, local=args.local)
    _console.print(f"[green]Pulled[/green] {args.remote} → {args.local}")


def _do_push(args: argparse.Namespace) -> None:
    ssh = Ssh(host=args.host, port=args.ssh_port)
    ssh.push(local=args.local, remote=args.remote)
    _console.print(f"[green]Pushed[/green] {args.local} → {args.remote}")


def _do_events(args: argparse.Namespace) -> None:
    vm = VM.connect(host=args.host, event_port=args.event_port, ssh_port=args.ssh_port)
    try:
        for evt in vm.events():
            if args.raw:
                _out.print_json(data={"name": evt.name, "ts": evt.ts.isoformat(), "data": evt.data})
            else:
                ui.event(evt)
    except KeyboardInterrupt:
        pass
    finally:
        vm.close()


def _do_wait(args: argparse.Namespace) -> None:
    try:
        event = Event(args.event)
    except ValueError:
        _console.print(f"[red]Unknown event:[/red] {args.event}")
        _console.print(f"[dim]Available: {', '.join(e.value for e in Event)}[/dim]")
        sys.exit(1)

    vm = VM.connect(host=args.host, event_port=args.event_port, ssh_port=args.ssh_port)
    try:
        evt = vm.wait_for(event, timeout=args.timeout)
        _out.print_json(data={"name": evt.name, "ts": evt.ts.isoformat(), "data": evt.data})
    finally:
        vm.close()


def _do_exec(args: argparse.Namespace) -> None:
    cmd_str = " ".join(args.cmd)
    vm = VM.connect(host=args.host, event_port=args.event_port, ssh_port=args.ssh_port)
    try:
        result = vm.exec(cmd_str, timeout=args.timeout)
        _out.print_json(
            data={
                "id": result.id,
                "ok": result.ok,
                "data": result.data,
                "error": result.error,
            }
        )
    finally:
        vm.close()


if __name__ == "__main__":
    main()
