use std::sync::{Arc, Mutex};
use tauri::{State, command};

pub struct ConnectionsSearch {
    value: Arc<Mutex<String>>,
}

impl ConnectionsSearch {
    pub fn new() -> Self {
        Self {
            value: Arc::new(Mutex::new(String::new())),
        }
    }
    fn get(&self) -> String {
        if let Ok(v) = self.value.lock() {
            v.clone()
        } else {
            String::new()
        }
    }
    fn set(&self, value: String) {
        if let Ok(mut v) = self.value.lock() {
            *v = value;
        }
    }
}

#[command]
pub fn set_connections_search(cs: State<ConnectionsSearch>, value: String) {
    cs.set(value);
}

#[command]
pub fn get_connections_search(cs: State<ConnectionsSearch>) -> String {
    cs.get()
}
