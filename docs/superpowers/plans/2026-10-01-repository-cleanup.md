# Repository Cleanup Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Status:** Proposed plan; no files have been relocated. The user selected a short root README linking into docs and authorized the six local deletions listed below to be committed and pushed.

**Goal:** Reduce the repository root to project entry files while keeping documentation links, build tools, diagnostics and packaged runtime resources working.

**Architecture:** Keep a short root README; put full documentation under docs and executable tooling under scripts. Resolve repository paths independently of script location and invocation directory. Separate mechanical moves from consolidation of older tools.

**Tech Stack:** Rust/Cargo, Bash, PowerShell, Python unittest, Markdown, GTK GResource, AppImage/Flatpak/deb/rpm packaging.

**Spec:** The user request and [proposed layout](#proposed-layout) in this document; the root README decision is confirmed, and the remaining mappings are proposed for review.

## Global Constraints

- Keep a short root README.md linking into docs.
- Keep Cargo.toml, Cargo.lock, build.rs and .gitignore at the root.
- Keep src/, native/, images/, themes/, icon/, resources/ and tests/ at their current locations during this cleanup.
- Preserve application data and the pic-rs storage migration implemented for issues #145 and #146.
- Preserve PIC_* environment overrides, dependency-installation consent, offline build behavior and package IDs.
- Do not delete build caches, downloads, logs, worktrees, screenshots or reference code as part of a move. The six user-authorized deletions below are already part of this cleanup.
- Preserve executable file modes and use git mv for tracked relocations.
- Keep all documentation links and screenshot references relative to the repository.
- Do not run packaging that installs packages, modifies the desktop, downloads dependencies or contacts network shares merely to verify file moves.

## Audit Evidence

Audit on 2026-10-01, starting from commit 10a788e:

- 32 tracked root files; the target root has five files: .gitignore, Cargo.toml, Cargo.lock, build.rs and README.md.
- 70 active local documentation links checked outside historical superpowers plans/specs; six are broken. All six target the absent README.release.candidate.md from the navigation, folders, library, edit and crop guides. Active screenshots resolve.
- PIC-build-linux-one-script.sh and older Linux packagers use their own directory as default project, output, Flatpak state and log directories. Moving them without repairing this would send work into scripts/.
- build-deb.sh and build-rpm.sh use their own directory as ROOT and optionally invoke a nonexistent build-linux-icons.sh. Their icon candidates do not include the tracked icon/pic-<size>.png layout, so available package icons can be missed.
- PIC-download-linux-dependencies.sh opens PIC-build-linux-one-script.sh relative to the working directory. Its FLATPAK_MODULE_SOURCES source list exists; the problem is path resolution, not a missing list.
- generate-icon.sh defaults to absent PIC-icon.png and writes into icons/, while tracked application icons are under icon/.
- resources/build-icon-bundle.sh derives resource output paths from its own directory; move requires a distinct resource directory.
- scripts/tests/test_build_dependencies.py and test_flatpak_test_sandbox.py target the root build script explicitly.
- scripts/test-shares.sh uses scripts/pic-smb-probe.c but root pic-nfs-probe.c; both probes should live together.
- build.windows.ps1 deliberately packages the executable in the caller's current directory. Moving it must preserve that input directory behavior.
- build-linux.sh and build-linux-icons-only.sh have approximately 98.5% line similarity. This is evidence for a separate consolidation review, not evidence that either can be deleted safely.
- big_pickle_network_reference.rs is reference code outside src/ and is not registered by src/main.rs.
- tests/20151128_144228.jpg is used by src/thumbnail/cache.rs. It is a required test fixture, not clutter.
- Baseline: 20 Python script tests pass; audited Bash scripts pass bash -n. Those checks do not currently establish that every package gets an icon or that every script works from another directory.

## User-authorized Deletions

After the audit, the user deleted these tracked files locally and authorized their removal from GitHub:

- ANIMATION_BASELINE.md
- DEVELOPMENT_HANDOVER.md
- TODO-HISTORY.md
- TODO-TextTweaks.md
- build-linux.sh
- build-linux-icons-only.sh

These files remain recoverable from Git history and are not candidates for relocation or restoration. README-PORTABLE-NFS.md now uses PIC-build-linux-one-script.sh in its current build examples. Historical notes and plans retain their original names/commands as records. The original 32-file root audit above describes the baseline before these deletions; 26 tracked root files remain before the proposed moves.

## Proposed Layout

```text
README.md                         short project entry and documentation links
Cargo.toml / Cargo.lock / build.rs / .gitignore
src/ native/                      application and linked native code
images/ themes/ icon/ resources/   existing runtime/embedded resources
samples/ tests/                    screenshots and test fixture, preserved
scripts/                          build, download, asset and diagnostic tools
  diagnostics/                    C probe sources
  tests/                          existing Python script tests
docs/                             full documentation
  README.md                       full current project overview
  guides/                         user guides
  build/                          Linux and portable-network build instructions
  releases/                       release candidate notes
  development/                    handover, technical notes, backlog and references
  theme-template/                 existing documented theme template
  superpowers/                    existing plans and specs
```


### Documentation mappings

| Current path | Proposed path |
| --- | --- |
| README.md full contents | docs/README.md; replace root with short entry page |
| README.Start.md | docs/guides/getting-started.md |
| README.navigation.md | docs/guides/navigation.md |
| README.folders.md | docs/guides/folders.md |
| README.library.md | docs/guides/library.md |
| README.edit-mode.md | docs/guides/edit-mode.md |
| README.crop-mode.md | docs/guides/crop-mode.md |
| README-LINUX.md | docs/build/linux.md |
| README-PORTABLE-NFS.md | docs/build/portable-nfs.md |
| README.ReleaseCandidate2.md | docs/releases/rc2.md |
| README.ReleaseCandidate3.md | docs/releases/rc3.md |
| ABOUT ME.md | docs/about-author.md |
| SUMMARY.md | docs/development/summary.md |
| notes/*.md | docs/development/notes/, retain basenames |
| resources/icons/README.md | docs/development/icon-bundle.md |
| big_pickle_network_reference.rs | docs/development/reference/network-browser.rs |

Keep docs/theme-template/README.md inside its existing docs directory. Keep docs/GALLERY_V2.md and docs/THEMES.md in place initially and link them from the documentation entry page. Repair the nonexistent release-candidate backlinks by linking user guides to docs/README.md, which owns the guide index. Preserve screenshot filenames and samples/; the new document depths determine ../samples/ or ../../samples/ paths. Historical absolute workspace paths in existing plans/notes remain identified as historical records rather than being rewritten as current instructions.

### Tool mappings

Move these root tools into scripts/, retaining their basenames initially:

- PIC-build-linux-one-script.sh
- PIC-download-linux-dependencies.sh
- build-deb.sh
- build-rpm.sh
- build.windows.ps1
- build-resize-PNG-AlbumCovers.sh
- generate-icon.sh

Also move resources/build-icon-bundle.sh to scripts/build-icon-bundle.sh; move pic-nfs-probe.c and scripts/pic-smb-probe.c into scripts/diagnostics/. Keep scripts/test-shares.sh, scripts/verify-native-link-order.sh and scripts/tests/ where they are.

## Review Focus

1. Scripts invoked from an unrelated directory or a checkout path containing spaces must resolve repository resources correctly.
2. Default project and artifact paths must stay at repository root; explicit PIC_* overrides must still win.
3. Documentation moves must preserve screenshot rendering, previous/next guide links and theme-template instructions.
4. Icon generation and package icon discovery must agree on the real icon/ layout; bundle generation must still publish to resources/.
5. Legacy packagers, offline dependency cache and the Windows caller-directory contract must not change silently during mechanical moves.

## Task 1: Relocate documentation and repair links

**Files:** Documentation mappings above; root README.md; docs/theme-template/README.md where references need adjustment; create scripts/tests/test_documentation_links.py.

**Interfaces:** Root README remains the GitHub entry. docs/README.md becomes the full overview and guide index. All active links resolve relative to their containing file.

- [ ] Add a documentation link check covering the root entry, user/build/release guides and asset documentation. Strip HTML comments and fenced code examples; check relative Markdown links and HTML img sources, URL-decoding spaces and ignoring external URLs and fragments. Exclude historical plans/specs from this active-documentation gate.
- [ ] Run it against the baseline and confirm the six missing README.release.candidate.md targets are reported.
- [ ] Relocate documents using the mapping, replace root README with a concise description, repository link, documentation link and the new main build command.
- [ ] Update guide navigation and screenshot paths based on each document's actual depth. Update template and asset-documentation links, and current build commands to scripts/ paths.
- [ ] Run the link check and review the root/full README rendering, including two screenshots and guide navigation.
- [ ] Commit this independently as documentation organization; do not move build tools in the same commit.

## Task 2: Move tools and preserve path contracts

**Files:** Tool mappings above; scripts/tests/test_build_dependencies.py; scripts/tests/test_flatpak_test_sandbox.py; add scripts/tests/test_repository_paths.py; current documentation commands.

**Interfaces:** For repository-based Bash tools, SCRIPT_DIR is scripts/ and REPO_ROOT is its parent when that parent contains Cargo.toml. A standalone downloaded main packager keeps SCRIPT_DIR as its working/output base and can still fetch GitHub source. For tools accepting a source checkout, --project and existing PIC_* overrides retain precedence. Windows input stays the caller's executable directory.

- [ ] Add behavior checks for the relocated main packager invoked from the checkout root, from another working directory and through a path containing spaces. Mock dependency/build commands using the existing test harness; confirm default source is the repository and outputs/logs/state do not land inside scripts/.
- [ ] Add a standalone-copy main-packager check: GitHub mode does not require a local Cargo.toml, and artifact/log defaults stay beside the downloaded script rather than unexpectedly targeting its parent directory.
- [ ] Add a downloader check with fake network commands/cached files: it finds its sibling build script regardless of working directory and still enumerates all three pinned module archives.
- [ ] Move root scripts into scripts/ and update existing tests to the new build-script location.
- [ ] Separate script location from repository root. Update default project, dist, logs and Flatpak state paths, prompts, help examples, companion-script lookups and usage documentation. Preserve explicit overrides.
- [ ] Move the icon-bundle tool and resolve ICON_DIR, custom-icons, XML and GResource output under REPO_ROOT/resources rather than SCRIPT_DIR.
- [ ] Move both C probes into scripts/diagnostics/ and update scripts/test-shares.sh source paths. Keep probe binaries under target/diagnostics/.
- [ ] Preserve build.windows.ps1's caller directory contract, and document invocation as a path to scripts/build.windows.ps1 from the executable folder. Do not add a repository-root default that packages the wrong executable.
- [ ] Run all Python script checks with PYTHONDONTWRITEBYTECODE=1 and Bash syntax checks. Run the compiler/native link-order check where its documented prerequisites are available. Review generated Flatpak manifests and launcher resource paths using mocked package commands.
- [ ] Commit tool relocation and path repairs together; no full release packaging is required to prove a filesystem move.

## Task 3: Repair icon path mismatches uncovered by the audit

**Files:** scripts/generate-icon.sh, scripts/build-deb.sh, scripts/build-rpm.sh, scripts/build-icon-bundle.sh, docs/development/icon-bundle.md; script test fixtures.

**Interfaces:** Tracked application icons stay at icon/pic-<size>.png. GResource icons stay at resources/icons/. Original artwork is an explicit generator input, not an invented filename.

- [ ] Add package-staging checks with fake build/package commands proving the existing icon/pic-<size>.png assets appear in deb/rpm payloads.
- [ ] Remove the misleading optional call to the absent build-linux-icons.sh, or replace it with a documented, validated asset-generation interface. Do not redirect it to build-linux-icons-only.sh, which is a full packager.
- [ ] Include tracked icon/pic-<size>.png in package icon discovery. Make the generator accept an explicit source path and output to icon/; fail clearly when input is absent. Avoid silently resampling existing artwork as an assumed design choice.
- [ ] Verify bundle regeneration against a temporary resource fixture, not by deleting/rebuilding the entire checked-in icon set during a test.
- [ ] Update stale asset instructions only after checking them against current src/main.rs and bundle behavior.
- [ ] Run script tests, inspect staged file paths and commit the icon repairs separately from the mechanical moves.

## Task 4: Group development references and define generated-file policy

**Files:** Remaining development/reference mappings; .gitignore; source-copy exclusions in scripts/PIC-build-linux-one-script.sh.

- [ ] Move the unused reference module into docs/development/reference/ without registering it in the application. Move notes into docs/development/notes/ and update active references.
- [ ] Keep target/, dist/, build-logs/, .flatpak-builder/, logs/ and .worktrees/ ignored and in place. Inventory .superpowers/ ownership before deciding whether it belongs in .gitignore; never remove session/worktree state as repository clutter.
- [ ] Add narrowly scoped ignores for Python __pycache__/ and generated Windows transcript logs. Avoid blanket *.zip ignores: first distinguish user archives from reproducible build products.
- [ ] Audit copy_source_tree exclusions. Prevent worktrees and generated logs/caches from being copied into packaging source trees, including recursive worktree copies. Keep source assets, scripts, tests, Cargo.lock and native sources available.
- [ ] Check a temporary source-copy fixture contains everything needed for an offline build and excludes generated directories. Do not actually build or clean live caches for this check.
- [ ] Confirm tests/20151128_144228.jpg, runtime resource trees and compiled icon bundle remain available at their original paths.
- [ ] Run git diff --check and inventory the expected five root files. Commit the generated-file/reference cleanup.

## Task 5: Verify the user-authorized legacy-tool retirement

**Files:** scripts/PIC-build-linux-one-script.sh after relocation; docs/build/; current command examples.

- [ ] Check current documentation and tooling for references to the deleted build-linux.sh and build-linux-icons-only.sh. Replace current instructions with the remaining main packager where its options apply; leave clearly historical records intact.
- [ ] If a supported workflow still needs a capability from a retired script, compare the implementation in Git history and document the gap before making a separate change to the main packager.
- [ ] Do not restore retired scripts or the four removed development documents during the file moves. The user's explicit deletion decision overrides the initial consolidation proposal.

## Final Validation and Handoff

- [ ] All active documentation links/screenshots resolve; root README is readable on GitHub.
- [ ] All script unit checks pass, including invocation-location, downloader, package-icon and source-copy regressions.
- [ ] Bash tools pass bash -n; PowerShell is checked on Windows or with pwsh if available, with any platform validation gap stated.
- [ ] cargo check --locked confirms native sources and compile-time resource references remain valid.
- [ ] Run affected thumbnail-fixture and runtime-resource discovery tests, plus scripts/verify-native-link-order.sh where available.
- [ ] Review the staged diff for unintended asset changes/deletions, missing executable bits and unrelated local files.
- [ ] Report which checks passed and any untested package/platform paths. Full AppImage/Flatpak/deb/rpm/Windows builds can follow with explicit package-test scope; they are not claimed based on syntax tests alone.
- [ ] Request review before integration. Commit/push the implementation only when authorized; do not close issues merely from the plan.
