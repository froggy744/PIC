#!/usr/bin/env bash
# Presentation-owner guard.
#
# Sidebar, Properties and InfoBar presentation each have one GTK adapter that
# is the only writer of their widgets (docs/architecture/STATUS.md,
# candidates 2-4):
#   - src/window/sidebar_widgets.rs    (SidebarWidgets)
#   - src/window/properties_widgets.rs (PropertiesWidgets)
#   - src/infobar.rs                   (InfoBar; the decision is infobar::presentation)
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
    echo "Route them through SidebarWidgets / PropertiesWidgets (AUDIT-INDEX.md, Quick-Edit Guide)." >&2
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
  cat >"$root/src/window/photo_actions.rs" <<'RS'
fn properties_placeholder(on_close: Rc<dyn Fn()>) -> gtk::Box {}
fn properties_photo_content(
RS
  cat >"$root/src/window/build.rs" <<'RS'
    // properties_split.set_show_sidebar(true) is PropertiesWidgets' job.
    properties_split.set_show_sidebar(false);
    lightbox_properties_panel.set_visible(false);
    main_split_for_drag_update.set_max_sidebar_width(width);
RS
  local entry apath atext
  for entry in "${ALLOW[@]}"; do
    IFS='|' read -r apath atext _ <<<"$entry"
    [[ "$apath" == "src/window/layout.rs" ]] && printf '    %s\n' "$atext" >>"$root/src/window/layout.rs"
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
    "dock shown in build.rs|src/window/build.rs|compact_dock.set_visible(true);|sidebar-parts"
    "column hidden in immersive.rs|src/window/immersive.rs|left_column.set_visible(false);|sidebar-parts"
    "shell child in build.rs|src/window/build.rs|sidebar_shell.set_child(Some(&compact_sidebar_page));|sidebar-parts"
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
