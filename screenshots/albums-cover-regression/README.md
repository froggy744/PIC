# Album cover appearance regression captures

Each paired screenshot shows the Albums index with the bookshelf background
disabled on the left and enabled on the right. It uses the same ten shipped
transparent frame assets and cover photos in both panels.

The GTK matrix test renders both states for every theme discovered in
`themes/`, then asserts that each cover's requested and allocated dimensions
and transparent-opening margins are unchanged:

```sh
PICASA_ALBUM_COVER_MATRIX_DIR=reports/gui-tests/albums-cover-regression \
  cargo test album_cover_appearance_matrix_across_supported_themes -- \
  --ignored --test-threads=1
python3 scripts/build_album_cover_comparisons.py \
  reports/gui-tests/albums-cover-regression screenshots/albums-cover-regression
```

The paired images in this directory are review references; the test writes
full-resolution PNG captures to the directory selected by the environment
variable.
