{
    let refresh = gtk::Button::from_icon_name("view-refresh-symbolic");
    refresh.set_tooltip_text(Some("Refresh library"));
    right_header.pack_end(&refresh);


    let sort_button = gtk::MenuButton::new();
    sort_button.set_icon_name(match sort.get().direction {
        SortDirection::Ascending => "view-sort-ascending-symbolic",
        SortDirection::Descending => "view-sort-descending-symbolic",
    });
    sort_button.set_tooltip_text(Some("Sort photos"));

    let sort_popover = gtk::Popover::new();
    let sort_box = gtk::Box::new(gtk::Orientation::Vertical, 4);
    sort_box.set_margin_top(8);
    sort_box.set_margin_bottom(8);
    sort_box.set_margin_start(8);
    sort_box.set_margin_end(8);

    let sort_heading = gtk::Label::new(Some("Sort by"));
    sort_heading.set_xalign(0.0);
    sort_heading.add_css_class("heading");
    sort_box.append(&sort_heading);

    let date_taken_sort = gtk::CheckButton::with_label("Date taken");
    let date_added_sort = gtk::CheckButton::with_label("Date added");
    let rating_sort = gtk::CheckButton::with_label("Rating");
    let name_sort = gtk::CheckButton::with_label("Name");
    let file_size_sort = gtk::CheckButton::with_label("File size");
    let dimensions_sort = gtk::CheckButton::with_label("Dimensions");
    for button in [
        &date_added_sort,
        &rating_sort,
        &name_sort,
        &file_size_sort,
        &dimensions_sort,
    ] {
        button.set_group(Some(&date_taken_sort));
    }
    match sort.get().field {
        SortField::DateTaken => date_taken_sort.set_active(true),
        SortField::DateAdded => date_added_sort.set_active(true),
        SortField::Rating => rating_sort.set_active(true),
        SortField::Name => name_sort.set_active(true),
        SortField::FileSize => file_size_sort.set_active(true),
        SortField::Dimensions => dimensions_sort.set_active(true),
    }
    for button in [
        &date_taken_sort,
        &date_added_sort,
        &rating_sort,
        &name_sort,
        &file_size_sort,
        &dimensions_sort,
    ] {
        sort_box.append(button);
    }

    sort_box.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
    let order_heading = gtk::Label::new(Some("Order"));
    order_heading.set_xalign(0.0);
    order_heading.add_css_class("heading");
    sort_box.append(&order_heading);
    let ascending_sort = gtk::CheckButton::with_label("Ascending");
    let descending_sort = gtk::CheckButton::with_label("Descending");
    descending_sort.set_group(Some(&ascending_sort));
    match sort.get().direction {
        SortDirection::Ascending => ascending_sort.set_active(true),
        SortDirection::Descending => descending_sort.set_active(true),
    }
    sort_box.append(&ascending_sort);
    sort_box.append(&descending_sort);

    sort_box.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
    let group_heading = gtk::Label::new(Some("Group by"));
    group_heading.set_xalign(0.0);
    group_heading.add_css_class("heading");
    sort_box.append(&group_heading);

    let group_none = gtk::CheckButton::with_label("None");
    let group_day = gtk::CheckButton::with_label("Day");
    let group_month = gtk::CheckButton::with_label("Month");
    group_day.set_group(Some(&group_none));
    group_month.set_group(Some(&group_none));
    match group_mode.get() {
        grid::GroupMode::None | grid::GroupMode::Folder | grid::GroupMode::History => {
            group_none.set_active(true)
        }
        grid::GroupMode::Day => group_day.set_active(true),
        grid::GroupMode::Month => group_month.set_active(true),
    }
    // Grouping belongs to the Library section, not to a particular sort field.
    // Date grouping will use Date Taken (or Date Added when that sort is active).
    // Albums and Folders keep the controls disabled.
    let group_available = is_library_filter(filter.get());
    group_none.set_sensitive(group_available);
    group_day.set_sensitive(group_available);
    group_month.set_sensitive(group_available);
    sort_box.append(&group_none);
    sort_box.append(&group_day);
    sort_box.append(&group_month);

    let connect_sort_field = |button: &gtk::CheckButton, field: SortField| {
        let sort = sort.clone();
        let group_mode = group_mode.clone();
        let group_none = group_none.clone();
        let _group_day = group_day.clone();
        let _group_month = group_month.clone();
        let connection = connection.clone();
        let filter = filter.clone();
        let search = search_text.clone();
        let gallery = gallery.clone();
        button.connect_toggled(move |button| {
            if !button.is_active() {
                return;
            }
            let value = PhotoSort {
                field,
                ..sort.get()
            };
            sort.set(value);
            // Name/size/dimensions sorting cannot form a coherent chronological
            // group sequence. Keep Group by available in Library, but selecting a
            // non-date sort while grouped returns the grouping mode to None.
            if group_date_for_sort(value).is_none()
                && group_mode.get() != grid::GroupMode::None
            {
                group_none.set_active(true);
            }
            apply_gallery_grouping(
                &gallery,
                filter.get(),
                value,
                group_mode.get(),
                search.borrow().is_empty(),
            );
            if let Err(error) =
                db::set_setting(&connection.borrow(), SORT_FIELD_SETTING_KEY, field.key())
            {
                eprintln!("Could not save photo sort field: {error}");
            }
            refresh_grid(&connection, filter.get(), &search.borrow(), value, &gallery);
        });
    };
    connect_sort_field(&date_taken_sort, SortField::DateTaken);
    connect_sort_field(&date_added_sort, SortField::DateAdded);
    connect_sort_field(&rating_sort, SortField::Rating);
    connect_sort_field(&name_sort, SortField::Name);
    connect_sort_field(&file_size_sort, SortField::FileSize);
    connect_sort_field(&dimensions_sort, SortField::Dimensions);

    let connect_sort_direction = |button: &gtk::CheckButton, direction: SortDirection| {
        let sort = sort.clone();
        let sort_button = sort_button.clone();
        let connection = connection.clone();
        let filter = filter.clone();
        let search = search_text.clone();
        let gallery = gallery.clone();
        button.connect_toggled(move |button| {
            if !button.is_active() {
                return;
            }
            let value = PhotoSort {
                direction,
                ..sort.get()
            };
            sort.set(value);
            sort_button.set_icon_name(match direction {
                SortDirection::Ascending => "view-sort-ascending-symbolic",
                SortDirection::Descending => "view-sort-descending-symbolic",
            });
            if let Err(error) = db::set_setting(
                &connection.borrow(),
                SORT_DIRECTION_SETTING_KEY,
                direction.key(),
            ) {
                eprintln!("Could not save photo sort direction: {error}");
            }
            refresh_grid(&connection, filter.get(), &search.borrow(), value, &gallery);
        });
    };
    connect_sort_direction(&ascending_sort, SortDirection::Ascending);
    connect_sort_direction(&descending_sort, SortDirection::Descending);

    let connect_group_mode = |button: &gtk::CheckButton, mode: grid::GroupMode| {
        let group_mode = group_mode.clone();
        let sort = sort.clone();
        let date_taken_sort = date_taken_sort.clone();
        let connection = connection.clone();
        let filter = filter.clone();
        let search = search_text.clone();
        let gallery = gallery.clone();
        button.connect_toggled(move |button| {
            if !button.is_active() {
                return;
            }
            if !is_library_filter(filter.get()) {
                return;
            }
            // Day/Month are chronological groups. If the user was sorting by
            // Name, Size or Dimensions, switch to Date Taken automatically
            // instead of greying out Group by.
            if mode != grid::GroupMode::None && group_date_for_sort(sort.get()).is_none() {
                date_taken_sort.set_active(true);
            }
            group_mode.set(mode);
            if let Err(error) = db::set_setting(
                &connection.borrow(),
                GROUP_MODE_SETTING_KEY,
                group_mode_key(mode),
            ) {
                eprintln!("Could not save photo group mode: {error}");
            }
            apply_gallery_grouping(
                &gallery,
                filter.get(),
                sort.get(),
                mode,
                search.borrow().is_empty(),
            );
        });
    };
    connect_group_mode(&group_none, grid::GroupMode::None);
    connect_group_mode(&group_day, grid::GroupMode::Day);
    connect_group_mode(&group_month, grid::GroupMode::Month);

    sort_popover.set_child(Some(&sort_box));

    // The active sidebar destination can change after this popover is built.
    // Re-evaluate Group by every time it is opened so all Library destinations
    // stay enabled and Albums/Folders are visibly disabled.
    let filter_for_group_controls = filter.clone();
    let group_none_for_visibility = group_none.clone();
    let group_day_for_visibility = group_day.clone();
    let group_month_for_visibility = group_month.clone();
    sort_popover.connect_visible_notify(move |popover| {
        if !popover.is_visible() {
            return;
        }
        let available = is_library_filter(filter_for_group_controls.get());
        group_none_for_visibility.set_sensitive(available);
        group_day_for_visibility.set_sensitive(available);
        group_month_for_visibility.set_sensitive(available);
    });

    sort_button.set_popover(Some(&sort_popover));

    // Keep rating filtering independent from sorting. The filtered subset can
    // still be sorted by Date, Rating, Name, Size, or Dimensions.
    let rating_filter_button = gtk::MenuButton::new();
    rating_filter_button.set_icon_name(if rating_filter.get() == RatingFilter::All {
        "non-starred-symbolic"
    } else {
        "starred-symbolic"
    });
    rating_filter_button.set_tooltip_text(Some(rating_filter.get().tooltip()));

    let rating_filter_popover = gtk::Popover::new();
    let rating_filter_box = gtk::Box::new(gtk::Orientation::Vertical, 4);
    rating_filter_box.set_margin_top(8);
    rating_filter_box.set_margin_bottom(8);
    rating_filter_box.set_margin_start(8);
    rating_filter_box.set_margin_end(8);

    let rating_filter_heading = gtk::Label::new(Some("Filter by rating"));
    rating_filter_heading.set_xalign(0.0);
    rating_filter_heading.add_css_class("heading");
    rating_filter_box.append(&rating_filter_heading);

    let rating_clear = gtk::CheckButton::with_label("Clear");
    let rating_all_stars = gtk::CheckButton::with_label("All Stars");
    let rating_unrated = gtk::CheckButton::with_label("Unrated");
    let rating_one = gtk::CheckButton::with_label("★ 1");
    let rating_two = gtk::CheckButton::with_label("★★ 2");
    let rating_three = gtk::CheckButton::with_label("★★★ 3");
    let rating_four = gtk::CheckButton::with_label("★★★★ 4");
    let rating_five = gtk::CheckButton::with_label("★★★★★ 5");
    for button in [
        &rating_all_stars,
        &rating_unrated,
        &rating_one,
        &rating_two,
        &rating_three,
        &rating_four,
        &rating_five,
    ] {
        button.set_group(Some(&rating_clear));
    }
    match rating_filter.get() {
        RatingFilter::All => rating_clear.set_active(true),
        RatingFilter::AllStars => rating_all_stars.set_active(true),
        RatingFilter::Unrated => rating_unrated.set_active(true),
        RatingFilter::One => rating_one.set_active(true),
        RatingFilter::Two => rating_two.set_active(true),
        RatingFilter::Three => rating_three.set_active(true),
        RatingFilter::Four => rating_four.set_active(true),
        RatingFilter::Five => rating_five.set_active(true),
    }
    for button in [
        &rating_clear,
        &rating_all_stars,
        &rating_unrated,
        &rating_one,
        &rating_two,
        &rating_three,
        &rating_four,
        &rating_five,
    ] {
        rating_filter_box.append(button);
    }

    let connect_rating_filter = |button: &gtk::CheckButton, value: RatingFilter| {
        let rating_filter = rating_filter.clone();
        let rating_filter_button = rating_filter_button.clone();
        let connection = connection.clone();
        let filter = filter.clone();
        let search = search_text.clone();
        let sort = sort.clone();
        let gallery = gallery.clone();
        button.connect_toggled(move |button| {
            if !button.is_active() {
                return;
            }
            rating_filter.set(value);
            rating_filter_button.set_icon_name(if value == RatingFilter::All {
                "non-starred-symbolic"
            } else {
                "starred-symbolic"
            });
            rating_filter_button.set_tooltip_text(Some(value.tooltip()));
            if let Err(error) =
                db::set_setting(&connection.borrow(), RATING_FILTER_SETTING_KEY, value.key())
            {
                eprintln!("Could not save photo rating filter: {error}");
            }
            refresh_grid(
                &connection,
                filter.get(),
                &search.borrow(),
                sort.get(),
                &gallery,
            );
        });
    };
    connect_rating_filter(&rating_clear, RatingFilter::All);
    connect_rating_filter(&rating_all_stars, RatingFilter::AllStars);
    connect_rating_filter(&rating_unrated, RatingFilter::Unrated);
    connect_rating_filter(&rating_one, RatingFilter::One);
    connect_rating_filter(&rating_two, RatingFilter::Two);
    connect_rating_filter(&rating_three, RatingFilter::Three);
    connect_rating_filter(&rating_four, RatingFilter::Four);
    connect_rating_filter(&rating_five, RatingFilter::Five);

    rating_filter_popover.set_child(Some(&rating_filter_box));
    rating_filter_button.set_popover(Some(&rating_filter_popover));

    let header_tools = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    header_tools.append(&sort_button);
    header_tools.append(&rating_filter_button);
    header_tools.append(&settings);
    right_header.pack_end(&header_tools);

    refresh
}
