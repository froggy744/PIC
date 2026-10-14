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

## Additional historical documentation staged on main (2026-10-14)

The following dated development records were relocated without changing their content. They are retained for review, **not approved for permanent deletion**:

| Original document | Reason for staging |
| --- | --- |
| `docs/superpowers/plans/2026-09-11-responsive-bookshelf-layout.md` | Historical implementation plan for the bookshelf layout |
| `docs/superpowers/plans/2026-09-16-lightbox-stabilization.md` | Historical lightbox stabilization plan |
| `docs/superpowers/plans/2026-09-23-history.md` | Historical Phase 1 history implementation checklist |
| `docs/superpowers/specs/2026-09-11-responsive-bookshelf-layout-design.md` | Historical design specification accompanying the bookshelf plan |

Each file is preserved at `to-be-deleted/<original path>`. Current guides, release notes, QA checklists with unresolved items, build instructions, runtime assets, scripts, and source files are untouched. These relocations have not been locally compiled or tested; run the documentation link test and `git diff --check` before further deletion.
