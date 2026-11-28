//! Continuous edit: the one rule that turns a gesture into Undo steps.
//!
//! A slider drag, or a burst of typing, is one Undo step however many
//! values it passes through; a keyboard or wheel step on the same slider is a
//! step of its own. The pure [`ContinuousEdit`] owns that rule against an
//! [`EditSession`]; [`bind_scale`] is its GTK adapter for a `gtk::Scale`
//! (press, physical release, cancel). Panels supply only "apply this value".

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gtk::glib;
use gtk::prelude::*;
use gtk4 as gtk;

use super::model::{EditRecipe, EditSession};

/// Whether a pointer (or a typing burst) currently holds one open action.
#[derive(Debug, Default)]
pub struct ContinuousEdit {
    held: bool,
}

impl ContinuousEdit {
    /// The pointer went down on the control: open one action.
    pub fn press(&mut self, session: &mut EditSession) {
        session.begin_action();
        self.held = true;
    }

    /// One new value. Held: part of the open action. Not held (keyboard,
    /// wheel): its own Undo step.
    pub fn change(&mut self, session: &mut EditSession, apply: impl FnOnce(&mut EditRecipe)) {
        if self.held {
            session.mutate_active(apply);
        } else {
            session.mutate(apply);
        }
    }

    /// One typed value: the first opens the action, the rest join it until
    /// [`settle`](Self::settle) (the typing pause).
    pub fn type_change(&mut self, session: &mut EditSession, apply: impl FnOnce(&mut EditRecipe)) {
        self.held = true;
        session.mutate_active(apply);
    }

    /// Release, cancel or typing pause: close the action, keeping the value
    /// reached. `None` when nothing was held; otherwise whether it added an
    /// Undo step.
    pub fn settle(&mut self, session: &mut EditSession) -> Option<bool> {
        if !std::mem::replace(&mut self.held, false) {
            return None;
        }
        Some(session.end_action())
    }

    pub fn held(&self) -> bool {
        self.held
    }
}

/// What a panel tells the scale adapter besides "apply this value".
pub struct ScaleHooks {
    /// True while the panel writes the scale from the recipe (no edit).
    pub syncing: Rc<Cell<bool>>,
    /// False when there is nothing to edit (no layer selected): presses and
    /// values are ignored.
    pub enabled: Rc<dyn Fn() -> bool>,
    /// While held, a value to set instead of the current one (a detent).
    pub snap: Option<Rc<dyn Fn(&gtk::Scale) -> Option<f64>>>,
    /// After each applied value (preview, canvas, history buttons).
    pub changed: Rc<dyn Fn()>,
    /// After the action closed (history buttons, final redraw).
    pub settled: Rc<dyn Fn()>,
    /// Print `PIC_EDIT slider_action_begin/end` under `PICASA_TRACE`.
    pub trace: bool,
}

/// Bind a scale to `session` under the continuous-edit rule. GtkScale's own
/// drag gesture can cancel a sibling `GestureClick` once the pointer moves,
/// so the physical release is also caught by a capture-phase legacy
/// controller, and a cancel settles too: a lost release cannot leave the
/// action open.
pub fn bind_scale(
    scale: &gtk::Scale,
    session: Rc<RefCell<EditSession>>,
    hooks: ScaleHooks,
    apply: impl Fn(&mut EditRecipe, f64) + 'static,
) {
    let edit = Rc::new(RefCell::new(ContinuousEdit::default()));
    let ScaleHooks {
        syncing,
        enabled,
        snap,
        changed,
        settled,
        trace,
    } = hooks;
    let settle: Rc<dyn Fn()> = {
        let edit = edit.clone();
        let session = session.clone();
        Rc::new(move || {
            let closed = edit.borrow_mut().settle(&mut session.borrow_mut());
            if let Some(added) = closed {
                settled();
                if trace && std::env::var_os("PICASA_TRACE").is_some() {
                    eprintln!("PIC_EDIT slider_action_end changed={added}");
                }
            }
        })
    };

    let press = gtk::GestureClick::new();
    press.set_button(1);
    press.set_propagation_phase(gtk::PropagationPhase::Capture);
    {
        let edit = edit.clone();
        let session = session.clone();
        let enabled = enabled.clone();
        press.connect_pressed(move |_, _, _, _| {
            if !enabled() {
                return;
            }
            edit.borrow_mut().press(&mut session.borrow_mut());
            if trace && std::env::var_os("PICASA_TRACE").is_some() {
                eprintln!("PIC_EDIT slider_action_begin");
            }
        });
    }
    {
        let settle = settle.clone();
        press.connect_released(move |_, _, _, _| settle());
    }
    {
        let settle = settle.clone();
        press.connect_cancel(move |_, _| settle());
    }
    scale.add_controller(press);

    let release = gtk::EventControllerLegacy::new();
    release.set_propagation_phase(gtk::PropagationPhase::Capture);
    {
        let settle = settle.clone();
        release.connect_event(move |_, event| {
            if event.event_type() == gtk::gdk::EventType::ButtonRelease
                && event
                    .downcast_ref::<gtk::gdk::ButtonEvent>()
                    .is_some_and(|button| button.button() == 1)
            {
                settle();
            }
            glib::Propagation::Proceed
        });
    }
    scale.add_controller(release);

    scale.connect_value_changed(move |scale| {
        if syncing.get() || !enabled() {
            return;
        }
        let held = edit.borrow().held();
        if held {
            if let Some(snapped) = snap.as_ref().and_then(|snap| snap(scale)) {
                if (scale.value() - snapped).abs() > f64::EPSILON {
                    // The nested value-changed applies the snapped value.
                    scale.set_value(snapped);
                    return;
                }
            }
        }
        let value = scale.value();
        edit.borrow_mut()
            .change(&mut session.borrow_mut(), |recipe| apply(recipe, value));
        changed();
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn exposure(session: &EditSession) -> f32 {
        session.recipe.exposure
    }

    #[test]
    #[ignore = "requires a GTK display; run under xvfb-run with --ignored --test-threads=1"]
    fn a_scale_drag_cancelled_by_gtk_is_one_undo_step() {
        gtk::init().unwrap();
        let scale = gtk::Scale::with_range(gtk::Orientation::Horizontal, 0.0, 1.0, 0.01);
        let session = Rc::new(RefCell::new(EditSession::new(EditRecipe::default())));
        let settled = Rc::new(Cell::new(0));
        bind_scale(
            &scale,
            session.clone(),
            ScaleHooks {
                syncing: Rc::new(Cell::new(false)),
                enabled: Rc::new(|| true),
                snap: None,
                changed: Rc::new(|| {}),
                settled: {
                    let settled = settled.clone();
                    Rc::new(move || settled.set(settled.get() + 1))
                },
                trace: false,
            },
            |recipe, value| recipe.exposure = value as f32,
        );
        let press = (0..)
            .map_while(|i| scale.observe_controllers().item(i))
            .find_map(|item| item.downcast::<gtk::GestureClick>().ok())
            .expect("the adapter's press gesture");

        // GtkScale's drag cancels the click gesture: `released` never comes.
        press.emit_by_name::<()>("pressed", &[&1i32, &5.0f64, &5.0f64]);
        for value in [0.2, 0.5, 0.8] {
            scale.set_value(value);
        }
        press.emit_by_name::<()>("cancel", &[&None::<gtk::gdk::EventSequence>]);
        assert_eq!(settled.get(), 1);
        // A keyboard step afterwards is its own step, not part of the drag.
        scale.set_value(0.9);

        let mut session = session.borrow_mut();
        assert!(!session.action_active());
        assert!(session.undo());
        assert_eq!(session.recipe.exposure, 0.8);
        assert!(session.undo());
        assert_eq!(session.recipe.exposure, 0.0);
        assert!(!session.undo());
    }

    #[test]
    fn a_drag_is_one_undo_step_and_keyboard_steps_are_their_own() {
        let mut session = EditSession::new(EditRecipe::default());
        let mut edit = ContinuousEdit::default();
        edit.press(&mut session);
        for value in [0.1, 0.4, 0.9] {
            edit.change(&mut session, |r| r.exposure = value);
        }
        assert_eq!(edit.settle(&mut session), Some(true));
        edit.change(&mut session, |r| r.exposure = 1.0);
        edit.change(&mut session, |r| r.exposure = 1.1);

        assert!(session.undo());
        assert_eq!(exposure(&session), 1.0);
        assert!(session.undo());
        assert_eq!(exposure(&session), 0.9);
        assert!(session.undo());
        assert_eq!(exposure(&session), 0.0);
        assert!(!session.undo());
    }

    #[test]
    fn a_cancelled_drag_keeps_its_value_as_one_step() {
        let mut session = EditSession::new(EditRecipe::default());
        let mut edit = ContinuousEdit::default();
        edit.press(&mut session);
        edit.change(&mut session, |r| r.exposure = 0.7);
        // Cancel and release both settle; the second finds nothing held.
        assert_eq!(edit.settle(&mut session), Some(true));
        assert_eq!(edit.settle(&mut session), None);
        assert_eq!(exposure(&session), 0.7);
        assert!(session.undo());
        assert_eq!(exposure(&session), 0.0);
    }

    #[test]
    fn a_press_without_a_change_adds_no_step() {
        let mut session = EditSession::new(EditRecipe::default());
        let mut edit = ContinuousEdit::default();
        edit.press(&mut session);
        assert_eq!(edit.settle(&mut session), Some(false));
        assert!(!session.can_undo());
    }

    #[test]
    fn undo_during_a_drag_leaves_the_rest_of_the_drag_undoable() {
        let mut session = EditSession::new(EditRecipe::default());
        let mut edit = ContinuousEdit::default();
        edit.press(&mut session);
        edit.change(&mut session, |r| r.exposure = 0.5);
        assert!(session.undo()); // Ctrl+Z with the button still down
        edit.change(&mut session, |r| r.exposure = 0.8);
        edit.change(&mut session, |r| r.exposure = 0.9);
        assert_eq!(edit.settle(&mut session), Some(true));

        assert_eq!(exposure(&session), 0.9);
        assert!(session.undo());
        assert_eq!(exposure(&session), 0.0);
    }

    #[test]
    fn a_long_drag_keeps_the_earlier_history() {
        let mut session = EditSession::new(EditRecipe::default());
        let mut edit = ContinuousEdit::default();
        for step in 1..=99 {
            edit.change(&mut session, |r| r.contrast = step as f32 / 100.0);
        }
        edit.press(&mut session);
        for tick in 1..=500 {
            edit.change(&mut session, |r| r.exposure = tick as f32 / 500.0);
        }
        edit.settle(&mut session);

        let mut steps = 0;
        while session.undo() {
            steps += 1;
        }
        assert_eq!(steps, 100, "99 keyboard steps and one drag");
        assert!(session.recipe.is_default());
    }

    #[test]
    fn typing_is_one_step_until_the_pause() {
        let mut session = EditSession::new(EditRecipe::default());
        let mut edit = ContinuousEdit::default();
        for value in [0.1, 0.2, 0.3] {
            edit.type_change(&mut session, |r| r.exposure = value);
        }
        assert_eq!(edit.settle(&mut session), Some(true));
        edit.type_change(&mut session, |r| r.exposure = 0.4);
        assert_eq!(edit.settle(&mut session), Some(true));

        assert!(session.undo());
        assert_eq!(exposure(&session), 0.3);
        assert!(session.undo());
        assert_eq!(exposure(&session), 0.0);
    }

    #[test]
    fn another_edit_during_typing_closes_the_typed_step_first() {
        let mut session = EditSession::new(EditRecipe::default());
        let mut edit = ContinuousEdit::default();
        edit.type_change(&mut session, |r| r.exposure = 0.3);
        session.mutate(|r| r.contrast = 0.5); // e.g. a font change
        edit.type_change(&mut session, |r| r.exposure = 0.4);
        edit.settle(&mut session);

        assert!(session.undo());
        assert_eq!((exposure(&session), session.recipe.contrast), (0.3, 0.5));
        assert!(session.undo());
        assert_eq!((exposure(&session), session.recipe.contrast), (0.3, 0.0));
        assert!(session.undo());
        assert!(session.recipe.is_default());
    }
}
