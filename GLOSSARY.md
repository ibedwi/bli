# bli

A personal CLI for running a development workflow: turning tickets into parallel units of agent work and carrying them through to merged code, independent of which tools host the work or track the tickets.

## Language

### Work

**Ticket**:
One unit of work recorded in an issue tracker, small enough for one agent to build.
_Avoid_: Issue, task, card

**Lane**:
One ticket's isolated unit of work: its branch, the checkout of that branch, the workspace showing it, and the agent working in it. A lane belongs to exactly one ticket.
_Avoid_: Worktree, workspace (each names only a part of a lane)

**Frontier**:
The tickets under a parent that are open, have no open blockers, and are unclaimed — the ones that can be started now.
_Avoid_: Ready queue, backlog

**Claim**:
The mark on a ticket saying a lane has been started for it, so a second fan-out does not start it again.
_Avoid_: Lock, assignment

**Base**:
The branch a lane is cut from and whose pull request it targets.
_Avoid_: Parent branch, target

**Fan-out**:
Starting one lane per frontier ticket so they build in parallel.

**Fan-in**:
Walking every lane and moving each one forward: shipping the finished, surfacing the blocked, cleaning up the merged.
_Avoid_: Collect, sync

**Ship**:
Pushing a finished lane's branch and opening its pull request against the base.

### Providers

**Lane host**:
The tool that provides a lane's checkout, workspace and agent.
_Avoid_: Terminal, workspace manager, runner

**Issue tracker**:
The tool where tickets live.
_Avoid_: Project management, PM tool, board
