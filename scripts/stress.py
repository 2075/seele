#!/usr/bin/env python3
"""Stress seele through each signal path and record what it sees.

Phases: idle → CPU burn → local LLM (Ollama, GPU) → request flood through Traefik (local+remote tiers) → cooldown.
Costs nothing: the LLM phase uses a local-tier key (.secrets/gw-local.key, model local-small); flood requests have no key, so they get 401 at LiteLLM and never reach OpenRouter.
Samples GET :6743/state every second and seele's own CPU usage, then prints a summary per phase.
Usage: .venv/bin/python scripts/stress.py [--seele-pid PID] [--scale 1.0]
"""
import argparse
import asyncio
import json
import multiprocessing as mp
import time
import urllib.request

import httpx
import psutil

KEY_FILE = __import__("pathlib").Path(__file__).resolve().parent.parent / ".secrets/gw-local.key"
KEYS = ["cpu", "gpu", "vram_models", "local_busy", "req_local", "req_remote", "err_rate", "egress_local", "tok_local", "spend_today"]


def burn(stop):
    while not stop.is_set():
        sum(i * i for i in range(10000))


def state():
    try:
        return json.load(urllib.request.urlopen("http://127.0.0.1:6743/state", timeout=1))
    except Exception:
        return None


async def llm(stop: asyncio.Event):
    """Real request path: Traefik → guard → LiteLLM gw-local → Ollama, so pulses, tok_local and local_busy all fire."""
    key = open(KEY_FILE).read().strip()
    async with httpx.AsyncClient(timeout=180, headers={"Authorization": f"Bearer {key}"}) as c:
        while not stop.is_set():
            await c.post("http://127.0.0.1:4000/v1/chat/completions", json={
                "model": "local-small", "max_tokens": 300,
                "messages": [{"role": "user", "content": "Count from 1 to 200 in words."}]})


async def flood(stop: asyncio.Event, rps: int, counts: dict):
    async with httpx.AsyncClient(timeout=5) as c:
        i = 0
        while not stop.is_set():
            path = "/v1/models" if i % 3 else "/open/v1/models"  # 2:1 local:remote
            try:
                r = await c.get(f"http://127.0.0.1:4000{path}")
                counts[r.status_code] = counts.get(r.status_code, 0) + 1
            except Exception:
                counts["err"] = counts.get("err", 0) + 1
            i += 1
            await asyncio.sleep(1 / rps)


async def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--seele-pid", type=int, help="default: auto-detect the python process running seele")
    ap.add_argument("--scale", type=float, default=1.0, help="multiply phase durations")
    a = ap.parse_args()
    def find_pid():
        import subprocess
        out = subprocess.run(["systemctl", "show", "seele", "-p", "MainPID", "--value"], capture_output=True, text=True).stdout.strip()
        if out and out != "0":
            return int(out)
        return next((p.pid for p in psutil.process_iter(["cmdline"])
                     if "seele" in " ".join(p.info["cmdline"] or [])
                     and "python" in (p.info["cmdline"] or [""])[0] and "stress" not in " ".join(p.info["cmdline"])), None)

    pid = a.seele_pid or find_pid()
    proc = psutil.Process(pid) if pid else None
    restarts = 0

    def seele_cpu():
        nonlocal proc, restarts
        try:
            return proc.cpu_percent(None) if proc else None
        except psutil.NoSuchProcess:  # seele restarted: follow the new process, count it
            restarts += 1
            new = find_pid()
            proc = psutil.Process(new) if new else None
            return None

    if proc:
        proc.cpu_percent(None)
    phases = [("idle", 10), ("cpu", 20), ("llm", 30), ("flood", 20), ("cooldown", 15)]
    rows, counts = [], {}
    for name, dur in phases:
        dur *= a.scale
        stop_mp, stop = mp.Event(), asyncio.Event()
        workers, tasks = [], []
        if name == "cpu":
            workers = [mp.Process(target=burn, args=(stop_mp,)) for _ in range(psutil.cpu_count())]
            [w.start() for w in workers]
        elif name == "llm":
            tasks = [asyncio.create_task(llm(stop))]
        elif name == "flood":
            tasks = [asyncio.create_task(flood(stop, 25, counts)) for _ in range(2)]  # ~50 req/s
        t_end = time.monotonic() + dur
        while time.monotonic() < t_end:
            await asyncio.sleep(1)
            s = state()
            rows.append((name, s["signals"] if s else {}, s["health"] if s else {},
                         seele_cpu()))
        stop.set(); stop_mp.set()
        for w in workers:
            w.join()
        for t in tasks:
            t.cancel()
        print(f"done: {name}", flush=True)

    print(f"\n{'phase':9}" + "".join(f"{k[:10]:>11}" for k in KEYS) + "   seele%  health")
    for name, _ in phases:
        rs = [r for r in rows if r[0] == name]
        peak = {k: max((r[1].get(k, 0) for r in rs), default=0) for k in KEYS}
        cpu = [r[3] for r in rs if r[3] is not None]
        health = sorted({f"{k}:{v}" for r in rs for k, v in r[2].items() if v != "ok"}) or ["all ok"]
        print(f"{name:9}" + "".join(f"{peak[k]:>11.2f}" for k in KEYS)
              + (f"   {max(cpu):5.1f}" if cpu else "      -") + "  " + ",".join(health))
    print("\n(values are per-phase peaks; seele% = seele's own CPU, of one core)")
    print("flood responses:", counts, "| seele restarts during test:", restarts)


if __name__ == "__main__":
    asyncio.run(main())
