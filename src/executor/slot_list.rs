pub struct SlotList<T> {
    tasks: Vec<Option<T>>,
    free_slots: Vec<usize>,
}

impl<T> SlotList<T> {
    pub fn new() -> Self {
        Self {
            tasks: Vec::new(),
            free_slots: Vec::new(),
        }
    }

    pub fn add(&mut self, data: T) -> usize {
        if let Some(id) = self.free_slots.pop() {
            let old = self.tasks[id].replace(data);
            assert!(old.is_none());
            id
        } else {
            let id = self.tasks.len();
            self.tasks.push(Some(data));
            id
        }
    }

    pub fn remove(&mut self, id: usize) -> T {
        let entry = self.tasks[id].take().unwrap();
        self.free_slots.push(id);
        entry
    }

    pub fn get(&self, id: usize) -> Option<&T> {
        self.tasks[id].as_ref()
    }
}
