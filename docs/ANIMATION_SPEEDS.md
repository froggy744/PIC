# Animation speeds

The app reads `ANIMATION_SPEEDS.toml` from its current working directory. Edit
the root file in the repository and restart the app. Settings load once at
startup. Missing keys keep their defaults. Values from 16 to 10,000 ms are
accepted; `0` restores the built-in default for that setting. A missing or
malformed file falls back to built-in defaults and is reported to stderr.

| Key | Controls | Default |
| --- | --- | ---: |
| `grid_resize_ms` | Legacy Library resize duration, used when `library_resize_ms` is omitted | 120 ms |
| `library_resize_style` | Library resize animation style: `in_place` or `none` | `in_place` |
| `library_resize_ms` | Library resize settle effect duration | 120 ms |
| `grid_zoom_style` | Grid zoom style: `in_place` or `none` | `in_place` |
| `grid_zoom_cutoff_ms` | Legacy snapshot-transition setting; unused by `in_place` | 120 ms |
| `grid_zoom_crossfade_ms` | Legacy snapshot-transition setting; unused by `in_place` | 180 ms |
| `grid_zoom_reflow_ms` | Grid zoom in-place effect duration | 120 ms |
| `gallery_wheel_scroll_ms` | Regular Library / All Photos grid wheel scrolling | 180 ms |
| `folder_column_reflow_ms` | Legacy duration fallback for folder resize | 120 ms |
| `folder_column_reflow_style` | Legacy folder column-change style: `in_place` or `none` | `in_place` |
| `folder_resize_style` | Folder resize style: `in_place` or `none` | `in_place` |
| `folder_resize_ms` | Folder resize settle effect duration | 120 ms |
| `folder_zoom_style` | Folder zoom style: `in_place` or `none` | `in_place` |
| `folder_zoom_ms` | Folder zoom in-place effect duration | 120 ms |
| `folder_scroll_ms` | Folder contents wheel scrolling | 180 ms |
| `folder_mode_settle_ms` | Delay before folder navigation after switching folder display mode | 100 ms |
| `sidebar_slide_ms` | Sidebar pane reveal and paired pane movement | 250 ms |
| `lightbox_open_ms` | Thumbnail-to-photo viewer opening transition | 250 ms |

Frame scheduling, loading, and debounce timers are not animation durations and
are intentionally not configurable here.
