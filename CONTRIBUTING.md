# Contributing

- **Set up from scratch:** [docs/BUILDING.md](docs/BUILDING.md) — hardware, tools, flashing, the Mac agent.
- **Day to day:** [docs/DEVELOPMENT.md](docs/DEVELOPMENT.md) — layout, tests, debugging, adding commands / sounds / events / screens.
- **The contract** between the boards and the Mac: [PROTOCOL.md](PROTOCOL.md).

Before you commit:

```sh
make dev-setup   # once: dev tools + pre-commit hook
make check       # lint + all tests
```

Keep commits to one topic, with an imperative subject line. Never commit `secrets.ini`.
