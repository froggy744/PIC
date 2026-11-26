//! Browse scope: one value for "what am I browsing". The destination, the
//! search, the toolbar rating and the metadata filters, loaded once and
//! passed explicitly into every browsing query, facet count and album count.
//!
//! The deep part is the SQL membership rule (`predicate`, `narrowing`): one
//! place decides how the destination, the search, the effective rating and
//! the metadata filters combine, so the grid, the facet counts, the photo
//! count and the album counts cannot drift apart. A scope is a plain value
//! (`Send`), so worker threads take a snapshot and callers compare snapshots
//! to drop stale results.
use super::*;
use rusqlite::types::Value;

/// The session rating setting the toolbar writes (cleared at startup and on a
/// library switch). [`BrowseScope::load`] is its only reader.
pub const RATING_SETTING: &str = "photo-rating-filter";

/// The toolbar rating choice. The Files choices (Corrupt, Offline, Online)
/// share the control but are not stored ratings: SQL cannot express them, so
/// they are checked on the loaded photos instead (see [`BrowseScope::files_check`]).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RatingFilter {
    #[default]
    All,
    AllStars,
    Unrated,
    One,
    Two,
    Three,
    Four,
    Five,
    Corrupt,
    Offline,
    Online,
}

impl RatingFilter {
    pub fn key(self) -> &'static str {
        match self {
            Self::All => "all",
            Self::AllStars => "stars",
            Self::Unrated => "unrated",
            Self::One => "1",
            Self::Two => "2",
            Self::Three => "3",
            Self::Four => "4",
            Self::Five => "5",
            Self::Corrupt => "corrupt",
            Self::Offline => "offline",
            Self::Online => "online",
        }
    }

    /// Reads the setting and Smart Album rule spellings ("0" is Unrated).
    /// Anything unknown, including "" and "all", is All.
    pub fn from_key(key: &str) -> Self {
        match key.trim() {
            "stars" => Self::AllStars,
            "unrated" | "0" => Self::Unrated,
            "1" => Self::One,
            "2" => Self::Two,
            "3" => Self::Three,
            "4" => Self::Four,
            "5" => Self::Five,
            "corrupt" => Self::Corrupt,
            "offline" => Self::Offline,
            "online" => Self::Online,
            _ => Self::All,
        }
    }

    /// The single star count this choice keeps, if it keeps exactly one.
    pub fn rating(self) -> Option<i32> {
        match self {
            Self::Unrated => Some(0),
            Self::One => Some(1),
            Self::Two => Some(2),
            Self::Three => Some(3),
            Self::Four => Some(4),
            Self::Five => Some(5),
            _ => None,
        }
    }

    /// Corrupt, Offline and Online: not stored ratings, checked on the photos.
    pub fn is_files_choice(self) -> bool {
        matches!(self, Self::Corrupt | Self::Offline | Self::Online)
    }

    /// The SQL condition on `{alias}.rating`; "1" for All and the Files choices.
    pub fn sql(self, alias: &str) -> String {
        match self {
            Self::AllStars => format!("{alias}.rating BETWEEN 1 AND 5"),
            _ => match self.rating() {
                Some(stars) => format!("{alias}.rating={stars}"),
                None => "1".into(),
            },
        }
    }

    pub fn tooltip(self) -> &'static str {
        match self {
            Self::All => "Filter by rating (clear)",
            Self::AllStars => "Rating filter: All Stars",
            Self::Unrated => "Rating filter: Unrated",
            Self::One => "Rating filter: 1 star",
            Self::Two => "Rating filter: 2 stars",
            Self::Three => "Rating filter: 3 stars",
            Self::Four => "Rating filter: 4 stars",
            Self::Five => "Rating filter: 5 stars",
            Self::Corrupt => "Filter: Corrupt photos",
            Self::Offline => "Filter: Offline photos",
            Self::Online => "Filter: Online photos",
        }
    }
}

/// Which photos a destination holds, before search, rating and filters.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Destination {
    /// Every photo (All Photos, Recently Added, the Folder stream, a search).
    #[default]
    AllPhotos,
    /// One folder and its subfolders.
    Folder(i64),
    Favorites,
    /// A manual or Smart Album (through [`album_membership`]).
    Album(i64),
    /// Photos in the edit history.
    History,
    /// Sidebar Ratings: every rated photo. Replaces the toolbar rating.
    Ratings,
    /// Sidebar Rating(n). Replaces the toolbar rating.
    Rating(u8),
    /// The Albums home: no photo grid; album counts use [`BrowseScope::narrowing`].
    Albums,
}

/// What the user is browsing, as one immutable value.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BrowseScope {
    pub destination: Destination,
    /// Free-text search ("" for none).
    pub search: String,
    /// The toolbar choice, even where the destination overrides it.
    pub rating: RatingFilter,
    /// The saved metadata filters.
    pub filters: filters::Filters,
}

impl BrowseScope {
    /// The scope for `destination` and `search` under the saved metadata
    /// filters and the session's toolbar rating: the one place either
    /// setting is read for browsing.
    pub fn load(connection: &Connection, destination: Destination, search: &str) -> Self {
        Self {
            destination,
            search: search.to_owned(),
            rating: RatingFilter::from_key(
                &setting(connection, RATING_SETTING)
                    .ok()
                    .flatten()
                    .unwrap_or_default(),
            ),
            filters: filters::Filters::load(connection),
        }
    }

    /// Every photo, narrowed only by `filters` and `rating` (Smart Album
    /// rules, the rules editor).
    pub fn library(filters: filters::Filters, rating: RatingFilter) -> Self {
        Self {
            rating,
            filters,
            ..Self::default()
        }
    }

    /// The same scope with other metadata filters (the panel's pending edits,
    /// a facet without its own field).
    pub fn with_filters(&self, filters: filters::Filters) -> Self {
        Self {
            filters,
            ..self.clone()
        }
    }

    /// Whether the toolbar rating narrows this destination. Ratings and
    /// Rating(n) choose their own ratings and ignore it.
    pub fn toolbar_rating_applies(&self) -> bool {
        !matches!(self.destination, Destination::Ratings | Destination::Rating(_))
    }

    /// The toolbar rating as it applies here: All where the destination
    /// ignores it.
    pub fn toolbar_rating(&self) -> RatingFilter {
        if self.toolbar_rating_applies() {
            self.rating
        } else {
            RatingFilter::All
        }
    }

    /// The one rating rule: the destination's for Ratings and Rating(n),
    /// otherwise the toolbar's.
    pub fn effective_rating(&self) -> RatingFilter {
        match self.destination {
            Destination::Ratings => RatingFilter::AllStars,
            Destination::Rating(stars) => {
                RatingFilter::from_key(&stars.min(5).to_string())
            }
            _ => self.rating,
        }
    }

    /// Whether the metadata filters or the toolbar rating narrow the
    /// destination, so a photo of the destination may not be shown.
    pub fn is_narrowed(&self) -> bool {
        !self.filters.is_empty() || self.toolbar_rating() != RatingFilter::All
    }

    /// The Files choice the loaded photos still need checking against, if any.
    pub fn files_check(&self) -> Option<RatingFilter> {
        Some(self.effective_rating()).filter(|rating| rating.is_files_choice())
    }

    /// The metadata filters and the effective rating, in terms of `p`, with
    /// parameters appended to `params` (numbered after those already there).
    /// No destination and no search: album counts use it.
    pub fn narrowing(&self, params: &mut Vec<Value>) -> String {
        let (metadata, mut args) = self.filters.predicate(params.len() + 1);
        params.append(&mut args);
        format!("({metadata}) AND {}", self.effective_rating().sql("p"))
    }

    /// Full membership in terms of `p` (destination, search, effective rating,
    /// metadata filters), appending parameters to `params`. Not `trashed`.
    pub fn predicate(&self, connection: &Connection, params: &mut Vec<Value>) -> Result<String> {
        let destination = match self.destination {
            Destination::AllPhotos
            | Destination::Albums
            | Destination::Ratings
            | Destination::Rating(_) => "1".to_owned(),
            Destination::Folder(folder_id) => {
                params.push(folder_id.into());
                format!(
                    "p.folder_id IN (WITH RECURSIVE descendants(id) AS (
                        SELECT id FROM folders WHERE id = ?{n}
                        UNION ALL
                        SELECT child.id FROM folders child JOIN descendants ON child.parent_id = descendants.id
                     ) SELECT id FROM descendants)",
                    n = params.len()
                )
            }
            Destination::Favorites => "p.favorite = 1".to_owned(),
            Destination::Album(album_id) => {
                let (sql, mut args) = album_membership(connection, album_id, params.len() + 1)?;
                params.append(&mut args);
                sql
            }
            Destination::History => "p.id IN (SELECT photo_id FROM recently_edited)".to_owned(),
        };
        let search = if self.search.is_empty() {
            "1".to_owned()
        } else {
            params.push(search_like_pattern(&self.search).into());
            params.push(search_date_pattern(&self.search).into());
            search_text_predicate(params.len() - 1, params.len())
        };
        let narrowing = self.narrowing(params);
        Ok(format!("({destination}) AND {search} AND {narrowing}"))
    }
}

/// What an asynchronous browse query was started for: the library and the
/// scope. Its result may be shown only while both are unchanged.
#[derive(Clone, Debug, PartialEq)]
pub struct BrowseSnapshot {
    library: Option<PathBuf>,
    pub scope: BrowseScope,
}

impl BrowseSnapshot {
    pub fn new(connection: &Connection, scope: BrowseScope) -> Self {
        Self {
            library: connection_path(connection).ok(),
            scope,
        }
    }

    /// Whether a result computed for this snapshot still describes what is
    /// browsed: same library behind `connection`, and `now` equals the scope.
    pub fn is_current(&self, connection: &Connection, now: &BrowseScope) -> bool {
        connection_path(connection).ok() == self.library && *now == self.scope
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use filters::{Field, Filters};

    fn scope(destination: Destination, rating: RatingFilter) -> BrowseScope {
        BrowseScope {
            destination,
            rating,
            ..BrowseScope::default()
        }
    }

    #[test]
    fn effective_rating_is_the_destinations_in_ratings_else_the_toolbars() {
        use RatingFilter::*;
        for (destination, toolbar, effective, toolbar_applied) in [
            (Destination::AllPhotos, Three, Three, Three),
            (Destination::Favorites, Unrated, Unrated, Unrated),
            (Destination::Album(4), Corrupt, Corrupt, Corrupt),
            (Destination::History, AllStars, AllStars, AllStars),
            (Destination::Albums, Five, Five, Five),
            (Destination::Ratings, Unrated, AllStars, All),
            (Destination::Ratings, All, AllStars, All),
            (Destination::Rating(3), Unrated, Three, All),
            (Destination::Rating(5), Offline, Five, All),
        ] {
            let scope = scope(destination, toolbar);
            assert_eq!(scope.effective_rating(), effective, "{destination:?} {toolbar:?}");
            assert_eq!(scope.toolbar_rating(), toolbar_applied, "{destination:?} {toolbar:?}");
        }
    }

    #[test]
    fn narrowed_means_metadata_filters_or_an_applied_toolbar_rating() {
        let mut camera = Filters::default();
        camera.toggle(Field::Camera, "Canon".into(), true);
        for (scope, narrowed) in [
            (scope(Destination::AllPhotos, RatingFilter::All), false),
            (scope(Destination::AllPhotos, RatingFilter::Three), true),
            (scope(Destination::AllPhotos, RatingFilter::Online), true),
            (scope(Destination::Favorites, RatingFilter::All), false),
            // The destination's own rating is not a narrowing of it.
            (scope(Destination::Ratings, RatingFilter::Unrated), false),
            (scope(Destination::Rating(2), RatingFilter::Five), false),
            (scope(Destination::Album(1), RatingFilter::All).with_filters(camera.clone()), true),
            (scope(Destination::Ratings, RatingFilter::All).with_filters(camera), true),
        ] {
            assert_eq!(scope.is_narrowed(), narrowed, "{scope:?}");
        }
    }

    #[test]
    fn files_choices_are_checked_on_photos_and_never_in_sql() {
        for rating in [RatingFilter::Corrupt, RatingFilter::Offline, RatingFilter::Online] {
            assert_eq!(rating.sql("p"), "1");
            assert_eq!(scope(Destination::AllPhotos, rating).files_check(), Some(rating));
            assert_eq!(scope(Destination::Ratings, rating).files_check(), None);
        }
        assert_eq!(scope(Destination::AllPhotos, RatingFilter::Three).files_check(), None);
    }

    #[test]
    fn rating_keys_round_trip_and_read_saved_rule_spellings() {
        use RatingFilter::*;
        for rating in [All, AllStars, Unrated, One, Two, Three, Four, Five, Corrupt, Offline, Online] {
            assert_eq!(RatingFilter::from_key(rating.key()), rating);
        }
        // Smart Album rules may hold "0"; the setting may hold "" or junk.
        assert_eq!(RatingFilter::from_key("0"), Unrated);
        assert_eq!(RatingFilter::from_key(""), All);
        assert_eq!(RatingFilter::from_key("7"), All);
        assert_eq!(Unrated.sql("p"), "p.rating=0");
        assert_eq!(AllStars.sql("mp"), "mp.rating BETWEEN 1 AND 5");
    }

    fn catalog() -> Connection {
        let c = Connection::open_in_memory().unwrap();
        c.execute_batch(SCHEMA).unwrap();
        c.execute_batch(
            "INSERT INTO folders(id,path,parent_id) VALUES (1,'/a',NULL),(2,'/a/b',1),(3,'/c',NULL);
             INSERT INTO photos(id,path,folder_id,rating,favorite,added_at) VALUES
               (1,'/a/one.jpg',1,0,0,1),(2,'/a/b/two.jpg',2,3,1,1),
               (3,'/c/three.jpg',3,5,0,1),(4,'/c/four.jpg',3,3,0,1);
             INSERT INTO albums(id,name) VALUES (1,'Trip');
             INSERT INTO album_photos(album_id,photo_id) VALUES (1,1),(1,2);
             INSERT INTO editing_events(id,action,edited_at) VALUES (1,'Edit',10);
             INSERT INTO recently_edited(photo_id,edited_at,event_id) VALUES (4,10,1);",
        )
        .unwrap();
        c
    }

    fn members(c: &Connection, scope: &BrowseScope) -> Vec<i64> {
        let mut params = Vec::new();
        let predicate = scope.predicate(c, &mut params).unwrap();
        let mut statement = c
            .prepare(&format!(
                "SELECT p.id FROM photos p WHERE p.trashed=0 AND {predicate} ORDER BY p.id"
            ))
            .unwrap();
        statement
            .query_map(rusqlite::params_from_iter(params), |row| row.get(0))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap()
    }

    #[test]
    fn sql_membership_per_destination_search_and_rating() {
        let c = catalog();
        use Destination::*;
        use RatingFilter::*;
        let search = |destination, text: &str, rating| BrowseScope {
            search: text.into(),
            ..scope(destination, rating)
        };
        for (scope, expected) in [
            (scope(AllPhotos, All), vec![1, 2, 3, 4]),
            (scope(AllPhotos, Three), vec![2, 4]),
            (scope(AllPhotos, Unrated), vec![1]),
            (scope(AllPhotos, AllStars), vec![2, 3, 4]),
            // Files choices are not SQL: every photo, checked afterwards.
            (scope(AllPhotos, Corrupt), vec![1, 2, 3, 4]),
            (scope(Folder(1), All), vec![1, 2]),
            (scope(Folder(2), All), vec![2]),
            (scope(Favorites, All), vec![2]),
            (scope(Album(1), All), vec![1, 2]),
            (scope(Album(1), Unrated), vec![1]),
            (scope(History, All), vec![4]),
            // Round-three bug 1: Ratings with the toolbar on Unrated is not empty.
            (scope(Ratings, Unrated), vec![2, 3, 4]),
            (scope(Rating(3), Five), vec![2, 4]),
            (scope(Albums, Five), vec![3]),
            (search(AllPhotos, "three", All), vec![3]),
            (search(AllPhotos, "/c/", Three), vec![4]),
            // A search matches album names too, as the grid always did.
            (search(AllPhotos, "trip", All), vec![1, 2]),
            (search(Album(1), "two", All), vec![2]),
        ] {
            assert_eq!(members(&c, &scope), expected, "{scope:?}");
        }
    }

    #[test]
    fn sql_membership_combines_metadata_filters_and_parameters_in_order() {
        let c = catalog();
        let mut filters = Filters::default();
        filters.toggle(Field::SourceFolder, "/c".into(), true);
        let scope = BrowseScope {
            destination: Destination::Album(1),
            search: "o".into(),
            rating: RatingFilter::All,
            filters: filters.clone(),
        };
        // Album 1 holds photos 1 and 2, neither in /c.
        assert!(members(&c, &scope).is_empty());
        let scope = BrowseScope {
            destination: Destination::Folder(3),
            search: "f".into(),
            rating: RatingFilter::Three,
            filters,
        };
        assert_eq!(members(&c, &scope), vec![4]);
    }

    #[test]
    fn stale_worker_results_are_rejected_after_a_scope_or_library_change() {
        let dir = std::env::temp_dir().join(format!("pic-browse-snapshot-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let first = Connection::open(dir.join("first.db")).unwrap();
        let second = Connection::open(dir.join("second.db")).unwrap();
        let scope = scope(Destination::Albums, RatingFilter::Three);
        let snapshot = BrowseSnapshot::new(&first, scope.clone());
        assert!(snapshot.is_current(&first, &scope));
        // The toolbar rating, the metadata filters or the destination moved on.
        assert!(!snapshot.is_current(&first, &BrowseScope { rating: RatingFilter::Four, ..scope.clone() }));
        let mut camera = filters::Filters::default();
        camera.toggle(filters::Field::Camera, "Canon".into(), true);
        assert!(!snapshot.is_current(&first, &scope.with_filters(camera)));
        assert!(!snapshot.is_current(&first, &BrowseScope { destination: Destination::AllPhotos, ..scope.clone() }));
        // Another library is open, even under an equal scope.
        assert!(!snapshot.is_current(&second, &scope));
        drop((first, second));
        let _ = std::fs::remove_dir_all(dir);
    }
}
