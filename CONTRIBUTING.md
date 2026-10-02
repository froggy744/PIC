# Contributing to PIC

Thanks for helping improve PIC.

## Before making a change

For a bug, open an issue with the PIC version/build, Linux distribution, steps to reproduce, expected behaviour, actual behaviour, and relevant logs. For a larger feature, open an issue first so the approach can be discussed before substantial work begins.

## Development

PIC is written in Rust using GTK4 and libadwaita. Clone the repository and work from the current `main` branch unless a maintainer asks you to use another branch.

Keep changes focused. Do not commit generated build output, local caches, personal photo libraries, credentials, or other machine-specific data.

Before submitting a pull request, run the project's tests and build the affected packaging target. The Linux packaging entry point is:

```sh
./scripts/PIC-build-linux-one-script.sh local
```

Use `--appimage-only` or `--flatpak-only` when only one package needs testing.

## Pull requests

Explain what changed and why, describe how you tested it, and mention any issue it fixes. Include screenshots for visible UI changes when useful. Keep unrelated changes in separate pull requests.

By contributing, you agree that your contribution may be distributed under the repository's MIT license.
