// Copyright (C) 2025  Braiins Systems s.r.o.

use indexmap::{Equivalent, IndexMap};
use slint::{Model, ModelNotify, ModelTracker};
use std::any::Any;
use std::cell::{Ref, RefCell};
use std::hash::Hash;

pub struct IndexMapModel<K, V> {
    map: RefCell<IndexMap<K, V>>,
    notify: ModelNotify,
}

impl<K, V> Default for IndexMapModel<K, V> {
    fn default() -> Self {
        Self::new(IndexMap::default())
    }
}

impl<K, V> IndexMapModel<K, V> {
    fn new(map: IndexMap<K, V>) -> Self {
        Self {
            map: RefCell::new(map),
            notify: ModelNotify::default(),
        }
    }
}

#[allow(unused, clippy::allow_attributes)]
impl<K: Hash + Eq + 'static, V: 'static> IndexMapModel<K, V> {
    pub fn get_index_of<Q: ?Sized + Hash + Equivalent<K>>(&self, key: &Q) -> Option<usize> {
        self.map.borrow().get_index_of(key)
    }

    // NOTE: similar to `Model::row_data`
    pub fn get<Q: ?Sized + Hash + Equivalent<K>>(&self, key: &Q) -> Option<Ref<'_, V>> {
        Ref::filter_map(self.map.borrow(), |map| map.get(key)).ok()
    }

    // NOTE: similar to `ModelExt::row_data_tracked`
    pub fn get_tracked<Q: ?Sized + Hash + Equivalent<K>>(&self, key: &Q) -> Option<Ref<'_, V>> {
        Ref::filter_map(self.map.borrow(), |map| {
            let (index, _key, value) = map.get_full(key)?;

            self.notify.track_row_data_changes(index);

            Some(value)
        })
        .ok()
    }

    // NOTE: similar to `Model::set_row_data`
    pub fn modify<Q: ?Sized + Hash + Equivalent<K>>(&self, key: &Q, modify: impl FnOnce(&mut V)) {
        let mut map = self.map.borrow_mut();

        if let Some((index, _key, value)) = map.get_full_mut(key) {
            modify(value);

            // must be dropped before calling notify
            drop(map);

            self.notify.row_changed(index);
        }
    }

    pub fn insert(&self, key: K, value: V) -> Option<V> {
        let (index, old_value) = self.map.borrow_mut().insert_full(key, value);

        // NOTE: insert replaces existing item without changing index
        if old_value.is_some() {
            self.notify.row_changed(index);
        } else {
            self.notify.row_added(index, 1);
        }

        old_value
    }

    pub fn shift_insert(&self, index: usize, key: K, value: V) -> Option<V> {
        let mut map = self.map.borrow_mut();
        let old_index = map.get_index_of(&key);
        let old_value = map.shift_insert(index, key, value);

        // must be dropped before calling notify
        drop(map);

        // NOTE: shift_insert moves existing item to new index, affecting other items as well
        if let Some(old_index) = old_index {
            self.multiple_rows_changed(old_index, index);
        } else {
            self.notify.row_added(index, 1);
        }

        old_value
    }

    pub fn move_index(&self, from_index: usize, to_index: usize) {
        // NOTE: move_index moves item to new index, affecting other items as well
        self.map.borrow_mut().move_index(from_index, to_index);

        self.multiple_rows_changed(from_index, to_index);
    }

    // NOTE: we provide only shift_remove, because remove/swap_remove would change order
    pub fn shift_remove<Q: ?Sized + Hash + Equivalent<K>>(&self, key: &Q) -> Option<V> {
        let (index, _key, value) = self.map.borrow_mut().shift_remove_full(key)?;

        self.notify.row_removed(index, 1);

        Some(value)
    }

    pub fn clear(&self) {
        self.map.borrow_mut().clear();
        self.notify.reset();
    }

    fn multiple_rows_changed(&self, from: usize, to: usize) {
        let start = from.min(to);
        let end = from.max(to);

        for index in start..=end {
            self.notify.row_changed(index);
        }
    }
}

impl<K, V> From<IndexMap<K, V>> for IndexMapModel<K, V> {
    fn from(map: IndexMap<K, V>) -> Self {
        Self::new(map)
    }
}

impl<K: Hash + Eq, V> FromIterator<(K, V)> for IndexMapModel<K, V> {
    fn from_iter<T: IntoIterator<Item = (K, V)>>(iter: T) -> Self {
        Self::new(IndexMap::from_iter(iter))
    }
}

impl<K: 'static, V: Clone + 'static> Model for IndexMapModel<K, V> {
    type Data = V;

    fn row_count(&self) -> usize {
        self.map.borrow().len()
    }

    fn row_data(&self, row: usize) -> Option<Self::Data> {
        let map = self.map.borrow();
        let (_key, value) = map.get_index(row)?;
        Some(value.clone())
    }

    fn set_row_data(&self, row: usize, data: Self::Data) {
        let mut map = self.map.borrow_mut();

        if let Some((_key, value)) = map.get_index_mut(row) {
            *value = data;

            // must be dropped before calling notify
            drop(map);

            self.notify.row_changed(row);
        }
    }

    fn model_tracker(&self) -> &dyn ModelTracker {
        &self.notify
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}
