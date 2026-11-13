from collections import deque

from seele.collectors.litellm import local_midnight_epoch, parse_rows, spend_ratio, tok_norm, window_tps


def test_tok_norm():
    assert tok_norm(0.5) == 0.0 and tok_norm(1) == 0.0
    assert abs(tok_norm(500) - 1.0) < 1e-9 and tok_norm(5000) == 1.0
    assert 0 < tok_norm(30) < 1


def test_parse_rows():
    assert parse_rows("100.5|20\nbad\n101|\n") == [(100.5, 20.0), (101.0, 0.0)]


def test_window():
    w = deque([(0, 100), (80, 300), (95, 300)])
    assert window_tps(w, 100) == 20.0  # (300+300)/30
    assert len(w) == 2


def test_spend():
    assert spend_ratio(2.5, 5) == 0.5 and spend_ratio(9, 5) == 1.0 and spend_ratio(1, 0) == 0.0


def test_midnight():
    assert local_midnight_epoch(1_800_000_000) <= 1_800_000_000
