use crate::color::Color;
use crate::surface::{PixelFormat, Point, Rect, Surface};
use std::os::raw::{c_int, c_uchar};
use std::ptr;

pub struct SurfaceHandle(Surface);

#[repr(C)]
pub struct LcColor {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl From<LcColor> for Color {
    fn from(value: LcColor) -> Self {
        Color::new(value.r, value.g, value.b, value.a)
    }
}

#[repr(C)]
pub struct LcSurfaceSnapshot {
    pub width: u32,
    pub height: u32,
    pub checksum: u32,
}

#[no_mangle]
pub extern "C" fn lc_surface_create_rgba(width: u32, height: u32) -> *mut SurfaceHandle {
    let surface = Surface::new(width, height, PixelFormat::Rgba8888);
    Box::into_raw(Box::new(SurfaceHandle(surface)))
}

#[no_mangle]
pub extern "C" fn lc_surface_from_rgba(
    pixels: *const c_uchar,
    len: usize,
    width: u32,
    height: u32,
) -> *mut SurfaceHandle {
    if pixels.is_null() {
        return ptr::null_mut();
    }

    let slice = unsafe { std::slice::from_raw_parts(pixels, len) };
    match Surface::from_bytes(width, height, PixelFormat::Rgba8888, slice.to_vec()) {
        Ok(surface) => Box::into_raw(Box::new(SurfaceHandle(surface))),
        Err(_) => ptr::null_mut(),
    }
}

#[no_mangle]
pub extern "C" fn lc_surface_free(surface: *mut SurfaceHandle) {
    if surface.is_null() {
        return;
    }
    unsafe {
        drop(Box::from_raw(surface));
    }
}

#[no_mangle]
pub extern "C" fn lc_surface_fill(surface: *mut SurfaceHandle, color: LcColor) -> bool {
    if surface.is_null() {
        return false;
    }
    let handle = unsafe { &mut *surface };
    handle.0.fill(color.into());
    true
}

#[no_mangle]
pub extern "C" fn lc_surface_set_pixel(
    surface: *mut SurfaceHandle,
    x: u32,
    y: u32,
    color: LcColor,
) -> bool {
    if surface.is_null() {
        return false;
    }
    let handle = unsafe { &mut *surface };
    handle.0.set_pixel(x, y, color.into()).is_ok()
}

#[no_mangle]
pub extern "C" fn lc_surface_blit(
    dest: *mut SurfaceHandle,
    src: *const SurfaceHandle,
    dest_x: c_int,
    dest_y: c_int,
) -> bool {
    if dest.is_null() || src.is_null() {
        return false;
    }
    let dest_handle = unsafe { &mut *dest };
    let src_handle = unsafe { &*src };
    dest_handle
        .0
        .blit(&src_handle.0, Point::new(dest_x as i32, dest_y as i32))
        .is_ok()
}

#[no_mangle]
pub extern "C" fn lc_surface_blit_region(
    dest: *mut SurfaceHandle,
    src: *const SurfaceHandle,
    src_x: c_int,
    src_y: c_int,
    width: u32,
    height: u32,
    dest_x: c_int,
    dest_y: c_int,
) -> bool {
    if dest.is_null() || src.is_null() {
        return false;
    }
    let dest_handle = unsafe { &mut *dest };
    let src_handle = unsafe { &*src };
    dest_handle
        .0
        .blit_region(
            &src_handle.0,
            Rect::new(src_x as i32, src_y as i32, width, height),
            Point::new(dest_x as i32, dest_y as i32),
        )
        .is_ok()
}

#[no_mangle]
pub extern "C" fn lc_surface_snapshot(surface: *const SurfaceHandle) -> LcSurfaceSnapshot {
    if surface.is_null() {
        return LcSurfaceSnapshot {
            width: 0,
            height: 0,
            checksum: 0,
        };
    }
    let handle = unsafe { &*surface };
    let snapshot = handle.0.snapshot();
    LcSurfaceSnapshot {
        width: snapshot.width(),
        height: snapshot.height(),
        checksum: snapshot.checksum(),
    }
}

#[no_mangle]
pub extern "C" fn lc_surface_copy_rgba(
    surface: *const SurfaceHandle,
    out_len: *mut usize,
) -> *mut c_uchar {
    if surface.is_null() || out_len.is_null() {
        return ptr::null_mut();
    }

    let handle = unsafe { &*surface };
    let bytes = handle.0.pixels().to_vec();
    let mut boxed = bytes.into_boxed_slice();
    let ptr = boxed.as_mut_ptr();
    let len = boxed.len();
    unsafe {
        *out_len = len;
    }
    std::mem::forget(boxed);
    ptr
}

#[no_mangle]
pub extern "C" fn lc_surface_buffer_free(buffer: *mut c_uchar, len: usize) {
    if buffer.is_null() {
        return;
    }
    unsafe {
        drop(Box::from_raw(std::slice::from_raw_parts_mut(buffer, len)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ffi_surface_blit_and_snapshot() {
        unsafe {
            let dest = lc_surface_create_rgba(2, 2);
            let src = lc_surface_create_rgba(2, 2);
            assert!(!dest.is_null());
            assert!(!src.is_null());

            let red = LcColor {
                r: 255,
                g: 0,
                b: 0,
                a: 128,
            };
            let black = LcColor {
                r: 0,
                g: 0,
                b: 0,
                a: 255,
            };
            assert!(lc_surface_fill(dest, black));
            assert!(lc_surface_fill(src, red));
            assert!(lc_surface_blit(dest, src, 0, 0));

            let snapshot = lc_surface_snapshot(dest);
            assert_eq!(snapshot.width, 2);
            assert_eq!(snapshot.height, 2);
            assert_ne!(snapshot.checksum, 0);

            let mut len: usize = 0;
            let data = lc_surface_copy_rgba(dest, &mut len as *mut usize);
            assert!(!data.is_null());
            assert_eq!(len, 16);
            let slice = std::slice::from_raw_parts(data as *const u8, len);
            assert_eq!(slice[0], 128);
            lc_surface_buffer_free(data, len);

            lc_surface_free(src);
            lc_surface_free(dest);
        }
    }
}
