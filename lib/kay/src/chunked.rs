use ::std::mem;
use ::std::mem::transmute;
use ::std::ptr;
use ::std::marker::PhantomData;
use ::std::ops::{Deref, DerefMut};
use super::allocators::{Allocator, DefaultHeap};

pub trait Chunker {
    fn chunk_size(&self) -> usize;
    fn name(&self) -> &str;
    fn with_chunk_size(&self, size: usize) -> Box<Chunker>;
    fn with_name(&self, name: &str) -> Box<Chunker>;
    fn child(&self, suffix: &str) -> Box<Chunker>;
    fn create_chunk(&mut self) -> *mut u8;
    fn load_chunk(&mut self, _index: usize) -> *mut u8 {
        self.create_chunk()
    }
    fn destroy_chunk(&mut self, ptr: *mut u8);
}

#[derive(Clone)]
pub struct MemChunker {
    name: String,
    chunk_size: usize
}

impl MemChunker {
    pub fn new(name: &str, chunk_size: usize) -> Box<Chunker> {
        box MemChunker{
            name: String::from(name),
            chunk_size: chunk_size
        }
    }
}

impl Chunker for MemChunker {
    fn chunk_size(&self) -> usize {self.chunk_size}
    fn name(&self) -> &str {self.name.as_str()}

    fn with_chunk_size(&self, size: usize) -> Box<Chunker> {
        let mut new = self.clone();
        new.chunk_size = size;
        box new
    }

    fn with_name(&self, name: &str) -> Box<Chunker> {
        let mut new = self.clone();
        new.name = String::from(name);
        box new
    }

    fn child(&self, suffix: &str) -> Box<Chunker> {
        box MemChunker{
            name: self.name.clone() + suffix,
            chunk_size: self.chunk_size
        }
    }

    fn create_chunk(&mut self) -> *mut u8 {
        DefaultHeap::allocate::<u8>(self.chunk_size)
    }

    fn destroy_chunk(&mut self, ptr: *mut u8) {
        // TODO: remove file?
        unsafe {
            DefaultHeap::deallocate(ptr, self.chunk_size);
        }
    }
}

pub struct ValueInChunk<T> {
    ptr: *mut u8,
    chunker: Box<Chunker>,
    _marker: PhantomData<T>
}

impl<T> ValueInChunk<T> {
    pub fn new(chunker: Box<Chunker>, default: T) -> ValueInChunk<T> {
        let mut chunker = chunker.with_chunk_size(mem::size_of::<T>());
        let ptr = chunker.create_chunk();
        unsafe {
            *(ptr as *mut T) = default;
        }
        ValueInChunk{
            ptr: ptr,
            chunker: chunker,
            _marker: PhantomData
        }
    }
}

impl<T> Deref for ValueInChunk<T> {
    type Target = T;

    fn deref(&self) -> &T {
        unsafe {
            (self.ptr as *const T).as_ref().unwrap()
        }
    }
}

impl<T> DerefMut for ValueInChunk<T> {
    fn deref_mut(&mut self) -> &mut T {
        unsafe {
            (self.ptr as *mut T).as_mut().unwrap()
        }
    }
}

impl<T> Drop for ValueInChunk<T> {
    fn drop(&mut self) {
        unsafe{::std::ptr::drop_in_place(self.ptr)};
        self.chunker.destroy_chunk(self.ptr);
    }
}

pub trait SizedChunkedCollection {
    fn new(chunker: Box<Chunker>, item_size: usize) -> Self;
}

pub struct SizedChunkedArena {
    pub chunker: Box<Chunker>,
    pub chunks: Vec<*mut u8>,
    pub item_size: usize,
    len: ValueInChunk<usize>
}

impl SizedChunkedArena {
    fn items_per_chunk(&self) -> usize {
        self.chunker.chunk_size() / self.item_size
    }

    fn pop_chunk(&mut self) {
        let ptr = self.chunks.pop().unwrap();
        self.chunker.destroy_chunk(ptr);
    }

    pub fn len(&self) -> usize {
        *self.len
    }

    pub fn push(&mut self) -> (*mut u8, usize) {
        if (*self.len + 1) > self.chunks.len() * self.items_per_chunk() {
            self.chunks.push(self.chunker.create_chunk());
        }
        let offset = (*self.len % self.items_per_chunk()) * self.item_size;
        let index = *self.len;
        *self.len += 1;
        unsafe {
            (self.chunks.last_mut().unwrap().offset(offset as isize), index)
        }
    }

    pub fn pop_away(&mut self) {
        *self.len -= 1;
        if *self.len + self.items_per_chunk() < self.chunks.len() * self.items_per_chunk() {
            self.pop_chunk();
        }
    }

    pub unsafe fn swap_remove(&mut self, index: usize) -> Option<*const u8> {
        let last_index = *self.len - 1;
        if last_index == index {
            self.pop_away();
            None
        } else {
            let last = self.at(*self.len - 1);
            let at_index = self.at_mut(index);
            ptr::copy_nonoverlapping(last, at_index, self.item_size);
            self.pop_away();
            Some(self.at(index))
        }
    }

    pub unsafe fn at(&self, index: usize) -> *const u8 {
        self.chunks[index / self.items_per_chunk()]
            .offset(((index % self.items_per_chunk()) * self.item_size) as isize)
    }

    pub unsafe fn at_mut(&mut self, index: usize) -> *mut u8 {
        let items_per_chunk = self.items_per_chunk();
        self.chunks[index / items_per_chunk]
            .offset(((index % items_per_chunk) * self.item_size) as isize)
    }
}

impl SizedChunkedCollection for SizedChunkedArena {
    fn new(chunker: Box<Chunker>, item_size: usize) -> Self {
        assert!(chunker.chunk_size() >= item_size);
        let mut arena = SizedChunkedArena {
            len: ValueInChunk::new(chunker.child("_len"), 0),
            chunker: chunker,
            chunks: Vec::new(),
            item_size: item_size,
        };

        while arena.chunks.len() < *arena.len / arena.items_per_chunk() {
            let next_chunk_index = arena.chunks.len();
            arena.chunks.push(arena.chunker.load_chunk(next_chunk_index));
        }

        arena
    }
}

// struct SizedChunkedArenaChunkIter {
//     item: *mut u8,
//     chunk_end: *mut u8,
//     item_size: usize
// }

// impl SizedChunkedArenaChunkIter {
//     fn uninitialized() -> Self {
//         SizedChunkedArenaChunkIter{
//             item: ::std::ptr::null_mut(),
//             chunk_end: ::std::ptr::null_mut(),
//             item_size: 0
//         }
//     }
// }

// impl Iterator for SizedChunkedArenaChunkIter {
//     type Item = *mut u8;
//     fn next(&mut self) -> Option<*mut u8> {
//         if self.item < self.chunk_end {
//             let item = self.item;
//             self.item = item.offset(self.item_size as isize);
//             Some(item)
//         } else {
//             None
//         }
//     }
// }

// struct SizedChunkedArenaIter {
//     chunks_iterator: ::std::vec::IntoIter<*mut u8>,
//     iterator_in_chunk: SizedChunkedArenaChunkIter,
//     chunk_size: usize,
//     item_size: usize
// }

// impl Iterator for SizedChunkedArenaIter {
//     type Item = *mut u8;
//     fn next(&mut self) -> Option<Self::Item> {
//         match self.iterator_in_chunk.next() {
//             None => match self.chunks_iterator.next() {
//                 Some(chunk) => {
//                     self.iterator_in_chunk = SizedChunkedArenaChunkIter{
//                         item: chunk,
//                         chunk_end: chunk.offset(self.chunk_size as isize),
//                         item_size: self.item_size
//                     };
//                     self.next()
//                 },
//                 None => None
//             },
//             Some(item) => Some(item)
//         }
//     } 
// }

// impl<'a> IntoIterator for &'a mut SizedChunkedArena {
//     type Item = *mut u8;
//     type IntoIter = SizedChunkedArenaIter;

//     fn into_iter(self) -> Self::IntoIter {
//         SizedChunkedArenaIter{
//             chunks_iterator: self.chunks.into_iter(),
//             iterator_in_chunk: SizedChunkedArenaChunkIter::uninitialized(),
//             chunk_size: self.chunker.chunk_size(),
//             item_size: self.item_size
//         }
//     }
// }

pub struct ChunkedVec<Item: Clone> {
    arena: SizedChunkedArena,
    marker: PhantomData<Item>
}

impl <Item: Clone> ChunkedVec<Item> {
    pub fn new(chunker: Box<Chunker>) -> Self {
        ChunkedVec {
            arena: SizedChunkedArena::new(chunker, mem::size_of::<Item>()),
            marker: PhantomData
        }
    }

    pub fn len(&self) -> usize {
        *self.arena.len
    }

    pub fn at(&self, index: usize) -> &Item {
        unsafe {
            &*(self.arena.at(index) as *const Item)
        }
    }

    pub fn at_mut(&mut self, index: usize) -> &mut Item {
        unsafe {
            &mut *(self.arena.at(index) as *mut Item)
        }
    }

    pub fn push(&mut self, item: Item) {
        unsafe {
            let item_ptr = self.arena.push().0 as *mut Item;
            *item_ptr = item;
        }
    }

    pub fn pop(&mut self) -> Option<Item> {
        if *self.arena.len == 0 {
            None
        } else {
            unsafe {
                let item_ptr : *const Item = transmute(self.arena.at(*self.arena.len - 1));
                self.arena.pop_away();
                Some((*item_ptr).clone())
            }
        }
    }
}

// TODO: replace this by concurrent MPSC queue
// add write_done and read_done indices
// if one thread finishes writing out-of-order,
// let it busy wait for the slower thread then
// increase the write_done counter
pub struct SizedChunkedQueue {
    chunker: Box<Chunker>,
    chunks: Vec<*mut u8>,
    item_size: usize,
    start_index: ValueInChunk<usize>,
    pub read_index: ValueInChunk<usize>,
    pub write_index: ValueInChunk<usize>,
    end_index: ValueInChunk<usize>,
    chunks_to_drop: Vec<*mut u8>,
    n_dropped_chunks: ValueInChunk<usize>
}

impl SizedChunkedQueue {
    fn items_per_chunk(&self) -> usize {
        self.chunker.chunk_size() / self.item_size
    }

    // TODO: separate into enqueue_start and enqueue_done
    // or return done_guard
    pub unsafe fn enqueue(&mut self) -> *mut u8 {
        if *self.write_index >= *self.end_index {
            self.chunks.push(self.chunker.create_chunk());
            *self.end_index += self.items_per_chunk();
        }

        let offset = ((*self.write_index % self.items_per_chunk()) * self.item_size) as isize;
        let ptr = self.chunks.last_mut().unwrap().offset(offset);
        *self.write_index += 1;
        ptr
    }

    // TODO: separate into dequeue_start and dequeue_done
    // or return done_guard
    pub unsafe fn dequeue(&mut self) -> Option<*const u8> {
        if *self.read_index == *self.write_index {
            None
        } else {
            let offset = ((*self.read_index % self.items_per_chunk()) * self.item_size) as isize;
            let ptr = self.chunks[0].offset(offset);
            *self.read_index += 1;
            if *self.read_index >= (*self.n_dropped_chunks + 1) * self.items_per_chunk() {
                self.chunks_to_drop.push(self.chunks.remove(0));
                *self.n_dropped_chunks += 1;
            }
            Some(ptr)
        }
    }

    pub unsafe fn drop_old_chunks(&mut self) {
        for chunk in self.chunks_to_drop.drain(..) {
            self.chunker.destroy_chunk(chunk);
        }
    }
}

impl SizedChunkedCollection for SizedChunkedQueue {
    fn new(chunker: Box<Chunker>, item_size: usize) -> Self {
        assert!(chunker.chunk_size() >= item_size);

        let mut queue = SizedChunkedQueue {
            start_index: ValueInChunk::new(chunker.child("_start"), 0),
            read_index: ValueInChunk::new(chunker.child("_read"), 0),
            write_index: ValueInChunk::new(chunker.child("_write"), 0),
            end_index: ValueInChunk::new(chunker.child("_end"), 0),
            n_dropped_chunks: ValueInChunk::new(chunker.child("_n_dropped"), 0),
            chunker: chunker,
            chunks: Vec::new(),
            chunks_to_drop: Vec::new(),
            item_size: item_size,
        };

        let mut chunk_index = *queue.start_index;
        while chunk_index < *queue.end_index {
            queue.chunks.push(queue.chunker.load_chunk(chunk_index));
            chunk_index += queue.chunker.chunk_size();
        }

        queue
    }
}

pub struct MultiSized<B: SizedChunkedCollection> {
    chunker: Box<Chunker>,
    base_size: usize,
    pub collections: Vec<B>,
    largest_size: ValueInChunk<usize>
}

impl<B: SizedChunkedCollection> MultiSized<B> {
    pub fn new(chunker: Box<Chunker>, base_size: usize) -> Self {
        let mut multi_sized = MultiSized{
            largest_size: ValueInChunk::new(chunker.child("_largest"), 0),
            chunker: chunker,
            collections: Vec::new(),
            base_size: base_size
        };

        while multi_sized.collections.len() < *multi_sized.largest_size {
            multi_sized.push_higher_sized_collection();
        }

        multi_sized
    }

    fn push_higher_sized_collection(&mut self) {
        let new_largest_size = 2u32.pow(self.collections.len() as u32) as usize * self.base_size;
        self.collections.push(B::new(
            self.chunker.child(format!("_{}", new_largest_size).as_str()),
            new_largest_size,
        ))
    }

    pub fn size_to_index(&self, size: usize) -> usize {
        // TODO: the log two part can probably optimized crazily: http://stackoverflow.com/a/11398748
        //|----------- rounding up int div -----------|
        (((size + self.base_size - 1) / self.base_size).next_power_of_two() as f32).log2() as usize
    }

    pub fn sized_for_mut(&mut self, size: usize) -> &mut B {
        let index = self.size_to_index(size);

        while *self.largest_size <= index {
            self.push_higher_sized_collection();
            *self.largest_size += 1;
        }

        &mut self.collections[index]
    }
}