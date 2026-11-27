//! Window keyboard dispatch (candidate 23): which command a key press means,
//! decided from plain inputs. The window's one capture-phase key controller
//! (`shortcuts.rs`) asks [`key_command`] and carries the command out.
//!
//! The rules, in order: the editor's Escape, then a photo context menu's;
//! typing in a text field keeps every other key except Escape, F11 and (in
//! an empty search field) Space; then the viewer, Immersive Mode and the
//! date browser; then the gallery's browse, selection and rating keys.
//! A shortcut held with Ctrl, Alt, Super or Meta is a different shortcut:
//! only Escape, F11 and the gallery's Ctrl keys accept modifiers.

use std::rc::Rc;

use gtk4::gdk::{Key, ModifierType};

use super::front_surface::Front;

/// Whether keyboard focus is in a text field.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Typing {
    No,
    /// An entry or text view that takes typed text.
    Text,
    /// An empty search field: Space still opens the selected photo.
    EmptySearch,
}

#[derive(Clone, Copy, Debug)]
pub(super) struct KeyContext {
    pub(super) front: Front,
    pub(super) typing: Typing,
    /// The photo editor's session is open (its Escape and Space handlers
    /// are installed).
    pub(super) editor_open: bool,
    pub(super) context_menu_open: bool,
    pub(super) immersive: bool,
    /// The search field's suggestion list is shown.
    pub(super) search_suggestions_shown: bool,
    /// The search field holds a query (the gallery shows its results).
    pub(super) search_active: bool,
    /// Focus is inside the gallery's grid (it moves its own focus with
    /// plain arrows).
    pub(super) focus_in_gallery: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum KeyCommand {
    /// Not a window shortcut: let the focused widget have it.
    Pass,
    /// Swallow the key, doing nothing.
    Consume,
    EditorEscape,
    DismissContextMenu,
    CloseViewer,
    /// Clear the search field, wherever focus is (issue #268).
    ClearSearch,
    LeaveImmersive,
    /// Step the date browser back; passes the key on when it cannot.
    DateBrowserBack,
    ToggleImmersive,
    ToggleFullscreen,
    /// -1 previous photo, +1 next.
    ViewerPhoto(i32),
    /// -1 previous collection, +1 next.
    ViewerCollection(i32),
    /// The editor's Space (Fit / 1:1).
    EditorSpace,
    ViewerToggleOneToOne,
    OpenSelected,
    /// A gallery browse key (arrows, Page Up/Down, Home/End, with Ctrl for
    /// Home/End/Up/Down); passes the key on when the gallery cannot move.
    GalleryBrowse {
        control: bool,
    },
    SelectAll,
    ClearSelection,
    /// 0 clears the rating, 1-5 set it.
    SetRating(u8),
}

/// The gallery's side of the window keys, filled in once the gallery
/// exists (the key controller is installed before it, to run first).
#[derive(Clone)]
pub(super) struct GalleryKeys {
    /// Whether focus is inside the gallery's grid.
    pub(super) focus_in_gallery: Rc<dyn Fn() -> bool>,
    /// Carry out a gallery command for `key`; false when nothing moved, so
    /// the key goes on to the focused widget.
    pub(super) run: Rc<dyn Fn(KeyCommand, Key) -> bool>,
}

fn held(mods: ModifierType) -> (bool, bool, bool) {
    let control = mods.contains(ModifierType::CONTROL_MASK);
    let other = mods
        .intersects(ModifierType::ALT_MASK | ModifierType::SUPER_MASK | ModifierType::META_MASK);
    let shift = mods.contains(ModifierType::SHIFT_MASK);
    (control, other, shift)
}

pub(super) fn key_command(key: Key, mods: ModifierType, context: &KeyContext) -> KeyCommand {
    use KeyCommand::*;
    let (control, other, shift) = held(mods);
    // Shift alone does not make a different shortcut (I and i are the same).
    let plain = !control && !other;
    let viewer = context.front == Front::Lightbox;
    let gallery = matches!(context.front, Front::Gallery | Front::DateOverview);

    // The editor may leave focus in the InfoBar, outside its own key
    // controller, so its Escape comes first; then a context menu, so one
    // Escape closes one layer.
    if key == Key::Escape && context.editor_open {
        return EditorEscape;
    }
    if key == Key::Escape && context.context_menu_open {
        return DismissContextMenu;
    }

    let typing_keeps_key = match context.typing {
        Typing::No => false,
        Typing::Text => !matches!(key, Key::F11 | Key::Escape),
        Typing::EmptySearch => !matches!(key, Key::F11 | Key::Escape | Key::space),
    };
    if typing_keeps_key {
        return Pass;
    }

    let back = key == Key::Escape || (key == Key::BackSpace && plain);
    if back && viewer {
        return CloseViewer;
    }
    // An active search is the next layer: after a click on a result tile
    // focus has left the search field, whose own Escape no longer applies.
    // A focused text field (the search field included) keeps its Escape.
    if key == Key::Escape && context.search_active && context.typing == Typing::No {
        return ClearSearch;
    }
    if key == Key::Escape && context.immersive {
        return LeaveImmersive;
    }
    if back {
        return DateBrowserBack;
    }
    if matches!(key, Key::i | Key::I) && plain {
        return ToggleImmersive;
    }
    if key == Key::F11 {
        return ToggleFullscreen;
    }

    if viewer {
        let photo = match key {
            Key::Left => Some(ViewerPhoto(-1)),
            Key::Right => Some(ViewerPhoto(1)),
            Key::Up => Some(ViewerCollection(-1)),
            Key::Down => Some(ViewerCollection(1)),
            _ => None,
        };
        if let Some(command) = photo {
            // Search suggestions own Up/Down while shown, even over the
            // viewer.
            if matches!(key, Key::Up | Key::Down) && context.search_suggestions_shown {
                return Pass;
            }
            // Consumed, not passed: the viewer's own controller would move
            // on any arrow.
            return if plain { command } else { Consume };
        }
    }

    if key == Key::space {
        if !plain {
            return Pass;
        }
        return if context.editor_open && context.front == Front::Editor {
            EditorSpace
        } else if viewer {
            ViewerToggleOneToOne
        } else {
            OpenSelected
        };
    }

    // The gallery's keys never apply while typing.
    if context.typing != Typing::No {
        return Pass;
    }

    if gallery && !other && !shift {
        let arrow = matches!(key, Key::Left | Key::Right | Key::Up | Key::Down);
        let browse = if control {
            matches!(key, Key::Home | Key::End | Key::Up | Key::Down)
        } else {
            arrow || matches!(key, Key::Page_Up | Key::Page_Down | Key::Home | Key::End)
        };
        // Plain arrows inside the grid move the grid's own focus.
        if browse && (!arrow || !context.focus_in_gallery || control) {
            return GalleryBrowse { control };
        }
    }

    if control && !other && matches!(key, Key::a | Key::A) {
        return if gallery { SelectAll } else { Pass };
    }
    if control && !other && matches!(key, Key::d | Key::D) {
        return if gallery { ClearSelection } else { Pass };
    }

    // Bare digits are reserved outside text fields, so a year such as 2026
    // cannot start type-to-search: 0-5 rate the current photos, 6-9 do
    // nothing.
    if plain {
        if let Some(digit) = key.to_unicode().and_then(|c| c.to_digit(10)) {
            return if digit <= 5 && (gallery || viewer) {
                SetRating(digit as u8)
            } else {
                Consume
            };
        }
    }
    Pass
}

#[cfg(test)]
mod tests {
    use super::*;
    use KeyCommand::*;

    const NONE: ModifierType = ModifierType::empty();
    const CTRL: ModifierType = ModifierType::CONTROL_MASK;
    const ALT: ModifierType = ModifierType::ALT_MASK;
    const SHIFT: ModifierType = ModifierType::SHIFT_MASK;
    const SUPER: ModifierType = ModifierType::SUPER_MASK;

    fn on(front: Front) -> KeyContext {
        KeyContext {
            front,
            typing: Typing::No,
            editor_open: false,
            context_menu_open: false,
            immersive: false,
            search_suggestions_shown: false,
            search_active: false,
            focus_in_gallery: false,
        }
    }

    fn check(cases: &[(&str, Key, ModifierType, KeyContext, KeyCommand)]) {
        for (case, key, mods, context, command) in cases {
            assert_eq!(key_command(*key, *mods, context), *command, "{case}");
        }
    }

    #[test]
    fn escape_closes_one_layer_at_a_time() {
        let editor = KeyContext {
            editor_open: true,
            context_menu_open: true,
            ..on(Front::Editor)
        };
        let menu_over_viewer = KeyContext {
            context_menu_open: true,
            ..on(Front::Lightbox)
        };
        let immersive_viewer = KeyContext {
            immersive: true,
            ..on(Front::Lightbox)
        };
        let immersive = KeyContext {
            immersive: true,
            ..on(Front::Gallery)
        };
        let typing_over_viewer = KeyContext {
            typing: Typing::Text,
            ..on(Front::Lightbox)
        };
        let searching = KeyContext {
            search_active: true,
            ..on(Front::Gallery)
        };
        let searching_immersive = KeyContext {
            immersive: true,
            ..searching
        };
        let searching_viewer = KeyContext {
            search_active: true,
            ..on(Front::Lightbox)
        };
        check(&[
            ("the viewer before a search", Key::Escape, NONE, searching_viewer, CloseViewer),
            ("then a search", Key::Escape, NONE, searching, ClearSearch),
            ("a search before Immersive Mode", Key::Escape, NONE, searching_immersive, ClearSearch),
            ("Backspace does not clear a search", Key::BackSpace, NONE, searching, DateBrowserBack),
            (
                "a focused text field keeps Escape",
                Key::Escape,
                NONE,
                KeyContext { typing: Typing::Text, ..searching },
                DateBrowserBack,
            ),
            ("editor first", Key::Escape, NONE, editor, EditorEscape),
            (
                "then a context menu",
                Key::Escape,
                NONE,
                menu_over_viewer,
                DismissContextMenu,
            ),
            (
                "then the viewer",
                Key::Escape,
                NONE,
                immersive_viewer,
                CloseViewer,
            ),
            (
                "then Immersive Mode",
                Key::Escape,
                NONE,
                immersive,
                LeaveImmersive,
            ),
            (
                "then the date browser",
                Key::Escape,
                NONE,
                on(Front::DateOverview),
                DateBrowserBack,
            ),
            (
                "also on other pages",
                Key::Escape,
                NONE,
                on(Front::Albums),
                DateBrowserBack,
            ),
            (
                "Escape takes modifiers",
                Key::Escape,
                CTRL,
                on(Front::Lightbox),
                CloseViewer,
            ),
            (
                "typing does not keep Escape",
                Key::Escape,
                NONE,
                typing_over_viewer,
                CloseViewer,
            ),
            (
                "Backspace closes the viewer",
                Key::BackSpace,
                NONE,
                on(Front::Lightbox),
                CloseViewer,
            ),
            (
                "Backspace steps back",
                Key::BackSpace,
                NONE,
                on(Front::DateOverview),
                DateBrowserBack,
            ),
            (
                "Ctrl+Backspace is not back",
                Key::BackSpace,
                CTRL,
                on(Front::Lightbox),
                Pass,
            ),
            (
                "Alt+Backspace is not back",
                Key::BackSpace,
                ALT,
                on(Front::DateOverview),
                Pass,
            ),
            (
                "Backspace does not leave Immersive",
                Key::BackSpace,
                NONE,
                immersive,
                DateBrowserBack,
            ),
        ]);
    }

    #[test]
    fn typing_keeps_its_keys() {
        let text = KeyContext {
            typing: Typing::Text,
            ..on(Front::Gallery)
        };
        let empty_search = KeyContext {
            typing: Typing::EmptySearch,
            ..on(Front::Gallery)
        };
        let search_over_viewer = KeyContext {
            typing: Typing::Text,
            ..on(Front::Lightbox)
        };
        check(&[
            ("space in text", Key::space, NONE, text, Pass),
            ("i in text", Key::i, NONE, text, Pass),
            ("digit in text", Key::_2, NONE, text, Pass),
            ("Ctrl+A in text", Key::a, CTRL, text, Pass),
            ("arrows in text", Key::Left, NONE, text, Pass),
            (
                "arrows in text over the viewer",
                Key::Left,
                NONE,
                search_over_viewer,
                Pass,
            ),
            ("F11 in text", Key::F11, NONE, text, ToggleFullscreen),
            (
                "space in an empty search opens",
                Key::space,
                NONE,
                empty_search,
                OpenSelected,
            ),
            (
                "letters in an empty search",
                Key::i,
                NONE,
                empty_search,
                Pass,
            ),
            (
                "digits in an empty search",
                Key::_1,
                NONE,
                empty_search,
                Pass,
            ),
        ]);
    }

    /// Candidate 23's bug: viewer arrows took no account of modifiers, so
    /// Alt+Left changed the photo.
    #[test]
    fn viewer_keys_need_no_modifier() {
        let viewer = on(Front::Lightbox);
        let suggestions = KeyContext {
            search_suggestions_shown: true,
            ..viewer
        };
        check(&[
            ("Left", Key::Left, NONE, viewer, ViewerPhoto(-1)),
            ("Right", Key::Right, NONE, viewer, ViewerPhoto(1)),
            ("Up", Key::Up, NONE, viewer, ViewerCollection(-1)),
            ("Down", Key::Down, NONE, viewer, ViewerCollection(1)),
            ("Shift+Right", Key::Right, SHIFT, viewer, ViewerPhoto(1)),
            ("Alt+Left", Key::Left, ALT, viewer, Consume),
            ("Ctrl+Right", Key::Right, CTRL, viewer, Consume),
            ("Super+Up", Key::Up, SUPER, viewer, Consume),
            ("suggestions own Up", Key::Up, NONE, suggestions, Pass),
            (
                "suggestions leave Left",
                Key::Left,
                NONE,
                suggestions,
                ViewerPhoto(-1),
            ),
            (
                "Space toggles 1:1",
                Key::space,
                NONE,
                viewer,
                ViewerToggleOneToOne,
            ),
            ("Ctrl+Space is not Space", Key::space, CTRL, viewer, Pass),
            ("rating in the viewer", Key::_4, NONE, viewer, SetRating(4)),
            ("Alt+digit is not a rating", Key::_4, ALT, viewer, Pass),
        ]);
    }

    #[test]
    fn space_immersive_and_fullscreen() {
        let editor = KeyContext {
            editor_open: true,
            ..on(Front::Editor)
        };
        let editor_fading_out = KeyContext {
            editor_open: true,
            ..on(Front::Gallery)
        };
        check(&[
            (
                "Space opens the selection",
                Key::space,
                NONE,
                on(Front::Gallery),
                OpenSelected,
            ),
            ("Space in the editor", Key::space, NONE, editor, EditorSpace),
            (
                "editor not in front",
                Key::space,
                NONE,
                editor_fading_out,
                OpenSelected,
            ),
            (
                "Space on Albums",
                Key::space,
                NONE,
                on(Front::Albums),
                OpenSelected,
            ),
            ("i", Key::i, NONE, on(Front::Gallery), ToggleImmersive),
            ("I", Key::I, SHIFT, on(Front::Gallery), ToggleImmersive),
            ("Ctrl+I", Key::i, CTRL, on(Front::Gallery), Pass),
            ("F11", Key::F11, NONE, on(Front::Editor), ToggleFullscreen),
        ]);
    }

    #[test]
    fn gallery_keys() {
        let gallery = on(Front::Gallery);
        let in_grid = KeyContext {
            focus_in_gallery: true,
            ..gallery
        };
        check(&[
            (
                "arrow outside the grid",
                Key::Left,
                NONE,
                gallery,
                GalleryBrowse { control: false },
            ),
            ("arrow inside the grid", Key::Left, NONE, in_grid, Pass),
            (
                "Page Down inside the grid",
                Key::Page_Down,
                NONE,
                in_grid,
                GalleryBrowse { control: false },
            ),
            (
                "Ctrl+Down inside the grid",
                Key::Down,
                CTRL,
                in_grid,
                GalleryBrowse { control: true },
            ),
            (
                "Ctrl+Home",
                Key::Home,
                CTRL,
                gallery,
                GalleryBrowse { control: true },
            ),
            ("Ctrl+Left", Key::Left, CTRL, gallery, Pass),
            ("Shift+Down", Key::Down, SHIFT, gallery, Pass),
            ("Alt+Home", Key::Home, ALT, gallery, Pass),
            (
                "date overview browses",
                Key::End,
                NONE,
                on(Front::DateOverview),
                GalleryBrowse { control: false },
            ),
            ("not on Albums", Key::End, NONE, on(Front::Albums), Pass),
            ("Ctrl+A", Key::a, CTRL, gallery, SelectAll),
            ("Ctrl+Shift+A", Key::A, CTRL | SHIFT, gallery, SelectAll),
            (
                "Ctrl+A off the gallery",
                Key::a,
                CTRL,
                on(Front::Lightbox),
                Pass,
            ),
            ("Ctrl+D", Key::d, CTRL, gallery, ClearSelection),
            ("Ctrl+Alt+D", Key::d, CTRL | ALT, gallery, Pass),
            ("0 clears", Key::_0, NONE, gallery, SetRating(0)),
            ("5", Key::_5, NONE, gallery, SetRating(5)),
            ("keypad 3", Key::KP_3, NONE, gallery, SetRating(3)),
            ("6 is reserved", Key::_6, NONE, gallery, Consume),
            (
                "digits on Albums are reserved",
                Key::_3,
                NONE,
                on(Front::Albums),
                Consume,
            ),
            ("Ctrl+1", Key::_1, CTRL, gallery, Pass),
        ]);
    }
}
