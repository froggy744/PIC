# Documentation cleanup review index

These files were **relocated, not deleted**. Review their usefulness before authorizing permanent removal. Git history also preserves the originals.

| Original path | Staged path | Reason |
| --- | --- | --- |
| `docs/superpowers/plans/2026-09-30-sidebar-auto-expand.md` | `to-be-deleted/docs/superpowers/plans/2026-09-30-sidebar-auto-expand.md` | Completed, dated implementation checklist tied to an old feature branch; not a current user guide. |
| `docs/superpowers/plans/sidebar-auto-expand-progress.md` | `to-be-deleted/docs/superpowers/plans/sidebar-auto-expand-progress.md` | Old worktree/session resume checkpoint; contains historical branch-specific instructions, not active product documentation. |

## Safety

- No source, build, runtime assets, test fixtures, network tools, release notes, screenshots or user guides were changed.
- These historical records remain readable in their staged locations.
- Before merging, run `python3 -m unittest scripts.tests.test_documentation_links` and `git diff --check main...HEAD`.
- Do not permanently delete these records until explicitly approved.
- Any references to old historical paths should be reviewed if those records are later removed.
