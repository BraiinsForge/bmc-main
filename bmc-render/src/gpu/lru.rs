// Copyright (C) 2026  Braiins Forge s.r.o.
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License
// along with this program.  If not, see <https://www.gnu.org/licenses/>.
//
// Braiins Systems s.r.o. and Braiins Forge s.r.o. each reserve the right
// to grant any party a license to this program, or any part thereof,
// under any terms, and such a grant shall be considered distinct from
// the grant above.

/// Stores the links threaded through an [`LruQueue`]'s entries.
/// `links` returns the neighbours toward the hot and cold ends, respectively.
pub(crate) trait LruStore<K: Copy> {
    fn links(&self, key: K) -> (Option<K>, Option<K>);
    fn set_links(&mut self, key: K, prev: Option<K>, next: Option<K>);
    fn set_prev(&mut self, key: K, prev: Option<K>);
    fn set_next(&mut self, key: K, next: Option<K>);
}

/// Intrusive LRU order with `head` as the most recently used entry
/// and `tail` as the eviction candidate.
/// Keeping links in the backing store makes promotion allocation-free.
#[derive(Debug)]
pub(crate) struct LruQueue<K> {
    head: Option<K>,
    tail: Option<K>,
}

impl<K: Copy + Eq> LruQueue<K> {
    pub(crate) const fn new() -> Self {
        Self {
            head: None,
            tail: None,
        }
    }

    /// Adds an unlinked key as the most recently used entry.
    pub(crate) fn push_hot(&mut self, store: &mut impl LruStore<K>, key: K) {
        debug_assert!(
            {
                let (prev, next) = store.links(key);
                prev.is_none() && next.is_none() && self.head != Some(key) && self.tail != Some(key)
            },
            "BUG: pushing an entry already held by the queue"
        );
        self.link_as_hot(store, key);
    }

    fn link_as_hot(&mut self, store: &mut impl LruStore<K>, key: K) {
        let old_head = self.head;
        store.set_links(key, None, old_head);

        if let Some(old_head) = old_head {
            store.set_prev(old_head, Some(key));
        } else {
            self.tail = Some(key);
        }
        self.head = Some(key);
    }

    pub(crate) fn unlink(&mut self, store: &mut impl LruStore<K>, key: K) {
        self.detach(store, key);
        store.set_links(key, None, None);
    }

    fn detach(&mut self, store: &mut impl LruStore<K>, key: K) {
        let (prev, next) = store.links(key);
        debug_assert!(
            (prev.is_some() || self.head == Some(key))
                && (next.is_some() || self.tail == Some(key)),
            "BUG: unlinking an entry the queue does not hold"
        );

        if let Some(prev) = prev {
            store.set_next(prev, next);
        } else {
            self.head = next;
        }
        if let Some(next) = next {
            store.set_prev(next, prev);
        } else {
            self.tail = prev;
        }
    }

    pub(crate) fn promote(&mut self, store: &mut impl LruStore<K>, key: K) {
        if self.head == Some(key) {
            return;
        }
        self.detach(store, key);
        self.link_as_hot(store, key);
    }

    pub(crate) const fn coldest(&self) -> Option<K> {
        self.tail
    }

    #[cfg(test)]
    pub(crate) const fn hottest(&self) -> Option<K> {
        self.head
    }
}

#[cfg(test)]
mod tests {
    use super::{LruQueue, LruStore};

    #[derive(Clone, Copy, Default)]
    struct Node {
        prev: Option<usize>,
        next: Option<usize>,
    }

    #[derive(Default)]
    struct CountingStore {
        nodes: Vec<Node>,
        link_replacements: usize,
    }

    impl LruStore<usize> for CountingStore {
        fn links(&self, index: usize) -> (Option<usize>, Option<usize>) {
            (self.nodes[index].prev, self.nodes[index].next)
        }

        fn set_links(&mut self, index: usize, prev: Option<usize>, next: Option<usize>) {
            self.link_replacements += 1;
            self.nodes[index].prev = prev;
            self.nodes[index].next = next;
        }

        fn set_prev(&mut self, index: usize, prev: Option<usize>) {
            self.nodes[index].prev = prev;
        }

        fn set_next(&mut self, index: usize, next: Option<usize>) {
            self.nodes[index].next = next;
        }
    }

    impl LruStore<usize> for Vec<Node> {
        fn links(&self, index: usize) -> (Option<usize>, Option<usize>) {
            (self[index].prev, self[index].next)
        }

        fn set_links(&mut self, index: usize, prev: Option<usize>, next: Option<usize>) {
            self[index].prev = prev;
            self[index].next = next;
        }

        fn set_prev(&mut self, index: usize, prev: Option<usize>) {
            self[index].prev = prev;
        }

        fn set_next(&mut self, index: usize, next: Option<usize>) {
            self[index].next = next;
        }
    }

    fn push_nodes(store: &mut Vec<Node>, lru: &mut LruQueue<usize>, count: usize) {
        let start = store.len();
        for index in start..start + count {
            store.push(Node::default());
            lru.push_hot(store, index);
        }
    }

    fn hot_to_cold(store: &[Node], lru: &LruQueue<usize>) -> Vec<usize> {
        let mut order = Vec::new();
        let mut cursor = lru.head;
        while let Some(index) = cursor {
            order.push(index);
            cursor = store[index].next;
        }
        order
    }

    fn cold_to_hot(store: &[Node], lru: &LruQueue<usize>) -> Vec<usize> {
        let mut order = Vec::new();
        let mut cursor = lru.tail;
        while let Some(index) = cursor {
            order.push(index);
            cursor = store[index].prev;
        }
        order
    }

    fn assert_order(store: &[Node], lru: &LruQueue<usize>, expected: &[usize]) {
        assert_eq!(hot_to_cold(store, lru), expected);
        assert_eq!(
            cold_to_hot(store, lru),
            expected.iter().rev().copied().collect::<Vec<_>>()
        );
    }

    #[test]
    fn promotion_makes_the_next_entry_the_coldest() {
        let mut store = Vec::new();
        let mut lru = LruQueue::new();
        push_nodes(&mut store, &mut lru, 3);

        assert_eq!(lru.coldest(), Some(0));
        lru.promote(&mut store, 0);
        assert_eq!(lru.coldest(), Some(1));
        assert_order(&store, &lru, &[0, 2, 1]);
    }

    #[test]
    fn unlinking_from_any_position_keeps_the_rest_ordered() {
        let mut store = Vec::new();
        let mut lru = LruQueue::new();
        push_nodes(&mut store, &mut lru, 3);

        lru.unlink(&mut store, 1);
        assert_order(&store, &lru, &[2, 0]);
        lru.unlink(&mut store, 2);
        assert_order(&store, &lru, &[0]);
        lru.unlink(&mut store, 0);
        assert_order(&store, &lru, &[]);
        assert_eq!(lru.coldest(), None);
    }

    #[test]
    fn promoting_the_hottest_entry_keeps_the_queue_intact() {
        let mut store = Vec::new();
        let mut lru = LruQueue::new();
        push_nodes(&mut store, &mut lru, 1);

        lru.promote(&mut store, 0);
        assert_order(&store, &lru, &[0]);
        push_nodes(&mut store, &mut lru, 2);
        lru.promote(&mut store, 2);
        assert_order(&store, &lru, &[2, 1, 0]);
    }

    #[test]
    fn an_empty_queue_has_no_coldest_entry() {
        assert_eq!(LruQueue::<usize>::new().coldest(), None);
    }

    #[test]
    fn millions_of_promotions_never_grow_the_store() {
        let mut store = Vec::with_capacity(64);
        let mut lru = LruQueue::new();
        push_nodes(&mut store, &mut lru, 64);
        let capacity = store.capacity();

        for index in 0..2_000_000_usize {
            lru.promote(&mut store, index % 64);
        }

        assert_eq!(store.capacity(), capacity);
        assert_eq!(store.len(), 64);
    }

    #[test]
    fn promotion_replaces_promoted_entry_links_once() {
        let mut store = CountingStore {
            nodes: vec![Node::default(); 3],
            link_replacements: 0,
        };
        let mut lru = LruQueue::new();
        for index in 0..3 {
            lru.push_hot(&mut store, index);
        }
        store.link_replacements = 0;

        lru.promote(&mut store, 0);

        assert_eq!(store.link_replacements, 1);
    }
}
