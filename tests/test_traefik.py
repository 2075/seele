from seele.collectors.traefik import Window, parse_line


def test_parse_local():
    r = parse_line('{"RequestPath":"/v1/models","DownstreamStatus":401,"Duration":2500000}')
    assert r == {"tier": "local", "status": 401, "ms": 2.5}


def test_parse_remote_and_skip():
    assert parse_line(b'{"RequestPath":"/open/v1/x","DownstreamStatus":200,"Duration":1000000}')["tier"] == "remote"
    assert parse_line('{"RequestPath":"/ping","DownstreamStatus":200,"Duration":1}') is None


def test_parse_garbage():
    assert parse_line("time=... level=info msg=hi") is None
    assert parse_line("{}") is None
    assert parse_line("") is None


def test_window():
    w = Window(10)
    for i in range(10):
        w.add(0.0, "local", 200)
    w.add(0.0, "remote", 429)
    w.add(0.0, "remote", 500)
    a, b, e = w.stats(1.0)
    assert a == 0.5 and b == 0.1 and abs(e - 2 / 12) < 1e-9
    assert w.stats(20.0) == (0.0, 0.0, 0.0)


def test_is_error_includes_auth_failures():
    from seele.collectors.traefik import is_error
    assert all(is_error(s) for s in (401, 403, 429, 500, 502))
    assert not any(is_error(s) for s in (200, 201, 404))
