---
name: fan-out
description: Spawn one herdr worktree per ready ticket, each running Claude on `/implement`, so tickets build in parallel.
disable-model-invocation: true
---

# Fan out

Start one **lane** per ticket: a herdr worktree on its own branch, with a Claude agent in it running `/implement` on that ticket. You start lanes and report; the agents do the building. Collect the finished lanes later with `/fan-in`.

Arguments: issue numbers (`/fan-out 20 24 40`), a parent issue (`/fan-out parent=17`), or nothing, which means the frontier of [Spec: provider-agnostic lane loop (fan-out, fan-in, remove-lane) in bli](https://github.com/ibedwi/bli/issues/1). Add `base=<branch>` to branch lanes from that branch instead of `main`.

## 1. Check you're inside herdr

`test "${HERDR_ENV:-}" = 1`. If it fails, tell the user to run this from a herdr pane, and stop.

Then take the repo root, `root=$(git rev-parse --show-toplevel)`, and pass `--cwd "$root"` to every `herdr worktree list` and `herdr worktree create`. Without it herdr can resolve a different repo than the one you're in, and list or create lanes there.

## 2. Pick the tickets

- **Numbers given**: use them.
- **Otherwise**, take the parent's **frontier**: open sub-issues with no open blockers and no assignee.

  ```bash
  gh api repos/{owner}/{repo}/issues/<parent>/sub_issues --paginate \
    --jq '.[] | select(.state=="open" and .issue_dependencies_summary.blocked_by==0 and (.assignees|length)==0) | "\(.number)\t\(.title)"'
  ```

Drop any ticket that already has a lane: a branch `ticket/<n>-*` in `herdr worktree list --cwd "$root"`, or an assignee.

Show the user the tickets with their titles and ask whether to start all of them. Always launch agents in `bypassPermissions` mode.

This step is done when the user has confirmed the list.

## 3. Start each lane

Every lane branches from the **remote** copy of the base, `origin/<base>` (`origin/main` by default), never the local branch: the local copy can lag behind merged work. First run `git fetch origin`, then confirm `git rev-parse --verify origin/<base>` succeeds; if it fails, tell the user the base branch is not on `origin`, and stop. Then, for each ticket `n`:

```bash
slug=<3-5 word kebab-case slug of the title>
out=$(herdr worktree create --cwd "$root" --branch "ticket/$n-$slug" --base "origin/$base" \
        --label "#$n $slug" --no-focus --trust-repository)
git config "branch.ticket/$n-$slug.fanOutBase" "$base"
pane=$(jq -r '.result.root_pane.pane_id' <<<"$out")
ws=$(jq -r '.result.workspace.workspace_id' <<<"$out")
herdr agent start "t$n" --kind claude --pane "$pane" -- --permission-mode bypassPermissions
herdr agent prompt "t$n" "/implement https://github.com/<owner>/<repo>/issues/$n"
herdr agent wait "t$n" --until working --until blocked --timeout 20000
gh issue edit "$n" --add-assignee @me
```

- Run lanes one at a time, and read each lane's IDs from the JSON responses.
- The `fanOutBase` config records the lane's base for `/fan-in`, which compares commits and opens the PR against it.
- The assignee is the **claim**: it keeps a second `/fan-out` from starting the same ticket.
- If `agent start` returns `agent_not_ready`, or the wait reports `blocked`, read the pane (`herdr agent read "t$n" --source recent-unwrapped --lines 60`) and show the user what it's asking. Answering it is the user's decision.

This step is done when every chosen ticket's agent has been seen `working`, or has been reported to the user as `blocked`.

## 4. Report

A table: ticket title (linked), agent name `t<n>`, workspace ID, branch. Then the commands to watch the lanes:
- `herdr agent list`: every agent's state;
- `herdr agent read t<n> --source recent-unwrapped --lines 80`: one lane's latest output.

When agents show `done`, run `/fan-in`.
