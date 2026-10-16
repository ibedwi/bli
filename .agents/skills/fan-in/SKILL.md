---
name: fan-in
description: Collect the herdr lanes started by `/fan-out`: open PRs for finished tickets, surface blocked agents, and clean up merged lanes.
disable-model-invocation: true
---

# Fan in

Walk every **lane** that `/fan-out` started (a herdr worktree on a `ticket/<n>-*` branch, with an agent named `t<n>`) and move each one forward. You never merge. Merging is the human's **gate**, and a lane is cleaned up only after its PR is merged.

## 1. Check you're inside herdr

`test "${HERDR_ENV:-}" = 1`. If it fails, tell the user to run this from a herdr pane, and stop.

Then take the repo root, `root=$(git rev-parse --show-toplevel)`, and pass `--cwd "$root"` to `herdr worktree list`. Without it herdr can resolve a different repo than the one you're in, and list that repo's lanes instead.

## 2. List the lanes

```bash
herdr worktree list --cwd "$root"   # lanes: worktrees whose branch starts with ticket/
herdr agent list      # each t<n> agent's state
```

For each lane, take the ticket number `n` from the branch name, plus its workspace ID, checkout path and agent state. Read its **base** from `git config branch.<branch>.bliBase`, which `bli lane start` records (lanes started before `bli` have `fanOutBase` instead); when neither is set, the base is `main`. Also look up its PR: `gh pr list --head <branch> --state all --json number,state,url`.

## 3. Act on each lane by state

| Lane state | Action |
|---|---|
| Agent `working` | Leave it. |
| Agent `blocked` | Read the agent (`herdr agent read t<n> --source recent-unwrapped --lines 60`) and show the user the question. Answering it is the user's decision. |
| Agent `idle`/`done`, no PR yet | **Ship it**, as below. |
| PR open | Report the PR link and its checks (`gh pr checks <number>`). |
| PR merged | **Clean it up**, as below. |
| PR closed without merging | Report it, and ask whether to remove the lane. |

**Ship it**
1. Run `git fetch origin`, then read the agent's last output and check `git -C <path> log --oneline origin/<base>..HEAD`. If the agent says it didn't finish, or there are no commits, report the lane as stalled and show the output. Don't open a PR.
2. Otherwise push the branch: `git -C <path> push -u origin <branch>`.
3. Open a PR into the lane's base (`gh pr create --base <base>`). Invoke the `pr` skill for the body, which must include `Closes #<n>` so merging closes the ticket.

**Clean it up**
1. `herdr worktree remove --workspace <ws>`
2. `git branch -D <branch>`
3. `git fetch origin`. If the main checkout is on the lane's base, fast-forward it with `git merge --ff-only origin/<base>`, which works with uncommitted changes where `git pull` (set to rebase) refuses.

This step is done when every lane has had its row's action applied.

## 4. Report

- A table: ticket title (linked), lane state, PR link, and what you did.
- Which tickets merging the open PRs would unblock, read from the blockers of the parent's open sub-issues.
- Once something has merged: new frontier tickets mean `/fan-out` can start the next round.
