use std::collections::VecDeque;

#[derive(Debug, Clone)]
pub struct HistoryBuffer<T> {
    capacity: usize,
    items: VecDeque<T>,
}

impl<T> HistoryBuffer<T> {
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity,
            items: VecDeque::with_capacity(capacity),
        }
    }

    pub fn push(&mut self, item: T) {
        if self.items.len() == self.capacity {
            self.items.pop_front();
        }
        self.items.push_back(item);
    }

    pub fn iter(&self) -> impl Iterator<Item = &T> {
        self.items.iter()
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

        let items: Vec<_> = history.iter().copied().collect();
        assert_eq!(items, vec![2, 3, 4]);
    }
}
