use libc::{c_void, free, malloc, memcmp, realloc};
use std::fs;
use std::path::Path;
use std::ptr;
use std::slice;

#[derive(Debug)]
pub struct StdBuf {
    storage: Storage,
}

#[derive(Debug)]
enum Storage {
    Empty,
    Owned(OwnedBuffer),
    Borrowed(BorrowedBuffer),
}

#[derive(Debug)]
struct OwnedBuffer {
    ptr: *mut u8,
    len: usize,
}

#[derive(Debug, Clone, Copy)]
struct BorrowedBuffer {
    ptr: *const u8,
    len: usize,
}

impl StdBuf {
    pub fn new() -> Self {
        Self {
            storage: Storage::Empty,
        }
    }

    pub fn with_slice(data: &[u8], copy: bool) -> Self {
        if copy {
            let mut buf = StdBuf::new();
            buf.copy_from_slice(data);
            buf
        } else {
            StdBuf {
                storage: Storage::Borrowed(BorrowedBuffer::new(data.as_ptr(), data.len())),
            }
        }
    }

    pub fn make_ref(ptr: *const u8, len: usize) -> Self {
        StdBuf {
            storage: Storage::Borrowed(BorrowedBuffer::new(ptr, len)),
        }
    }

    /// Creates a buffer that owns a freshly allocated region sized `len` bytes.
    pub fn take_from_pointer(ptr: *mut u8, len: usize) -> Self {
        if ptr.is_null() {
            StdBuf::new()
        } else {
            StdBuf {
                storage: Storage::Owned(OwnedBuffer { ptr, len }),
            }
        }
    }

    /// Equivalent to C++ `StdBuf::New`.
    pub fn new_buffer(&mut self, len: usize) {
        self.clear();
        if len == 0 {
            return;
        }
        self.storage = Storage::Owned(OwnedBuffer::allocate(len));
    }

    pub fn take_pointer(ptr: *mut u8, len: usize) -> Self {
        if ptr.is_null() {
            return StdBuf::new();
        }
        StdBuf {
            storage: Storage::Owned(OwnedBuffer { ptr, len }),
        }
    }

    pub fn is_null(&self) -> bool {
        matches!(self.storage, Storage::Empty) || self.is_empty()
    }

    pub fn is_ref(&self) -> bool {
        matches!(self.storage, Storage::Borrowed(_))
    }

    pub fn len(&self) -> usize {
        match &self.storage {
            Storage::Empty => 0,
            Storage::Owned(o) => o.len,
            Storage::Borrowed(b) => b.len,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn data(&self) -> &[u8] {
        match &self.storage {
            Storage::Empty => &[],
            Storage::Owned(o) => unsafe { o.as_slice() },
            Storage::Borrowed(b) => unsafe { b.as_slice() },
        }
    }

    pub fn data_mut(&mut self) -> &mut [u8] {
        self.ensure_owned();
        match &mut self.storage {
            Storage::Owned(o) => unsafe { o.as_mut_slice() },
            Storage::Empty | Storage::Borrowed(_) => unreachable!(),
        }
    }

    pub fn clear(&mut self) {
        self.storage.clear();
    }

    pub fn ref_from(&mut self, other: &StdBuf) {
        self.clear();
        self.storage = Storage::Borrowed(other.borrowed_view());
    }

    pub fn ref_from_ptr(&mut self, ptr: *const u8, len: usize) {
        self.clear();
        self.storage = Storage::Borrowed(BorrowedBuffer::new(ptr, len));
    }

    pub fn ref_buf(&mut self, other: &StdBuf) {
        let view = other.borrowed_view();
        self.clear();
        self.storage = Storage::Borrowed(view);
    }

    pub fn take_from_ptr(&mut self, ptr: *mut u8, len: usize) {
        self.clear();
        if !ptr.is_null() {
            self.storage = Storage::Owned(OwnedBuffer { ptr, len });
        }
    }

    pub fn take_from(&mut self, other: &mut StdBuf) {
        let ptr = other.grab_pointer();
        let len = other.len();
        self.take_from_ptr(ptr, len);
    }

    pub fn take(&mut self, other: &mut StdBuf) {
        self.take_from(other);
    }

    pub fn ref_data(&mut self, data: &[u8]) {
        self.ref_from_ptr(data.as_ptr(), data.len());
    }

    pub fn copy_from_buf(&mut self, other: &StdBuf) {
        self.copy_from_slice(other.data());
    }

    pub fn copy_from_slice(&mut self, data: &[u8]) {
        if data.is_empty() {
            self.clear();
            return;
        }
        self.ensure_owned_with_len(data.len());
        unsafe {
            ptr::copy_nonoverlapping(data.as_ptr(), self.as_mut_ptr(), data.len());
        }
    }

    pub fn duplicate(&self) -> StdBuf {
        let mut dup = StdBuf::new();
        dup.copy_from_slice(self.data());
        dup
    }

    pub fn get_ref(&self) -> StdBuf {
        StdBuf {
            storage: Storage::Borrowed(self.borrowed_view()),
        }
    }

    pub fn write(&mut self, data: &[u8], offset: usize) {
        assert!(offset + data.len() <= self.len());
        if data.is_empty() {
            return;
        }
        self.ensure_owned();
        unsafe {
            ptr::copy_nonoverlapping(data.as_ptr(), self.as_mut_ptr().add(offset), data.len());
        }
    }

    pub fn move_within(&mut self, from: usize, len: usize, to: usize) {
        assert!(from + len <= self.len());
        assert!(to + len <= self.len());
        if len == 0 {
            return;
        }
        self.ensure_owned();
        unsafe {
            ptr::copy(self.as_ptr().add(from), self.as_mut_ptr().add(to), len);
        }
    }

    pub fn compare_raw(&self, data: &[u8], offset: usize) -> i32 {
        assert!(offset + data.len() <= self.len());
        unsafe {
            memcmp(
                self.as_ptr().add(offset) as *const c_void,
                data.as_ptr() as *const c_void,
                data.len(),
            ) as i32
        }
    }

    pub fn compare(&self, data: &[u8], offset: usize) -> std::cmp::Ordering {
        assert!(offset + data.len() <= self.len());
        self.data()[offset..offset + data.len()].cmp(data)
    }

    pub fn append_slice(&mut self, data: &[u8]) {
        if data.is_empty() {
            return;
        }
        let old_len = self.len();
        self.grow(data.len());
        self.write(data, old_len);
    }

    pub fn append_buf(&mut self, other: &StdBuf) {
        self.append_slice(other.data());
    }

    pub fn grow(&mut self, amount: usize) {
        if amount == 0 {
            return;
        }
        if self.is_ref() {
            let new_len = self.len() + amount;
            let existing = self.data().to_vec();
            self.ensure_owned_with_len(new_len);
            unsafe {
                ptr::copy_nonoverlapping(existing.as_ptr(), self.as_mut_ptr(), existing.len());
            }
            return;
        }
        self.ensure_owned_with_len(self.len() + amount);
    }

    pub fn shrink(&mut self, amount: usize) {
        assert!(amount <= self.len());
        if amount == 0 {
            return;
        }
        if self.is_ref() {
            let new_len = self.len() - amount;
            let existing = self.data()[..new_len].to_vec();
            self.ensure_owned_with_len(new_len);
            unsafe {
                ptr::copy_nonoverlapping(existing.as_ptr(), self.as_mut_ptr(), existing.len());
            }
            return;
        }
        self.resize_owned(self.len() - amount);
    }

    pub fn set_size(&mut self, new_len: usize) {
        let current = self.len();
        if new_len > current {
            self.grow(new_len - current);
        } else {
            self.shrink(current - new_len);
        }
    }

    pub fn copy_resized(&mut self, new_len: usize) {
        let existing = self.data().to_vec();
        self.new_buffer(new_len);
        let to_copy = existing.len().min(new_len);
        if to_copy > 0 {
            self.write(&existing[..to_copy], 0);
        }
    }

    pub fn copy(&mut self) {
        let len = self.len();
        self.copy_resized(len);
    }

    pub fn copy_from_ptr(&mut self, ptr: *const u8, len: usize) {
        self.ref_from_ptr(ptr, len);
        self.copy();
    }

    pub fn get_part(&self, start: usize, len: usize) -> StdBuf {
        assert!(start + len <= self.len());
        if len == 0 {
            return StdBuf::new();
        }
        let slice = &self.data()[start..start + len];
        StdBuf::with_slice(slice, true)
    }

    pub fn append(&mut self, other: &StdBuf) {
        self.append_buf(other);
    }

    pub fn load_from_file<P: AsRef<Path>>(&mut self, path: P) -> bool {
        match fs::read(path) {
            Ok(data) => {
                self.copy_from_slice(&data);
                true
            }
            Err(_) => false,
        }
    }

    pub fn save_to_file<P: AsRef<Path>>(&self, path: P) -> bool {
        fs::write(path, self.data()).is_ok()
    }

    pub fn grab_pointer(&mut self) -> *mut u8 {
        self.ensure_owned();
        let len = self.len();
        let mut ptr = ptr::null_mut();
        if let Storage::Owned(owned) = &mut self.storage {
            ptr = owned.ptr;
            owned.ptr = ptr::null_mut();
            owned.len = 0;
        }
        if !ptr.is_null() {
            self.storage = Storage::Borrowed(BorrowedBuffer::new(ptr as *const u8, len));
        }
        ptr
    }

    pub fn delete_pointer(ptr: *mut u8) {
        if !ptr.is_null() {
            unsafe { free(ptr as *mut c_void) };
        }
    }

    pub fn take_or_ref(other: &mut StdBuf) -> StdBuf {
        if other.is_ref() {
            other.get_ref()
        } else {
            let ptr = other.grab_pointer();
            StdBuf::take_pointer(ptr, other.len())
        }
    }

    fn ensure_owned(&mut self) {
        if matches!(self.storage, Storage::Owned(_)) {
            return;
        }
        let data = self.data().to_vec();
        self.ensure_owned_with_len(data.len());
        unsafe {
            ptr::copy_nonoverlapping(data.as_ptr(), self.as_mut_ptr(), data.len());
        }
    }

    fn ensure_owned_with_len(&mut self, len: usize) {
        match &mut self.storage {
            Storage::Owned(o) => unsafe {
                o.resize(len);
            },
            _ => {
                self.storage = Storage::Owned(OwnedBuffer::allocate(len));
            }
        }
    }

    fn resize_owned(&mut self, len: usize) {
        if let Storage::Owned(o) = &mut self.storage {
            unsafe { o.resize(len) };
        } else {
            self.ensure_owned_with_len(len);
        }
    }

    fn as_ptr(&self) -> *const u8 {
        match &self.storage {
            Storage::Empty => ptr::null(),
            Storage::Owned(o) => o.ptr as *const u8,
            Storage::Borrowed(b) => b.ptr,
        }
    }

    fn as_mut_ptr(&mut self) -> *mut u8 {
        match &mut self.storage {
            Storage::Owned(o) => o.ptr,
            _ => ptr::null_mut(),
        }
    }

    fn borrowed_view(&self) -> BorrowedBuffer {
        match &self.storage {
            Storage::Empty => BorrowedBuffer::new(ptr::null(), 0),
            Storage::Owned(o) => BorrowedBuffer::new(o.ptr as *const u8, o.len),
            Storage::Borrowed(b) => *b,
        }
    }
}

impl Storage {
    fn clear(&mut self) {
        if let Storage::Owned(owned) = self {
            unsafe {
                owned.free();
            }
        }
        *self = Storage::Empty;
    }
}

impl OwnedBuffer {
    fn allocate(len: usize) -> Self {
        if len == 0 {
            return Self {
                ptr: ptr::null_mut(),
                len: 0,
            };
        }
        let ptr = unsafe { malloc(len) as *mut u8 };
        if ptr.is_null() {
            panic!("StdBuf allocation failed for {} bytes", len);
        }
        Self { ptr, len }
    }

    unsafe fn as_slice(&self) -> &[u8] {
        if self.ptr.is_null() {
            &[]
        } else {
            slice::from_raw_parts(self.ptr as *const u8, self.len)
        }
    }

    unsafe fn as_mut_slice(&mut self) -> &mut [u8] {
        if self.ptr.is_null() {
            &mut []
        } else {
            slice::from_raw_parts_mut(self.ptr, self.len)
        }
    }

    unsafe fn resize(&mut self, len: usize) {
        if len == 0 {
            if !self.ptr.is_null() {
                free(self.ptr as *mut c_void);
            }
            self.ptr = ptr::null_mut();
            self.len = 0;
            return;
        }
        let ptr = if self.ptr.is_null() {
            malloc(len)
        } else {
            realloc(self.ptr as *mut c_void, len)
        } as *mut u8;
        if ptr.is_null() {
            panic!("StdBuf realloc failed for {} bytes", len);
        }
        self.ptr = ptr;
        self.len = len;
    }

    unsafe fn free(&mut self) {
        if !self.ptr.is_null() {
            free(self.ptr as *mut c_void);
        }
        self.ptr = ptr::null_mut();
        self.len = 0;
    }
}

impl BorrowedBuffer {
    fn new(ptr: *const u8, len: usize) -> Self {
        Self { ptr, len }
    }

    unsafe fn as_slice(&self) -> &[u8] {
        if self.ptr.is_null() {
            &[]
        } else {
            slice::from_raw_parts(self.ptr, self.len)
        }
    }
}

impl Drop for Storage {
    fn drop(&mut self) {
        if let Storage::Owned(owned) = self {
            unsafe { owned.free() };
        }
    }
}

pub struct StdStrBuf {
    inner: StdBuf,
}

const WINDOWS_1252_EXTRA: [&str; 32] = [
    "€", "?", "‚", "ƒ", "„", "…", "†", "‡", "ˆ", "‰", "Š", "‹", "Œ", "?", "Ž", "?", "?", "‘", "’",
    "“", "”", "•", "–", "—", "˜", "™", "š", "›", "œ", "?", "ž", "Ÿ",
];

impl StdStrBuf {
    pub fn new() -> Self {
        Self {
            inner: StdBuf::new(),
        }
    }

    pub fn from_str(data: &str, copy: bool) -> Self {
        let mut buf = StdStrBuf::new();
        if copy {
            buf.copy(data);
        } else {
            buf.inner = StdBuf::with_slice(data.as_bytes(), false);
        }
        buf
    }

    pub fn is_null(&self) -> bool {
        self.inner.is_null()
    }

    pub fn is_ref(&self) -> bool {
        self.inner.is_ref()
    }

    pub fn len(&self) -> usize {
        self.trimmed_bytes().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn size(&self) -> usize {
        self.inner.len()
    }

    pub fn as_bytes(&self) -> &[u8] {
        self.trimmed_bytes()
    }

    pub fn as_bytes_with_nul(&self) -> &[u8] {
        self.inner.data()
    }

    pub fn copy(&mut self, data: &str) {
        self.copy_bytes(data.as_bytes());
    }

    pub fn copy_bytes(&mut self, data: &[u8]) {
        if data.is_empty() {
            self.inner.clear();
            return;
        }
        let mut owned = Vec::with_capacity(data.len() + 1);
        owned.extend_from_slice(data);
        owned.push(0);
        self.inner.copy_from_slice(&owned);
    }

    pub fn set_length(&mut self, length: usize) {
        if length == self.len() && !self.is_null() {
            return;
        }
        self.ensure_length(length);
    }

    pub fn append(&mut self, data: &str) {
        self.append_bytes(data.as_bytes());
    }

    pub fn append_bytes(&mut self, data: &[u8]) {
        if data.is_empty() {
            return;
        }
        let current_len = self.len();
        self.ensure_length(current_len + data.len());
        let total_size = self.inner.len();
        let slice = self.inner.data_mut();
        slice[current_len..current_len + data.len()].copy_from_slice(data);
        slice[total_size - 1] = 0;
    }

    pub fn append_chars(&mut self, ch: u8, count: usize) {
        if count == 0 {
            return;
        }
        let fill = vec![ch; count];
        self.append_bytes(&fill);
    }

    pub fn append_char(&mut self, ch: u8) {
        self.append_chars(ch, 1);
    }

    pub fn insert_char(&mut self, ch: u8, insert_before: usize) {
        assert!(insert_before <= self.len());
        self.ensure_length(self.len() + 1);
        let end = self.len();
        let total_size = self.inner.len();
        let slice = self.inner.data_mut();
        slice.copy_within(insert_before..end, insert_before + 1);
        slice[insert_before] = ch;
        slice[total_size - 1] = 0;
    }

    pub fn replace_end(&mut self, position: usize, new_end: &str) {
        let length = self.len();
        assert!(position <= length);
        if position > length {
            return;
        }
        if length - position != new_end.len() {
            self.set_length(position + new_end.len());
        }
        if new_end.is_empty() {
            return;
        }
        let total_size = self.inner.len();
        let slice = self.inner.data_mut();
        slice[position..position + new_end.len()].copy_from_slice(new_end.as_bytes());
        slice[total_size - 1] = 0;
    }

    pub fn replace(&mut self, old: &str, new: &str, start: usize) -> usize {
        self.replace_bytes(old.as_bytes(), new.as_bytes(), start)
    }

    pub fn replace_bytes(&mut self, old: &[u8], new: &[u8], start: usize) -> usize {
        if old.is_empty() || start > self.len() {
            return 0;
        }
        let mut bytes = self.as_bytes().to_vec();
        if old.len() > bytes.len() {
            return 0;
        }
        let mut cursor = start;
        let mut replaced = 0;
        while cursor <= bytes.len() {
            if let Some(pos) = Self::find_subslice(&bytes[cursor..], old) {
                let absolute = cursor + pos;
                bytes.splice(absolute..absolute + old.len(), new.iter().copied());
                cursor = absolute + new.len();
                replaced += 1;
            } else {
                break;
            }
        }
        if replaced > 0 {
            self.copy_bytes(&bytes);
        }
        replaced
    }

    pub fn replace_char(&mut self, old: u8, mut new: u8, _start: usize) -> usize {
        if old == 0 {
            return 0;
        }
        if new == 0 {
            new = b'_';
        }
        let mut bytes = self.as_bytes().to_vec();
        let mut count = 0;
        for b in &mut bytes {
            if *b == old {
                *b = new;
                count += 1;
            }
        }
        if count > 0 {
            self.copy_bytes(&bytes);
        }
        count
    }

    pub fn validate_chars(&self, initial: &str, mid: &str) -> bool {
        let bytes = self.as_bytes();
        for (idx, ch) in bytes.iter().enumerate() {
            let allowed = if idx == 0 {
                initial.as_bytes()
            } else {
                mid.as_bytes()
            };
            if !allowed.contains(ch) {
                return false;
            }
        }
        true
    }

    pub fn escape_string(&mut self) {
        self.replace("\\", "\\\\", 0);
        self.replace("\"", "\\\"", 0);
    }

    pub fn append_until(&mut self, input: &str, until: u8) {
        match input.as_bytes().iter().position(|&b| b == until) {
            Some(pos) => self.append_bytes(&input.as_bytes()[..pos]),
            None => self.append(input),
        }
    }

    pub fn copy_until(&mut self, input: &str, until: u8) {
        self.clear();
        self.append_until(input, until);
    }

    pub fn split_at_char(&mut self, split: u8, out: &mut StdStrBuf) -> bool {
        if self.is_null() {
            return false;
        }
        let bytes = self.as_bytes();
        if let Some(pos) = bytes.iter().position(|&b| b == split) {
            let tail_start = pos + 1;
            if tail_start <= bytes.len() {
                out.copy_bytes(&bytes[tail_start..]);
            }
            self.set_length(pos);
            true
        } else {
            false
        }
    }

    pub fn copy_part(&self, start: usize, size: usize) -> StdStrBuf {
        assert!(start + size <= self.inner.len());
        if size == 0 {
            return StdStrBuf::new();
        }
        let mut result = StdStrBuf::new();
        result.copy_bytes(&self.as_bytes()[start..start + size]);
        result
    }

    pub fn get_section(&self, mut index: usize, separator: u8, out: &mut StdStrBuf) -> bool {
        out.clear();
        let data = self.as_bytes();
        if data.is_empty() {
            return false;
        }
        let mut start = 0;
        loop {
            let sep_pos = data[start..]
                .iter()
                .position(|&b| b == separator)
                .map(|pos| start + pos);
            if index == 0 {
                let end = sep_pos.unwrap_or(data.len());
                if end > start {
                    out.copy_bytes(&data[start..end]);
                }
                return true;
            }
            match sep_pos {
                Some(pos) => {
                    start = pos + 1;
                    if start > data.len() {
                        return false;
                    }
                    index -= 1;
                }
                None => return false,
            }
        }
    }

    pub fn ensure_unicode(&mut self) {
        let data = self.as_bytes();
        if std::str::from_utf8(data).is_ok() {
            return;
        }

        let mut converted = Vec::with_capacity(data.len());
        for &byte in data {
            if byte < 0x80 {
                converted.push(byte);
            } else if byte >= 0xA0 {
                converted.push(0xC0 | (byte >> 6));
                converted.push(0x80 | (byte & 0x3F));
            } else {
                let replacement = WINDOWS_1252_EXTRA[(byte - 0x80) as usize].as_bytes();
                converted.extend_from_slice(replacement);
            }
        }
        self.copy_bytes(&converted);
    }

    pub fn trim_spaces(&mut self) -> bool {
        let bytes = self.as_bytes().to_vec();
        if bytes.is_empty() {
            return false;
        }
        let mut left = 0;
        while left < bytes.len() && bytes[left].is_ascii_whitespace() {
            left += 1;
        }
        if left == bytes.len() {
            self.clear();
            return true;
        }
        let mut right = 0;
        while right < bytes.len() && bytes[bytes.len() - 1 - right].is_ascii_whitespace() {
            right += 1;
        }
        if left == 0 && right == 0 {
            return false;
        }
        if left == 0 {
            self.set_length(bytes.len() - right);
            return true;
        }
        let trimmed = &bytes[left..bytes.len() - right];
        self.copy_bytes(trimmed);
        true
    }

    pub fn get_ref(&self) -> StdStrBuf {
        StdStrBuf {
            inner: self.inner.get_ref(),
        }
    }

    pub fn ref_from(&mut self, other: &StdStrBuf) {
        self.inner.ref_buf(&other.inner);
    }

    pub fn take(&mut self, other: &mut StdStrBuf) {
        self.inner.take(&mut other.inner);
    }

    pub fn clear(&mut self) {
        self.inner.clear();
    }

    pub fn clone_inner(&self) -> StdBuf {
        self.inner.duplicate()
    }

    pub fn load_from_file<P: AsRef<Path>>(&mut self, path: P) -> bool {
        match fs::read(path) {
            Ok(data) => {
                self.copy_bytes(&data);
                true
            }
            Err(_) => false,
        }
    }

    pub fn save_to_file<P: AsRef<Path>>(&self, path: P) -> bool {
        fs::write(path, self.as_bytes()).is_ok()
    }

    fn trimmed_bytes(&self) -> &[u8] {
        let data = self.inner.data();
        if data.is_empty() {
            &[]
        } else if let Some((&0, rest)) = data.split_last() {
            rest
        } else {
            data
        }
    }

    fn ensure_length(&mut self, length: usize) {
        let target_size = if length == 0 { 0 } else { length + 1 };
        let current_size = self.inner.len();
        if target_size == current_size {
            if target_size > 0 {
                let slice = self.inner.data_mut();
                slice[target_size - 1] = 0;
            }
            return;
        }
        if target_size > current_size {
            let grow_by = target_size - current_size;
            self.inner.grow(grow_by);
        } else {
            self.inner.shrink(current_size - target_size);
        }
        if target_size > 0 {
            let slice = self.inner.data_mut();
            slice[target_size - 1] = 0;
        }
    }

    fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
        if needle.is_empty() || needle.len() > haystack.len() {
            return None;
        }
        haystack
            .windows(needle.len())
            .position(|window| window == needle)
    }
}

impl Default for StdBuf {
    fn default() -> Self {
        Self::new()
    }
}

impl Default for StdStrBuf {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn copy_and_modify() {
        let mut buf = StdBuf::with_slice(&[1, 2, 3], true);
        assert_eq!(buf.data(), &[1, 2, 3]);
        buf.write(&[4, 5], 1);
        assert_eq!(buf.data(), &[1, 4, 5]);
    }

    #[test]
    fn borrow_and_copy_on_write() {
        let data = [10, 20, 30, 40];
        let mut buf = StdBuf::with_slice(&data, false);
        assert!(buf.is_ref());
        buf.grow(2);
        assert!(!buf.is_ref());
        assert_eq!(&buf.data()[..4], &data);
    }

    #[test]
    fn grab_pointer_transfers_ownership() {
        let mut buf = StdBuf::with_slice(&[1, 2, 3, 4], true);
        let ptr = buf.grab_pointer();
        assert!(buf.is_ref());
        unsafe {
            assert_eq!(*ptr, 1);
        }
        StdBuf::delete_pointer(ptr);
    }

    #[test]
    fn std_str_buf_append() {
        let mut s = StdStrBuf::from_str("Hello", true);
        s.append(" World");
        assert_eq!(s.as_bytes(), b"Hello World");
        assert_eq!(s.as_bytes_with_nul().last(), Some(&0));
    }

    #[test]
    fn std_str_buf_set_length_shrink() {
        let mut s = StdStrBuf::from_str("Testing", true);
        s.set_length(4);
        assert_eq!(s.as_bytes(), b"Test");
        assert_eq!(s.len(), 4);
    }

    #[test]
    fn std_str_buf_replace_substring() {
        let mut s = StdStrBuf::from_str("foofoo", true);
        let replaced = s.replace("foo", "bar", 0);
        assert_eq!(replaced, 2);
        assert_eq!(s.as_bytes(), b"barbar");
    }

    #[test]
    fn std_str_buf_replace_char() {
        let mut s = StdStrBuf::from_str("banana", true);
        let replaced = s.replace_char(b'a', b'o', 1);
        assert_eq!(replaced, 3);
        assert_eq!(s.as_bytes(), b"bonono");
    }

    #[test]
    fn std_str_buf_trim_spaces() {
        let mut s = StdStrBuf::from_str("   spaced string   ", true);
        assert!(s.trim_spaces());
        assert_eq!(s.as_bytes(), b"spaced string");
    }

    #[test]
    fn std_str_buf_replace_end() {
        let mut s = StdStrBuf::from_str("texture.bmp", true);
        s.replace_end(8, "png");
        assert_eq!(s.as_bytes(), b"texture.png");
    }

    #[test]
    fn std_str_buf_validate_chars() {
        let s = StdStrBuf::from_str("name123", true);
        assert!(s.validate_chars(
            "abcdefghijklmnopqrstuvwxyz",
            "abcdefghijklmnopqrstuvwxyz0123456789"
        ));
        assert!(!s.validate_chars("abc", "abc"));
    }

    #[test]
    fn std_str_buf_get_section() {
        let s = StdStrBuf::from_str("alpha;beta;;delta", true);
        let mut out = StdStrBuf::new();
        assert!(s.get_section(1, b';', &mut out));
        assert_eq!(out.as_bytes(), b"beta");
        assert!(s.get_section(2, b';', &mut out));
        assert_eq!(out.len(), 0);
        assert!(!s.get_section(5, b';', &mut out));
    }

    #[test]
    fn std_str_buf_ensure_unicode() {
        let mut s = StdStrBuf::new();
        s.copy_bytes(&[0x41, 0x80]);
        s.ensure_unicode();
        assert_eq!(s.as_bytes(), "A€".as_bytes());
    }

    #[test]
    fn std_str_buf_append_until_and_copy_part() {
        let mut s = StdStrBuf::from_str("foo", true);
        s.append_until("bar;baz", b';');
        assert_eq!(s.as_bytes(), b"foobar");
        s.copy_until("qux;quux", b';');
        assert_eq!(s.as_bytes(), b"qux");
        let part = s.copy_part(1, 2);
        assert_eq!(part.as_bytes(), b"ux");
    }

    #[test]
    fn std_str_buf_split_at_char() {
        let mut s = StdStrBuf::from_str("key=value", true);
        let mut tail = StdStrBuf::new();
        assert!(s.split_at_char(b'=', &mut tail));
        assert_eq!(s.as_bytes(), b"key");
        assert_eq!(tail.as_bytes(), b"value");
        assert!(!s.split_at_char(b'=', &mut tail));
    }

    #[test]
    fn std_str_buf_escape_string() {
        let mut s = StdStrBuf::from_str("He said \"hi\" \\ path", true);
        s.escape_string();
        assert_eq!(s.as_bytes(), b"He said \\\"hi\\\" \\\\ path");
    }

    #[test]
    fn std_buf_file_roundtrip() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("buf.bin");
        let buf = StdBuf::with_slice(&[1u8, 2, 3, 4, 5], true);
        assert!(buf.save_to_file(&path));

        let mut loaded = StdBuf::new();
        assert!(loaded.load_from_file(&path));
        assert_eq!(loaded.data(), &[1, 2, 3, 4, 5]);
    }

    #[test]
    fn std_str_buf_file_roundtrip() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("string.txt");
        let s = StdStrBuf::from_str("Legacy", true);
        assert!(s.save_to_file(&path));

        let mut loaded = StdStrBuf::new();
        assert!(loaded.load_from_file(&path));
        assert_eq!(loaded.as_bytes(), b"Legacy");
    }
}
