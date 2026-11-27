# PIC patch for rawler 0.8.0

This is the crates.io 0.8.0 source with its camera and lens data and LGPL-2.1
license. The registry package checksum is
`51cb9d46a5b34946878eb236d1180adec0806eee260738ce299517b083a232e8`.
The upstream test fixture corpus under `data/testdata` is omitted.

The PIC changes are confined to `src/pixarray.rs` and `src/imgop/develop.rs`.
The extra entry point lets full-size Nikon scene development consume the
owned RGB buffer when applying rawler's default crop. Standard rawler
callers keep the upstream allocating path. The consuming path retains the
original vector capacity until the caller drops the result.

When updating rawler, check whether upstream provides an equivalent owned
crop. Otherwise reapply these two changes and run PIC's
`owned_rgb_crop_reuses_storage_and_matches_allocating_crop` and
`owned_nef_crop_matches_allocating_crop_pixel_for_pixel` tests. The second
test requires `PIC_TEST_RAW` and `--ignored`.
