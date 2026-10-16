# Fixed: Flatpak build failed on GTK-display test

Date: 2026-09-28
Log: `build-logs/build-2026-09-28-195207-47620.log`

## Symptom

`./build-linux-icons-only.sh local --flatpak-only` failed during the
`cargo test --release --locked --offline` step inside the Flatpak sandbox:

```
---- css::theme_discovery::tests::standard_theme_corner_radius_parses stdout ----

thread 'css::theme_discovery::tests::standard_theme_corner_radius_parses' (5579)
panicked at src/css/theme_discovery.rs:308:22:
called `Result::unwrap()` on an `Err` value: BoolError { message: "Failed to
initialize GTK", filename: "/run/build/picasa-rs/vendor/gtk4/src/rt.rs",
function: "gtk4::rt::init", line: 159 }

test result: FAILED. 399 passed; 1 failed; 29 ignored

error: test failed, to rerun pass `--bin pic-rs`
Error: module picasa-rs: Child process exited with code 101
WARN: Flatpak build failed.
```

## Root cause

The test `css::theme_discovery::tests::standard_theme_corner_radius_parses`
(src/css/theme_discovery.rs:308) starts with `gtk4::init().unwrap()`.
GTK initialisation requires a display server.

- On the host the test passed because a live Wayland/X session was running.
- Inside the flatpak-builder sandbox there is no display, so `gtk4::init()`
  failed, the unwrap panicked, `cargo test` exited with code 101, and
  flatpak-builder aborted the `picasa-rs` module.

## Fix

Marked the test with the same ignore attribute used by the 27 other
GTK-display-dependent tests in the codebase, e.g. `src/albums_view.rs:56`:

```rust
#[test]
#[ignore = "requires a GTK display; run with --ignored --test-threads=1"]
fn standard_theme_corner_radius_parses() { ... }
```

The test still runs on a machine with a display via:

```
cargo test --release --ignored -- --test-threads=1
```

## Result

Re-ran `./build-linux-icons-only.sh local --flatpak-only`:

```
Flatpak:  dist/PIC-1.0.0-c9e110e252-x86_64.flatpak
Build session: SUCCESS (00:07:08, exit code 0)
```
