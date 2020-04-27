use std::collections::{HashMap, VecDeque};
use std::hash::Hash;
use std::iter::FromIterator;
use std::sync::RwLock;

#[derive(Debug)]
pub struct Map<K: Eq + Hash, V> {
    map: RwLock<HashMap<K, V>>,
}

impl<K: Eq + Hash, V: Clone> Map<K, V> {
    pub fn new() -> Map<K, V> {
        Map {
            map: RwLock::new(HashMap::new()),
        }
    }

    pub fn contains_key(&self, key: &K) -> bool {
        self.map.read().unwrap().contains_key(key)
    }

    pub fn get(&self, key: &K) -> Option<V> {
        match self.map.read().unwrap().get(key) {
            Some(val) => Some(val.clone()),
            None => None,
        }
    }

    pub fn insert(&self, key: K, value: V) -> Option<V> {
        self.map.write().unwrap().insert(key, value)
    }
}

impl<K: Eq + Hash, V> FromIterator<(K, V)> for Map<K, V> {
    fn from_iter<T: IntoIterator<Item = (K, V)>>(i: T) -> Map<K, V> {
        let mut map: HashMap<K, V> = HashMap::new();
        for (k, v) in i {
            map.insert(k, v);
        }
        Map {
            map: RwLock::new(map),
        }
    }
}

#[derive(Debug)]
pub struct Queue<V: Clone> {
    queue: RwLock<Vec<V>>,
}

impl<V: Clone> Queue<V> {
    pub fn new() -> Queue<V> {
        Queue {
            queue: RwLock::new(vec![]),
        }
    }

    pub fn push(&self, item: V) {
        self.queue.write().unwrap().push(item)
    }

    pub fn pop(&self) -> Option<V> {
        self.queue.write().unwrap().pop()
    }

    pub fn reverse(&self) {
        self.queue.write().unwrap().reverse()
    }

    pub fn to_vec(&self) -> Vec<V> {
        self.queue.read().unwrap().iter().cloned().collect()
    }
}

#[derive(Debug)]
pub struct Deque<V> {
    deque: RwLock<VecDeque<V>>,
}

impl<V> Deque<V> {
    pub fn new() -> Deque<V> {
        Deque {
            deque: RwLock::new(VecDeque::new()),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.deque.read().unwrap().is_empty()
    }

    pub fn pop_front(&self) -> Option<V> {
        self.deque.write().unwrap().pop_front()
    }

    pub fn push_back(&self, item: V) {
        self.deque.write().unwrap().push_back(item)
    }
}

pub struct Value<T: Clone> {
    value: RwLock<T>,
}

impl<T: Clone> Value<T> {
    pub fn of(value: T) -> Value<T> {
        Value {
            value: RwLock::new(value),
        }
    }

    pub fn read(&self) -> T {
        self.value.read().unwrap().clone()
    }

    pub fn write(&self, value: T) {
        *self.value.write().unwrap() = value
    }
}

impl<T: Clone + PartialEq> PartialEq for Value<T> {
    fn eq(&self, other: &Self) -> bool {
        *self.value.read().unwrap() == *other.value.read().unwrap()
    }
}

impl<T: Clone + Eq> Eq for Value<T> {}