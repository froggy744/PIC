# Experimental GPU Edit and safe photo Export

**Scope:** draft PR #212 on \`feature/gpu-edit-pipeline\` (base
\`feature/gpu-live-edit\`). The stable \`main\` branch is unchanged.

## GPU Edit status (verified on the owner’s Fedora RX 9070 XT)

- Photo adjustments on a GTK \`GLArea\` desktop OpenGL shader: Exposure,
  Contrast, Saturation, Temperature, Fill Light, Shadows, Highlights, Sharpen,
  Clarity, Grain, B&W/Sepia and all fourteen named creative presets.
- Crop, Straighten, library quarter-turn rotations, Fit/1:1, pan and source
  texture caching. Interactive image overlays and rasterized Pango text.
- Seventeen GPU filter-picker covers from one decoded source preview.
- A failed GL context, shader or draw returns to the CPU render path; image
  originals and the database recipe are never modified by GPU operations.
- Owner tested and approved the GPU Edit milestone at commit
  \`35b227e40633c0b38c1ce903eabd2f168121dc13\` (tag
  \`gpu-edit-hardware-verified-2026-10-18\`). This **does not** verify every
  later Export change or every distribution runtime.

**Still CPU for accuracy:** Auto Colour and/or Auto Contrast combined with
named creative presets or cropped/straightened geometry. The current GPU
statistics path has produced 31–51/255 channel deviations against the CPU
reference on synthetic fixtures. Keep CPU fallback until strict parity tests
prove those combinations safe. \`PIC_EDIT_RENDERER=auto\` intentionally retains
the CPU default; do not auto-enable GPU across distributions prematurely.

## Export status and implementation

Export uses the full decoded source through the CPU recipe reference. It is
independent of the GPU preview framebuffer, preserves transparent PNG/WebP
overlays, and flattens JPEG alpha against white. It does not modify the source
file. Crop, rotate, tone, filters, layers and text are applied before encoding.
JPEG/PNG/WebP save through a staging file alongside the final destination; on
Linux a \`RENAME_NOREPLACE\` publication prevents concurrent overwrites.

EXIF is rebuilt from a selected set of nonlocation camera/lens/exposure/date
fields. The exported orientation is always 1 and pixel dimensions describe
the rendered file, never the unedited source. GPS, author fields, MakerNotes,
embedded previews and source-side EXIF pointers are intentionally omitted by
default. The output is tagged with a generated sRGB ICC profile, not blindly
with the input file's potentially incompatible profile.

The automated editor quality workflow (including output metadata, Export
pixels, non-destructive originals, concurrency, overlay safety and software
OpenGL pixel parity) passed at \`0700ac3\`:
https://github.com/froggy744/PIC/actions/runs/39199032850

**Important Nikon distinction:** For DSC_5791.NEF, the available full-size
embedded JPEG is 3936×2624. Export at those dimensions uses the camera's JPEG
preview; it does **not** demosaic Nikon NEF RAW sensor data. RAW-development
colour, detail and profile fidelity require a separate demosaicing pipeline.
Do not describe the embedded-JPEG export as full RAW development.

## Development and verification

\`\`\`sh
cd ~/PIC/.worktrees/gpu-edit-test
git fetch origin feature/gpu-edit-pipeline
git switch --detach origin/feature/gpu-edit-pipeline

PIC_EDIT_RENDERER=gpu PICASA_TRACE=1 cargo run --release \
  2>&1 | tee /tmp/pic-gpu-test.log
\`\`\`

For CPU reference:
\`\`\`sh
PIC_EDIT_RENDERER=cpu cargo run
\`\`\`

The experimental Linux packaging workflow builds AppImage/Flatpak without
publishing a release. Passing package builds are not a substitute for testing
desktop GL contexts and host Mesa/GLVND/Flatpak GL extensions on Fedora
Wayland hardware. AppImage must never bundle vendor GPU drivers.

## Release gate

Before enabling GPU automatically or proposing a merge to \`main\`, require
strict parity on automatic-adjustment combinations, real full-res JPEG/PNG
NEF-preview exports with EXIF and ICC review, AppImage/Flatpak smoke results,
and owner approval. Never publish personal test photos.
