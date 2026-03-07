use bytes::BytesMut;
use parking_lot::Mutex;
use std::cell::RefCell;

pub const DEFAULT_BUFFER_CAPACITY: usize = 8192;

pub const MAX_POOL_SIZE: usize = 32;

#[derive(Debug)]
pub struct BufferPool {
    buffers: Mutex<Vec<BytesMut>>,
    default_capacity: usize,
    max_pool_size: usize,
}

impl Default for BufferPool {
    fn default() -> Self {
        Self::new()
    }
}

impl BufferPool {
    pub fn new() -> Self {
        Self::with_capacity(DEFAULT_BUFFER_CAPACITY, MAX_POOL_SIZE)
    }

    pub fn try_with_capacity(
        default_capacity: usize,
        max_pool_size: usize,
    ) -> Result<Self, &'static str> {
        if default_capacity == 0 {
            return Err("default_capacity must be greater than zero");
        }
        if max_pool_size == 0 {
            return Err("max_pool_size must be greater than zero");
        }

        Ok(Self {
            buffers: Mutex::new(Vec::with_capacity(max_pool_size)),
            default_capacity,
            max_pool_size,
        })
    }
    pub fn with_capacity(default_capacity: usize, max_pool_size: usize) -> Self {
        Self::try_with_capacity(default_capacity, max_pool_size)
            .expect("BufferPool::with_capacity() called with zero capacity or pool size")
    }

    pub fn get(&self) -> BytesMut {
        let mut buffers = self.buffers.lock();
        buffers
            .pop()
            .unwrap_or_else(|| BytesMut::with_capacity(self.default_capacity))
    }

    pub fn get_with_capacity(&self, min_capacity: usize) -> BytesMut {
        let mut buffers = self.buffers.lock();

        if let Some(pos) = buffers.iter().position(|b| b.capacity() >= min_capacity) {
            return buffers.swap_remove(pos);
        }

        if let Some(mut buf) = buffers.pop() {
            if buf.capacity() < min_capacity {
                buf.reserve(min_capacity);
            }
            debug_assert!(
                buf.capacity() >= min_capacity,
                "BufferPool: capacity {} < requested {}",
                buf.capacity(),
                min_capacity
            );
            return buf;
        }

        BytesMut::with_capacity(min_capacity.max(self.default_capacity))
    }

    pub fn put(&self, mut buffer: BytesMut) {
        buffer.clear();

        let mut buffers = self.buffers.lock();

        if buffers.len() < self.max_pool_size {
            buffers.push(buffer);
        }
    }

    pub fn len(&self) -> usize {
        self.buffers.lock().len()
    }

    pub fn is_empty(&self) -> bool {
        self.buffers.lock().is_empty()
    }

    pub fn clear(&self) {
        self.buffers.lock().clear();
    }
}

thread_local! {
    static THREAD_LOCAL_POOL: RefCell<BufferPool> = RefCell::new(BufferPool::new());
}

pub fn get_buffer() -> BytesMut {
    THREAD_LOCAL_POOL.with(|pool| pool.borrow().get())
}

pub fn get_buffer_with_capacity(min_capacity: usize) -> BytesMut {
    THREAD_LOCAL_POOL.with(|pool| pool.borrow().get_with_capacity(min_capacity))
}

pub fn return_buffer(buffer: BytesMut) {
    THREAD_LOCAL_POOL.with(|pool| pool.borrow().put(buffer));
}

pub struct PooledBuffer {
    buffer: Option<BytesMut>,
}

impl PooledBuffer {
    pub fn new() -> Self {
        Self {
            buffer: Some(get_buffer()),
        }
    }

    pub fn with_capacity(min_capacity: usize) -> Self {
        Self {
            buffer: Some(get_buffer_with_capacity(min_capacity)),
        }
    }

    pub fn as_bytes_mut(&mut self) -> &mut BytesMut {
        self.buffer.as_mut().expect("buffer already taken")
    }

    pub fn take(mut self) -> BytesMut {
        self.buffer.take().expect("buffer already taken")
    }
}

impl Default for PooledBuffer {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for PooledBuffer {
    fn drop(&mut self) {
        if let Some(buffer) = self.buffer.take() {
            return_buffer(buffer);
        }
    }
}

impl std::ops::Deref for PooledBuffer {
    type Target = BytesMut;

    fn deref(&self) -> &Self::Target {
        self.buffer.as_ref().expect("buffer already taken")
    }
}

impl std::ops::DerefMut for PooledBuffer {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.buffer.as_mut().expect("buffer already taken")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_buffer_pool_basic() {
        let pool = BufferPool::new();

        // Get a buffer
        let buf1 = pool.get();
        assert!(buf1.capacity() >= DEFAULT_BUFFER_CAPACITY);

        // Return it
        pool.put(buf1);
        assert_eq!(pool.len(), 1);

        // Get it back
        let buf2 = pool.get();
        assert!(pool.is_empty());
        assert!(buf2.capacity() >= DEFAULT_BUFFER_CAPACITY);
    }

    #[test]
    fn test_buffer_pool_capacity() {
        let pool = BufferPool::new();

        let buf = pool.get_with_capacity(16384);
        assert!(buf.capacity() >= 16384);
    }

    #[test]
    fn test_get_with_capacity_returns_sufficient_capacity() {
        let pool = BufferPool::new();

        // Put a buffer with capacity 8192 (the default)
        let buf = BytesMut::with_capacity(8192);
        pool.put(buf);

        // Request a capacity larger than existing but less than 2x existing.
        // This is the exact scenario that triggered the heap corruption bug:
        // reserve(min_capacity - old_cap) was a no-op when old_cap >= min_capacity/2.
        let result = pool.get_with_capacity(12000);
        assert!(
            result.capacity() >= 12000,
            "Expected capacity >= 12000, got {}",
            result.capacity()
        );
    }

    #[test]
    fn test_get_with_capacity_exact_boundary() {
        let pool = BufferPool::new();

        let buf = BytesMut::with_capacity(8192);
        pool.put(buf);

        // Request exactly the existing capacity
        let result = pool.get_with_capacity(8192);
        assert!(result.capacity() >= 8192);
    }

    #[test]
    fn test_get_with_capacity_much_larger() {
        let pool = BufferPool::new();

        let buf = BytesMut::with_capacity(4096);
        pool.put(buf);

        // Request much larger than existing
        let result = pool.get_with_capacity(65536);
        assert!(
            result.capacity() >= 65536,
            "Expected capacity >= 65536, got {}",
            result.capacity()
        );
    }

    #[test]
    fn test_get_with_capacity_prefers_matching_buffer() {
        let pool = BufferPool::new();

        // Put a small and a large buffer
        pool.put(BytesMut::with_capacity(4096));
        pool.put(BytesMut::with_capacity(16384));

        // Should find the 16384 buffer directly
        let result = pool.get_with_capacity(12000);
        assert!(result.capacity() >= 12000);
        assert_eq!(pool.len(), 1);
    }

    #[test]
    fn test_pooled_buffer_guard() {
        let initial_count = THREAD_LOCAL_POOL.with(|p| p.borrow().len());

        {
            let mut buf = PooledBuffer::new();
            buf.extend_from_slice(b"hello");
            assert_eq!(&buf[..], b"hello");
        }

        // Buffer should be returned to pool
        let final_count = THREAD_LOCAL_POOL.with(|p| p.borrow().len());
        assert!(final_count >= initial_count);
    }
}
