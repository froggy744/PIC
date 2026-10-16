# PIC Guide 4 — Edit Mode: Quick Fixes, Sliders & Filters

> **PIC V1 guide.** Screenshots are being refreshed for the updated interface; the older images have been hidden until replacements are ready.

*Part of the [PIC user guides](../../README.md).*

PIC's Edit Mode is for the everyday fixes — the ones you used to open a heavy photo editor for. Brighten a dark room, warm up a sunset, straighten a horizon, try a filter. Done in a minute, without ever leaving your library.

**And it's completely safe:** editing is *non-destructive*. Your original file is never overwritten. Every edit is just a recipe saved in PIC's library — Reset always brings the photo back to exactly how it was.

---

## Opening Edit Mode

You have three easy ways in:

- Select a photo and click the **✏️ pencil** in the bottom info bar,
- **Right-click** the photo and choose Edit, or
- Press the Edit button while viewing the photo large.

The photo area becomes the editor: controls on the left, your photo big on the right, and the photo's details still visible in the bottom bar.

<!-- V1 SCREENSHOT: updated Edit Mode capture to be added. -->

[Download the Edit Mode demonstration (MP4, 20 seconds)](https://raw.githubusercontent.com/froggy744/PIC/main/screenshots/edit-mode.mp4).

At the top you'll find the **back arrow**, **Undo / Redo**, **Reset**, **Export** and **Done**. The shared zoom slider in the bottom bar controls editor magnification. Use the 1:1 control to inspect detail.

## The Five Tabs: Tools · Filters · Crop · Overlays · Text

The left panel has five tabs:

### 🛠 Tools — sliders and one-click fixes

This is where the everyday corrections live, in three friendly groups:

**Quick fixes** are on the Filters tab: Auto Contrast, Auto Colour, B&W, and Sepia. The automatic tools use conservative image-adaptive adjustments, not guaranteed exposure or white-balance recovery.

**Filter Amount** sits immediately below Auto Contrast and Auto Colour. Adjust the strength of Newspaper, Old Photo, any named filter, B&W or Sepia from **0% (unfiltered) to 100% (full style)**. The slider is disabled while Original is selected. It affects only the creative filter, not Auto Contrast/Colour, Fill Light, Shadows or other manual tools. Amount is stored in the non-destructive recipe; older saved photos default to 100%.

**Light:**
- **Exposure** — brighten or darken the whole photo
- **Contrast** — punchy or soft
- **Fill Light** — broadly lift lower midtones using a luminance-preserving curve
- **Highlights** — darken or brighten bright regions without affecting deep shadows
- **Shadows** — independently open or deepen the darker tonal regions

**Colour:**
- **Saturation** — from gentle to vivid
- **Warmth** — cooler blues or golden warmth

**Detail:**
- **Sharpen** — enhance small-scale image edges
- **Clarity** — increase or soften midtone local contrast without pushing all colours
- **Grain** — add deterministic monochrome film texture; zero leaves the image unchanged

**Geometry:**
- **Straighten** — one slider to level a crooked horizon

Move any slider and the preview updates instantly. The little numbers next to each slider show exactly how far you've pushed it.

### 🎨 Filters — one-tap looks with live previews

The Filters tab shows your photo as a grid of **live preview tiles** — every filter rendered on *your actual photo*, so what you see is exactly what you get. Try **Clarendon, Gingham, Juno, Lark, Valencia, Walden**, plus fun ones like **Pencil Sketch, Disposable Film, Digicam Flash, Dream Glow, Cinematic** and **Golden Hour**, **Old Photo** (warm, faded film texture), and **Newspaper** (warm paper and ordered black-ink halftone). Click a tile to apply it, click **Original** to return to the unfiltered recipe.

Filter looks, Clarity, Grain, Auto Contrast and Auto Colour all use the same non-destructive recipe for editing and export. Auto Contrast protects against extreme percentile stretches; Auto Colour uses near-neutral areas rather than neutralising intentional sunsets or colourful subjects.

### ✂️ Crop — trim and straighten

Full details in the next guide: [Crop Mode →](crop-mode.md)

### 🖼 Overlays — logos, badges & banners

Drop a PNG or JPG on top of your photo without touching the original file. Click **Import Overlay Image…**, pick a logo or banner, then **drag** it into place on the canvas and use the corner handle to resize (aspect ratio stays locked).

The panel lists every overlay on the photo. Select one to adjust:

- **Opacity** — fade the graphic in or out
- **Position** — jump the overlay to a corner or centre of the photo; the handle stays on the opposite corner while resizing
- **Fit to Width / Fit to Screen** — stretch across the photo, or shrink until fully visible
- **Reset Placement** — back to centre at full opacity
- **Remove Overlay** — delete it from this photo

Overlays composite last — after crop, rotation and tone — so they sit cleanly on top of every edit. Export and Print include them; the live editor preview draws them interactively on the canvas.

Edits are still non-destructive: import, move, and opacity all live in the edit recipe.

### 🔤 Text — captions, titles & callouts

Click **Add Text** to drop an editable text layer on the photo, then type your words straight into the editor or **drag** the text around on the canvas; the corner handle resizes it (the font scales with it). The panel lists every text layer. Select one to adjust:

- **Text** — edit the content; every line stays exactly as you typed it
- **Font, size & colour** — pick any installed font and a colour; size is a percentage of the photo height so it exports sharp at any resolution
- **Bold / Italic, Left / Centre / Right** — style the text; alignment lines up multiline entries inside their box
- **Opacity** — fade the text in or out
- **Position** — jump the text to a corner or centre of the photo
- **Reset Placement** — back to centre at full opacity (keeps your wording and styling)
- **Remove Text** — delete it from this photo

Text layers live in the same edit recipe as overlays: they composite above everything else, survive copy/paste of edits, export at full quality, and **Undo / Redo** treats a burst of typing as one step.

---

## Undo, Redo, Reset — Experiment Freely

- **Undo / Redo** — step backwards and forwards through your changes.
- **Reset** — one click wipes *all* edits and returns the photo to its original look.
- **Zoom slider / 1:1 control** — inspect your edits up close while you work.

Because nothing touches the original file, you can experiment as wildly as you like. The worst case is clicking Reset.

## Copy Edits Between Photos

Fixed ten sunset photos from the same shoot? Edit one, **copy its edits**, select the others and **paste**. Consistent looks across a whole batch in seconds.

Right-click also offers **Paste Text & Overlays Only** — when the clipboard holds at least one overlay or text layer, this swaps only those layers onto the selected photos and leaves exposure, crop, and the rest of each photo's recipe alone. Pasting twice is safe: the destination layers are *replaced*, never duplicated.

## Export — Save Your Edited Photo

Editing happens inside PIC's library, so to get the edited version out as a real image file, click **Export**. PIC renders a fresh, full-quality copy with all your edits applied — your original stays untouched as always.

## Done!

When you're happy, click **Done**. You return to the grid exactly where you left off, and edited photos wear a small **pencil badge** so you can spot them. The saved recipe is used for edited previews and exported results.

---

**Previous guide:** [← Library](library.md) · **Next guide:** [Crop Mode →](crop-mode.md)

## V1 editing workflow

PIC keeps ordinary adjustments and creative edits as recipes in the library. Use the on-screen **Undo**, **Redo** and **Reset** controls to compare results, and **Export** to generate a separate image. V1 includes additional tonal/colour controls and effects, including clarity, grain, old-photo and newspaper-style looks. Exact available controls depend on the tool selected.

For a creator-facing walkthrough, see the [V1 demonstration outline](../creator-kit.md).
