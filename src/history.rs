// history.rs

pub struct History {
    entries: Vec<String>,
}

impl History {
    pub fn new() -> Self {
        Self { entries: Vec::new() }
    }
    pub fn add(&mut self, entry: String) {
        self.entries.push(entry);
    }
    pub fn get(&self, n: usize) -> Option<&String> {
        self.entries.get(n)
    }
    pub fn len(&self) -> usize {
        self.entries.len()
    }
    pub fn clear(&mut self) {
        self.entries.clear();
    }
    pub fn all(&self) -> &Vec<String> {
        &self.entries
    }
} 