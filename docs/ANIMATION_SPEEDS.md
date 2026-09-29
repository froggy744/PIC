# Animation speeds

The app reads `ANIMATION_SPEEDS.toml` from its current working directory. Edit
the root file in the repository and restart the app. Settings load once at
startup. Missing keys keep their defaults. Values from 16 to 10,000 ms are
accepted; `0` restores the built-in default for that setting. A missing or
malformed file falls back to built-in defaults and is reported to stderr.

| Key | Controls | Default |
| --- | --- | ---: |
| `grid_resize_ms` | Regular Library / All Photos grid thumbnail movement on window resize | 170 ms |
| `grid_zoom_cutoff_ms` | Grid zoom scale and opacity fade before layout commit | 120 ms |
| `grid_zoom_crossfade_ms` | Grid zoom snapshot crossfade after layout commit | 180 ms |
| `grid_zoom_reflow_ms` | Thumbnail position settling after grid zoom | 170 ms |
| `gallery_wheel_scroll_ms` | Regular Library / All Photos grid wheel scrolling | 180 ms |
| `folder_column_reflow_ms` | Folder photo stream column-change crossfade | 220 ms |
| `folder_column_reflow_style` | `old_fades_over_new` keeps new layout fully visible; `crossfade` fades both layers | `old_fades_over_new` |
| `folder_scroll_ms` | Folder contents wheel scrolling | 180 ms |
| `folder_mode_settle_ms` | Delay before folder navigation after switching folder display mode | 100 ms |
| `sidebar_slide_ms` | Sidebar pane reveal and paired pane movement | 250 ms |
| `lightbox_open_ms` | Thumbnail-to-photo viewer opening transition | 250 ms |

Frame scheduling, loading, and debounce timers are not animation durations and
are intentionally not configurable here.
