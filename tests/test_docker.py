from seele.collectors.docker import bad_share, net_bytes, norm_bytes_rate, parse_event


def test_norm():
    assert norm_bytes_rate(500) == 0
    assert norm_bytes_rate(1000) == 0
    assert abs(norm_bytes_rate(1e7) - 1) < 1e-9
    assert norm_bytes_rate(1e12) == 1
    assert 0.2 < norm_bytes_rate(1e4) < 0.3


def test_net_bytes():
    assert net_bytes({"networks": {"a": {"rx_bytes": 5, "tx_bytes": 7}, "b": {"rx_bytes": 1, "tx_bytes": 1}}}) == 14
    assert net_bytes({}) == 0


def test_bad_share():
    cs = [_c("a", status="Up 1h (healthy)"), _c("b", status="Up (unhealthy)"),
          _c("c", state="exited", status="Exited"), _c("d")]
    assert bad_share(cs, expected=()) == 0.5
    assert bad_share([], expected=()) == 0


def test_parse_event():
    ev = {"Action": "health_status: unhealthy", "Actor": {"Attributes": {"name": "x"}}}
    assert parse_event(ev) == ("x", "unhealthy")
    assert parse_event({"Action": "die", "Actor": {"Attributes": {"name": "y"}}}) == ("y", "die")
    assert parse_event({"Action": "exec_start: sh", "Actor": {}}) is None


def _c(svc, state="running", status="Up"):
    return {"Labels": {"com.docker.compose.service": svc}, "State": state, "Status": status}


def test_bad_share_counts_missing_expected_services():
    assert bad_share([], expected=("edge", "guard")) == 1.0          # compose down: everything gone
    assert bad_share([_c("edge")], expected=("edge", "guard")) == 0.5
    assert bad_share([_c("edge"), _c("guard", status="Up (unhealthy)")], expected=("edge", "guard")) == 0.5
    assert bad_share([_c("edge"), _c("guard")], expected=("edge", "guard")) == 0.0
