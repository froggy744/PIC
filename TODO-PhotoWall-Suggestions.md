Yes. The overall Photo Wall plan is good, but I’d change a few things **before letting OpenCode/Codex implement it**. There are two important mismatches between the plan and the source files you uploaded.

### 1. Verify the checkout first — the plan and code do not completely agree

Your plan says the active tile already supports independent width/height and that the scanner stores display-oriented dimensions. :chatgpt-content-reference{index="0"}

But the uploaded `grid.rs` shows `SquareTile` still has a single `size` value, reports square measurements, and allocates its child as `size × size`. :chatgpt-content-reference{index="1"} :chatgpt-content-reference{index="2"}

That matters a lot for Photo Wall. Rectangular wall cells cannot simply reuse this widget unchanged.

I would change Task 3 so it explicitly says:

> Before implementation, verify that the checked-out active `SquareTile` supports independent width and height. If it still uses one `size`, refactor it minimally to support rectangular allocations while preserving existing Grid behaviour.

That should happen **before** building Photo Wall around an assumption that isn't true.

---

### 2. The EXIF/dimension assumption also needs checking

The plan says:

> scanner now saves display-oriented dimensions, swapping axes for EXIF orientations 5–8. :chatgpt-content-reference{index="3"}

But your uploaded scanner currently obtains dimensions and then stores them directly:

`width → PhotoMetadata.width`
`height → PhotoMetadata.height`

There is no visible orientation-based width/height swap in this code path. :chatgpt-content-reference{index="4"} :chatgpt-content-reference{index="5"}

Meanwhile thumbnail generation **does** apply EXIF orientation to the decoded image. :chatgpt-content-reference{index="6"}

And then your separate user rotation is applied again only for the database rotation value, which is correct conceptually. :chatgpt-content-reference{index="7"}

So this is probably the single biggest technical issue I would resolve first.

I would define one very clear invariant:

> **Database `width × height` always represents the photograph after embedded EXIF orientation, but before PIC's user rotation.**

Then:

```text
file pixels
    ↓
EXIF orientation
    ↓
database width × height
    ↓
PIC user rotation
    ↓
Photo Wall aspect ratio
```

That removes ambiguity everywhere.

---

### 3. Don't make Photo Wall responsible for repairing old metadata

Your Task 2 currently combines two fairly risky features:

- Photo Wall
- legacy metadata migration/repair

I'd separate them.

Photo Wall should do:

```rust
photo_wall_aspect_ratio()
```

with:

```text
valid width/height → use them
invalid/missing → 1.0 fallback
rotation 90/270 → invert ratio
```

No disk access. No EXIF reads. No network access.

That matches the plan's excellent rule that layout calculation performs no filesystem access or decoding. :chatgpt-content-reference{index="8"}

Then make metadata normalization a separate maintenance job.

Otherwise a simple UI feature can unexpectedly turn into:

> Photo Wall → metadata repair → NFS access → RAW metadata → scanner → DB updates → layout invalidation

Given the NFS problems you've already worked through, I would keep those paths very deliberately separated.

---

### 4. Add a geometry generation number

You already mention stale restoration callbacks, but I'd formalize it.

Something like:

```rust
struct PhotoWallState {
    generation: u64,
    layout: PhotoWallLayout,
}
```

Increment generation whenever these change:

```text
viewport width
zoom
photo collection
folder/group boundaries
metadata dimensions
rotation
caption visibility
```

Every deferred anchor/reflow callback captures the generation.

Then:

```rust
if captured_generation != current_generation {
    return;
}
```

This will save you from a lot of unpleasant resize → source change → scroll restore races.

Your plan already identifies this class of problem. :chatgpt-content-reference{index="9"}

---

### 5. Keep Photo Wall geometry completely dumb

This part of the plan is exactly right and I would enforce it strongly:

```rust
PhotoWallLayout::calculate(
    aspect_ratios,
    sections,
    viewport_width,
    target_height,
    caption_height,
)
```

It should know absolutely nothing about:

```text
PhotoObject
GTK
SQLite
paths
thumbnails
EXIF
selection
network shares
albums
```

Only indexes and numbers.

That lets you throw 50,000 synthetic images at it in a unit test without GTK.

The plan already heads in exactly this direction. :chatgpt-content-reference{index="10"}

---

### 6. I'd simplify the row algorithm slightly

The proposed "candidate before or after threshold, whichever is closest to target" is good.

But I would explicitly clamp the justified height:

```rust
row_height = calculated_height.min(target_height * MAX_STRETCH)
```

Something like:

```rust
const MAX_JUSTIFIED_SCALE: f64 = 1.20;
```

Otherwise a row containing two narrow portrait photos can become comically tall.

Similarly, consider a lower bound:

```rust
target_height * 0.65
```

If a row would have to shrink much farther than that, move an item onto the next row.

This tends to make justified layouts feel much more like Google Photos/Flickr instead of mathematically justified but visually erratic.

---

### 7. Don't use a hard-coded 180-widget pool limit as an architectural rule

Your plan correctly observes that the existing limits are spare-widget limits rather than absolute viewport limits. :chatgpt-content-reference{index="11"}

I'd make the invariant:

```text
live widgets =
    widgets intersecting viewport+overscan
    + bounded reusable spare pool
```

Not:

```text
live widgets <= 180
```

An ultrawide 4K display at the smallest zoom could legitimately need a large number of visible tiles.

---

### 8. Preserve selection by ID, not index, during layout changes

This is especially important because your `Gallery` currently maintains a `MultiSelection` over the list store. :chatgpt-content-reference{index="12"}

Grid → Photo Wall should never rebuild/reorder the photo model just to change appearance.

Ideally:

```text
same ListStore
same MultiSelection
same PhotoObjects
same ordering

different geometry only
```

Your plan says this already, and I would make it a **hard acceptance requirement**, not merely an architectural preference. :chatgpt-content-reference{index="13"}

---

### 9. Use one anchor algorithm everywhere

I wouldn't write separate anchor logic for:

- resize
- zoom
- Grid → Wall
- Wall → Grid
- sidebar open/close

Make one structure:

```rust
struct ViewAnchor {
    photo_id: i64,
    viewport_y_offset: f64,
}
```

Then:

```rust
capture_anchor()
change_geometry()
restore_anchor()
```

Every geometry-changing operation uses it.

That will substantially reduce bugs later.

---

### 10. I like your decision about the bottom-right control

This is important:

> don't replace the Collage button accidentally.

Your plan correctly identified that the existing control is Collage, not a Grid/Wall toggle. :chatgpt-content-reference{index="14"}

I would make the menu:

```text
▦
────────────
● Grid
○ Photo Wall
────────────
Create Collage…
```

rather than adding another button to the already busy bottom toolbar.

---

### 11. I'd add one more acceptance test: **no full image read during Wall switching**

This would be a very valuable regression test given your network-share history.

Turn on tracing and assert that:

```text
Grid → Photo Wall
Photo Wall zoom
Photo Wall resize
Photo Wall scrolling
```

produce **zero calls** to:

```rust
source::read()
source::materialize()
```

unless an existing thumbnail/presentation worker independently needs the original.

Your source abstraction can potentially trigger full remote reads through `load_contents`. :chatgpt-content-reference{index="15"}

You absolutely do not want Wall layout calculation accidentally reaching that path.

---

### 12. I would change the implementation order slightly

Your current order is:

> Geometry → metadata/orientation → recycled rendering → viewport/navigation → menu... :chatgpt-content-reference{index="16"}

I'd use:

**Geometry → rectangular tile support → basic Wall rendering → navigation/virtualization → EXIF/legacy metadata repair → UI switch/zoom → torture testing.**

That gets something visible working sooner and isolates layout bugs from metadata migration bugs.

### The two things I'd fix in the plan before coding

The biggest ones are:

1. **Re-check `SquareTile`: the uploaded implementation is square-only.**
2. **Define and verify the width/height orientation invariant before touching Photo Wall.**

Apart from those, I think the architecture choice is strong. Extending the existing recycled/sectioned surface is much better than introducing a second gallery implementation, and your constraints around virtualization, no filesystem work in geometry, preserving Grid, and keeping selection/model identity are exactly the right ones. :chatgpt-content-reference{index="17"}

If this were my next step, I would amend the plan with those two corrections **before giving it to Codex/OpenCode**, because both could otherwise cause a fairly large rework halfway through implementation.
