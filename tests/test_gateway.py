from seele.collectors.gateway import classify


def test_classify():
    assert classify({"local": True, "remote": True}) == ("ok", 0.0)
    assert classify({"local": True, "remote": False}) == ("degraded", 0.5)
    assert classify({"local": False, "remote": False}) == ("down", 1.0)
