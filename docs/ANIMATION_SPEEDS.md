# Animation speeds

The app reads `ANIMATION_SPEEDS.toml` from its current working directory. Edit
the root file in the repository and restart the app. Settings load once at
startup. Missing keys keep their defaults. Values from 16 to 10,000 ms are
accepted; `0` restores the built-in default for that setting. A missing or
malformed file falls back to built-in defaults and is reported to stderr.

| Key | Controls | Default |
| --- | --- | ---: |
| `grid_resize_ms` | Legacy Library resize duration, used when `library_resize_ms` is omitted | 170 ms |
| `library_resize_style` | Library resize animation style: `tile_motion` or `none` | `tile_motion` |
| `library_resize_ms` | Library resize settle movement duration | 170 ms |
| `grid_zoom_style` | `tile_motion`, `old_fades_over_new`, or `crossfade` zoom presentation | `tile_motion` |
| `grid_zoom_cutoff_ms` | Crossfade style only: scale/dim phase before layout commit | 120 ms |
| `grid_zoom_crossfade_ms` | Snapshot fade/crossfade phase for snapshot styles | 180 ms |
| `grid_zoom_reflow_ms` | Tile motion duration for `tile_motion` style | 170 ms |
| `gallery_wheel_scroll_ms` | Regular Library / All Photos grid wheel scrolling | 180 ms |
| `folder_column_reflow_ms` | Legacy duration fallback for folder resize | 140 ms |
| `folder_column_reflow_style` | Folder column-change style: `tile_motion` or `none` | `tile_motion` |
| `folder_resize_style` | Folder resize animation style: `tile_motion` or `none` | `tile_motion` |
| `folder_resize_ms` | Folder resize movement duration | 140 ms |
| `folder_zoom_style` | Folder zoom style: `tile_motion` or `crossfade` | `tile_motion` |
| `folder_zoom_ms` | Folder zoom tile movement duration | 170 ms |
| `folder_scroll_ms` | Folder contents wheel scrolling | 180 ms |
| `folder_mode_settle_ms` | Delay before folder navigation after switching folder display mode | 100 ms |
| `sidebar_slide_ms` | Sidebar pane reveal and paired pane movement | 250 ms |
| `lightbox_open_ms` | Thumbnail-to-photo viewer opening transition | 250 ms |

Frame scheduling, loading, and debounce timers are not animation durations and
are intentionally not configurable here.
