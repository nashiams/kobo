/// Classification of a resource-kind binding.
///
/// Resource kinds are never wrapped in `Rc<RefCell<T>>` — they receive
/// `ScopedHandle<T>` instead, which enforces single ownership through
/// structured lifetimes.
#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug)]
pub enum ResourceKind {
    /// Heap-allocated memory with explicit lifecycle (not a standard `Vec` or `Box`).
    Memory,
    /// File handles: `std::fs::File`, `BufReader<File>`, etc.
    File,
    /// Mutex, RwLock, or other synchronization primitives.
    Lock,
    /// Sockets, streams, and other network resources.
    Network,
    /// OS-level handles: processes, signals, timers.
    Os,
}
