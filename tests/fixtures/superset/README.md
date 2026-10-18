# Superset output

**Derived, not captured.** Superset was not installed when these were written, so each file is built from the types in Superset's source (`github.com/superset-sh/superset` at `0e0c87f7`, CLI 1.38.0), not saved from a real run. Swap each for real output during the one-off manual check against a throwaway scratch lane, the way `../herdr` was captured.

The CLI prints a command's data as pretty JSON with no wrapper, and reports every error as plain text on stderr (`Error: <message>` and an optional `Hint: <hint>` line) with exit code 1.

- `projects_list.json`: `superset projects list --local --json`, from `commands/projects/list/command.ts`.
- `workspaces_create.json`: `superset workspaces create --local --project <id> --name <label> --branch <branch> --skip-branch-prefix --json`, a `WorkspaceCreateResult`. It carries no worktree path.
- `workspaces_get.json`: `superset workspaces get <id> --local --json`, from `hostDetail` in `commands/workspaces/get/command.ts`.
- `workspaces_list.json`: `superset workspaces list --local --project <id> --json`, with the project's own checkout (`type: "local"`), a lane, and a workspace on a branch outside the pattern.
- `snapshot.json`: `superset snapshot --json`, from `buildSnapshot` in `lib/host-snapshot.ts`. Agent states are `starting`, `working`, `blocked` or `idle` (`agentRunState` in `packages/shared/src/agent-status.ts`).
- `agents_create.json`: `superset agents create --local --workspace <id> --agent claude --prompt <prompt> --json`, an `AgentRunResult`. Its `sessionId` is the agent's terminal.
- `agents_wait.json`: `superset agents wait --local --workspace <id> --terminal <id> --until working --timeout 20 --json`.
- `error_wait_timeout.txt`: the stderr of `agents wait` when it times out, from `commands/agents/wait/command.ts`.
- `terminals_read.json`: `superset terminals read --local --workspace <id> --terminal <id> --max-lines 4 --json`.
- `workspaces_archive.json`: `superset workspaces archive <id> --local --json`. It always forces, and keeps the branch.
- `agents_list.json`: `superset agents list --local --json`, the host's agent presets with the built-in Claude preset's `--dangerously-skip-permissions`.
- `status_running.json`, `status_not_running.json`: `superset status --json`, from `commands/status/command.ts`.
- `error_not_logged_in.txt`: the stderr of any command run with no login, from `lib/resolve-auth.ts`.
