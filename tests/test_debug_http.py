import asyncio
import json

from seele.debug_http import serve
from seele.scene import severity
from seele.state import State


def test_state_endpoint_reports_severity():
    async def go():
        s = State()
        s.push("containers_bad", 0.14)
        s.severity = severity(s)
        task = asyncio.create_task(serve(s, port=6798))
        await asyncio.sleep(0.2)
        r, w = await asyncio.open_connection("127.0.0.1", 6798)
        w.write(b"GET /state HTTP/1.1\r\n\r\n")
        body = json.loads((await r.read()).split(b"\r\n\r\n", 1)[1])
        task.cancel()
        return body

    body = asyncio.run(go())
    assert body["severity"] == "degraded"
    assert "signals" in body and "health" in body


def test_severity_levels():
    def st(**kv):
        s = State()
        for k, v in kv.items():
            s.push(k, v)
        return s

    assert severity(st()) == "ok"
    assert severity(st(containers_bad=0.14)) == "degraded"   # one of seven core services down
    assert severity(st(gateway_down=1.0)) == "outage"         # edge stopped
    assert severity(st(containers_bad=1.0)) == "outage"       # compose down
