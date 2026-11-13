"""Generate the parity fixture: a 30 s scripted scenario sampled at 10 Hz, covering every scene layer."""
import json

steps = []
for i in range(300):
    t = round(i * 0.1, 1)
    sig, health, ev = {}, {}, []
    sig.update(cpu=0.05, gpu=0.0, mem=0.1, cpu_temp=0.3, gpu_temp=0.1, req_local=0, req_remote=0, tok_local=0,
               tok_remote=0, local_busy=0, spend_today=0, containers_bad=0, gateway_down=0)
    health.update(traefik="ok", docker="ok", ollama="ok", litellm="ok", gateway="ok")
    if 3 <= t < 6:   # load ramp
        x = (t - 3) / 3
        sig.update(cpu=0.5 + 0.5 * x, cpu_temp=0.6, gpu=0.8 * x, gpu_temp=0.4)
    if 6 <= t < 10:  # traffic: pulses of every kind
        sig.update(req_local=0.5, req_remote=0.3, tok_local=0.4, tok_remote=0.1)
        ev = {6.0: [{"kind": "request", "tier": "local", "status": 200, "ms": 100}],
              6.3: [{"kind": "request", "tier": "remote", "status": 200, "ms": 7000}],
              6.6: [{"kind": "request", "tier": "local", "status": 500, "ms": 50}],
              7.0: [{"kind": "request", "tier": "remote", "status": 429, "ms": 20},
                    {"kind": "request", "tier": "local", "status": 401, "ms": 5}],
              8.0: [{"kind": "request", "tier": "local", "status": 200, "ms": 3000}]}.get(t, [])
    if 10 <= t < 12:
        sig.update(local_busy=1.0, gpu=0.9)
    if 12 <= t < 14:
        sig.update(spend_today=0.6)
    if t == 14.0:
        ev = [{"kind": "alert", "source": "docker", "level": "crit", "msg": "die"}]
    if 16 <= t < 19:
        sig.update(containers_bad=0.14)
    if 19 <= t < 22:
        sig.update(gateway_down=1.0, containers_bad=0.14)
    if 22 <= t < 25:
        health.update(litellm="down")
    if 25 <= t < 28:
        sig.update(req_remote=0.8, tok_remote=0.5)
    steps.append({"t": t, "signals": sig, "health": health, "events": ev})

json.dump({"sizes": [4, 8, 8, 1, 16, 10], "steps": steps}, open("tests/parity/fixture.json", "w"))
print(len(steps), "steps")
