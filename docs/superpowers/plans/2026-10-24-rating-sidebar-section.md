# Ratings Sidebar Collection Implementation Plan

**Goal:** Add a library-wide Ratings destination and five exact rating destinations beneath Favourites, independent of the toolbar metadata filter.

**Architecture:** Use `SidebarFilter::Ratings` and `SidebarFilter::Rating(u8)` as normal Library sidebar destinations. Query all nontrashed photos, then apply the selected sidebar rating. Keep the existing toolbar filter for other destinations. Persist the destination through the existing last-view key.

**Spec:** `docs/superpowers/specs/2026-10-24-rating-sidebar-section-design.md`

## Tasks

1. Add nontrashed 1–5 star counts to `SidebarCounts`, with a focused database test.
2. Add Ratings and star child rows to the Library `ListBox`, using standard destination selection. Keep children collapsed until the arrow or a title double-click expands them; update counts without rebuilding rows. Add a GTK test.
3. Add saved keys and gallery queries for both destination types. Add a test for toolbar independence and Favourites membership after navigating away from Ratings.
4. Refresh Ratings views after a photo rating changes. Check startup and library switch paths.
5. Run targeted tests, `cargo check --locked`, and `git diff --check`.
