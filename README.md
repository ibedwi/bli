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
[lane]                              # optional; these are the defaults
host   = "herdr"                    # herdr
base   = "main"                     # the branch lanes are cut from, on origin
branch = "ticket/{id}-{slug}"       # lane branch names; must contain {id}
prompt = "/implement {ticket_url}"  # the agent's first prompt

[tracker]
kind   = "github"   # github
parent = 1          # optional default parent for `bli frontier`
```

`bli` keeps no state: lanes are found by matching the lane host's live list against `branch` (ADR 0001), so changing the pattern hides existing lanes.

## Commands

- `bli doctor [--json]`: checks this repo's setup before a fan-out and reports each check as `pass` or `warn`: `git` is installed, then the issue tracker's checks (on GitHub: `gh` is installed and `gh auth status` succeeds), then the lane host's (on herdr: `herdr` is installed, and the permission-mode check, which passes when the herdr server is running, since `bli` passes `--permission-mode bypassPermissions` itself). `--json` prints `{"checks": [{"name", "status", "detail", "hint"}]}`, with `hint` left out when there is none. When any check warns, `doctor` still prints every check, then exits 1 with a summary on stderr.
- `bli frontier [--parent <id>] [--json]`: the tickets under a parent that are open, unblocked, unclaimed and have no lane. `--json` prints `{"parent": "<id>", "tickets": [{"id", "title", "url"}]}`.
- `bli lane start <id>... [--base <branch>] [--slug <slug>] [--json]`: from inside the lane host, fetches `origin`, then for each ticket in turn creates a lane on a new branch from `origin/<base>`, records the base as `git config branch.<branch>.bliBase`, starts Claude in bypass-permissions mode with the configured prompt, and claims the ticket. Tickets with a lane or a claim are skipped. The slug is the first five words of the title in kebab-case; `--slug` overrides it for a single ticket. `--json` prints `{"base", "lanes": [{"ticket", "title", "url", "outcome", "branch", "base", "path", "workspace", "status", "reason", "error"}]}`, where `outcome` is `started`, `skipped` or `failed` and absent fields are left out. One lane failing doesn't stop the rest. When any lane fails, `lane start` still prints every lane's outcome on stdout, then exits 1 with a summary on stderr; skipped lanes are not failures.
- `bli lane list [--json]`: every lane, from the lane host's live list, with its pull request. `--json` prints `{"lanes": [{"ticket", "branch", "base", "path", "workspace", "status", "pr"}]}`, where `pr` is `null` or `{"number", "url", "state"}` and `state` is `open`, `merged` or `closed`.
- `bli lane read <id> [--lines <n>] [--json]`: the last `n` (default 80) lines of the lane agent's output, through the lane host. `--json` prints `{"ticket", "branch", "status", "output"}`.
- `bli lane ship <id> --body-file <file> [--title <title>] [--json]`: fetches `origin`, pushes the lane's branch and opens a pull request against the lane's recorded base, titled with the ticket's title unless `--title` is given. On GitHub Issues, `Closes #<id>` is added to the body unless it already closes the ticket. Refuses when the branch has no commits beyond `origin/<base>`. When the branch already has a pull request, in any state, it is reported instead and nothing is pushed. `--json` prints `{"ticket", "branch", "base", "outcome", "pr"}`, where `outcome` is `opened` or `existing`.

`status` is the lane host's hint about the agent (`working`, `blocked`, `idle` or `unknown`), not a verdict.

Pull requests live on GitHub, through `gh`, whichever issue tracker holds the tickets. `bli` never merges a pull request: merging is the developer's decision.

Errors go to stderr with a non-zero exit. `bli` never prompts.
