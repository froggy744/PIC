# Lightbox 1:1 geometry investigation — 2026-10-02

Baseline: `23cebc36c89a15874451c115e521c61d23181072`.
The current change includes geometry diagnostics and a fix for the confirmed
delayed viewport allocation.

## Code and history findings

Inspected lightbox construction, rendering and navigation; picture requests,
alignment and expansion; the implicit GtkViewport; adjustment centring; drag,
double-click, outside-click and context-menu controllers; focus requests; cursor
updates; resize/open/transition ticks; and the editor's pan path.

Reviewed `3eb263d` (one centring owner), `fd92bdf` (range-change anchoring), and
the reverted pan/focus/kinetic/controller experiments leading to the baseline.

Both cached 1:1 and newly decoded native 1:1 paths now use
`fit_one_to_one_picture`, which applies `fit_picture(-1.0)` (Start alignment on
overflowing axes) and primes the centered adjustment ranges from the native
picture size before GTK's queued viewport allocation. Before this change, the
decode path retained Fit's Fill alignment, expansion and viewport-sized request;
the user's trace also showed stale child bounds on the cached path, so that
alignment difference alone did not explain the jump.

The editor already pans through a stationary ScrolledWindow using the same
adjustment-minus-pointer-displacement calculation. Copying that calculation
does not address whether lightbox child bounds match the adjustments.

## Runtime evidence and confirmed cause

The user's 1510×1062 desktop trace recorded the expected horizontal adjustment
2253 and vertical adjustment 1477. The pointer press leaves bounds at `(0,0)`;
on the first drag update, less than one pixel of pointer movement changes the
painted bounds to `(-2253,-1476)`. This is the jump.

[GtkViewport's allocation implementation](https://github.com/GNOME/gtk/blob/4.22.5/gtk/gtkviewport.c#L459)
freezes adjustment notifications, configures the ranges, allocates the child,
and then thaws notifications.
[GtkAdjustment's property dispatcher](https://github.com/GNOME/gtk/blob/4.22.5/gtk/gtkadjustment.c#L301)
emits `changed` when those notifications are dispatched. Thus PIC's centring
handler can run after child placement, contrary to the baseline comments.

The adjustment remains centred within one pixel. Later drag updates track
their pointer deltas correctly. The cached 1:1 path also starts with stale
bounds, though it already has Start alignment and native size requests, so
alignment alone is not the cause.

GTK configures the child-size ranges while allocating the implicit viewport,
then delivers its `changed` signal after child placement. PIC's centering
handler writes the correct adjustment during that notification, but that
write comes too late to position the child in the same allocation. The first
pan write queues the next allocation, which applies the already-correct centre
and creates the jump.

The fix primes the centered ranges from the native picture size before queued
layout, in both cached and newly decoded 1:1 paths. The adjustment remains the
sole centering owner; the drag calculation is unchanged. The ignored GTK
regression test asserts that the native picture bounds and adjustment agree
before any pointer update. Run it on the desktop with
`cargo test native_picture_is_centered_before_the_first_pointer_update
-- --ignored --test-threads=1`.

## Verification

- `cargo check`: passed; existing compiler warnings remain.
- `cargo test lightbox::`: 24 passed; the GTK regression test requires an
  explicit `--ignored --test-threads=1` desktop run.
- GTK regression test: passed with desktop access. GTK allocated a 203×81
  viewport, rather than the requested 200×120 window size. The test now derives
  the expected centre from the actual adjustment page sizes and requires
  overflow on both axes. The resulting adjustments `(98,110)` and picture
  bounds `(-98,-110)` agree before any pointer update.
- Controlled reproduction: temporarily disabling only the range-priming call
  made the same GTK test fail its picture-origin assertion. Adjustments remained
  `(98,110)` but picture bounds were `(0,0)`. Restoring range priming made the
  test pass again.
- Unfiltered `cargo test` with desktop access: 513 passed, 54 ignored,
  zero failures or filtered tests. Earlier sandbox-only cache, D-Bus and SMB
  restrictions no longer apply to this final verification run.
- `git diff --check`: passed. Repository-wide `cargo fmt --check` reports
  existing formatting differences across unrelated files.
- The production change is limited to priming 1:1 viewport ranges before
  allocation. Loading, PNG transition, Space, slider zoom, centring math and
  pan adjustment math are unchanged.
