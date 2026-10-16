use std::{ops::Deref, rc::Rc, time::Instant};

use color_eyre::{
    eyre::{ensure, OptionExt, WrapErr},
    Result,
};
use egl::API as egl;
use image::{DynamicImage, RgbaImage};
use log::error;
use smithay_client_toolkit::reexports::client::protocol::wl_output::Transform;

use crate::{
    display_info::DisplayInfo,
    gl_check,
    render::{
        initialize_objects,
        shader::{create_shader, FRAGMENT_SHADER_SOURCE, VERTEX_SHADER_SOURCE},
    },
    wallpaper_info::BackgroundMode,
};

use super::{gl, wallpaper::Wallpaper, Transition};

fn black_image() -> RgbaImage {
    RgbaImage::from_raw(1, 1, vec![0, 0, 0, 255]).unwrap()
}

#[derive(Debug)]
pub enum TransitionStatus {
    /// Transition has been requested but no frame has been drawn yet.
    /// The Instant is unused; the clock starts on the first actual draw.
    Started,
    Running {
        started: Instant,
        progress: f32,
    },
    Ended,
}

pub struct Renderer {
    gl: Rc<gl::Gl>,
    pub program: gl::types::GLuint,
    vbo: gl::types::GLuint,
    eab: gl::types::GLuint,
    // milliseconds time for the transition
    pub transition_time: u32,
    prev_wallpaper: Wallpaper,
    current_wallpaper: Wallpaper,
    //transparent_texture: gl::types::GLuint,
    /// contains the progress of the current animation
    transition_status: TransitionStatus,
    motion: Option<super::motion::MotionRenderer>,
    prepared_motion: Option<super::motion::MotionRenderer>,
}

impl Renderer {
    pub unsafe fn new(
        transition_time: u32,
        transition: Transition,
        display_info: &DisplayInfo,
    ) -> Result<Self> {
        let gl = Rc::new(gl::Gl::load_with(|name| {
            egl.get_proc_address(name)
                .ok_or_eyre("Cannot find openGL ES")
                .unwrap() as *const std::ffi::c_void
        }));

        let program =
            create_program(&gl, transition).wrap_err("Failed to create openGL program")?;

        let (vbo, eab) = initialize_objects(&gl).wrap_err("Failed to initialize openGL objects")?;

        let current_wallpaper = Wallpaper::new(gl.clone(), black_image().into(), true)?;
        let prev_wallpaper = Wallpaper::new(gl.clone(), black_image().into(), false)?;

        let renderer = Self {
            gl,
            program,
            vbo,
            eab,
            transition_time,
            prev_wallpaper,
            current_wallpaper,
            transition_status: TransitionStatus::Ended,
            motion: None,
            prepared_motion: None,
        };

        renderer
            .set_projection_matrix(display_info.transform)
            .wrap_err("Failed to set projection matrix for openGL context")?;

        Ok(renderer)
    }

    #[inline]
    pub fn check_error(&self, msg: &str) -> Result<()> {
        unsafe {
            gl_check!(self.gl, msg);
        }
        Ok(())
    }

    pub unsafe fn draw(&mut self) -> Result<()> {
        self.restore_still()?;
        self.gl.Clear(gl::COLOR_BUFFER_BIT);
        self.check_error("Failed to clear the screen")?;

        let loc = self
            .gl
            .GetUniformLocation(self.program, b"progress\0".as_ptr() as *const _);
        self.check_error("Failed to get the uniform location for progress")?;
        self.gl.Uniform1f(
            loc,
            match &self.transition_status {
                TransitionStatus::Started => 0.0,
                TransitionStatus::Running { progress, .. } => *progress,
                TransitionStatus::Ended => 1.0,
            },
        );
        self.check_error("Failed to set the progress in the openGL shader")?;

        self.gl
            .DrawElements(gl::TRIANGLES, 6, gl::UNSIGNED_INT, std::ptr::null());
        self.check_error("Failed to draw the vertices")?;

        Ok(())
    }

    /// Update the transition status using wall-clock time.
    ///
    /// Returns `true` if the transition is still running, `false` if it ended or was already over.
    /// Using `Instant` rather than the compositor frame timestamp avoids a bug where calling
    /// `update_transition_status` with `time=0` (e.g. from a non-frame code path) recorded
    /// `started=0`, causing the first real frame (at a large compositor timestamp) to compute
    /// `progress = 1.0` and skip the entire transition.
    #[inline]
    pub fn update_transition_status(&mut self) -> bool {
        let started = match self.transition_status {
            // First actual draw: start the clock now so the full transition duration is shown
            // even if the surface was hidden between image load and first frame.
            TransitionStatus::Started => {
                let now = Instant::now();
                self.transition_status = TransitionStatus::Running {
                    started: now,
                    progress: 0.0,
                };
                return true;
            }
            TransitionStatus::Running { started, .. } => started,
            TransitionStatus::Ended => return false,
        };
        let elapsed_ms = started.elapsed().as_millis() as f32;
        let progress = (elapsed_ms / self.transition_time as f32).min(1.0);
        // Recalculate the current progress; the transition might end now
        if progress >= 1.0 {
            self.transition_finished();
            false
        } else {
            self.transition_status = TransitionStatus::Running { started, progress };
            true
        }
    }

    pub fn load_wallpaper(
        &mut self,
        image: DynamicImage,
        mode: BackgroundMode,
        offset: Option<f32>,
        display_info: &DisplayInfo,
    ) -> Result<()> {
        std::mem::swap(&mut self.prev_wallpaper, &mut self.current_wallpaper);
        unsafe {
            self.gl.ActiveTexture(gl::TEXTURE0);
            self.check_error("Failed to activate texture TEXTURE0")?;
        }
        self.prev_wallpaper.bind()?;

        // Load image into TEXTURE1
        self.current_wallpaper.load_image(image, true)?;

        self.set_mode(mode, offset, display_info)?;

        Ok(())
    }

    pub fn set_mode(
        &mut self,
        mode: BackgroundMode,
        offset: Option<f32>,
        display_info: &DisplayInfo,
    ) -> Result<()> {
        unsafe {
            self.gl.UseProgram(self.program);
        }
        let display_width = display_info.scaled_width() as f32;
        let display_height = display_info.scaled_height() as f32;
        let display_ratio = display_width / display_height;
        let gen_texture_scale = |image_width: f32, image_height: f32| {
            let image_ratio: f32 = image_width / image_height;
            Box::new(match mode {
                BackgroundMode::Stretch => [1.0, 1.0],
                BackgroundMode::Center => [
                    (display_ratio / image_ratio).min(1.0),
                    (image_ratio / display_ratio).min(1.0),
                ],
                BackgroundMode::Fit | BackgroundMode::FitBorderColor => {
                    // Portrait mode
                    // In this case we calculate the width relative to the height of the
                    // screen with the ratio of the image
                    let width = display_height * image_ratio;
                    // Same thing as above, just with the width
                    let height = display_width / image_ratio;
                    // Then we calculate the proportions
                    [
                        (display_width / width).max(1.0),
                        (display_height / height).max(1.0),
                    ]
                }
                BackgroundMode::Tile => {
                    let width_proportion = display_width / image_width * display_ratio;
                    let height_proportion = display_height / image_height * display_ratio;
                    if display_ratio > image_ratio {
                        // Portrait mode
                        if height_proportion.max(1.0) == 1.0 {
                            // Same as Fit
                            let width = display_height * image_ratio;
                            [display_width / width, 1.0]
                        } else {
                            [width_proportion, height_proportion]
                        }
                    } else {
                        // Landscape mode
                        if width_proportion.max(1.0) == 1.0 {
                            // Same as Fit
                            let height = display_width / image_ratio;
                            [1.0, display_height / height]
                        } else {
                            [width_proportion, height_proportion]
                        }
                    }
                }
            })
        };
        let texture_scale = gen_texture_scale(
            self.current_wallpaper.get_image_width() as f32,
            self.current_wallpaper.get_image_height() as f32,
        );
        let (prev_image_width, prev_image_height) = (
            self.prev_wallpaper.get_image_width() as f32,
            self.prev_wallpaper.get_image_height() as f32,
        );

        let prev_texture_scale = gen_texture_scale(prev_image_width, prev_image_height);

        unsafe {
            let loc = self
                .gl
                .GetUniformLocation(self.program, b"textureScale\0".as_ptr() as *const _);
            self.check_error("Failed to get the uniform location for textureScale")?;
            ensure!(loc >= 0, "Failed to find uniform textureScale");
            self.gl
                .Uniform2fv(loc, 1, texture_scale.as_ptr() as *const _);
            self.check_error("Failed to set uniform textureScale")?;

            let loc = self
                .gl
                .GetUniformLocation(self.program, b"prevTextureScale\0".as_ptr() as *const _);
            self.check_error("Failed to get the uniform location for prevTextureScale")?;
            ensure!(loc >= 0, "Failed to find the uniform prevTextureScale");
            self.gl
                .Uniform2fv(loc, 1, prev_texture_scale.as_ptr() as *const _);
            self.check_error("Failed to set the value for prevTextureScale")?;

            let loc = self
                .gl
                .GetUniformLocation(self.program, b"ratio\0".as_ptr() as *const _);
            self.check_error("Failed to get the uniform location for ratio")?;
            self.gl.Uniform1f(loc, display_ratio);
            self.check_error("Failed to set the value for the uniform ratio")?;

            let offset = match (offset, mode) {
                (
                    None,
                    BackgroundMode::Stretch
                    | BackgroundMode::Center
                    | BackgroundMode::Fit
                    | BackgroundMode::FitBorderColor,
                ) => 0.5,
                (None, BackgroundMode::Tile) => 0.0,
                (Some(offset), _) => offset,
            };

            let loc = self
                .gl
                .GetUniformLocation(self.program, b"texture_offset\0".as_ptr() as *const _);
            self.check_error("Failed to get the location for the uniform texture_offset")?;
            self.gl.Uniform1f(loc, offset);
            self.check_error("Failed to set the value for the uniform texture_offset")?;

            let texture_wrap = match mode {
                BackgroundMode::Stretch | BackgroundMode::Center | BackgroundMode::Fit => {
                    gl::CLAMP_TO_BORDER_EXT
                }
                BackgroundMode::Tile => gl::REPEAT,
                BackgroundMode::FitBorderColor => gl::CLAMP_TO_EDGE,
            } as i32;

            self.gl.ActiveTexture(gl::TEXTURE0);
            self.check_error("Failed to activate texture TEXTURE0")?;
            self.gl
                .TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_WRAP_S, texture_wrap);
            self.check_error("Failed to set the attribute TEXTURE_WRAP_S for TEXTURE0")?;
            self.gl
                .TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_WRAP_T, texture_wrap);
            self.check_error("Failed to set the attribute TEXTURE_WRAP_T FOR TEXTURE0")?;

            self.gl.ActiveTexture(gl::TEXTURE1);
            self.check_error("Failed to activate texture TEXTURE1")?;
            self.gl
                .TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_WRAP_S, texture_wrap);
            self.check_error("Failed to set the attribute TEXTURE_WRAP_S for TEXTURE1")?;
            self.gl
                .TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_WRAP_T, texture_wrap);
            self.check_error("Failed to set the attribute TEXTURE_WRAP_T for TEXTURE1")?;
        }

        Ok(())
    }

    #[inline]
    pub fn start_transition(&mut self, transition_time: u32) {
        // Always restart the transition regardless of the current state. An image
        // can finish loading faster than a frame event arrives (e.g. cached image),
        // so Started/Running is a valid state to enter here.
        // A transition_time of 0 means instant switch; no animation needed.
        if transition_time == 0 {
            self.transition_finished();
        } else {
            self.transition_status = TransitionStatus::Started;
            self.transition_time = transition_time;
        }
    }

    #[inline]
    pub fn clear_after_draw(&self) -> Result<()> {
        unsafe {
            // Unbind the framebuffer and renderbuffer before deleting.
            // self.gl.BindBuffer(gl::PIXEL_UNPACK_BUFFER, 0);
            // self.check_error("unbinding the unpack buffer")?;
            self.gl.BindFramebuffer(gl::FRAMEBUFFER, 0);
            self.check_error("Failed to unbind the framebuffer")?;
            self.gl.BindRenderbuffer(gl::RENDERBUFFER, 0);
            self.check_error("Failed to unbind the renderbuffer")?;
        }

        Ok(())
    }

    pub fn resize(&mut self, display_info: &DisplayInfo) -> Result<()> {
        unsafe {
            self.gl.Viewport(
                0,
                0,
                display_info.adjusted_width(),
                display_info.adjusted_height(),
            );
            self.check_error("Failed to resize the openGL viewport")
        }
    }

    #[inline]
    pub fn update_transition_time(&mut self, transition_time: u32) {
        self.transition_time = transition_time;
    }

    #[inline]
    pub fn transition_finished(&mut self) {
        self.transition_status = TransitionStatus::Ended;
    }

    /// Swap in the program for `transition`. Uniforms are per program and start at zero, so
    /// everything `set_mode` and `set_projection_matrix` wrote must be written again.
    pub fn update_transition(
        &mut self,
        transition: Transition,
        mode: BackgroundMode,
        offset: Option<f32>,
        display_info: &DisplayInfo,
    ) {
        match create_program(&self.gl, transition) {
            Ok(program) => {
                unsafe {
                    self.gl.DeleteProgram(self.program);
                }
                // Stop the transition immediately
                if self.transition_running() {
                    self.transition_finished();
                }
                self.program = program;
                unsafe {
                    if let Err(err) = self
                        .set_projection_matrix(display_info.transform)
                        .wrap_err("Failed to set the projection matrix")
                    {
                        error!("{err:?}");
                    }
                }
                // Without textureScale the shader samples one border texel: a flat dark frame.
                if let Err(err) = self
                    .set_mode(mode, offset, display_info)
                    .wrap_err("Failed to restore the wallpaper mode for the new transition")
                {
                    error!("{err:?}");
                }
            }
            Err(err) => error!("{err:?}"),
        }
    }

    #[inline]
    pub fn transition_running(&self) -> bool {
        match self.transition_status {
            TransitionStatus::Started | TransitionStatus::Running { .. } => true,
            TransitionStatus::Ended => false,
        }
    }

    pub unsafe fn set_projection_matrix(&self, transform: Transform) -> Result<()> {
        self.gl.UseProgram(self.program);
        let projection_matrix = projection_matrix(transform);
        let loc = self
            .gl
            .GetUniformLocation(self.program, b"projection_matrix\0".as_ptr() as *const _);
        self.check_error("Failed to get the uniform location for projection_matrix")?;
        ensure!(loc >= 0, "Failed to find uniform projection_matrix");
        self.gl
            .UniformMatrix2fv(loc, 1, 0, projection_matrix.as_ptr());
        //self.gl
        //    .UniformMatrix2fv(loc, 1, 0, [1.0, 0.0, 0.0, 1.0].as_ptr());

        self.check_error("calling Uniform1i")?;

        Ok(())
    }

    pub fn draw_scene(&mut self, frame: &crate::scene::Frame, display: &DisplayInfo) -> Result<()> {
        if self
            .motion
            .as_ref()
            .is_none_or(|m| m.scene_sha256 != frame.scene.sha256)
        {
            self.motion = None;
            self.motion = Some(super::motion::MotionRenderer::new(
                self.gl.clone(),
                &frame.scene,
            )?);
        }
        unsafe {
            self.gl.BindBuffer(gl::ARRAY_BUFFER, self.vbo);
            self.gl.BindBuffer(gl::ELEMENT_ARRAY_BUFFER, self.eab);
            self.motion.as_ref().unwrap().draw(frame, display)
        }
    }

    pub fn clear_motion(&mut self) -> Result<()> {
        self.motion = None;
        self.prepared_motion = None;
        unsafe { self.restore_still() }
    }

    pub fn prepare_motion(&mut self, scene: &crate::scene::Scene) -> Result<()> {
        self.prepared_motion = None;
        let result = super::motion::MotionRenderer::new(self.gl.clone(), scene);
        self.restore_after_preparation()?;
        self.prepared_motion = Some(result?);
        Ok(())
    }
    fn restore_after_preparation(&self) -> Result<()> {
        unsafe { self.restore_still() }
    }
    pub fn discard_prepared_motion(&mut self) {
        self.prepared_motion = None;
    }
    pub fn install_prepared_motion(&mut self) {
        self.motion = self.prepared_motion.take();
    }

    /// Called with the owning EGL context current and still alive.
    pub fn release_resources(&mut self) {
        self.motion = None;
        self.prepared_motion = None;
        self.prev_wallpaper.release();
        self.current_wallpaper.release();
        unsafe {
            if self.eab != 0 {
                self.gl.DeleteBuffers(1, &self.eab);
                self.eab = 0;
            }
            if self.vbo != 0 {
                self.gl.DeleteBuffers(1, &self.vbo);
                self.vbo = 0;
            }
            if self.program != 0 {
                self.gl.DeleteProgram(self.program);
                self.program = 0;
            }
        }
    }
    /// When binding failed, let named EGL context destruction reclaim GPU handles.
    pub fn disarm_resources(&mut self) {
        if let Some(motion) = self.motion.as_mut() {
            motion.disarm();
        }
        if let Some(prepared) = self.prepared_motion.as_mut() {
            prepared.disarm();
        }
        self.prev_wallpaper.disarm();
        self.current_wallpaper.disarm();
        self.eab = 0;
        self.vbo = 0;
        self.program = 0;
    }

    unsafe fn restore_still(&self) -> Result<()> {
        self.gl.Disable(gl::BLEND);
        self.gl.ClearColor(0.0, 0.0, 0.0, 1.0);
        self.gl.UseProgram(self.program);
        self.gl.BindBuffer(gl::ARRAY_BUFFER, self.vbo);
        self.gl.BindBuffer(gl::ELEMENT_ARRAY_BUFFER, self.eab);
        self.gl.ActiveTexture(gl::TEXTURE0);
        self.prev_wallpaper.bind()?;
        self.gl.ActiveTexture(gl::TEXTURE1);
        self.current_wallpaper.bind()?;
        self.check_error("Failed to restore still renderer")
    }
}

fn create_program(gl: &gl::Gl, transition: Transition) -> Result<gl::types::GLuint> {
    unsafe {
        let program = gl.CreateProgram();
        gl_check!(gl, "Failed to create openGL program");

        let vertex_shader = create_shader(gl, gl::VERTEX_SHADER, &[VERTEX_SHADER_SOURCE.as_ptr()])
            .wrap_err("Failed to create vertex shader")?;
        let (uniform_callback, shader) = transition.clone().shader();
        let fragment_shader = create_shader(
            gl,
            gl::FRAGMENT_SHADER,
            &[FRAGMENT_SHADER_SOURCE.as_ptr(), shader.as_ptr()],
        )
        .wrap_err_with(|| {
            format!("Failed to create fragment shader for transisition {transition:?}")
        })?;

        gl.AttachShader(program, vertex_shader);
        gl_check!(gl, "Failed to attach vertices shader");
        gl.AttachShader(program, fragment_shader);
        gl_check!(gl, "Failed to attach fragment shader");
        gl.LinkProgram(program);
        gl_check!(gl, "Failed to link the openGL program");
        gl.DeleteShader(vertex_shader);
        gl_check!(gl, "Failed to delete the vertices shader");
        gl.DeleteShader(fragment_shader);
        gl_check!(gl, "Failed to delete the fragment shader");
        gl.UseProgram(program);
        gl_check!(gl, "Failed to switch to the newly created openGL program");

        // We need to setup the uniform each time we create a program
        let loc = gl.GetUniformLocation(program, b"u_prev_texture\0".as_ptr() as *const _);
        gl_check!(gl, "Failed to get the uniform location for u_prev_texture");
        ensure!(loc >= 0, "Failed to find the uniform u_prev_texture");
        gl.Uniform1i(loc, 0);
        gl_check!(gl, "Failed to set the value for uniform u_prev_texture");
        let loc = gl.GetUniformLocation(program, b"u_texture\0".as_ptr() as *const _);
        gl_check!(gl, "Failed to get the uniform location for u_texture");
        ensure!(loc >= 0, "Failed to find the uniform u_texture");
        gl.Uniform1i(loc, 1);
        gl_check!(gl, "Failed to set the value for uniform u_texture");

        uniform_callback(gl, program)?;

        Ok(program)
    }
}

#[rustfmt::skip]
pub(super) fn projection_matrix(transform: Transform) -> [f32; 4] {
    match transform {
        Transform::Normal => {
            [
                1.0, 0.0,
                0.0, 1.0,
            ]
        }
        Transform::_90 => {
            [
                0.0, -1.0,
                1.0, 0.0,
            ]
        }
        Transform::_180 => {
            [
                -1.0, 0.0,
                0.0, -1.0,
            ]
        }
        Transform::_270 => {
            [
                0.0, 1.0,
                -1.0, 0.0,
            ]
        }
        Transform::Flipped => {
            [
                -1.0, 0.0,
                0.0, 1.0,
            ]
        }
        Transform::Flipped90 => {
            [
                0.0, -1.0,
                -1.0, 0.0,
            ]
        }
        Transform::Flipped180 => {
            [
                1.0, 0.0,
                0.0, -1.0,
            ]
        }
        Transform::Flipped270 => {
            [
                0.0, 1.0,
                1.0, 0.0,
            ]
        }
        _ => unreachable!()
    }
}

impl Deref for Renderer {
    type Target = gl::Gl;

    fn deref(&self) -> &Self::Target {
        &self.gl
    }
}

impl Drop for Renderer {
    fn drop(&mut self) {
        self.release_resources();
    }
}

#[cfg(test)]
mod resource_tests {
    use super::*;
    #[test]
    fn disarmed_renderer_and_all_texture_destructors_issue_no_gl_calls() {
        // Any GL call through this intentionally unloaded dispatch table panics.
        let gl = Rc::new(gl::Gl::load_with(|_| std::ptr::null()));
        let mut renderer = Renderer {
            gl: gl.clone(),
            program: 3,
            vbo: 4,
            eab: 5,
            transition_time: 0,
            prev_wallpaper: Wallpaper::test_texture(gl.clone(), 6),
            current_wallpaper: Wallpaper::test_texture(gl.clone(), 7),
            transition_status: TransitionStatus::Ended,
            motion: Some(super::super::motion::MotionRenderer::test_resources(
                gl.clone(),
            )),
            prepared_motion: Some(super::super::motion::MotionRenderer::test_resources(gl)),
        };
        renderer.disarm_resources();
        assert_eq!((renderer.program, renderer.vbo, renderer.eab), (0, 0, 0));
        drop(renderer);
    }
}

#[cfg(test)]
mod transition_program_tests {
    use super::*;
    use std::{cell::RefCell, collections::BTreeSet, ffi::c_void, ffi::CStr};

    #[derive(Default)]
    struct Driver {
        next_name: u32,
        current: u32,
        locations: Vec<String>,
        written: BTreeSet<(u32, String)>,
    }
    thread_local! {
        static DRIVER: RefCell<Driver> = RefCell::new(Driver::default());
    }
    unsafe extern "system" fn name_object(_: u32) -> u32 {
        DRIVER.with(|d| {
            let mut d = d.borrow_mut();
            d.next_name += 1;
            d.next_name
        })
    }
    unsafe extern "system" fn create_program() -> u32 {
        unsafe { name_object(0) }
    }
    unsafe extern "system" fn ignore_1(_: u32) {}
    unsafe extern "system" fn ignore_2(_: u32, _: u32) {}
    unsafe extern "system" fn ignore_3i(_: u32, _: u32, _: i32) {}
    unsafe extern "system" fn source(_: u32, _: i32, _: *const *const i8, _: *const i32) {}
    unsafe extern "system" fn compiled(_: u32, _: u32, value: *mut i32) {
        unsafe { *value = 1 };
    }
    unsafe extern "system" fn use_program(program: u32) {
        DRIVER.with(|d| d.borrow_mut().current = program);
    }
    unsafe extern "system" fn location(_: u32, name: *const i8) -> i32 {
        let name = unsafe { CStr::from_ptr(name) }
            .to_string_lossy()
            .into_owned();
        DRIVER.with(|d| {
            let mut d = d.borrow_mut();
            d.locations.push(name);
            d.locations.len() as i32 - 1
        })
    }
    fn write(location: i32) {
        DRIVER.with(|d| {
            let mut d = d.borrow_mut();
            let entry = (d.current, d.locations[location as usize].clone());
            d.written.insert(entry);
        });
    }
    unsafe extern "system" fn uniform_1i(location: i32, _: i32) {
        write(location);
    }
    unsafe extern "system" fn uniform_1f(location: i32, _: f32) {
        write(location);
    }
    unsafe extern "system" fn uniform_v(location: i32, _: i32, _: *const f32) {
        write(location);
    }
    unsafe extern "system" fn uniform_matrix(location: i32, _: i32, _: u8, _: *const f32) {
        write(location);
    }
    unsafe extern "system" fn no_error() -> u32 {
        gl::NO_ERROR
    }
    fn driver() -> Rc<gl::Gl> {
        Rc::new(gl::Gl::load_with(|name| match name {
            "glCreateProgram" => create_program as *const c_void,
            "glCreateShader" => name_object as *const c_void,
            "glShaderSource" => source as *const c_void,
            "glCompileShader" | "glLinkProgram" | "glDeleteShader" | "glDeleteProgram"
            | "glActiveTexture" => ignore_1 as *const c_void,
            "glGetShaderiv" => compiled as *const c_void,
            "glAttachShader" => ignore_2 as *const c_void,
            "glTexParameteri" => ignore_3i as *const c_void,
            "glUseProgram" => use_program as *const c_void,
            "glGetUniformLocation" => location as *const c_void,
            "glUniform1i" => uniform_1i as *const c_void,
            "glUniform1f" => uniform_1f as *const c_void,
            "glUniform2fv" => uniform_v as *const c_void,
            "glUniformMatrix2fv" => uniform_matrix as *const c_void,
            "glGetError" => no_error as *const c_void,
            _ => std::ptr::null(),
        }))
    }

    /// A config reload that changes only the transition must leave the new program drawable.
    #[test]
    fn replaced_transition_program_receives_every_still_uniform() {
        DRIVER.with(|d| *d.borrow_mut() = Driver::default());
        let gl = driver();
        let mut renderer = Renderer {
            gl: gl.clone(),
            program: 100,
            vbo: 0,
            eab: 0,
            transition_time: 2000,
            prev_wallpaper: Wallpaper::test_texture(gl.clone(), 101),
            current_wallpaper: Wallpaper::test_texture(gl, 102),
            transition_status: TransitionStatus::Running {
                started: Instant::now(),
                progress: 0.5,
            },
            motion: None,
            prepared_motion: None,
        };
        let display = DisplayInfo {
            name: "DP-1".into(),
            description: String::new(),
            width: 3200,
            height: 1800,
            scale: 1,
            transform: Transform::_90,
        };
        renderer.update_transition(
            Transition::Directional { direction: None },
            BackgroundMode::Center,
            None,
            &display,
        );
        let program = renderer.program;
        assert_ne!(program, 100, "transition program was not replaced");
        assert!(!renderer.transition_running());
        DRIVER.with(|d| {
            let d = d.borrow();
            for uniform in [
                "u_prev_texture",
                "u_texture",
                "direction",
                "projection_matrix",
                "textureScale",
                "prevTextureScale",
                "ratio",
                "texture_offset",
            ] {
                assert!(
                    d.written.contains(&(program, uniform.to_owned())),
                    "{uniform} left at its zero default on the new program"
                );
            }
        });
        renderer.disarm_resources();
    }
}
