"""Tiny JSON endpoint: GET http://127.0.0.1:6743/state → signals and health."""
import asyncio
import json


async def serve(state, host: str = "127.0.0.1", port: int = 6743) -> None:
    async def handle(reader, writer):
        try:
            await reader.readuntil(b"\r\n\r\n")
            body = json.dumps({"signals": {k: round(s.value, 3) for k, s in sorted(state.signals.items())},
                               "health": state.health,
                               "severity": getattr(state, "severity", None)}).encode()
            writer.write(b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nConnection: close\r\n"
                         + f"Content-Length: {len(body)}\r\n\r\n".encode() + body)
            await writer.drain()
        except Exception:
            pass
        finally:
            writer.close()

    server = await asyncio.start_server(handle, host, port)
    async with server:
        await server.serve_forever()
