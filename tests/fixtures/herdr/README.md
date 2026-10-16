# Saved herdr output

Real output of herdr 0.9.3, saved 2026-10-16 using a throwaway `scratch/bli-fixture` lane, removed afterwards.

- `worktree_create.json`: `herdr worktree create --cwd <main checkout> --branch scratch/bli-fixture --base origin/main --label "#0 bli-fixture" --no-focus --trust-repository`.
- `worktree_list.json`: `herdr worktree list --cwd <main checkout>` with the main checkout, the scratch lane, and the `ticket/3-fan-out-on-herdr` lane.
- `agent_list.json`: `herdr agent list`, with agent `t3` working in workspace `w2`.
- `agent_wait.json`: `herdr agent wait t3 --until working --timeout 2000`.
- `agent_read.txt`: `herdr agent read t5 --source recent-unwrapped --lines 4`, the bottom of a Claude pane. Unlike the other commands, `agent read` prints plain text, not JSON.
- `error_agent_read_not_found.json`: the stderr of `herdr agent read` on an unknown agent.
- `error_timeout.json`: the stderr of `herdr agent wait t3 --until blocked --timeout 100` while `t3` was working.
- `error_agent_not_found.json`: the stderr of `herdr agent wait` on an unknown agent. herdr reports errors this way, as JSON on stderr with a non-zero exit.

`herdr worktree create --cwd <linked worktree>` fails with `linked_worktree_source`, which is why `bli` always passes the main checkout.
- `status_server_running.json`: `herdr status server --json` with the server running. Unlike the socket API commands, its reply is not wrapped in `result`.
- `status_server_not_running.json`: `herdr --session bli-fixture status server --json`, for a session with no server. It exits 0 all the same.
