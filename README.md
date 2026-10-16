# bli

A CLI for running tickets as parallel **lanes** of agent work, independent of which lane host runs them or which issue tracker holds the tickets. See `GLOSSARY.md` for the vocabulary and `docs/adr/` for decisions.

## Install

```sh
nix profile install github:ibedwi/bli   # or: nix build
cargo install --path .                  # fallback
```

Develop with `nix develop`, then `cargo test`.

## Configure

Each repo commits a `bli.toml` at its root:

```toml
[tracker]
kind   = "github"   # github
parent = 1          # optional default parent for `bli frontier`
```

## Commands

- `bli frontier [--parent <id>] [--json]`: the tickets under a parent that are open, unblocked and unclaimed. `--json` prints `{"parent": "<id>", "tickets": [{"id", "title", "url"}]}`.

Errors go to stderr with a non-zero exit. `bli` never prompts.
