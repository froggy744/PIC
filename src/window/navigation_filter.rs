//! The sidebar destination being browsed, with a change notification.
//!
//! Several destinations share the Photos stack page, so a page change alone
//! does not say that the destination changed. Readers that must follow the
//! destination subscribe with [`NavigationFilter::connect_changed`] instead
//! of polling it every frame (candidate 24 removed such a tick).

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use crate::sidebar::SidebarFilter;

type Listener = Rc<dyn Fn(SidebarFilter)>;

pub(crate) struct NavigationFilter {
    value: Cell<SidebarFilter>,
    listeners: RefCell<Vec<Listener>>,
}

impl NavigationFilter {
    pub(crate) fn new(value: SidebarFilter) -> Rc<Self> {
        Rc::new(Self {
            value: Cell::new(value),
            listeners: RefCell::default(),
        })
    }

    pub(crate) fn get(&self) -> SidebarFilter {
        self.value.get()
    }

    /// Store `value`; listeners run, in connection order, only when it
    /// differs from the current one. A listener may set the filter again.
    pub(crate) fn set(&self, value: SidebarFilter) {
        if self.value.replace(value) == value {
            return;
        }
        let listeners = self.listeners.borrow().clone();
        for listener in listeners {
            listener(value);
        }
    }

    pub(crate) fn connect_changed(&self, listener: impl Fn(SidebarFilter) + 'static) {
        self.listeners.borrow_mut().push(Rc::new(listener));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn listeners_hear_changes_only() {
        let filter = NavigationFilter::new(SidebarFilter::All);
        let heard: Rc<RefCell<Vec<SidebarFilter>>> = Rc::default();
        {
            let heard = heard.clone();
            filter.connect_changed(move |value| heard.borrow_mut().push(value));
        }
        filter.set(SidebarFilter::All);
        filter.set(SidebarFilter::Favorites);
        filter.set(SidebarFilter::Favorites);
        filter.set(SidebarFilter::Folder(4));
        assert_eq!(filter.get(), SidebarFilter::Folder(4));
        assert_eq!(
            heard.borrow().as_slice(),
            &[SidebarFilter::Favorites, SidebarFilter::Folder(4)]
        );
    }

    #[test]
    fn a_listener_may_set_the_filter_again() {
        let filter = NavigationFilter::new(SidebarFilter::All);
        let calls = Rc::new(Cell::new(0));
        {
            let (weak, calls) = (Rc::downgrade(&filter), calls.clone());
            filter.connect_changed(move |value| {
                calls.set(calls.get() + 1);
                if value == SidebarFilter::Albums {
                    weak.upgrade().unwrap().set(SidebarFilter::All);
                }
            });
        }
        filter.set(SidebarFilter::Albums);
        assert_eq!(filter.get(), SidebarFilter::All);
        assert_eq!(calls.get(), 2);
    }
}
