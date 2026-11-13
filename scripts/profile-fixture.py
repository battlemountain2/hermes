import argparse
import math
import os
import pathlib
import signal
import subprocess
import sys
import tempfile
import time


def descendants(root):
    parents = {}
    for entry in pathlib.Path("/proc").iterdir():
        if not entry.name.isdigit():
            continue
        try:
            parent = next(line.split()[1] for line in (entry / "status").read_text().splitlines() if line.startswith("PPid:"))
            parents.setdefault(int(parent), []).append(int(entry.name))
        except (OSError, StopIteration, ValueError):
            continue
    result = [root]
    for parent in result:
        result.extend(parents.get(parent, []))
    return result


def sample_memory(root):
    rss = 0
    pss = 0
    for process in descendants(root):
        try:
            status = pathlib.Path(f"/proc/{process}/status").read_text()
            rss += sum(int(line.split()[1]) for line in status.splitlines() if line.startswith("VmRSS:"))
            rollup = pathlib.Path(f"/proc/{process}/smaps_rollup").read_text()
            pss += sum(int(line.split()[1]) for line in rollup.splitlines() if line.startswith("Pss:"))
        except (OSError, ValueError):
            continue
    return rss, pss


def stop_tree(app):
    if app.poll() is not None:
        return
    handles = []
    try:
        for process in descendants(app.pid):
            try:
                handles.append(os.pidfd_open(process))
            except OSError:
                continue
        for action in (signal.SIGTERM, signal.SIGKILL):
            for handle in handles:
                try:
                    signal.pidfd_send_signal(handle, action)
                except OSError:
                    pass
            try:
                app.wait(timeout=3)
            except subprocess.TimeoutExpired:
                continue
    finally:
        for handle in handles:
            os.close(handle)


def isolated(args):
    root = pathlib.Path(args.root)
    env = os.environ.copy()
    for key, name in (("HOME", "home"), ("XDG_CONFIG_HOME", "config"), ("XDG_STATE_HOME", "state"), ("XDG_CACHE_HOME", "cache")):
        directory = root / name
        directory.mkdir()
        env[key] = str(directory)
    env.update(HERMES_DEV="1", RUST_LOG="hermes=debug", GIO_USE_VFS="local", GIO_USE_VOLUME_MONITOR="unix", GTK_USE_PORTAL="0")
    display = None
    app = None
    peak_rss = peak_pss = 0
    finished = False
    try:
        if os.environ.get("HERMES_PROFILE_HEADLESS", os.environ.get("STRATA_PROFILE_HEADLESS")) == "1":
            env.update(GDK_BACKEND="broadway", BROADWAY_DISPLAY=f":{1000 + os.getpid() % 1000}")
            display = subprocess.Popen(["gtk4-broadwayd", "-a", "127.0.0.1", env["BROADWAY_DISPLAY"]], env=env, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            time.sleep(0.2)
            if display.poll() is not None:
                raise RuntimeError("Unable to start isolated Broadway display")
        with open(args.log, "w") as log:
            app = subprocess.Popen([args.binary, args.fixture], env=env, stdout=log, stderr=log)
            deadline = time.monotonic() + args.timeout
            while app.poll() is None and time.monotonic() < deadline:
                rss, pss = sample_memory(app.pid)
                peak_rss, peak_pss = max(peak_rss, rss), max(peak_pss, pss)
                if "directory load finished" in pathlib.Path(args.log).read_text(errors="replace"):
                    finished = True
                    time.sleep(0.25)
                    rss, pss = sample_memory(app.pid)
                    peak_rss, peak_pss = max(peak_rss, rss), max(peak_pss, pss)
                    break
                time.sleep(0.05)
        print(f"Sampled process-tree peak RSS: {peak_rss} KB")
        print(f"Sampled process-tree peak PSS: {peak_pss} KB")
        markers = ("window presented", "first directory batch ready", "first directory batch rendered", "directory load finished")
        for line in pathlib.Path(args.log).read_text(errors="replace").splitlines():
            if any(marker in line for marker in markers):
                print(line)
        if not finished:
            print("Profile incomplete: application exited or enumeration timed out", file=sys.stderr)
        return 0 if finished else 1
    finally:
        if app is not None:
            stop_tree(app)
        if display is not None:
            stop_tree(display)


def main():
    parser = argparse.ArgumentParser(description="Isolated Hermes enumeration/process-tree memory probe; not a paint or Dolphin benchmark")
    parser.add_argument("fixture")
    parser.add_argument("timeout", nargs="?", type=float, default=15)
    parser.add_argument("--binary", default=os.environ.get("HERMES_BINARY", os.environ.get("STRATA_BINARY", "target/debug/hermes")))
    parser.add_argument("--root", help=argparse.SUPPRESS)
    parser.add_argument("--log", help=argparse.SUPPRESS)
    args = parser.parse_args()
    args.fixture = str(pathlib.Path(args.fixture).resolve(strict=True))
    args.binary = str(pathlib.Path(args.binary).resolve(strict=True))
    if not pathlib.Path(args.fixture).is_dir() or not os.access(args.binary, os.X_OK) or not math.isfinite(args.timeout) or args.timeout <= 0:
        parser.error("Expected a directory, executable binary and positive timeout")
    if args.root:
        return isolated(args)
    with tempfile.NamedTemporaryFile(prefix="hermes-profile.", suffix=".log", delete=False) as log:
        log_path = log.name
    with tempfile.TemporaryDirectory(prefix="hermes-profile-state.") as root:
        runner = subprocess.Popen(["dbus-run-session", "--", sys.executable, str(pathlib.Path(__file__).resolve()), args.fixture, str(args.timeout), "--binary", args.binary, "--root", root, "--log", log_path])
        try:
            result = runner.wait()
        except KeyboardInterrupt:
            result = 130
        finally:
            stop_tree(runner)
    print(f"Full log: {log_path}")
    return result


def interrupted(_signum, _frame):
    raise KeyboardInterrupt


if __name__ == "__main__":
    signal.signal(signal.SIGTERM, interrupted)
    sys.exit(main())
