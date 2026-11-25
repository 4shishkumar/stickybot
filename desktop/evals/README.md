# Agent test set

`assistant.json` holds the questions the StickyBot agent is scored on; `fixtures/` is the folder it
may read during the run (copied to a scratch home and made a Git repository; `.env` and
`credentials.json` are added there at run time, never committed). Format and scoring:
`desktop/crates/stickybot-server/src/evals.rs`.

    make eval                    # the local brain (Goose + the local model)
    make eval BRAIN=claude       # Claude Code
    cargo run -p stickybot-server --bin stickybot-eval -- --brain local --only safety --repeat 3

It needs a Mac with the brain set up (the local runtime installed in the app, or Claude Code logged
in). It never uses the real stick: a fake stick denies every approval unless a case says otherwise.
