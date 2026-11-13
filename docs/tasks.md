# Tasks

## Open

- [x] Exercise tok_local via gateway key (seele-stress, local-small)
- [x] Outage drill + visual check of amber/red (2026-11-13)
- [x] Quiet hours dim outage/degraded too (user decision 2026-11-13, revisit if alerts get missed)
- [ ] ../machine: gw-open has no valid OpenRouter key (upstream 401); blocks spend/tok_remote tests
- [x] 401/403 count as errors (err_rate + red pulses), decided 2026-11-12
- [x] Look signed off by user with traffic preview (2026-11-13): defaults in config.toml [palette]/[pulses]
- [ ] Optional: docker-socket-proxy instead of the raw socket; Prometheus metrics in ../machine
- [ ] Initialise git and add a GitHub remote, then migrate tasks to GitHub Issues

- [x] Phase 1: OpenRGB hardware readiness (i2c-dev, udev, enumerate zones)
- [x] Phase 2: daemon skeleton, dry-run renderer, host + NVML load layer
- [x] NZXT channels identified: Ch1 = fans, Ch2 = strip
- [x] Phase 3: Docker health/egress stats, Traefik log tailer (local vs remote)
- [x] Phase 4: LiteLLM SpendLogs, Ollama /api/ps + /slots
- [x] Phase 5: scenes, alerts, quiet hours, /state endpoint, systemd units (code done)
- [x] Install services (done by user)
- [x] Services reinstalled 2026-11-12 22:43: SDK bound to 127.0.0.1, seele on current code (verified)

- [x] Design freeze declared 2026-11-13 (remote signals unverified live)
- [x] Rust port implemented: parity exact, 26 tests + 7 live, clippy clean (2026-11-13)
- [x] Cut over to Rust (2026-11-13)
- [ ] Verify spend_today/tok_remote on Rust once gw-open has an upstream key
- [ ] Retire Python tree after a week on Rust (keep tests/parity + golden)

## Done

- [x] 2026-11-12: Scaffold the agent-skills config (AGENTS.md, docs/agents/)
- [x] Rust: retry OpenRGB connect for 30 s at startup (boot race seen 2026-11-13 09:07)
- [x] `seele-rgb install` verified ALL OK; old scripts and deploy/ removed (2026-11-13)
- [x] Dual license MIT OR Apache-2.0, author 2075, git init (2026-11-13)
- [ ] GitHub remote (gh re-auth needed), then repository URL in Cargo.toml, migrate tasks to issues, cargo publish
