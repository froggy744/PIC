#!/usr/bin/env bash
# Presentation-owner guard.
#
# Sidebar, Properties and InfoBar presentation each have one GTK adapter that
# is the only writer of their widgets (docs/architecture/STATUS.md,
# candidates 2-4):
#   - src/window/sidebar_widgets.rs    (SidebarWidgets)
#   - src/window/properties_widgets.rs (PropertiesWidgets)
#   - src/infobar.rs                   (InfoBar; the decision is infobar::presentation)
# Photo mark commands (candidate 9) are likewise the only writer of photo
# favourites and ratings:
#   - src/window/photo_mark_commands.rs (the decision is window::photo_marks)
# Photo change (candidate 16) gives every photo change one fan-out; it is the
# only writer of the shown photo's facts (InfoBar::photo_updated) and the
# Library counts after a change, and writes the gallery's marks:
#   - src/window/photo_change_widgets.rs (photo_changed; the decision is
#     window::photo_change)
# The current photo (candidate 10) has one owner, the InfoBar
# (`InfoBar::current_photo`); no window code may keep a hand-synced copy in a
# `selected_photo` cell again.
# The front surface (candidate 12) is the one place that decides which
# surface is in front (viewer, editor, collage, date overview, gallery):
#   - src/window/front_surface.rs (FrontSurface::current, gallery_covered,
#     date_browser). The viewer's own tests are viewer internals.
# The InfoBar, Properties and the metadata panel follow the front through
# one listener (candidate 13):
#   - src/window/front_mirrors.rs (FrontMirrors)
# The shared zoom slider has one writer of what it shows (candidate 6):
#   - src/window/zoom_slider_widgets.rs (ZoomSliderWidgets; the decision is
#     window::zoom_slider). The InfoBar builds the slider. The user's own
#     input (dragging, the wheel over the slider) is not presentation.
# The page change handlers keep reading their own `stack` / `root` argument
# until the events stage; `matches!` over page names is not matched (the
# ALLOW table cannot hold a "|"), so review those by hand.
# The window session (candidate 11: last view, viewport, window size, split
# widths) is loaded and saved in one place; no other file names its keys:
#   - src/window/session_state.rs (SessionState::load, record). Window test
#     fixtures may seed a library with them.
# Gallery scroll motion (candidate 5: scrub, target sampling, settle and the
# thumbnail pump) is decided in one place; window code keeps no hand-kept
# scrub cells or pump constants of its own:
#   - src/window/scroll_motion.rs (ScrollMotion, ALL_PHOTOS, FOLDERS).
# A gallery destination's photo list (candidate 15: formats, RAW+JPEG
# pairing, Recently Added limit, Files check, order) is built in one place;
# no window code applies those steps by hand again:
#   - src/window/photo_list.rs (photo_list, photo_list_has_photos). Tests of
#     the steps live there (they are private) and in src/image_format.rs,
#     which defines and tests the pairing step.
# The album surfaces (candidate 17: the sidebar's album rows and the Albums
# home) have one writer, which picks the list for each:
#   - src/window/album_list_widgets.rs (AlbumList::changed; the decision is
#     window::album_list).
# The InfoBar's metadata and selection widgets are private, so only its
# public buttons' enabled state needs a rule here.
# This script fails when any other file writes those widgets directly.
#
# Usage:
#   scripts/check-presentation-owners.sh              check this repository
#   scripts/check-presentation-owners.sh --root DIR   check another tree
#   scripts/check-presentation-owners.sh --self-test  prove the rules pass and fail
#
# Matching is textual: a rule matches a writer by the variable name it is
# called on, as the window code names these widgets today. A renamed variable
# escapes the guard, so update the rule in the same change as the rename.
set -euo pipefail

SIDEBAR_OWNER='src/window/sidebar_widgets.rs'
PROPERTIES_OWNER='src/window/properties_widgets.rs'
INFOBAR_OWNER='src/infobar.rs'
MARKS_OWNER='src/window/photo_mark_commands.rs'
PHOTO_CHANGE_OWNER='src/window/photo_change_widgets.rs'
FRONT_OWNER='src/window/front_surface.rs'
MIRRORS_OWNER='src/window/front_mirrors.rs'
ZOOM_OWNER='src/window/zoom_slider_widgets.rs'
SESSION_OWNER='src/window/session_state.rs'
SCROLL_OWNER='src/window/scroll_motion.rs'
PHOTO_LIST_OWNER='src/window/photo_list.rs'
ALBUM_LIST_OWNER='src/window/album_list_widgets.rs'
SESSION_FIXTURES='src/window/immersive_tests.rs src/window/responsive_tests.rs'
ID='[[:alnum:]_]'

# id ~ owner files (space separated) ~ extended regex ~ what it guards
# ("~" separates fields because the regexes use "|".)
RULES=(
  "properties-button~$PROPERTIES_OWNER~\.set_properties_(active|enabled)\(~InfoBar Properties button state"
  "properties-content~$PROPERTIES_OWNER~(^|[^[:alnum:]_])properties_(photo_content|placeholder)\(~Properties panel rebuilds"
  "properties-visibility~$PROPERTIES_OWNER~(properties_split$ID*\.set_show_sidebar|lightbox_properties_panel$ID*\.set_visible)\(~gallery split / viewer panel visibility"
  "sidebar-mirror~$SIDEBAR_OWNER src/sidebar.rs~(^|[^[:alnum:]_])(set_pinned|set_hover_open|clear_hover_open)\(~sidebar.rs pin and hover-peek mirror"
  "sidebar-split~$SIDEBAR_OWNER~main_split$ID*\.(set_show_sidebar|set_collapsed|set_pin_sidebar|set_enable_show_gesture|set_enable_hide_gesture|set_sidebar)\(~main split visibility, pin, gestures and sidebar child"
  "infobar-actions~$INFOBAR_OWNER~\.(favorite|rating|edit|add_to_album|rotate|export|print|more|view_toggle|grid_zoom)\.set_sensitive\(~InfoBar photo-action and gallery-control enabled state"
  "photo-marks~$MARKS_OWNER $PHOTO_CHANGE_OWNER src/library_home_tests.rs~(db::set_(favorite|favorite_for_photos|rating_for_photos)|\.update_(favorites|ratings))\(~favourite and rating writes (library and gallery objects)"
  "current-photo~$INFOBAR_OWNER~(^|[^[:alnum:]_])selected_(photo|for)$ID*(\.replace\(|\.borrow_mut\(|:[[:space:]]*Rc<)~a hand-kept copy of the current photo (read InfoBar::current_photo)"
  "front-surface~$FRONT_OWNER src/lightbox/tests.rs~((lightbox$ID*|viewer)\.root\.is_visible\(\)|visible_child_name\(\)\.as_deref\(\)[[:space:]]*[!=]=[[:space:]]*Some\(\"(edit|photos|collage)\"\)|\.is_overview_visible\(\)|date_browser_overview)~which surface is in front (ask FrontSurface)"
  "front-mirrors~$MIRRORS_OWNER $INFOBAR_OWNER src/window/metadata_panel.rs~(\.(set_lightbox_shown|set_date_overview_shown|set_page_allowed|set_lightbox_open|set_navigation_view)\(|(^|[^[:alnum:]_])info$ID*\.set_page\()~front inputs of the InfoBar, Properties and metadata panel (FrontMirrors)"
  "zoom-slider~$ZOOM_OWNER $INFOBAR_OWNER~grid_zoom$ID*\.(set_range|set_value|set_increments|set_round_digits|clear_marks|set_tooltip_text)\(~zoom slider range, steps, tooltip and shown value (ZoomSliderWidgets)"
  "session-state~$SESSION_OWNER $SESSION_FIXTURES~(LAST_(VIEW|VIEW_PHOTO_ID|ACTIVATED_PHOTO_ID|VIEW_SCROLL|ALBUMS_SCROLL)|WINDOW_(WIDTH|HEIGHT|MAXIMIZED)|SIDEBAR_WIDTH_FRACTION|SIDEBAR_COLLAPSED_WIDTH|PROPERTIES_WIDTH_FRACTION|COMPACT_SIDEBAR_WIDTH)_SETTING_KEY|\"(last-view|last-view-photo-id|last-activated-photo-id|last-view-scroll|last-albums-scroll|window-width|window-height|window-maximized|sidebar-width-fraction|properties-width-fraction|sidebar-collapsed-width|compact-sidebar-width-v2)\"~window session keys (load and save through window::session_state)"
  "scroll-motion~$SCROLL_OWNER~(^|[^[:alnum:]_])(grid|folder)_(direct_)?scrub_(active|generation|frames|last_queue|sampler)([^[:alnum:]_(]|_for|$)|THUMBNAIL_MOTION_PUMP_FRAMES|ScrollbarScrub~a hand-kept gallery scrub, settle or pump state (ask window::scroll_motion)"
  "photo-list~$PHOTO_LIST_OWNER src/image_format.rs~(^|[^[:alnum:]_])(retain_raw_jpeg_pair_mode|limit_recently_added|apply_files_check|collection_photos)\(~a photo-list pipeline step by hand (build lists with window::photo_list)"
  "photo-change~$PHOTO_CHANGE_OWNER $INFOBAR_OWNER src/infobar_responsive_tests.rs src/sidebar.rs~(^|[^[:alnum:]_])(photo_updated|update_library_counts)\(~the shown photo's facts or the Library counts after a photo change (say photo_changed)"
  "album-list~$ALBUM_LIST_OWNER~(albums_view::refresh|sidebar::refresh_album_rows)\(~the sidebar album rows or the Albums home (say AlbumList::changed)"
  "sidebar-parts~$SIDEBAR_OWNER~(^|[^[:alnum:]_])(compact_dock|dock|left_column|sidebar|sidebar_shell|sidebar_resize_handle|compact_sidebar_page|compact_immersive_toggle)\.(set_visible|set_child)\(~sidebar shell, column, dock, rail and handle visibility/placement"
)

# Writes allowed outside the owner, by file and exact trimmed source line.
# Each entry allows ONE occurrence. Construction-time defaults run before the
# adapter exists; anything else listed here is a known exception to remove.
# path | trimmed line | reason
ALLOW=(
  "src/window/build.rs|properties_split.set_show_sidebar(false);|construction default before attach_gallery"
  "src/window/build.rs|lightbox_properties_panel.set_visible(false);|construction default before attach_lightbox"
  "src/window/layout.rs|main_split.set_pin_sidebar(true);|construction default before SidebarWidgets::new"
  "src/window/layout.rs|main_split.set_sidebar(Some(&sidebar_shell));|construction: initial split child"
  "src/window/layout.rs|compact_dock.set_visible(false);|construction default before SidebarWidgets::new"
  "src/window/layout.rs|compact_dock.set_child(Some(&compact_sidebar_page));|construction: initial dock child"
  "src/window/layout.rs|compact_immersive_toggle.set_visible(false);|construction default before SidebarWidgets::new"
  "src/window/layout.rs|sidebar_shell.set_child(Some(&left_column));|construction: initial shell child"
  "src/window/library.rs|db::set_rating_for_photos(&connection, &[1], rating).unwrap();|test fixture (rating filter refresh tests)"
  "src/window/library.rs|db::set_rating_for_photos(&connection, &[1], 0).unwrap();|test fixture (rating filter refresh tests)"
  "src/window/library.rs|db::set_rating_for_photos(&connection, &[2], 0).unwrap();|test fixture (rating filter refresh tests)"
  "src/window/library.rs|db::set_rating_for_photos(&connection, &[1], 3).unwrap();|test fixture (rating filter refresh tests)"
  "src/window/build.rs|&& stack.visible_child_name().as_deref() == Some(\"photos\")|page change handler (candidate 12 decision 1: until the events stage)"
  "src/window/build.rs|panel_for_close.set_lightbox_open(false);|viewer close_started: the fade begins before the front changes (candidate 13 decision 7)"
  "src/window/build.rs|panel.set_navigation_view(page.as_deref().unwrap_or(\"\"), filter_for_navigation.get());|metadata panel tick: follows filter changes within one page (candidate 13 decision 1)"
  "src/window.rs|LAST_VIEW_SETTING_KEY, SIDEBAR_WIDTH_FRACTION_SETTING_KEY, WINDOW_HEIGHT_SETTING_KEY,|test-only re-export for the window test fixtures"
  "src/window.rs|WINDOW_WIDTH_SETTING_KEY,|test-only re-export for the window test fixtures"
  "src/window/metadata_panel.rs|(LAST_VIEW_SETTING_KEY, format!(\"folder:{folder}\")),|test fixture (metadata panel window test)"
  "src/albums_view.rs|db::set_setting(&connection.borrow(), \"last-view\", \"albums\").unwrap();|test fixture (cover flow full-app validation)"
  "src/window/build.rs|sidebar::update_library_counts(&sidebar_for_events, counts);|the scan's own batched Library counts (candidate 16 F4)"
  "src/window/photo_actions.rs|info.photo_updated(&destination);|test fixture (the shown photo)"
  "src/window/photo_actions.rs|info.photo_updated(&viewer_photo);|test fixture (the shown photo)"
  "src/window/photo_actions.rs|sidebar::update_library_counts(&sidebar, db::sidebar_counts(&connection.borrow()).unwrap());|test fixture (trash counts test)"
  "src/window/photo_mark_commands.rs|info.photo_updated(&anchor);|test fixture (the shown photo)"
  "src/window/layout.rs|dock.set_visible(false);|KNOWN EXCEPTION: compact_dock visible-notify immersive guard predates SidebarWidgets"
)

trim() {
  local s="$1"
  s="${s#"${s%%[![:space:]]*}"}"
  s="${s%"${s##*[![:space:]]}"}"
  printf '%s' "$s"
}

# check ROOT: print violations; exit status 1 if any.
check() {
  local root="$1" failures=0
  if [[ ! -d "$root/src" ]]; then
    echo "check-presentation-owners: no src/ under $root" >&2
    return 2
  fi
  declare -A allow_used=()
  local rule id owners pattern guards match rel line text skip owner entry apath atext
  for rule in "${RULES[@]}"; do
    IFS='~' read -r id owners pattern guards <<<"$rule"
    while IFS= read -r match; do
      [[ -z "$match" ]] && continue
      if [[ "$match" == "GREP_ERROR "* ]]; then
        echo "check-presentation-owners: grep failed on rule [$id] (${match#GREP_ERROR }): bad pattern?" >&2
        return 2
      fi
      rel="${match#"$root"/}"
      line="${rel#*:}"
      rel="${rel%%:*}"
      text="$(trim "${line#*:}")"
      line="${line%%:*}"
      skip=0
      for owner in $owners; do
        [[ "$rel" == "$owner" ]] && skip=1
      done
      # Comments and the functions' own definitions are not writes.
      [[ "$text" == //* ]] && skip=1
      [[ "$text" =~ (^|[[:space:]])fn[[:space:]] ]] && skip=1
      if ((skip == 0)); then
        for entry in "${ALLOW[@]}"; do
          IFS='|' read -r apath atext _ <<<"$entry"
          if [[ "$rel" == "$apath" && "$text" == "$atext" && -z "${allow_used[$entry]:-}" ]]; then
            allow_used[$entry]=1
            skip=1
            break
          fi
        done
      fi
      if ((skip == 0)); then
        echo "$rel:$line: [$id] $text"
        echo "    writes $guards outside its owner ($owners)"
        failures=$((failures + 1))
      fi
    done < <(grep -rnE --include='*.rs' -- "$pattern" "$root/src"; status=$?; ((status <= 1)) || echo "GREP_ERROR $status")
  done
  # A stale entry would silently permit a future write with the same text.
  for entry in "${ALLOW[@]}"; do
    if [[ -z "${allow_used[$entry]:-}" ]]; then
      IFS='|' read -r apath atext _ <<<"$entry"
      echo "$apath: [stale-allow] '$atext' is no longer in this file; remove its ALLOW entry"
      failures=$((failures + 1))
    fi
  done
  if ((failures > 0)); then
    echo "check-presentation-owners: $failures direct write(s) outside the presentation owners." >&2
    echo "Route them through SidebarWidgets / PropertiesWidgets / InfoBar / photo_mark_commands / InfoBar::current_photo / FrontSurface / FrontMirrors / ZoomSliderWidgets / session_state / scroll_motion / photo_list / AlbumList / photo_changed (AUDIT-INDEX.md, Quick-Edit Guide)." >&2
    return 1
  fi
  echo "check-presentation-owners: ok"
}

# A minimal tree that follows every rule, including each allowed line.
write_fixture() {
  local root="$1"
  mkdir -p "$root/src/window"
  cat >"$root/$SIDEBAR_OWNER" <<'RS'
fn apply(&self) {
    split.set_show_sidebar(true);
    main_split.set_collapsed(false);
    dock.set_visible(true);
    crate::sidebar::set_pinned(&sidebar, true);
    crate::sidebar::set_hover_open(&sidebar, false);
    crate::sidebar::clear_hover_open(&sidebar);
}
RS
  cat >"$root/$PROPERTIES_OWNER" <<'RS'
fn apply(&self) {
    self.info.set_properties_active(true);
    self.info.set_properties_enabled(true);
    let content = properties_photo_content(photo, context, surface, close);
    let empty = properties_placeholder(close);
    lightbox_properties_panel.set_visible(true);
}
RS
  cat >"$root/src/sidebar.rs" <<'RS'
pub fn set_pinned(sidebar: &gtk::ScrolledWindow, pinned: bool) {}
pub fn set_hover_open(sidebar: &gtk::ScrolledWindow, open: bool) {}
pub fn clear_hover_open(sidebar: &gtk::ScrolledWindow) { set_hover_open(sidebar, false); }
RS
  cat >"$root/src/infobar.rs" <<'RS'
    pub fn set_properties_active(&self, active: bool) {}
    pub fn set_properties_enabled(&self, enabled: bool) {}
    self.favorite.set_sensitive(actions);
    self.grid_zoom.set_sensitive(target.gallery_controls_enabled());
RS
  cat >"$root/$MARKS_OWNER" <<'RS'
        MarkChange::Favorite(favorite) => db::set_favorite_for_photos(connection, &plan.ids, favorite),
        MarkChange::Rating(rating) => db::set_rating_for_photos(connection, &plan.ids, rating),
            MarkChange::Favorite(favorite) => gallery.update_favorites(&plan.ids, favorite),
RS
  cat >"$root/$FRONT_OWNER" <<'RS'
            .is_some_and(|browser| browser.is_overview_visible())
        front(page, self.lightbox_root.is_visible(), self.date_browser())
RS
  mkdir -p "$root/src/lightbox"
  cat >"$root/src/lightbox/tests.rs" <<'RS'
        assert!(lightbox.root.is_visible());
RS
  cat >"$root/src/grid.rs" <<'RS'
    pub fn update_favorites(&self, ids: &[i64], favorite: bool) {
RS
  cat >"$root/src/window/photo_actions.rs" <<'RS'
fn properties_placeholder(on_close: Rc<dyn Fn()>) -> gtk::Box {}
fn properties_photo_content(
RS
  cat >"$root/src/window/build.rs" <<'RS'
    // properties_split.set_show_sidebar(true) is PropertiesWidgets' job.
    properties_split.set_show_sidebar(false);
    lightbox_properties_panel.set_visible(false);
    main_split_for_drag_update.set_max_sidebar_width(width);
    if front.current() == Front::Lightbox {
                && stack.visible_child_name().as_deref() == Some("photos")
            panel_for_close.set_lightbox_open(false);
            panel.set_navigation_view(page.as_deref().unwrap_or(""), filter_for_navigation.get());
                sidebar::update_library_counts(&sidebar_for_events, counts);
RS
  cat >"$root/$PHOTO_CHANGE_OWNER" <<'RS'
                context.info.photo_updated(shown);
                sidebar::update_library_counts(sidebar, counts);
                    MarkChange::Rating(rating) => gallery.update_ratings(&plan.ids, rating),
RS
  cat >"$root/$SESSION_OWNER" <<'RS'
pub(super) const LAST_VIEW_SETTING_KEY: &str = "last-view";
        SessionChange::CompactWidth(width) => store.set(COMPACT_SIDEBAR_WIDTH_SETTING_KEY, &width),
RS
  cat >"$root/$PHOTO_LIST_OWNER" <<'RS'
    crate::image_format::retain_raw_jpeg_pair_mode(&mut photos, pair_mode);
        limit_recently_added(connection, request.filter, &mut photos);
    apply_files_check(&mut photos, &request.scope);
RS
  cat >"$root/$ALBUM_LIST_OWNER" <<'RS'
                sidebar::refresh_album_rows(sidebar, albums, &on_delete_album);
            crate::albums_view::refresh(
RS
  cat >"$root/$SCROLL_OWNER" <<'RS'
// The old grid_scrub_active cell and FolderScrollbarScrub live here now.
pub(super) struct ScrollbarScrubNote;
RS
  cat >"$root/src/window/responsive_tests.rs" <<'RS'
        (WINDOW_WIDTH_SETTING_KEY, "1440"),
RS
  local entry apath atext
  for entry in "${ALLOW[@]}"; do
    IFS='|' read -r apath atext _ <<<"$entry"
    [[ "$apath" == src/window/layout.rs || "$apath" == src/window/library.rs \
      || "$apath" == src/window.rs || "$apath" == src/window/metadata_panel.rs \
      || "$apath" == src/albums_view.rs || "$apath" == src/window/photo_actions.rs \
      || "$apath" == "$MARKS_OWNER" ]] \
      && printf '    %s\n' "$atext" >>"$root/$apath"
  done
}

self_test() {
  local tmp failed=0 case_root output
  tmp="$(mktemp -d)"
  trap 'rm -rf "$tmp"' RETURN

  write_fixture "$tmp/ok"
  if output="$(check "$tmp/ok" 2>&1)"; then
    echo "pass: compliant tree accepted"
  else
    echo "FAIL: compliant tree rejected:"; echo "$output"; failed=1
  fi

  # name | file to append to | line | rule id the check must report
  local cases=(
    "button write in build.rs|src/window/build.rs|info.set_properties_active(true);|properties-button"
    "button write in immersive.rs|src/window/immersive.rs|info.set_properties_enabled(false);|properties-button"
    "panel rebuild outside owner|src/window/photo_actions.rs|let c = properties_photo_content(p, ctx, s, close);|properties-content"
    "placeholder outside owner|src/window/build.rs|panel.append(&properties_placeholder(close));|properties-content"
    "gallery panel shown in build.rs|src/window/build.rs|properties_split.set_show_sidebar(true);|properties-visibility"
    "second allowed default|src/window/build.rs|properties_split.set_show_sidebar(false);|properties-visibility"
    "viewer panel shown in immersive.rs|src/window/immersive.rs|lightbox_properties_panel.set_visible(true);|properties-visibility"
    "pin mirror in layout.rs|src/window/layout.rs|crate::sidebar::set_pinned(&sidebar, false);|sidebar-mirror"
    "hover mirror in immersive.rs|src/window/immersive.rs|crate::sidebar::set_hover_open(&sidebar, true);|sidebar-mirror"
    "split shown in immersive.rs|src/window/immersive.rs|main_split.set_show_sidebar(true);|sidebar-split"
    "split alias collapsed|src/window/layout.rs|main_split_for_breakpoint.set_collapsed(true);|sidebar-split"
    "allowed line in another file|src/window/build.rs|main_split.set_pin_sidebar(true);|sidebar-split"
    "photo action enabled in build.rs|src/window/build.rs|info.rotate.set_sensitive(true);|infobar-actions"
    "zoom disabled in layout.rs|src/window/layout.rs|info_for_zoom.grid_zoom.set_sensitive(false);|infobar-actions"
    "favourite written in build.rs|src/window/build.rs|db::set_favorite(&db.borrow(), id, true)|photo-marks"
    "ratings written in photo_actions.rs|src/window/photo_actions.rs|db::set_rating_for_photos(&c, &ids, 3)|photo-marks"
    "gallery objects marked in build.rs|src/window/build.rs|gallery_for_keys.update_ratings(&ids, rating);|photo-marks"
    "current photo copied in build.rs|src/window/build.rs|selected_photo_for_grid.replace(photo);|current-photo"
    "current photo cell in window.rs|src/window/photo_actions.rs|selected_photo: Rc<RefCell<Option<PhotoObject>>>,|current-photo"
    "dock shown in build.rs|src/window/build.rs|compact_dock.set_visible(true);|sidebar-parts"
    "column hidden in immersive.rs|src/window/immersive.rs|left_column.set_visible(false);|sidebar-parts"
    "shell child in build.rs|src/window/build.rs|sidebar_shell.set_child(Some(&compact_sidebar_page));|sidebar-parts"
    "viewer check in build.rs|src/window/build.rs|if lightbox_for_keys.root.is_visible() {|front-surface"
    "viewer check in shortcuts.rs|src/window/shortcuts.rs|} else if viewer.root.is_visible() {|front-surface"
    "edit page check in layout.rs|src/window/layout.rs|if main_stack.visible_child_name().as_deref() == Some(\"edit\") {|front-surface"
    "photos page check in build.rs|src/window/build.rs|let photos_page = stack.visible_child_name().as_deref() != Some(\"photos\");|front-surface"
    "overview read in toolbar.rs|src/window/toolbar.rs|if browser.is_overview_visible() {|front-surface"
    "overview cell back in build.rs|src/window/build.rs|let date_browser_overview_visible = Rc::new(Cell::new(false));|front-surface"
    "slider range set in build.rs|src/window/build.rs|info.grid_zoom.set_range(0.0, 100.0);|zoom-slider"
    "slider value set in layout.rs|src/window/layout.rs|info.grid_zoom.set_value(value);|zoom-slider"
    "slider tooltip set by an alias|src/window/build.rs|grid_zoom_for_visibility.set_tooltip_text(Some(tip));|zoom-slider"
    "InfoBar viewer fed in build.rs|src/window/build.rs|info.set_lightbox_shown(root.is_visible());|front-mirrors"
    "InfoBar page fed in layout.rs|src/window/layout.rs|info.set_page(page);|front-mirrors"
    "overview fed in build.rs|src/window/build.rs|info.set_date_overview_shown(true);|front-mirrors"
    "Properties page fed in build.rs|src/window/build.rs|properties_for_page.set_page_allowed(allowed);|front-mirrors"
    "session key read in build.rs|src/window/build.rs|let width = numeric_setting(&c, WINDOW_WIDTH_SETTING_KEY);|session-state"
    "session key literal in layout.rs|src/window/layout.rs|db::set_setting(&c, \"sidebar-width-fraction\", &f)|session-state"
    "compact width key back in layout.rs|src/window/layout.rs|const COMPACT_SIDEBAR_WIDTH_SETTING_KEY: &str = \"x\";|session-state"
    "scrub cell back in build.rs|src/window/build.rs|let grid_scrub_active = Rc::new(Cell::new(false));|scroll-motion"
    "pump constant back in build.rs|src/window/build.rs|const FOLDER_THUMBNAIL_MOTION_PUMP_FRAMES: u8 = 12;|scroll-motion"
    "pairing by hand in build.rs|src/window/build.rs|crate::image_format::retain_raw_jpeg_pair_mode(&mut photos, mode);|photo-list"
    "Albums home fed in layout.rs|src/window/layout.rs|albums_view::refresh(&albums_home, &albums, connection.clone(), size, on_album, on_change);|album-list"
    "album rows fed in albums.rs|src/window/albums.rs|sidebar::refresh_album_rows(sidebar, &albums, &context.delete_album);|album-list"
    "Files check by hand in library.rs|src/window/library.rs|apply_files_check(&mut photos, &scope);|photo-list"
    "scrub sampler back in build.rs|src/window/build.rs|let folder_scrub_sampler = Rc::new(RefCell::new(FolderScrollbarScrub::default()));|scroll-motion"
    "metadata panel fed in immersive.rs|src/window/immersive.rs|panel.set_navigation_view(\"photos\", filter);|front-mirrors"
    "shown photo written in build.rs|src/window/build.rs|info.photo_updated(&photo);|photo-change"
    "Library counts written in albums.rs|src/window/albums.rs|sidebar::update_library_counts(&sidebar, counts);|photo-change"
  )
  local entry name file line rule
  for entry in "${cases[@]}"; do
    IFS='|' read -r name file line rule <<<"$entry"
    case_root="$tmp/case"
    rm -rf "$case_root"
    cp -R "$tmp/ok" "$case_root"
    printf '    %s\n' "$line" >>"$case_root/$file"
    if output="$(check "$case_root" 2>&1)"; then
      echo "FAIL: $name: violation accepted"; failed=1
    elif [[ "$output" != *"[$rule]"* ]]; then
      echo "FAIL: $name: expected [$rule], got:"; echo "$output"; failed=1
    else
      echo "pass: $name rejected as [$rule]"
    fi
  done

  case_root="$tmp/case"
  rm -rf "$case_root"
  cp -R "$tmp/ok" "$case_root"
  grep -vF 'compact_dock.set_visible(false);' "$tmp/ok/src/window/layout.rs" >"$case_root/src/window/layout.rs"
  if output="$(check "$case_root" 2>&1)"; then
    echo "FAIL: removed allowed line: stale entry accepted"; failed=1
  elif [[ "$output" != *"[stale-allow]"* ]]; then
    echo "FAIL: removed allowed line: expected [stale-allow], got:"; echo "$output"; failed=1
  else
    echo "pass: removed allowed line rejected as [stale-allow]"
  fi

  if ((failed)); then
    echo "check-presentation-owners self-test: FAILED" >&2
    return 1
  fi
  echo "check-presentation-owners self-test: ok"
}

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
case "${1:-}" in
  --self-test) self_test ;;
  --root) check "$(cd "${2:?--root needs a directory}" && pwd)" ;;
  "") check "$root" ;;
  *) echo "usage: $0 [--root DIR | --self-test]" >&2; exit 2 ;;
esac
