"""Run every Constella service in one container and restart any that exits
(exponential backoff, capped). The feeder runs once every FEEDER_INTERVAL
seconds (ECB publishes on business days). Logs are prefixed per service."""
import os
import subprocess
import sys
import threading
import time

SERVICES = {
    "proxy": (["python", "/app/proxy.py"], "/app"),
    "relayer": (["constella-relayer"], "/app"),
    "keeper": (["constella-keeper"], "/data"),
    "watchdog": (["constella-watchdog"], "/data"),
    "thesis-api": (["python", "-m", "src.api"], "/app/services/ai-engine"),
}


def pump(name, stream):
    for line in iter(stream.readline, b""):
        sys.stdout.write(f"[{name}] {line.decode(errors='replace')}")
    stream.close()


def run_forever(name, cmd, cwd):
    delay = 2
    while True:
        started = time.time()
        p = subprocess.Popen(cmd, cwd=cwd, stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
        pump(name, p.stdout)
        code = p.wait()
        if time.time() - started > 60:
            delay = 2  # it ran fine for a while: restart quickly
        sys.stdout.write(f"[supervisor] {name} exited ({code}); restarting in {delay}s\n")
        time.sleep(delay)
        delay = min(delay * 2, 300)


def feeder_loop():
    interval = int(os.environ.get("FEEDER_INTERVAL", "21600"))
    while True:
        if os.environ.get("FEEDER_SECRET"):
            p = subprocess.run(["constella-feeder"], stdout=subprocess.PIPE, stderr=subprocess.STDOUT, cwd="/data")
            for line in p.stdout.decode(errors="replace").splitlines():
                sys.stdout.write(f"[feeder] {line}\n")
        time.sleep(interval)


if __name__ == "__main__":
    enabled = os.environ.get("SERVICES", ",".join(SERVICES)).split(",")
    for name, (cmd, cwd) in SERVICES.items():
        if name in enabled:
            threading.Thread(target=run_forever, args=(name, cmd, cwd), daemon=True).start()
    if "feeder" in os.environ.get("SERVICES", "feeder"):
        threading.Thread(target=feeder_loop, daemon=True).start()
    while True:
        time.sleep(3600)
