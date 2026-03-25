use std::collections::{VecDeque, vec_deque};

/// Fixed-capacity history buffer used for short in-memory time series.
///
/// This type behaves like a sliding window:
/// - When the buffer reaches `capacity`, pushing a new item drops the oldest item.
/// - Iteration yields items from oldest to newest.
///
/// This keeps memory usage stable while preserving the latest samples for charts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryBuffer<T> {
    capacity: usize,
    items: VecDeque<T>,
}

impl<T> HistoryBuffer<T> {
    /// Creates a new history buffer with a fixed maximum number of items.
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity,
            items: VecDeque::with_capacity(capacity),
        }
    }

    /// Appends one sample to the history.
    ///
    /// If the buffer is full, the oldest sample is removed first.
    pub fn push(&mut self, item: T) {
        if self.items.len() == self.capacity {
            self.items.pop_front();
        }
        self.items.push_back(item);
    }

    /// Returns the number of items currently stored in the history.
    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// Returns `true` if the history is empty.
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// Returns the maximum number of items the history can hold.
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// Clears all items from the history, leaving it empty but retaining capacity.
    pub fn clear(&mut self) {
        self.items.clear();
    }

    /// Returns an iterator over the items from oldest to newest without consuming the buffer.
    pub fn iter(&self) -> vec_deque::Iter<'_, T> {
        self.items.iter()
    }

    /// Returns a reference to the oldest item in the history, or `None` if empty.
    pub fn front(&self) -> Option<&T> {
        self.items.front()
    }

    /// Returns a reference to the newest item in the history, or `None` if empty.
    pub fn back(&self) -> Option<&T> {
        self.items.back()
    }
}

impl<'a, T> IntoIterator for &'a HistoryBuffer<T> {
    type Item = &'a T;
    type IntoIter = vec_deque::Iter<'a, T>;

    /// Iterates over the history from oldest to newest without consuming it.
    fn into_iter(self) -> Self::IntoIter {
        self.items.iter()
    }
}

impl<T> IntoIterator for HistoryBuffer<T> {
    type Item = T;
    type IntoIter = vec_deque::IntoIter<T>;

    /// Consumes the buffer and yields items from oldest to newest.
    fn into_iter(self) -> Self::IntoIter {
        self.items.into_iter()
    }
}
impl<T> Extend<T> for HistoryBuffer<T> {
    /// Extends the history with items from an iterator.
    ///
    /// If the total number of items exceeds capacity, the oldest items are dropped.
    fn extend<I: IntoIterator<Item = T>>(&mut self, iter: I) {
        for item in iter {
            self.push(item);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::HistoryBuffer;

    #[test]
    fn keeps_only_latest_items() {
        let mut history = HistoryBuffer::new(3);
        history.push(1);
        history.push(2);
        history.push(3);
        history.push(4);

        let items: Vec<_> = (&history).into_iter().copied().collect();
        assert_eq!(items, vec![2, 3, 4]);
    }
}
