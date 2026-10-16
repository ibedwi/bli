# Saved `gh` output

- `sub_issues.json`: real output of `gh api repos/ibedwi/bli/issues/1/sub_issues --paginate`, saved 2026-10-16. #2 is assigned (claimed); every other ticket is blocked, so its frontier is empty.
- `sub_issues_mixed.json`: the same output with #3 unblocked (`blocked_by` 0) and #4 closed and unblocked, so the frontier holds only #3.
- `issue_claimed.json`, `issue_unclaimed.json`: real output of `gh issue view <n> --json number,title,url,state,assignees` for #3 (assigned) and #4 (unassigned), saved 2026-10-16.
- `pr_list_merged.json`: real output of `gh pr list --head ticket/3-fan-out-on-herdr --state all --json number,url,state`, saved 2026-10-16, for the merged PR #12. A branch with no PR gives `[]`.

`gh pr create` prints only the new PR's URL on stdout; it isn't saved here, since capturing it means opening a real PR.
