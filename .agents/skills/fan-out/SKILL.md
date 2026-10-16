---
name: fan-out
description: Start one lane per frontier ticket with `bli`, each running an agent on the ticket, so tickets build in parallel.
disable-model-invocation: true
---

# Fan out

Start one **lane** per ticket through `bli`, which creates the lane on the repo's lane host, starts its agent on the ticket, and claims the ticket. You choose the tickets with the developer and report; the agents do the building. Collect the finished lanes later with `/fan-in`.

Arguments: ticket IDs (`/fan-out 20 24 40`), a parent ticket (`/fan-out parent=17`), or nothing, which means the frontier of the parent set in `bli.toml`. Add `base=<branch>` to cut lanes from that branch instead of the configured base.

Every `bli` command takes `--json`; read its output rather than its text. On a non-zero exit, show the developer the stderr (it carries a `hint:`) and stop.

## 1. Pick the tickets

- **IDs given**: use them.
- **Otherwise**: `bli frontier --json`, adding `--parent <id>` when a parent was given. Each ticket has `id`, `title` and `url`; the frontier already leaves out tickets with a lane or a claim.

Show the developer the tickets with their titles and ask whether to start all of them. Starting lanes is their decision.

This step is done when the developer has confirmed the list. An empty frontier ends the run: say so.

## 2. Start the lanes

```bash
bli lane start <id>... --json        # add --base <branch> when base= was given
```

`bli` starts the lanes one at a time and reports each one in `lanes`, with an `outcome`:

- `started`: the lane exists, the ticket is claimed, and its agent was prompted. `status` is the host's hint about the agent: `working`, `blocked`, `idle` or `unknown`.
- `skipped`: the ticket already has a lane or a claim; `reason` says which.
- `failed`: `error` says why. When `path` is present, the lane exists and the ticket is claimed, but its agent did not start.

For each `blocked` lane, read its agent (`herdr agent read t<id> --source recent-unwrapped --lines 60`) and show the developer what it is asking. Answering it is the developer's decision.

This step is done when every confirmed ticket has an outcome, and each `blocked` agent's question has been shown to the developer.

## 3. Report

A table: ticket title (linked), outcome, branch, status hint, and the reason or error for any lane not started. Then how to watch the lanes:

- `bli lane list`: every lane with its ticket, branch, base and status hint;
- `herdr agent read t<id> --source recent-unwrapped --lines 80`: one lane's latest output.

When the agents are done, run `/fan-in`.

This step is done when every confirmed ticket has a row in the table.
