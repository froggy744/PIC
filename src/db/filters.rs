//! One SQL predicate for browsing and metadata facets. Never reads original files.
use super::*;
use rusqlite::types::Value;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const SETTING: &str = "photo-metadata-filters";
pub const MISSING: &str = "\u{2400}";

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Field {
    Camera,
    CameraMake,
    Lens,
    LensMake,
    Iso,
    Aperture,
    Shutter,
    FocalLength,
    CaptureDate,
    FileType,
    Rating,
    Files,
    Flash,
    Exposure,
    WhiteBalance,
    Width,
    Height,
    Orientation,
    Gps,
    Favorite,
    Edited,
    Metadata,
    /// The imported folder a photo belongs to, derived from `photos.folder_id`.
    SourceFolder,
    /// User-authored tags are stored in scrapbook_details for every photo.
    Tags,
}
impl Field {
    pub const ALL: [Self; 24] = [
        Self::Camera,
        Self::CameraMake,
        Self::Lens,
        Self::LensMake,
        Self::Iso,
        Self::Aperture,
        Self::Shutter,
        Self::FocalLength,
        Self::CaptureDate,
        Self::FileType,
        Self::Rating,
        Self::Files,
        Self::Flash,
        Self::Exposure,
        Self::WhiteBalance,
        Self::Width,
        Self::Height,
        Self::Orientation,
        Self::Gps,
        Self::Favorite,
        Self::Edited,
        Self::Metadata,
        Self::SourceFolder,
        Self::Tags,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::Camera => "Camera",
            Self::CameraMake => "Camera manufacturer",
            Self::Lens => "Lens",
            Self::LensMake => "Lens manufacturer",
            Self::Iso => "ISO",
            Self::Aperture => "Aperture",
            Self::Shutter => "Shutter speed",
            Self::FocalLength => "Focal length",
            Self::CaptureDate => "Capture date",
            Self::FileType => "File type",
            Self::Rating => "Rating",
            Self::Files => "Files",
            Self::Flash => "Flash used",
            Self::Exposure => "Exposure compensation",
            Self::WhiteBalance => "White balance",
            Self::Width => "Width (pixels)",
            Self::Height => "Height (pixels)",
            Self::Orientation => "Orientation",
            Self::Gps => "GPS",
            Self::Favorite => "Favourite",
            Self::Edited => "Edited",
            Self::Metadata => "EXIF metadata",
            Self::SourceFolder => "Source folder",
            Self::Tags => "Tags",
        }
    }
    pub fn numeric(self) -> bool {
        matches!(
            self,
            Self::Iso
                | Self::Aperture
                | Self::Shutter
                | Self::FocalLength
                | Self::Exposure
                | Self::Width
                | Self::Height
        )
    }
    pub fn expression(self) -> &'static str {
        match self {
            Self::Camera => "COALESCE(m.camera_key,NULLIF(lower(trim(mp.camera)),''))",
            Self::CameraMake => "m.camera_make_key",
            Self::Lens => "COALESCE(m.lens_key,NULLIF(lower(trim(mp.lens)),''))",
            Self::LensMake => "m.lens_make_key",
            Self::Iso => "NULLIF(mp.iso,0)",
            Self::Aperture => "NULLIF(mp.aperture,0)",
            Self::Shutter => "NULLIF(mp.shutter_speed,0)",
            Self::FocalLength => "NULLIF(mp.focal_length,0)",
            Self::CaptureDate => "substr(m.captured_at,1,10)",
            Self::FileType => "m.file_type",
            Self::Rating => "mp.rating",
            Self::Files => "NULL",
            Self::Flash => "m.flash",
            Self::Exposure => "mp.exposure_bias",
            Self::WhiteBalance => "m.white_balance",
            Self::Width => "NULLIF(mp.width,0)",
            Self::Height => "NULLIF(mp.height,0)",
            Self::Orientation => "CASE WHEN mp.width IS NULL OR mp.height IS NULL OR mp.width<=0 OR mp.height<=0 THEN 'Unknown' WHEN mp.width<mp.height THEN 'Portrait' WHEN mp.width>mp.height THEN 'Landscape' ELSE 'Square' END",
            Self::Gps => "m.gps",
            Self::Favorite => "mp.favorite",
            Self::Edited => "(mp.edit_recipe != '' OR mp.rotation != 0)",
            Self::Metadata => "m.has_exif",
            // Both predicate and facet builders provide this join.
            Self::SourceFolder => "sf.path",
            // Tags are a set, so clauses()/facets() must handle them using
            // json_each rather than treating the whole array as one value.
            Self::Tags => "NULL",
        }
    }
    pub fn display(self, value: &str) -> String {
        if value == MISSING {
            return if matches!(self, Self::Metadata | Self::Gps) {
                "Not indexed / unknown"
            } else {
                "Missing / unknown"
            }
            .into();
        }
        if self == Self::Rating {
            return match value {
                "0" => "Unrated".into(),
                "1" => "1 star".into(),
                "2" => "2 stars".into(),
                "3" => "3 stars".into(),
                "4" => "4 stars".into(),
                "5" => "5 stars".into(),
                _ => value.into(),
            };
        }
        if matches!(
            self,
            Self::Flash | Self::Gps | Self::Favorite | Self::Edited | Self::Metadata
        ) {
            return match (self, value) {
                (Self::Gps | Self::Metadata, "1") => "Present",
                (Self::Gps | Self::Metadata, "0") => "Missing",
                (_, "1") => "Yes",
                (_, "0") => "No",
                _ => value,
            }
            .into();
        }
        if let Ok(number) = value.parse::<f64>() {
            return match self {
                Self::Aperture => format!("f/{number}"),
                Self::Shutter if number > 0.0 && number < 1.0 => format!("1/{:.0} s", 1.0 / number),
                Self::Shutter => format!("{number} s"),
                Self::FocalLength => format!("{number} mm"),
                Self::Exposure => format!("{number:+} EV"),
                _ => value.into(),
            };
        }
        value.into()
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Selection {
    pub values: Vec<String>,
    pub min: Option<String>,
    pub max: Option<String>,
}
impl Selection {
    pub fn is_empty(&self) -> bool {
        self.values.is_empty() && self.min.is_none() && self.max.is_none()
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Filters {
    pub categories: BTreeMap<Field, Selection>,
}
impl Filters {
    pub fn is_empty(&self) -> bool {
        self.categories.values().all(Selection::is_empty)
    }
    pub fn load(c: &Connection) -> Self {
        setting(c, SETTING)
            .ok()
            .flatten()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }
    pub fn save(&self, c: &Connection) -> Result<()> {
        set_setting(c, SETTING, &serde_json::to_string(self)?)
    }
    pub fn toggle(&mut self, field: Field, value: String, active: bool) {
        let selection = self.categories.entry(field).or_default();
        selection.values.retain(|v| v != &value);
        if active {
            selection.values.push(value);
        }
        if selection.is_empty() {
            self.categories.remove(&field);
        }
    }
    pub fn range(&mut self, field: Field, min: &str, max: &str) -> Result<()> {
        anyhow::ensure!(
            field.numeric() || field == Field::CaptureDate,
            "This field has no range"
        );
        let mut values = Vec::new();
        for input in [min, max] {
            let value = input.trim();
            if !value.is_empty() {
                if field == Field::CaptureDate {
                    chrono::NaiveDate::parse_from_str(value, "%Y-%m-%d")?;
                } else {
                    anyhow::ensure!(value.parse::<f64>()?.is_finite(), "Enter a finite number");
                }
            }
            values.push((!value.is_empty()).then(|| value.to_owned()));
        }
        if let (Some(a), Some(b)) = (&values[0], &values[1]) {
            anyhow::ensure!(
                if field.numeric() {
                    a.parse::<f64>()? <= b.parse::<f64>()?
                } else {
                    a <= b
                },
                "Minimum must not exceed maximum"
            );
        }
        let selection = self.categories.entry(field).or_default();
        selection.min = values[0].clone();
        selection.max = values[1].clone();
        if selection.is_empty() {
            self.categories.remove(&field);
        }
        Ok(())
    }
    fn clauses(&self, first: usize) -> (String, Vec<Value>) {
        let mut args = Vec::new();
        let mut and = Vec::new();
        for (field, selection) in &self.categories {
            let expr = field.expression();
            let mut or = Vec::new();
            for value in &selection.values {
                if *field == Field::Tags {
                    if value == MISSING {
                        or.push("NOT EXISTS (SELECT 1 FROM scrapbook_details sd
                            JOIN json_each(sd.tags) jt ON 1=1
                            WHERE sd.photo_id=mp.id)".to_string());
                    } else {
                        let n = first + args.len();
                        args.push(Value::Text(value.clone()));
                        or.push(format!("EXISTS (SELECT 1 FROM scrapbook_details sd
                            JOIN json_each(sd.tags) jt ON 1=1
                            WHERE sd.photo_id=mp.id AND jt.value=?{n})"));
                    }
                } else if value == MISSING {
                    or.push(format!("{expr} IS NULL"));
                } else if *field == Field::SourceFolder {
                    let n = first + args.len();
                    args.push(Value::Text(value.clone()));
                    or.push(format!(
                        "mp.folder_id IN (WITH RECURSIVE descendants(id) AS (
                            SELECT id FROM folders WHERE path = ?{n}
                            UNION SELECT child.id FROM folders child JOIN descendants d ON child.parent_id = d.id
                        ) SELECT id FROM descendants)"
                    ));
                } else {
                    let n = first + args.len();
                    args.push(Value::Text(value.clone()));
                    or.push(format!("CAST({expr} AS TEXT)=?{n}"));
                }
            }
            if !or.is_empty() {
                and.push(format!("({})", or.join(" OR ")));
            }
            for (value, op) in [(&selection.min, ">="), (&selection.max, "<=")] {
                if let Some(value) = value {
                    let n = first + args.len();
                    args.push(Value::Text(value.clone()));
                    let bound = if field.numeric() {
                        format!("CAST(?{n} AS REAL)")
                    } else {
                        format!("?{n}")
                    };
                    and.push(format!("{expr} {op} {bound}"));
                }
            }
        }
        (
            if and.is_empty() {
                "1".into()
            } else {
                and.join(" AND ")
            },
            args,
        )
    }
    /// The metadata filters in terms of `p`, parameters numbered from
    /// `first`. No rating: that is the browse scope's (`BrowseScope::narrowing`)
    /// or a Smart Album's (`AlbumRules::predicate`).
    pub fn predicate(&self, first: usize) -> (String, Vec<Value>) {
        if self.is_empty() {
            return ("1".into(), vec![]);
        }
        let (sql, args) = self.clauses(first);
        (format!("p.id IN (SELECT mp.id FROM photos mp LEFT JOIN photo_metadata m ON m.photo_id=mp.id LEFT JOIN folders sf ON sf.id=mp.folder_id WHERE {sql})"),args)
    }
}

/// Rating choices a Smart Album can save. The Files choices (online, offline,
/// corrupt) share the rating setting but are not SQL, so they never are.
pub const RULE_RATINGS: [&str; 8] = ["unrated", "stars", "0", "1", "2", "3", "4", "5"];

/// The saved membership rules of a Smart Album: an independent copy of
/// supported filter state. Unlike [`Filters`], the rating choice is
/// serialized, so a saved album keeps it.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct AlbumRules {
    #[serde(default)]
    pub categories: BTreeMap<Field, Selection>,
    #[serde(default)]
    pub rating: String,
}

/// What [`AlbumRules::capture`] could not keep from the browsing filters.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Unsaved {
    /// The Files choice (Online, Offline or Corrupt photos), when one was active.
    pub files: Option<&'static str>,
}

impl AlbumRules {
    /// Copy the supported part of what is being browsed: the metadata
    /// filters and the rating the view uses (`BrowseScope::effective_rating`).
    /// The copy is independent: later browsing changes leave it alone.
    pub fn capture(scope: &super::BrowseScope) -> (Self, Unsaved) {
        let mut categories = scope.filters.categories.clone();
        categories.remove(&Field::Files);
        categories.retain(|_, selection| !selection.is_empty());
        let rating = scope.effective_rating();
        let files = match rating {
            super::RatingFilter::Online => Some("Online"),
            super::RatingFilter::Offline => Some("Offline"),
            super::RatingFilter::Corrupt => Some("Corrupt photos"),
            _ => None,
        };
        let rating = if RULE_RATINGS.contains(&rating.key()) {
            rating.key().to_owned()
        } else {
            String::new()
        };
        (Self { categories, rating }, Unsaved { files })
    }
    pub fn is_empty(&self) -> bool {
        self.rating.is_empty() && self.categories.values().all(Selection::is_empty)
    }
    /// A saved album needs at least one rule, and every rule must be one the
    /// filter predicate supports. Anything else is rejected rather than
    /// matched loosely, so a damaged album never shows the whole library.
    pub fn validate(&self) -> Result<()> {
        anyhow::ensure!(!self.is_empty(), "A Smart Album needs at least one rule");
        anyhow::ensure!(
            self.rating.is_empty() || RULE_RATINGS.contains(&self.rating.as_str()),
            "Unsupported rating rule"
        );
        for (field, selection) in &self.categories {
            anyhow::ensure!(*field != Field::Files, "Online, offline and corrupt photos cannot be saved in a Smart Album");
            anyhow::ensure!(!selection.is_empty(), "The {} rule has no value", field.label());
            anyhow::ensure!(
                selection.values.iter().all(|value| !value.trim().is_empty()),
                "The {} rule has an empty value",
                field.label()
            );
            if selection.min.is_some() || selection.max.is_some() {
                let mut check = Filters::default();
                check.range(
                    *field,
                    selection.min.as_deref().unwrap_or(""),
                    selection.max.as_deref().unwrap_or(""),
                )?;
            }
        }
        Ok(())
    }
    /// Read saved rules; invalid or unsupported rules are an error.
    pub fn parse(json: &str) -> Result<Self> {
        let rules: Self = serde_json::from_str(json)?;
        rules.validate()?;
        Ok(rules)
    }
    pub fn to_json(&self) -> Result<String> {
        Ok(serde_json::to_string(self)?)
    }
    /// The metadata part of the rules.
    pub fn to_filters(&self) -> Filters {
        Filters {
            categories: self.categories.clone(),
        }
    }
    /// The saved rating rule, typed. Saved spellings ("unrated", "stars",
    /// "0".."5") all read; "" is no rule.
    pub fn rating_filter(&self) -> super::RatingFilter {
        super::RatingFilter::from_key(&self.rating)
    }
    /// The whole library narrowed by these rules, for the rules editor.
    pub fn to_scope(&self) -> super::BrowseScope {
        super::BrowseScope::library(self.to_filters(), self.rating_filter())
    }
    /// Membership in terms of `p`, parameters numbered from `first`.
    pub fn predicate(&self, first: usize) -> (String, Vec<Value>) {
        let (metadata, args) = self.to_filters().predicate(first);
        (format!("({metadata}) AND {}", self.rating_filter().sql("p")), args)
    }
}

#[derive(Debug, Clone)]
pub struct Facet {
    pub value: String,
    pub count: i64,
}

/// Counts per value of `field` over what `scope` browses, ignoring the
/// scope's own selection for that field (and, for Rating, the toolbar
/// rating; a Ratings destination keeps its own).
pub fn facets(c: &Connection, scope: &super::BrowseScope, field: Field) -> Result<Vec<Facet>> {
    let mut other = scope.filters.clone();
    other.categories.remove(&field);
    let mut scope = scope.with_filters(other);
    if field == Field::Rating {
        scope.rating = super::RatingFilter::All;
    }
    let mut params = Vec::new();
    let membership = scope.predicate(c, &mut params)?;
    let expr = if field == Field::Tags { "NULLIF(TRIM(jt.value),'')" } else { field.expression() };
    let tags_join = if field == Field::Tags {
        "LEFT JOIN scrapbook_details sd ON sd.photo_id=mp.id
         LEFT JOIN json_each(CASE WHEN json_valid(sd.tags) THEN sd.tags ELSE '[]' END) jt ON 1=1"
    } else {
        ""
    };
    let sql=format!("SELECT CAST({expr} AS TEXT),COUNT(DISTINCT mp.id) FROM photos mp LEFT JOIN photo_metadata m ON m.photo_id=mp.id LEFT JOIN folders sf ON sf.id=mp.folder_id {tags_join}
        WHERE mp.trashed=0 AND mp.id IN (SELECT p.id FROM photos p WHERE {membership})
        GROUP BY {expr} ORDER BY {expr}");
    let mut stmt = c.prepare(&sql)?;
    let rows = stmt.query_map(rusqlite::params_from_iter(params), |r| {
        Ok(Facet {
            value: r
                .get::<_, Option<String>>(0)?
                .unwrap_or_else(|| MISSING.into()),
            count: r.get(1)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

/// Count the photos `scope` browses, with the same predicate as the grid
/// query. Files choices (Online, Offline, Corrupt) are not SQL and do not
/// narrow the count.
pub fn count_matching(c: &Connection, scope: &super::BrowseScope) -> Result<i64> {
    let mut params = Vec::new();
    let membership = scope.predicate(c, &mut params)?;
    Ok(c.query_row(
        &format!("SELECT COUNT(*) FROM photos p WHERE p.trashed=0 AND {membership}"),
        rusqlite::params_from_iter(params),
        |row| row.get(0),
    )?)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn catalog() -> Connection {
        let c = Connection::open_in_memory().unwrap();
        c.execute_batch(SCHEMA).unwrap();
        c.execute_batch(
            "INSERT INTO folders(id,path) VALUES (1,'/a'),(2,'/b');
          INSERT INTO albums(id,name) VALUES (1,'Test');",
        )
        .unwrap();
        for (id, ext, camera, lens, iso) in [
            (1, "NEF", "NIKON Z 5", "NIKKOR Z 24-70mm f/4 S", 100),
            (2, "jpg", "nikon   z 5", "NIKKOR Z 24-70mm f/4 S", 800),
            (3, "png", "Canon R5", "RF 50mm", 100),
        ] {
            let m = PhotoMetadata {
                camera: Some(camera.into()),
                lens: Some(lens.into()),
                iso: Some(iso),
                searchable: metadata::SearchMetadata {
                    camera_make: Some(if id < 3 { "NIKON CORPORATION" } else { "Canon" }.into()),
                    camera_model: Some(camera.into()),
                    lens_model: Some(lens.into()),
                    has_exif: Some(true),
                    gps: Some(false),
                    ..Default::default()
                },
                ..Default::default()
            };
            upsert_photo(
                &c,
                Path::new(&format!("/a/keep{id}.{ext}")),
                Some(if id < 3 { 1 } else { 2 }),
                &m,
            )
            .unwrap();
        }
        upsert_photo(
            &c,
            Path::new("/a/no-exif.jpg"),
            Some(1),
            &PhotoMetadata {
                searchable: metadata::SearchMetadata {
                    has_exif: Some(false),
                    gps: Some(false),
                    ..Default::default()
                },
                ..Default::default()
            },
        )
        .unwrap();
        c.execute_batch("INSERT INTO album_photos VALUES (1,1),(1,3); UPDATE photos SET rating=3 WHERE id=1; UPDATE photos SET favorite=1 WHERE id=2;").unwrap();
        c
    }

    #[test]
    fn tags_are_globally_filterable_and_faceted_per_photo_not_per_json_array() {
        let c = catalog();
        scrapbook_add_tag(&c, 1, "Family").unwrap();
        scrapbook_add_tag(&c, 1, "Holiday").unwrap();
        scrapbook_add_tag(&c, 2, "Family").unwrap();
        let rows = facets(&c, &BrowseScope::default(), Field::Tags).unwrap();
        assert!(rows.iter().any(|row| row.value == "Family" && row.count == 2));
        assert!(rows.iter().any(|row| row.value == "Holiday" && row.count == 1));
        let mut f = Filters::default();
        f.toggle(Field::Tags, "Family".into(), true);
        assert_eq!(ids(&c, &f), vec![1, 2]);
        f.toggle(Field::Tags, "Holiday".into(), true);
        assert_eq!(ids(&c, &f), vec![1, 2]);
        f.toggle(Field::Tags, "Family".into(), false);
        assert_eq!(ids(&c, &f), vec![1]);
        assert_eq!(count_matching(&c, &BrowseScope::default().with_filters(f.clone())).unwrap(), 1);
    }

    #[test]
    fn rating_column_lists_all_ratings_even_when_toolbar_rating_is_active() {
        let connection = Connection::open_in_memory().unwrap();
        connection.execute_batch(SCHEMA).unwrap();
        connection
            .execute_batch(
                "INSERT INTO folders(id,path) VALUES (1,'/a');
             INSERT INTO photos(id,path,folder_id,rating) VALUES
                (101,'/a/unrated.jpg',1,0),
                (102,'/a/three.jpg',1,3),
                (103,'/a/five.jpg',1,5);",
            )
            .unwrap();
        let scope = BrowseScope {
            rating: RatingFilter::Three,
            ..BrowseScope::default()
        };
        let rows = facets(&connection, &scope, Field::Rating).unwrap();
        assert_eq!(
            rows.iter()
                .map(|row| (row.value.as_str(), row.count))
                .collect::<Vec<_>>(),
            [("0", 1), ("3", 1), ("5", 1)]
        );
    }
    fn ids(c: &Connection, f: &Filters) -> Vec<i64> {
        let mut ids = browse_photos(c, &BrowseScope::default().with_filters(f.clone()))
            .unwrap()
            .into_iter()
            .map(|p| p.id)
            .collect::<Vec<_>>();
        ids.sort();
        ids
    }
    #[test]
    fn count_matches_grid_filters_and_scope() {
        let c = catalog();
        let mut filters = Filters::default();
        filters.toggle(Field::Iso, "100".into(), true);
        let scope = BrowseScope::default().with_filters(filters.clone());
        assert_eq!(count_matching(&c, &scope).unwrap(), 2);

        let folder_scope = BrowseScope {
            destination: Destination::Folder(1),
            ..scope.clone()
        };
        assert_eq!(count_matching(&c, &folder_scope).unwrap(), 1);

        let mut album_scope = BrowseScope {
            destination: Destination::Album(1),
            ..scope.clone()
        };
        assert_eq!(count_matching(&c, &album_scope).unwrap(), 2);

        album_scope.rating = RatingFilter::Three;
        assert_eq!(count_matching(&c, &album_scope).unwrap(), 1);
    }
    #[test]
    fn metadata_single_combined_or_clear_and_empty() {
        let c = catalog();
        let mut f = Filters::default();
        f.toggle(Field::Camera, "nikon z 5".into(), true);
        assert_eq!(ids(&c, &f), vec![1, 2]);
        f.toggle(Field::Lens, "nikkor z 24-70mm f/4 s".into(), true);
        f.range(Field::Iso, "50", "200").unwrap();
        assert_eq!(ids(&c, &f), vec![1]);
        f.categories.remove(&Field::Iso);
        f.toggle(Field::FileType, "RAW".into(), true);
        assert_eq!(ids(&c, &f), vec![1]);
        f.toggle(Field::FileType, "JPEG".into(), true);
        assert_eq!(ids(&c, &f), vec![1, 2]);
        f.range(Field::Iso, "900", "1000").unwrap();
        assert!(ids(&c, &f).is_empty());
        f.categories.clear();
        assert_eq!(ids(&c, &f), vec![1, 2, 3, 4]);
    }
    #[test]
    fn metadata_missing_is_distinct_from_pending() {
        let c = catalog();
        c.execute("INSERT INTO photos(path) VALUES ('/pending.jpg')", [])
            .unwrap();
        let mut f = Filters::default();
        f.toggle(Field::Metadata, "0".into(), true);
        assert_eq!(ids(&c, &f), vec![4]);
        f.categories.clear();
        f.toggle(Field::Metadata, MISSING.into(), true);
        assert_eq!(ids(&c, &f), vec![5]);
        f.categories.clear();
        f.toggle(Field::Iso, MISSING.into(), true);
        assert_eq!(ids(&c, &f), vec![4, 5]);
    }
    #[test]
    fn source_folder_facets_and_selection_include_only_descendants() {
        let c = catalog();
        c.execute_batch(
            "INSERT INTO folders(id,path,name,parent_id) VALUES (3,'/a/child','child',1);
             INSERT INTO photos(id,path,folder_id) VALUES
                 (6,'/a/child/photo.jpg',3), (7,'/no-folder.jpg',NULL),
                 (8,'/unknown-folder.jpg',999);",
        )
        .unwrap();

        let facets = facets(&c, &BrowseScope::default(), Field::SourceFolder)
        .unwrap()
        .into_iter()
        .map(|facet| (facet.value, facet.count))
        .collect::<Vec<_>>();
        assert_eq!(
            facets,
            vec![
                (MISSING.into(), 2),
                ("/a".into(), 3),
                ("/a/child".into(), 1),
                ("/b".into(), 1),
            ]
        );

        let mut filters = Filters::default();
        filters.toggle(Field::SourceFolder, "/a".into(), true);
        assert_eq!(ids(&c, &filters), vec![1, 2, 4, 6]);

        filters.categories.clear();
        filters.toggle(Field::SourceFolder, "/a/child".into(), true);
        assert_eq!(ids(&c, &filters), vec![6]);

        filters.toggle(Field::SourceFolder, "/b".into(), true);
        assert_eq!(ids(&c, &filters), vec![3, 6]);

        filters.categories.clear();
        filters.toggle(Field::SourceFolder, MISSING.into(), true);
        assert_eq!(ids(&c, &filters), vec![7, 8]);

        filters.categories.clear();
        filters.toggle(Field::SourceFolder, "/removed".into(), true);
        assert!(ids(&c, &filters).is_empty());
    }
    #[test]
    fn metadata_folder_album_search_and_existing_rating() {
        let c = catalog();
        let mut f = Filters::default();
        f.toggle(Field::Camera, "nikon z 5".into(), true);
        let scope = |destination, search: &str| BrowseScope {
            destination,
            search: search.into(),
            filters: f.clone(),
            ..BrowseScope::default()
        };
        assert_eq!(browse_photos(&c, &scope(Destination::Folder(2), "")).unwrap().len(), 0);
        assert_eq!(
            browse_photos(&c, &scope(Destination::Album(1), ""))
                .unwrap()
                .iter()
                .map(|p| p.id)
                .collect::<Vec<_>>(),
            vec![1]
        );
        assert_eq!(
            browse_photos(&c, &scope(Destination::AllPhotos, "keep2")).unwrap()[0].id,
            2
        );
        f.save(&c).unwrap();
        set_setting(&c, "photo-rating-filter", "3").unwrap();
        assert_eq!(
            browse_photos(&c, &BrowseScope::load(&c, Destination::AllPhotos, ""))
                .unwrap()
                .iter()
                .map(|p| p.id)
                .collect::<Vec<_>>(),
            vec![1]
        );
        // Bulk operations still see the full unfiltered set.
        assert_eq!(photos(&c, None, false, None).unwrap().len(), 4);
    }
    #[test]
    fn metadata_facets_scope_and_disjunctive_counts() {
        let c = catalog();
        let mut f = Filters::default();
        f.toggle(Field::Camera, "nikon z 5".into(), true);
        let album = BrowseScope {
            destination: Destination::Album(1),
            filters: f.clone(),
            ..BrowseScope::default()
        };
        let rows = facets(&c, &album, Field::Camera).unwrap();
        assert_eq!(rows.len(), 2);
        assert!(rows.iter().all(|r| r.count == 1));
        let rows = facets(&c, &BrowseScope::default().with_filters(f.clone()), Field::Iso).unwrap();
        assert_eq!(rows.len(), 2);
    }
    #[test]
    fn metadata_ranges_validate_and_injection_is_data() {
        let c = catalog();
        let mut f = Filters::default();
        assert!(f.range(Field::Iso, "NaN", "").is_err());
        assert!(f.range(Field::Iso, "100", "1").is_err());
        assert!(f.range(Field::CaptureDate, "2026-02-30", "").is_err());
        f.toggle(Field::Camera, "' OR 1=1 --".into(), true);
        assert!(ids(&c, &f).is_empty());
    }
    #[test]
    fn metadata_normalization_retains_originals_and_edits() {
        let c = catalog();
        assert_eq!(
            metadata::model_key(Some("NIKON CORPORATION"), Some("NIKON Z 5"), None).as_deref(),
            Some("nikon z 5")
        );
        assert_eq!(
            metadata::model_key(None, None, Some("NIKON CORPORATION NIKON Z 5")).as_deref(),
            Some("nikon z 5")
        );
        let original: String = c
            .query_row(
                "SELECT camera_model FROM photo_metadata WHERE photo_id=1",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(original, "NIKON Z 5");
        c.execute(
            "UPDATE photos SET edit_recipe='saved',rotation=90,mtime=10,size_bytes=20 WHERE id=1",
            [],
        )
        .unwrap();
        let m = PhotoMetadata {
            iso: Some(1600),
            ..Default::default()
        };
        assert!(!metadata::apply_backfill(&c, 1, "/a/keep1.NEF", Some(9), Some(20), &m).unwrap());
        assert!(metadata::apply_backfill(&c, 1, "/a/keep1.NEF", Some(10), Some(20), &m).unwrap());
        let p = photo(&c, 1).unwrap().unwrap();
        assert_eq!(p.edit_recipe, "saved");
        assert_eq!(p.rotation, 90);
        assert_eq!(p.mtime, Some(10));
    }
    #[test]
    fn metadata_additive_schema_preserves_existing_library() {
        let c = Connection::open_in_memory().unwrap();
        c.execute_batch("CREATE TABLE photos(id INTEGER PRIMARY KEY,path TEXT UNIQUE NOT NULL,folder_id INTEGER,taken_at TEXT,camera TEXT,aperture REAL,width INTEGER,height INTEGER,size_bytes INTEGER,mtime INTEGER,rotation INTEGER DEFAULT 0,favorite INTEGER DEFAULT 0,trashed INTEGER DEFAULT 0);
            INSERT INTO photos(id,path,camera) VALUES (42,'/old.nef','NIKON CORPORATION NIKON Z 5');").unwrap();
        c.execute_batch(SCHEMA).unwrap();
        super::super::migrate_photo_schema(&c).unwrap();
        c.execute_batch(SCHEMA).unwrap();
        super::super::migrate_photo_schema(&c).unwrap();
        assert_eq!(photos(&c, None, false, None).unwrap()[0].id, 42);
        metadata::seed_existing(&c).unwrap();
        metadata::seed_existing(&c).unwrap();
        let seeded: (String, String, i64) = c
            .query_row(
                "SELECT camera_key,file_type,version FROM photo_metadata WHERE photo_id=42",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!(seeded, ("nikon z 5".into(), "RAW".into(), 0));

        assert_eq!(
            c.query_row("SELECT count(*) FROM photo_metadata", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            1
        );
    }
    #[test]
    fn metadata_large_catalog_query() {
        let mut c = catalog();
        let tx = c.transaction().unwrap();
        tx.execute_batch("WITH RECURSIVE n(x) AS (VALUES(10) UNION ALL SELECT x+1 FROM n WHERE x<100009)
          INSERT INTO photos(id,path,iso,rating) SELECT x,'/offline/'||x||'.NEF',100*(1+x%8),x%6 FROM n;
          INSERT INTO photo_metadata(photo_id,camera_key,lens_key,file_type,has_exif,version)
          SELECT id,'nikon z 5','nikkor z 24-70mm f/4 s','RAW',1,1 FROM photos WHERE id>=10;").unwrap();
        tx.commit().unwrap();
        let mut f = Filters::default();
        f.toggle(Field::Camera, "nikon z 5".into(), true);
        f.range(Field::Iso, "100", "200").unwrap();
        let start = std::time::Instant::now();
        let result = ids(&c, &f);
        let elapsed = start.elapsed();
        assert_eq!(result.len(), 25001);
        eprintln!(
            "PIC_METADATA_BENCH photos=100004 matches={} elapsed_ms={}",
            result.len(),
            elapsed.as_millis()
        );
        assert!(
            elapsed < std::time::Duration::from_secs(5),
            "100k SQL query exceeded 5s: {elapsed:?}"
        );
    }
}
