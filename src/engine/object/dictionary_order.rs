//! Insertion order beside a dictionary shape's compact physical slots.
//!
//! Removing a property swap-removes its physical slot. These links repair the
//! moved slot's neighbours, preserving key order without tombstones or a scan.
//! Atom ownership and parallel property values belong to shape/heap storage.

#[derive(Clone, Copy, Debug)]
struct Links {
    previous: Option<usize>,
    next: Option<usize>,
}

#[derive(Clone, Debug)]
pub(super) struct DictionaryOrder {
    links: Vec<Links>,
    first: Option<usize>,
    last: Option<usize>,
}

impl DictionaryOrder {
    pub(super) fn new(len: usize) -> Self {
        Self {
            links: (0..len)
                .map(|index| Links {
                    previous: index.checked_sub(1),
                    next: (index + 1 < len).then_some(index + 1),
                })
                .collect(),
            first: (len != 0).then_some(0),
            last: len.checked_sub(1),
        }
    }

    pub(super) fn first(&self) -> Option<usize> {
        self.first
    }

    pub(super) fn next(&self, index: usize) -> Option<usize> {
        self.links[index].next
    }

    pub(super) fn append(&mut self) {
        let index = self.links.len();
        self.links.push(Links {
            previous: self.last,
            next: None,
        });
        if let Some(last) = self.last {
            self.links[last].next = Some(index);
        } else {
            self.first = Some(index);
        }
        self.last = Some(index);
    }

    pub(super) fn swap_remove(&mut self, index: usize) {
        let removed = self.links[index];
        if let Some(previous) = removed.previous {
            self.links[previous].next = removed.next;
        } else {
            self.first = removed.next;
        }
        if let Some(next) = removed.next {
            self.links[next].previous = removed.previous;
        } else {
            self.last = removed.previous;
        }

        let previous_last_slot = self.links.len() - 1;
        self.links.swap_remove(index);
        if index != previous_last_slot {
            // The former last physical slot has moved, even when its logical
            // position is in the middle of insertion order.
            let moved = self.links[index];
            if let Some(previous) = moved.previous {
                self.links[previous].next = Some(index);
            } else {
                self.first = Some(index);
            }
            if let Some(next) = moved.next {
                self.links[next].previous = Some(index);
            } else {
                self.last = Some(index);
            }
        }
        let len = self.links.len();
        if self.links.capacity() > len.saturating_mul(4).saturating_add(16) {
            self.links
                .shrink_to(len.saturating_mul(2).saturating_add(8));
        }
    }

    pub(super) fn is_valid(&self, len: usize) -> bool {
        if self.links.len() != len {
            return false;
        }
        let mut seen = vec![false; len];
        let mut previous = None;
        let mut cursor = self.first;
        let mut count = 0;
        while let Some(index) = cursor {
            let Some(link) = self.links.get(index) else {
                return false;
            };
            if seen[index] || link.previous != previous {
                return false;
            }
            seen[index] = true;
            count += 1;
            previous = cursor;
            cursor = link.next;
        }
        count == len && previous == self.last
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn swap_removal_preserves_insertion_order_through_repeated_slot_moves() {
        let mut order = DictionaryOrder::new(0);
        let mut physical = Vec::new();
        let mut expected = Vec::new();
        let mut seed = 0x348a_6571_u64;
        for label in 0..4096 {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
            if !physical.is_empty() && seed % 3 != 0 {
                let index = (seed >> 32) as usize % physical.len();
                let removed = physical.swap_remove(index);
                expected.retain(|value| *value != removed);
                order.swap_remove(index);
            } else {
                order.append();
                physical.push(label);
                expected.push(label);
            }
            assert!(order.is_valid(physical.len()));
            let actual = std::iter::successors(order.first(), |&index| order.next(index))
                .map(|index| physical[index])
                .collect::<Vec<_>>();
            assert_eq!(actual, expected);
        }
    }
}
