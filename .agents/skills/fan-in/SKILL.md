---
name: fan-in
description: "Collect the lanes started by `/fan-out` with `bli`: ship finished lanes as PRs, surface stalled or questioning agents, clean up merged lanes, and report what merging the open PRs would unblock."
disable-model-invocation: true
---

# Fan in

Walk every **lane** and move each one forward. `bli` does the mechanical work; you supply the judgement: reading each lane's output to decide whether it is finished, and writing PR bodies. You never merge. Merging is the developer's **gate**, and a lane is cleaned up only after its PR is merged.

Every `bli` command takes `--json`; read its output rather than its text. On a non-zero exit, show the developer the stderr (it carries a `hint:`) and carry on with the next lane.

## 1. List the lanes

```bash
bli lane list --json
```

Each lane has `ticket`, `branch`, `base`, `path`, `workspace`, `status` and `pr` (`null`, or `number`, `url` and `state`: `open`, `merged` or `closed`).

`status` is the lane host's **hint** about the agent: `working`, `blocked`, `idle` or `unknown`. It is advisory: a host can report `idle` for an agent that is waiting on a question, or one that is finished. The lane's output is the evidence; the hint only says which lanes to read.

## 2. Act on each lane

| Lane | Action |
|---|---|
| `pr` is `open` | Report the PR link and its checks (`gh pr checks <number>`). |
| `pr` is `merged` | Leave it for step 3, which cleans it up. |
| `pr` is `closed` | Report it, and ask whether to remove the lane (with `/remove-lane`). |
| No PR, `status` is `working` | Leave it. |
| No PR, any other `status` | **Judge it**, as below. |

**Judge it.** Read the agent's recent output, `bli lane read <ticket> --json` (its `output`; add `--lines <n>` for more than 80 lines), and decide from that output alone which it is:

- **Going**: still mid-task, e.g. running tests or a tool. Leave it.
- **Asking**: ends on a question or a choice for the developer. Show them the question; answering it is their decision.
- **Stalled**: stopped without finishing, e.g. on an error it gave up on, or saying it couldn't complete. Report it with the relevant output.
- **Finished**: says the work is done and committed. **Ship it.**

When the output leaves you unsure, treat the lane as stalled: show the developer and let them decide.

**Ship it.**
1. Write the PR body with the `pr` skill, from the lane's ticket and its commits (`git -C <path> log --oneline origin/<base>..HEAD`), to a file in your scratch directory.
2. `bli lane ship <ticket> --body-file <file> --json`. It pushes the branch and opens the PR against the lane's `base`, titled with the ticket's title; on GitHub Issues it adds `Closes #<ticket>` when the body lacks it. `outcome` is `opened`, or `existing` when the lane already had a PR, which it reports instead of opening another.
3. When `bli` refuses because the lane has no commits beyond its base, report the lane as stalled.

This step is done when every lane has had its row's action applied, and every judged lane has a verdict.

## 3. Clean up merged lanes

```bash
bli lane clean --json
```

It removes every lane whose PR was merged (workspace, checkout, local branch and recorded base), applies the repo's `on_merge` action to each one's ticket, and fast-forwards the main checkout when it is on a cleaned lane's base. It needs no confirmation: the merge was the developer's decision.

Each entry of `lanes` has the lane's fields, its `pr`, and an `outcome`:

- `cleaned`: removed, with `on_merge` the action applied to its ticket. An `error` here means its branch could not be deleted; show it with its `hint`.
- `left`: its PR was closed without merging. `bli` never removes these; they are the closed rows of step 2.
- `failed`: merged, but not removed, e.g. its checkout has uncommitted files or its branch has commits made after the merge. Show the developer its `error` and `hint`; the hint holds the `bli lane rm` that removes it by hand, losing that work, which is `/remove-lane`'s decision, not yours.

`fast_forwarded` is the main checkout's branch when it was brought up to `origin`, else `null`. An exit 1 with JSON on stdout means some of the work failed: report the stderr alongside what succeeded.

This step is done when `bli lane clean` has run and every `failed` lane and stderr message has been shown to the developer.

## 4. Report

- A table: ticket (linked), the verdict or PR state, PR link, and what you did.
- Which tickets merging the open PRs would unblock: an open ticket blocked by an open PR's ticket, whose open blockers all have open PRs. On GitHub Issues, for each open PR's ticket list the tickets it blocks with `gh api 'repos/{owner}/{repo}/issues/<ticket>/dependencies/blocking'`, then each open one's blockers with `.../issues/<blocked>/dependencies/blocked_by`, keeping those whose blockers with `state` `open` are all open PRs' tickets, counting PRs shipped in step 2. Say "none" when nothing would.
- Once something has merged: new frontier tickets mean `/fan-out` can start the next round.
