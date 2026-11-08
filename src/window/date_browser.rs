use chrono::Datelike;
use gtk4 as gtk;
use gtk::prelude::*;
use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::rc::Rc;
use crate::photo_object::PhotoObject;

/// The calendar bucket used by the existing capture-date presentation.
pub(super) fn capture_month(value: Option<&str>) -> Option<(i32, u32)> {
    let date = crate::grid::parse_photo_date(value?)?;
    Some((date.year(), date.month()))
}

/// Photo indices grouped by year and month. Unknown dates remain selectable.
pub(super) fn buckets<'a>(dates: impl IntoIterator<Item = Option<&'a str>>) -> BTreeMap<Option<(i32, u32)>, Vec<usize>> {
    let mut buckets = BTreeMap::new();
    for (index, date) in dates.into_iter().enumerate() {
        buckets.entry(capture_month(date)).or_insert_with(Vec::new).push(index);
    }
    buckets
}

fn year_groups(groups: &BTreeMap<Option<(i32, u32)>, Vec<usize>>) -> Vec<(Option<i32>, usize, usize)> {
    let mut years = BTreeMap::<i32, (usize, usize)>::new();
    for (key, indices) in groups {
        if let Some((year, _)) = key {
            if let Some(&first) = indices.first() {
                let entry = years.entry(*year).or_insert((first, 0));
                entry.0 = entry.0.min(first);
                entry.1 += indices.len();
            }
        }
    }
    let mut result = years.into_iter().rev().map(|(year, (cover, count))| (Some(year), cover, count)).collect::<Vec<_>>();
    if let Some(indices) = groups.get(&None) {
        if let Some(&first) = indices.first() { result.push((None, first, indices.len())); }
    }
    result
}

/// Overview widget. The owner keeps the normal gallery beside this widget;
/// overview callbacks hide it and the month callback reveals its filtered view.
#[derive(Clone)]
pub(super) struct DateBrowser(Rc<BrowserInner>);

struct BrowserInner {
    root: gtk::Box,
    breadcrumbs: gtk::Box,
    scroll: gtk::ScrolledWindow,
    tiles: gtk::FlowBox,
    photos: RefCell<Vec<PhotoObject>>,
    groups: RefCell<BTreeMap<Option<(i32, u32)>, Vec<usize>>>,
    level: Cell<BrowserLevel>,
    descending: Cell<bool>,
    sort_field: Cell<super::SortField>,
    tile_width: Cell<i32>,
    on_month: Rc<dyn Fn(Option<(i32, u32)>)>,
    on_overview: Rc<dyn Fn()>,
    on_exit: Rc<dyn Fn()>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BrowserLevel {
    Years,
    Months(i32),
    Photos(Option<(i32, u32)>),
}

impl DateBrowser {
    pub(super) fn new(on_month: Rc<dyn Fn(Option<(i32, u32)>)>, on_overview: Rc<dyn Fn()>, on_exit: Rc<dyn Fn()>) -> Self {
        let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
        root.set_vexpand(true);
        let breadcrumbs = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        breadcrumbs.set_margin_start(20);
        breadcrumbs.set_margin_end(20);
        breadcrumbs.set_margin_top(8);
        breadcrumbs.set_margin_bottom(8);
        let tiles = gtk::FlowBox::new();
        tiles.set_selection_mode(gtk::SelectionMode::None);
        tiles.set_homogeneous(false);
        tiles.set_valign(gtk::Align::Start);
        tiles.set_max_children_per_line(6);
        tiles.set_min_children_per_line(1);
        tiles.set_column_spacing(16);
        tiles.set_row_spacing(16);
        tiles.set_margin_start(20);
        tiles.set_margin_end(20);
        tiles.set_margin_top(12);
        tiles.set_margin_bottom(20);
        let scroll = gtk::ScrolledWindow::builder().vexpand(true).child(&tiles).build();
        root.append(&breadcrumbs);
        root.append(&scroll);
        Self(Rc::new(BrowserInner { root, breadcrumbs, scroll, tiles, photos: RefCell::new(Vec::new()), groups: RefCell::new(BTreeMap::new()), level: Cell::new(BrowserLevel::Years), descending: Cell::new(true), sort_field: Cell::new(super::SortField::DateTaken), tile_width: Cell::new(180), on_month, on_overview, on_exit }))
    }

    pub(super) fn root(&self) -> &gtk::Box { &self.0.root }
    pub(super) fn is_overview_visible(&self) -> bool { self.0.scroll.is_visible() }

    pub(super) fn set_photos(&self, photos: &[PhotoObject]) {
        let dates = photos.iter().map(PhotoObject::taken_at).collect::<Vec<_>>();
        self.0.groups.replace(buckets(dates.iter().map(|date| date.as_deref())));
        self.0.photos.replace(photos.to_vec());
    }

    pub(super) fn indices(&self, bucket: Option<(i32, u32)>) -> Vec<usize> {
        let mut indices = self.0.groups.borrow().get(&bucket).cloned().unwrap_or_default();
        let photos = self.0.photos.borrow();
        let field = self.0.sort_field.get();
        indices.sort_by(|&left, &right| {
            let a = &photos[left];
            let b = &photos[right];
            let order = match field {
                super::SortField::DateTaken => {
                    let a = a.taken_at().and_then(|date| crate::grid::parse_photo_date(&date)).map(|date| date.timestamp_millis());
                    let b = b.taken_at().and_then(|date| crate::grid::parse_photo_date(&date)).map(|date| date.timestamp_millis());
                    a.cmp(&b)
                }
                super::SortField::DateAdded => a.mtime().cmp(&b.mtime()),
                super::SortField::Rating => a.rating().cmp(&b.rating()),
                super::SortField::Name => std::path::Path::new(&a.path()).file_name().cmp(&std::path::Path::new(&b.path()).file_name()),
                super::SortField::FileSize => a.size_bytes().cmp(&b.size_bytes()),
                super::SortField::Dimensions => (a.width() * a.height()).cmp(&(b.width() * b.height())),
            };
            let order = if self.0.descending.get() { order.reverse() } else { order };
            order.then_with(|| a.path().to_lowercase().cmp(&b.path().to_lowercase()))
        });
        indices
    }

    pub(super) fn is_photo_view(&self) -> bool { matches!(self.0.level.get(), BrowserLevel::Photos(_)) }
    pub(super) fn current_bucket(&self) -> Option<Option<(i32, u32)>> {
        match self.0.level.get() {
            BrowserLevel::Photos(bucket) => Some(bucket),
            _ => None,
        }
    }

    pub(super) fn resume_month(&self, bucket: Option<(i32, u32)>) {
        if let Some((year, _)) = bucket { self.show_months(year); }
        else { self.show_years(); }
        self.open_month(bucket);
    }

    pub(super) fn set_photo_sort(&self, field: super::SortField, descending: bool) {
        self.0.sort_field.set(field);
        self.0.descending.set(descending);
        if let BrowserLevel::Photos(bucket) = self.0.level.get() { (self.0.on_month)(bucket); }
    }

    pub(super) fn set_descending(&self, descending: bool) {
        self.0.descending.set(descending);
        match self.0.level.get() {
            BrowserLevel::Years => self.show_years(),
            BrowserLevel::Months(year) => self.show_months(year),
            BrowserLevel::Photos(bucket) => (self.0.on_month)(bucket),
        }
    }

    pub(super) fn set_tile_width(&self, width: i32) {
        self.0.tile_width.set(width.clamp(100, 300));
        match self.0.level.get() {
            BrowserLevel::Years => self.show_years(),
            BrowserLevel::Months(year) => self.show_months(year),
            BrowserLevel::Photos(_) => {}
        }
    }

    pub(super) fn show_years(&self) {
        self.0.level.set(BrowserLevel::Years);
        self.0.root.set_vexpand(true);
        (self.0.on_overview)();
        self.breadcrumbs(None, None);
        self.clear_tiles();
        self.0.scroll.set_visible(true);
        let mut years = year_groups(&self.0.groups.borrow());
        years.sort_by(|left, right| match (left.0, right.0) {
            (Some(a), Some(b)) if self.0.descending.get() => b.cmp(&a),
            (Some(a), Some(b)) => a.cmp(&b),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => std::cmp::Ordering::Equal,
        });
        for (year, cover, count) in years {
            let label = year.map_or_else(|| "Unknown Date".to_string(), |year| year.to_string());
            let weak = Rc::downgrade(&self.0);
            self.add_tile(&label, cover, count, move || {
                if let Some(inner) = weak.upgrade() {
                    let browser = DateBrowser(inner);
                    if let Some(year) = year { browser.show_months(year); }
                    else { browser.open_month(None); }
                }
            });
        }
    }

    pub(super) fn show_months(&self, year: i32) {
        self.0.level.set(BrowserLevel::Months(year));
        self.0.root.set_vexpand(true);
        (self.0.on_overview)();
        self.breadcrumbs(Some(year), None);
        self.clear_tiles();
        self.0.scroll.set_visible(true);
        let groups = self.0.groups.borrow();
        let months = groups.iter().filter_map(|(key, indices)| {
            let Some((group_year, month)) = key else { return None; };
            (*group_year == year).then_some((*month, indices))
        }).collect::<Vec<_>>();
        let months: Box<dyn Iterator<Item = (u32, &Vec<usize>)>> = if self.0.descending.get() {
            Box::new(months.into_iter().rev())
        } else {
            Box::new(months.into_iter())
        };
        for (month, indices) in months {
            let Some(&cover) = indices.first() else { continue; };
            let weak = Rc::downgrade(&self.0);
            self.add_tile(month_name(month), cover, indices.len(), move || {
                if let Some(inner) = weak.upgrade() { DateBrowser(inner).open_month(Some((year, month))); }
            });
        }
    }

    pub(super) fn show_month_photo(&self, bucket: Option<(i32, u32)>) {
        self.0.level.set(BrowserLevel::Photos(bucket));
        self.0.root.set_vexpand(false);
        self.0.scroll.set_visible(false);
        self.breadcrumbs(bucket.map(|(year, _)| year), Some(bucket));
    }

    pub(super) fn back(&self) -> bool {
        match self.0.level.get() {
            BrowserLevel::Years => (self.0.on_exit)(),
            BrowserLevel::Months(_) | BrowserLevel::Photos(None) => self.show_years(),
            BrowserLevel::Photos(Some((year, _))) => self.show_months(year),
        }
        true
    }

    fn open_month(&self, bucket: Option<(i32, u32)>) {
        self.show_month_photo(bucket);
        (self.0.on_month)(bucket);
    }

    fn clear_tiles(&self) {
        while let Some(child) = self.0.tiles.first_child() { self.0.tiles.remove(&child); }
    }

    fn breadcrumbs(&self, year: Option<i32>, month: Option<Option<(i32, u32)>>) {
        while let Some(child) = self.0.breadcrumbs.first_child() { self.0.breadcrumbs.remove(&child); }
        let all = gtk::Button::with_label("All Photos");
        all.add_css_class("flat");
        let weak = Rc::downgrade(&self.0);
        all.connect_clicked(move |_| { if let Some(inner) = weak.upgrade() { DateBrowser(inner).show_years(); } });
        self.0.breadcrumbs.append(&all);
        if let Some(year) = year {
            self.0.breadcrumbs.append(&gtk::Label::new(Some("›")));
            let button = gtk::Button::with_label(&year.to_string());
            button.add_css_class("flat");
            let weak = Rc::downgrade(&self.0);
            button.connect_clicked(move |_| { if let Some(inner) = weak.upgrade() { DateBrowser(inner).show_months(year); } });
            self.0.breadcrumbs.append(&button);
        }
        if let Some(month) = month {
            self.0.breadcrumbs.append(&gtk::Label::new(Some("›")));
            let text = month.map_or("Unknown Date", |(_, month)| month_name(month));
            let label = gtk::Label::new(Some(text));
            label.add_css_class("heading");
            self.0.breadcrumbs.append(&label);
        }
        let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        spacer.set_hexpand(true);
        self.0.breadcrumbs.append(&spacer);
        let exit = gtk::Button::with_label("Exit browsing");
        exit.add_css_class("flat");
        let callback = self.0.on_exit.clone();
        exit.connect_clicked(move |_| callback());
        self.0.breadcrumbs.append(&exit);
    }

    fn add_tile(&self, label: &str, index: usize, count: usize, activate: impl Fn() + 'static) {
        let button = gtk::Button::new();
        button.add_css_class("album-card");
        button.set_has_frame(false);
        let width = self.0.tile_width.get();
        button.set_size_request(width, -1);
        button.set_valign(gtk::Align::Start);
        button.set_vexpand(false);
        let column = gtk::Box::new(gtk::Orientation::Vertical, 4);
        let cover = gtk::Overlay::new();
        cover.add_css_class("photo-frame");
        cover.add_css_class("album-cover");
        let height = (f64::from(width) * (2.0 / 3.0)).round() as i32;
        cover.set_size_request(width, height);
        let placeholder = gtk::Image::from_icon_name("image-x-generic-symbolic");
        placeholder.add_css_class("dim-label");
        cover.set_child(Some(&placeholder));
        let picture = gtk::Picture::new();
        picture.add_css_class("thumbnail");
        picture.set_size_request(width, height);
        picture.set_content_fit(gtk::ContentFit::Cover);
        picture.set_can_shrink(true);
        picture.set_hexpand(true);
        picture.set_vexpand(true);
        if let Some(photo) = self.0.photos.borrow().get(index).cloned() {
            let source = photo.path();
            let mtime = (photo.mtime() > 0).then_some(photo.mtime());
            let size = (photo.size_bytes() > 0).then_some(photo.size_bytes());
            if let Some(path) = photo.cached_thumbnail_path().map(std::path::PathBuf::from)
                .or_else(|| crate::thumbnail::existing_cache_path(&source, mtime, size).ok().flatten()) {
                picture.set_filename(Some(path));
            } else if !source.is_empty() {
                // Request only the cover, through the same deduplicated worker
                // as gallery thumbnails. These bounded retries never decode an
                // original on the GTK thread and stop when the tile is gone.
                crate::thumbnail::request_priority(source.clone(), mtime, size, true);
                for delay in [2, 8] {
                    let picture = picture.downgrade();
                    let source = source.clone();
                    glib::timeout_add_local_once(std::time::Duration::from_secs(delay), move || {
                        if let Some(picture) = picture.upgrade() {
                            if let Ok(Some(path)) = crate::thumbnail::existing_cache_path(&source, mtime, size) {
                                picture.set_filename(Some(path));
                            }
                        }
                    });
                }
            }
        }
        cover.add_overlay(&picture);
        column.append(&cover);
        let title = gtk::Label::new(Some(label));
        title.set_xalign(0.0);
        title.add_css_class("heading");
        column.append(&title);
        let count = gtk::Label::new(Some(&format!("{count} {}", if count == 1 { "photo" } else { "photos" })));
        count.set_xalign(0.0);
        count.add_css_class("dim-label");
        column.append(&count);
        button.set_child(Some(&column));
        button.connect_clicked(move |_| activate());
        self.0.tiles.insert(&button, -1);
    }
}

fn month_name(month: u32) -> &'static str {
    match month {
        1 => "January", 2 => "February", 3 => "March", 4 => "April", 5 => "May", 6 => "June",
        7 => "July", 8 => "August", 9 => "September", 10 => "October", 11 => "November", 12 => "December", _ => "Unknown Date",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn groups_capture_dates_and_keeps_unknown_photos() {
        let buckets = buckets([Some("2026-03-01 12:00:00"), Some("2026-03-03 12-00-00"), Some("invalid"), None, Some("2025-07-01 12:00:00")]);
        assert_eq!(buckets[&Some((2026, 3))], [0, 1]);
        assert_eq!(buckets[&Some((2025, 7))], [4]);
        assert_eq!(buckets[&None], [2, 3]);
    }

    #[test]
    fn year_tiles_are_descending_and_include_unknown() {
        let groups = buckets([Some("2024-04-02 12:00:00"), None, Some("2026-01-01 12:00:00"), Some("2024-05-01 12:00:00")]);
        let years = year_groups(&groups);
        assert_eq!(years, vec![(Some(2026), 2, 1), (Some(2024), 0, 2), (None, 1, 1)]);
    }

    #[test]
    #[ignore = "requires a GTK display; run with --ignored --test-threads=1"]
    fn browser_navigates_year_month_and_unknown_without_losing_indices() {
        gtk::init().unwrap();
        let chosen = Rc::new(RefCell::new(Vec::new()));
        let output = chosen.clone();
        let exits = Rc::new(Cell::new(0));
        let exit_count = exits.clone();
        let browser = DateBrowser::new(Rc::new(move |bucket| output.borrow_mut().push(bucket)), Rc::new(|| {}), Rc::new(move || exit_count.set(exit_count.get() + 1)));
        let photos = [
            glib::Object::builder::<PhotoObject>().property("id", 1_i64).property("taken-at", "2026-03-01 12:00:00").build(),
            glib::Object::builder::<PhotoObject>().property("id", 2_i64).build(),
        ];
        browser.set_photos(&photos);
        browser.show_years();
        assert_eq!(browser.0.level.get(), BrowserLevel::Years);
        browser.show_months(2026);
        assert_eq!(browser.0.level.get(), BrowserLevel::Months(2026));
        assert_eq!(browser.indices(Some((2026, 3))), [0]);
        browser.open_month(Some((2026, 3)));
        assert_eq!(browser.0.level.get(), BrowserLevel::Photos(Some((2026, 3))));
        assert!(!browser.0.scroll.is_visible());
        assert!(!browser.0.root.vexpands());
        browser.back();
        assert_eq!(browser.0.level.get(), BrowserLevel::Months(2026));
        browser.back();
        browser.open_month(None);
        assert_eq!(browser.indices(None), [1]);
        assert_eq!(&*chosen.borrow(), &[Some((2026, 3)), None]);
        browser.back();
        assert_eq!(browser.0.level.get(), BrowserLevel::Years);
        browser.back();
        assert_eq!(exits.get(), 1);
    }
}
