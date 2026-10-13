use std::rc::Rc;

use color_eyre::Result;
use image::DynamicImage;
use log::warn;

use crate::{gl_check, render::gl};

use super::load_texture;

pub struct Wallpaper {
    gl: Rc<gl::Gl>,
    texture: gl::types::GLuint,
    image_width: u32,
    image_height: u32,
}

impl Wallpaper {
    pub fn new(gl: Rc<gl::Gl>, image: DynamicImage, current: bool) -> Result<Self> {
        let image_width = image.width();
        let image_height = image.height();
        let mut texture = 0;
        unsafe {
            gl.GenTextures(1, &mut texture);
        }
        // Own the allocation before any fallible operation, including activation and upload.
        // The caller has made this texture's EGL context current; Drop releases it on error.
        let wallpaper = Self {
            gl,
            texture,
            image_width,
            image_height,
        };
        let gl = &wallpaper.gl;
        unsafe {
            gl_check!(gl, "Failed to generate the texture");
            gl.ActiveTexture(if current { gl::TEXTURE1 } else { gl::TEXTURE0 });
            gl_check!(
                gl,
                format!(
                    "Failed to activate the texture TEXTURE{}",
                    if current { 1 } else { 0 }
                )
            );
            gl.BindTexture(gl::TEXTURE_2D, texture);
            gl_check!(
                gl,
                format!(
                    "Failed to bind the texture TEXTURE{}",
                    if current { 1 } else { 0 }
                )
            );
        }
        load_texture(gl, image)?;

        Ok(wallpaper)
    }

    pub fn bind(&self) -> Result<()> {
        unsafe {
            self.gl.BindTexture(gl::TEXTURE_2D, self.texture);
            gl_check!(self.gl, "Failed to bind the texture");
        }

        Ok(())
    }

    pub fn get_image_height(&self) -> u32 {
        self.image_height
    }

    pub fn get_image_width(&self) -> u32 {
        self.image_width
    }

    pub fn load_image(&mut self, image: DynamicImage, current: bool) -> Result<()> {
        self.image_width = image.width();
        self.image_height = image.height();

        unsafe {
            self.gl
                .ActiveTexture(if current { gl::TEXTURE1 } else { gl::TEXTURE0 });
            gl_check!(
                self.gl,
                format!(
                    "Failed to activate the texture TEXTURE{}",
                    if current { 1 } else { 0 }
                )
            );
            self.bind()?;
        }
        load_texture(&self.gl, image)
    }

    /// The owner must have bound this texture's context before releasing it.
    pub fn release(&mut self) {
        if self.texture == 0 {
            return;
        }
        let texture = std::mem::replace(&mut self.texture, 0);
        unsafe {
            self.gl.DeleteTextures(1, &texture);
        }
        let check_err = || -> Result<()> {
            unsafe {
                gl_check!(self.gl, "Failed to delete the previous texture");
            }
            Ok(())
        };
        if let Err(err) = check_err() {
            warn!("{:?}", err.wrap_err("Failed to drop the wallpaper"));
        }
    }
    /// EGL context destruction reclaims the GPU allocation; Rust data still drops normally.
    pub fn disarm(&mut self) {
        self.texture = 0;
    }
    #[cfg(test)]
    pub(super) fn test_texture(gl: Rc<gl::Gl>, texture: u32) -> Self {
        Self {
            gl,
            texture,
            image_width: 1,
            image_height: 1,
        }
    }
}

impl Drop for Wallpaper {
    fn drop(&mut self) {
        self.release();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{cell::RefCell, collections::BTreeSet, ffi::c_void};

    #[derive(Default)]
    struct Driver {
        next: u32,
        live: BTreeSet<u32>,
        deleted: Vec<u32>,
        failure: &'static str,
        error: u32,
        context: char,
        string_calls: usize,
    }
    thread_local! {
        static DRIVER: RefCell<Driver> = RefCell::new(Driver::default());
    }
    unsafe extern "system" fn generate(n: i32, textures: *mut u32) {
        assert_eq!(n, 1);
        DRIVER.with(|driver| {
            let mut d = driver.borrow_mut();
            assert_eq!(d.context, 'A');
            d.next += 1;
            let id = d.next;
            d.live.insert(id);
            unsafe { *textures = id };
        });
    }
    unsafe extern "system" fn delete(n: i32, textures: *const u32) {
        assert_eq!(n, 1);
        DRIVER.with(|driver| {
            let mut d = driver.borrow_mut();
            assert_eq!(d.context, 'A');
            let id = unsafe { *textures };
            assert!(d.live.remove(&id), "duplicate or foreign deletion");
            d.deleted.push(id);
        });
    }
    fn maybe_fail(step: &str) {
        DRIVER.with(|driver| {
            let mut d = driver.borrow_mut();
            assert_eq!(d.context, 'A');
            if d.failure == step {
                d.error = gl::OUT_OF_MEMORY;
            }
        });
    }
    unsafe extern "system" fn active(_: u32) {
        maybe_fail("activate");
    }
    unsafe extern "system" fn bind(_: u32, texture: u32) {
        DRIVER.with(|d| assert!(d.borrow().live.contains(&texture)));
        maybe_fail("bind");
    }
    unsafe extern "system" fn upload(
        _: u32,
        _: i32,
        _: i32,
        _: i32,
        _: i32,
        _: i32,
        _: u32,
        _: u32,
        _: *const c_void,
    ) {
        maybe_fail("upload");
    }
    unsafe extern "system" fn mipmap(_: u32) {
        maybe_fail("mipmap");
    }
    unsafe extern "system" fn parameter(_: u32, _: u32, _: i32) {
        maybe_fail("filter");
    }
    unsafe extern "system" fn error() -> u32 {
        DRIVER.with(|d| std::mem::replace(&mut d.borrow_mut().error, gl::NO_ERROR))
    }
    unsafe extern "system" fn string(name: u32) -> *const u8 {
        DRIVER.with(|driver| {
            let mut d = driver.borrow_mut();
            d.string_calls += 1;
            if !matches!(
                name,
                gl::VENDOR
                    | gl::RENDERER
                    | gl::VERSION
                    | gl::SHADING_LANGUAGE_VERSION
                    | gl::EXTENSIONS
            ) {
                d.error = gl::INVALID_ENUM;
            }
        });
        std::ptr::null()
    }
    fn driver() -> Rc<gl::Gl> {
        Rc::new(gl::Gl::load_with(|name| match name {
            "glGenTextures" => generate as *const c_void,
            "glDeleteTextures" => delete as *const c_void,
            "glActiveTexture" => active as *const c_void,
            "glBindTexture" => bind as *const c_void,
            "glTexImage2D" => upload as *const c_void,
            "glGenerateMipmap" => mipmap as *const c_void,
            "glTexParameteri" => parameter as *const c_void,
            "glGetError" => error as *const c_void,
            "glGetString" => string as *const c_void,
            _ => std::ptr::null(),
        }))
    }
    #[test]
    fn failed_texture_construction_releases_each_allocation_and_preserves_existing_texture() {
        DRIVER.with(|d| {
            *d.borrow_mut() = Driver {
                context: 'A',
                ..Driver::default()
            }
        });
        let gl = driver();
        let retained = Wallpaper::new(gl.clone(), DynamicImage::new_rgba8(2, 2), false).unwrap();
        for step in ["activate", "bind", "upload", "mipmap", "filter"] {
            DRIVER.with(|d| d.borrow_mut().failure = step);
            for _ in 0..20 {
                assert!(Wallpaper::new(gl.clone(), DynamicImage::new_rgba8(2, 2), false).is_err());
                DRIVER.with(|d| assert_eq!(d.borrow().live, BTreeSet::from([retained.texture])));
                assert_eq!(unsafe { gl.GetError() }, gl::NO_ERROR);
                DRIVER.with(|d| d.borrow_mut().failure = "");
                retained.bind().unwrap();
                DRIVER.with(|d| d.borrow_mut().failure = step);
            }
        }
        DRIVER.with(|d| d.borrow_mut().failure = "");
        let replacement = Wallpaper::new(gl, DynamicImage::new_rgba8(2, 2), true).unwrap();
        drop(replacement);
        drop(retained);
        DRIVER.with(|d| {
            let d = d.borrow();
            assert!(d.live.is_empty());
            assert_eq!(d.deleted.len(), 102);
            assert_eq!(d.string_calls, 0);
        });
    }

    #[test]
    fn gl_check_preserves_original_error_without_querying_get_string() {
        DRIVER.with(|d| *d.borrow_mut() = Driver::default());
        let gl = driver();
        DRIVER.with(|d| d.borrow_mut().error = gl::OUT_OF_MEMORY);
        let operation = || -> Result<()> {
            unsafe { gl_check!(gl, "injected operation") };
            Ok(())
        };
        let error = operation().unwrap_err();
        assert_eq!(error.to_string(), "injected operation");
        assert!(format!("{error:?}").contains(&format!("OpenGL error {}", gl::OUT_OF_MEMORY)));
        DRIVER.with(|d| assert_eq!(d.borrow().string_calls, 0));
        assert_eq!(unsafe { gl.GetError() }, gl::NO_ERROR);
        operation().unwrap();
    }
}
