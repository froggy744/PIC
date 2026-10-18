# Experimental LiveGPU editing renderer

PIC's editor has an opt-in GTK `GLArea` renderer for live tone controls. It
keeps the decoded source in one OpenGL texture and changes shader uniforms for
Exposure, Contrast, Saturation, Warmth, Fill Light, Shadows, Highlights and
Sharpen. Crop and straighten use the existing CPU geometry path. Filters,
auto controls, clarity and grain use the CPU renderer. All editing buffers
retain source resolution; during CPU processing the last sharp frame stays
visible. Unsupported CPU effects can still lag. The CPU renderer remains the
default and the export path is unchanged.

See [the recovery handoff](livegpu-recovery-handoff.md) for verified behaviour,
measurements and unresolved failures. Vulkan selection fell back to OpenGL in
the latest isolated test; actual Vulkan compatibility remains unverified.

```sh
PIC_EDIT_RENDERER=cpu cargo run
PIC_EDIT_RENDERER=auto cargo run
PIC_EDIT_RENDERER=gpu PICASA_TRACE=1 cargo run
```

`auto` currently selects CPU intentionally. `gpu` requests desktop OpenGL via
`GtkGLArea`; context creation, shader, upload or draw errors hide the GL layer
and re-render through CPU. The GPU area is sized from the scrolled viewport;
the full-resolution source texture is sampled using the current scroll offset
and zoom. It does not create a source-sized framebuffer. Uniform-only changes
do not upload the source texture again. `PICASA_TRACE=1` records source upload,
GL draw submission, frame-clock after-paint latency, CPU fallback latency and
preview dimensions.

## Verification

The OpenGL parity and full-source viewport benchmark tests are ignored in the normal
suite because they need a GTK display and GL context. On Linux, run:

```sh
GSK_RENDERER=gl PICASA_TRACE=1 xvfb-run -a cargo test \
  gtk_gl_area_matches_cpu_reference_at_native_pixel_size \
  -- --ignored --test-threads=1 --nocapture
GSK_RENDERER=vulkan PICASA_TRACE=1 xvfb-run -a cargo test \
  gtk_gl_area_matches_cpu_reference_at_native_pixel_size \
  -- --ignored --test-threads=1 --nocapture
GSK_RENDERER=gl PICASA_TRACE=1 xvfb-run -a cargo test \
  gtk_gl_area_benchmarks_full_source_upload_and_viewport_sized_draw \
  -- --ignored --test-threads=1 --nocapture
```

The Vulkan selection here tests GTK's Vulkan scene renderer with an independent
desktop OpenGL `GLArea`. It does not make the edit shader a Vulkan renderer.
Parity uses GL framebuffer readback only in tests; normal editing has no GPU to
CPU readback.

## Packaging checks still required

This branch has only been exercised in the current Linux development
environment. Fedora, Flatpak and AppImage runtimes need separate validation
before `auto` can select GPU rendering:

- Fedora: verify the GTK GLArea desktop GL context with the installed Mesa or
  vendor driver on both X11 and Wayland.
- Flatpak: verify the app runtime includes GTK GLArea support and that the
  matching host `org.freedesktop.Platform.GL` extension is available.
- AppImage: verify `libGL.so.1` resolves to the host GLVND/driver stack. Do not
  bundle a vendor-specific GL driver in the AppImage.

The application loads GL entry points dynamically. If `libGL.so.1` or a
compatible context is unavailable, the GPU request falls back to CPU and emits
a `PIC_EDIT_GPU init_failed` trace.
