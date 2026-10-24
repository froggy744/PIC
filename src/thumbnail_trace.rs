use std::sync::OnceLock;

static TRACE_FILTER: OnceLock<Option<String>> = OnceLock::new();

fn filter_matches(filter: &str, uri: &str) -> bool {
    !filter.is_empty() && uri.contains(filter)
}

pub(crate) fn event(uri: &str, stage: &str, outcome: &str) {
    let Some(filter) = TRACE_FILTER
        .get_or_init(|| std::env::var("PIC_THUMBNAIL_TRACE_URI").ok())
        .as_deref()
    else {
        return;
    };
    if filter_matches(&filter, uri) {
        eprintln!("PIC_THUMBNAIL_FLOW stage={stage} outcome={outcome} uri={uri}");
    }
}

#[cfg(test)]
mod tests {
    use super::filter_matches;

    #[test]
    fn uri_filter_matches_only_the_selected_substring() {
        let selected = "nfs://DietPi.local/photos/DSC_4097.jpg";
        assert!(filter_matches("DSC_4097.jpg", selected));
        assert!(!filter_matches(
            "DSC_4097.jpg",
            "nfs://DietPi.local/photos/DSC_4098.jpg"
        ));
        assert!(!filter_matches("", selected));

        let encoded = "nfs://DietPi.local/photos/Tatiana%20SETTINGS/DSC_4097.jpg";
        assert!(filter_matches("Tatiana%20SETTINGS", encoded));
        assert!(!filter_matches("Tatiana SETTINGS", encoded));
    }
}
