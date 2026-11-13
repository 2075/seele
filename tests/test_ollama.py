from seele.collectors.ollama import any_busy, parse_ports, vram_fraction


def test_ports():
    out = "123 /x/llama-server --model m --port 42773 -c 4096\n124 llama-server --port=36641\n125 pgrep -af llama-server"
    assert parse_ports(out) == [36641, 42773]
    assert parse_ports("") == []


def test_vram():
    ps = {"models": [{"size_vram": 2 * 1024**3}, {"size_vram": 1024**3}]}
    assert vram_fraction(ps, 8 * 1024**3) == 3 / 8
    assert vram_fraction({"models": []}, 8) == 0.0
    assert vram_fraction({"models": [{"size_vram": 99}]}, 8) == 1.0


def test_busy():
    assert any_busy([{"is_processing": False}, {"is_processing": True}])
    assert not any_busy([{"is_processing": False}])
    assert not any_busy({"error": 1})


def test_parse_ports_ignores_shells_mentioning_llama_server():
    out = "1 /usr/bin/zsh -c pgrep -af llama-server --port 9\n2 /usr/local/lib/ollama/llama-server --model x --port 42773"
    assert parse_ports(out) == [42773]
