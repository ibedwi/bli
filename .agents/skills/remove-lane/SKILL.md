---
name: remove-lane
description: Remove one lane with `bli`, after showing the developer what removal would lose and getting their explicit yes.
disable-model-invocation: true
---

# Remove a lane

Tear down one **lane** through `bli`: its workspace and checkout, and its branch only when the developer chooses. You show what removal would lose and ask; `bli` does the removal and enforces the safety rules. Removing a lane is the developer's decision, made with an explicit yes.

Arguments: the lane, named by ticket ID, branch, checkout path or host workspace ID (`/remove-lane 4`). Add `dry-run` to inspect only.

Every `bli` command takes `--json`; read its output rather than its text. On a non-zero exit, show the developer the stderr (its `hint:` lists the candidate lanes when the name matched none or several) and stop.

## 1. Inspect

```bash
bli lane inspect <lane> --json
```

Show the developer the report, phrased as what removal would lose:

- `ticket`, `branch`, `path`, and `status`, the host's hint about the agent. A `working` or `blocked` agent is still running in the lane: say so.
- `modified` and `untracked`: files lost with the checkout.
- `unpushed`: commits on no remote, lost if the branch is deleted.
- `upstream`, and `pr` with its `state` and `url`. A `merged` PR means the work has landed.

With `dry-run`, the run ends here.

This step is done when the developer has seen every non-empty field of the report.

## 2. Ask

Ask three separate questions, each answered on its own:

1. **Remove** the lane's workspace and checkout?
2. **Delete the branch** too? The branch's recorded base goes with it. Without this, the branch stays for later.
3. **Force**, only when `modified` or `untracked` is non-empty, or the branch is being deleted with `unpushed` commits: name exactly the files and commits that would be lost.

Only an explicit yes counts. Anything else, including silence or a hedge, is a no to that question; a no to question 1 ends the run.

This step is done when each question that applies has an explicit answer.

## 3. Remove

Name the lane by its `branch` from the report, which is unambiguous:

```bash
bli lane rm <branch> --json      # add --delete-branch and --force only on an explicit yes
```

On a refusal, show the developer the stderr; retry only with a new explicit yes to the choice it names.

Report what `bli` removed: the lane's path, and whether its branch was deleted (`branch_deleted`) or kept.

This step is done when `bli lane rm` has exited 0 and the developer has the report, or has seen its refusal.
