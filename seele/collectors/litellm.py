"""Tokens/s and daily spend from LiteLLM spend logs (read-only SELECTs via docker exec psql).

Access: `docker exec -e PGOPTIONS=-c default_transaction_read_only=on <db> psql -U gw_x -d gw_x`.
The container's pg_hba trusts the local socket, so no password is used or read. Only metadata
columns (startTime, total_tokens, spend) are selected. Spend is USD; treated as ~EUR.
"""
import asyncio
import logging
import math
import time
from collections import deque
from datetime import datetime

log = logging.getLogger(__name__)
CONTAINER = "research-stack-litellm-db-1"
TIERS = {"local": "gw_local", "remote": "gw_open"}
WINDOW = 30.0


def tok_norm(tps: float) -> float:
    """Tokens/s log-normalised: 1 -> 0, 500 -> 1."""
    return max(0.0, min(1.0, math.log10(tps) / math.log10(500))) if tps > 1 else 0.0


def parse_rows(out: str) -> list[tuple[float, float]]:
    """Lines 'epoch|tokens' -> [(epoch, tokens)]; bad lines are skipped."""
    rows = []
    for line in out.splitlines():
        try:
            t, n = line.split("|")[:2]
            rows.append((float(t), float(n or 0)))
        except ValueError:
            continue
    return rows


def window_tps(win: deque, now: float, span: float = WINDOW) -> float:
    while win and win[0][0] < now - span:
        win.popleft()
    return sum(n for _, n in win) / span


def spend_ratio(spend: float, budget: float) -> float:
    return max(0.0, min(1.0, spend / budget)) if budget > 0 else 0.0


def local_midnight_epoch(now: float | None = None) -> float:
    d = datetime.fromtimestamp(now if now is not None else time.time())
    return d.replace(hour=0, minute=0, second=0, microsecond=0).timestamp()


async def _psql(db: str, sql: str) -> str:
    p = await asyncio.create_subprocess_exec(
        "docker", "exec", "-e", "PGOPTIONS=-c default_transaction_read_only=on", CONTAINER,
        "psql", "-U", db, "-d", db, "-At", "-c", sql,
        stdout=asyncio.subprocess.PIPE, stderr=asyncio.subprocess.PIPE,
    )
    out, err = await asyncio.wait_for(p.communicate(), 8)
    if p.returncode:
        raise RuntimeError(err.decode(errors="replace").strip()[:200])
    return out.decode()


_EPOCH = "extract(epoch from \"startTime\")"  # startTime is stored in UTC


async def run(state, budget_eur_day: float = 5.0, **opts) -> None:
    wins = {t: deque() for t in TIERS}
    last = {t: time.time() - WINDOW for t in TIERS}
    last_log, fails = 0.0, 0
    while True:
        try:
            for tier, db in TIERS.items():
                sql = (f'SELECT {_EPOCH}, total_tokens FROM "LiteLLM_SpendLogs" '
                       f'WHERE {_EPOCH} > {last[tier]:.3f} ORDER BY "startTime"')
                rows = parse_rows(await _psql(db, sql))
                for t, n in rows:
                    wins[tier].append((t, n))
                    last[tier] = max(last[tier], t)
                state.push(f"tok_{tier}", tok_norm(window_tps(wins[tier], time.time())), tau=2)
            m = local_midnight_epoch()
            out = await _psql(TIERS["remote"],
                              f'SELECT COALESCE(SUM(spend),0) FROM "LiteLLM_SpendLogs" WHERE {_EPOCH} >= {m:.0f}')
            state.push("spend_today", spend_ratio(float(out.strip() or 0), budget_eur_day), tau=5)
            state.health["litellm"] = "ok"
            fails = 0
            await asyncio.sleep(5)
        except Exception as e:
            fails += 1
            state.health["litellm"] = "down"
            for k in ("tok_local", "tok_remote", "spend_today"):
                state.push(k, 0.0, tau=2)
            if time.monotonic() - last_log > 60:
                last_log = time.monotonic()
                log.warning("litellm collector: %r", e)
            await asyncio.sleep(min(60, 5 * fails))
