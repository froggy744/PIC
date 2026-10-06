use crate::photo_object::PhotoObject;
use std::collections::HashMap;

/// Catalog enrichment updates arrive throughout an import. Looking up each
/// object by scanning the gallery would make those updates quadratic.
#[derive(Default)]
pub(super) struct CatalogObjectIndex {
    generation: Option<u64>,
    indexed: usize,
    first: Option<glib::WeakRef<PhotoObject>>,
    objects: HashMap<i64, glib::WeakRef<PhotoObject>>,
}

impl CatalogObjectIndex {
    pub fn clear(&mut self) {
        self.generation = None;
        self.indexed = 0;
        self.first = None;
        self.objects.clear();
    }
    pub fn photo(
        &mut self,
        photos: &[PhotoObject],
        generation: u64,
        id: i64,
    ) -> Option<PhotoObject> {
        let replaced_prefix = self
            .first
            .as_ref()
            .is_some_and(|first| first.upgrade().as_ref() != photos.first());
        if self.generation != Some(generation) || photos.len() < self.indexed || replaced_prefix {
            self.clear();
            self.generation = Some(generation);
        }
        if self.first.is_none() {
            if let Some(photo) = photos.first() {
                let first = glib::WeakRef::new();
                first.set(Some(photo));
                self.first = Some(first);
            }
        }
        for photo in &photos[self.indexed..] {
            let reference = glib::WeakRef::new();
            reference.set(Some(photo));
            self.objects.insert(photo.id(), reference);
        }
        self.indexed = photos.len();
        self.objects.get(&id).and_then(glib::WeakRef::upgrade)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn catalog_updates_find_appended_objects_and_follow_replacements() {
        let first = glib::Object::builder::<PhotoObject>()
            .property("id", 1i64)
            .build();
        let second = glib::Object::builder::<PhotoObject>()
            .property("id", 2i64)
            .build();
        let replacement = glib::Object::builder::<PhotoObject>()
            .property("id", 1i64)
            .build();
        let mut index = CatalogObjectIndex::default();
        assert_eq!(index.photo(&[first.clone()], 1, 1).unwrap(), first);
        assert_eq!(
            index.photo(&[first.clone(), second.clone()], 1, 2).unwrap(),
            second
        );
        assert_eq!(
            index.photo(&[replacement.clone()], 2, 1).unwrap(),
            replacement
        );
        assert!(index.photo(&[replacement.clone()], 2, 2).is_none());
        index.clear();
        assert_eq!(index.photo(&[first.clone()], 2, 1).unwrap(), first);
    }

    #[test]
    fn first_progressive_batch_can_replace_a_smaller_view_in_the_same_generation() {
        let old = glib::Object::builder::<PhotoObject>()
            .property("id", 1i64)
            .build();
        let replacement = glib::Object::builder::<PhotoObject>()
            .property("id", 1i64)
            .build();
        let appended = glib::Object::builder::<PhotoObject>()
            .property("id", 2i64)
            .build();
        let mut index = CatalogObjectIndex::default();
        index.photo(&[old.clone()], 2, 1);
        assert_eq!(
            index.photo(&[replacement.clone(), appended], 2, 1).unwrap(),
            replacement
        );
    }
}
