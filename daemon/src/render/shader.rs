use std::ffi::CStr;

use color_eyre::Result;

use crate::gl_check;

use super::gl;

// The caller must keep this allocation's EGL context current until construction returns.
struct ShaderAllocation<'a> {
    gl: &'a gl::Gl,
    shader: gl::types::GLuint,
}
impl Drop for ShaderAllocation<'_> {
    fn drop(&mut self) {
        if self.shader != 0 {
            unsafe { self.gl.DeleteShader(self.shader) };
        }
    }
}

pub unsafe fn create_shader(
    gl: &gl::Gl,
    shader: gl::types::GLenum,
    sources: &[*const std::ffi::c_char],
) -> Result<gl::types::GLuint> {
    let shader = gl.CreateShader(shader);
    let mut allocation = ShaderAllocation { gl, shader };
    gl_check!(gl, "calling CreateShader");
    gl.ShaderSource(
        shader,
        sources.len() as i32,
        sources.as_ptr().cast(),
        std::ptr::null(),
    );
    gl_check!(gl, "Failed to set the shader source");
    gl.CompileShader(shader);
    gl_check!(gl, "Failed to compile the shader");

    let mut status: i32 = 0;
    gl.GetShaderiv(shader, gl::COMPILE_STATUS, &mut status as *mut _);
    gl_check!(gl, "Failed to get the shader compile status");
    if status == 0 {
        let mut max_length: i32 = 0;
        let mut length: i32 = 0;
        gl.GetShaderiv(shader, gl::INFO_LOG_LENGTH, &mut max_length as *mut _);
        gl_check!(
            gl,
            "Failed to get the size of the information about the shader error"
        );
        let mut log: Vec<u8> = vec![0; max_length as _];
        gl.GetShaderInfoLog(
            shader,
            max_length,
            &mut length as *mut _,
            log.as_mut_ptr() as _,
        );
        gl_check!(gl, "Failed to get the information about the shader error");
        let res = String::from_utf8(log);
        match res {
            Ok(log) => Err(color_eyre::eyre::eyre!(log)),
            Err(err) => Err(color_eyre::eyre::eyre!(err)),
        }
    } else {
        // Successful construction transfers ownership to the existing program caller.
        allocation.shader = 0;
        Ok(shader)
    }
}

pub const VERTEX_SHADER_SOURCE: &CStr = c"
#version 300 es
precision mediump float;

layout (location = 0) in vec2 aPosition;
layout (location = 1) in vec2 aTexCoord;

uniform mat2 projection_matrix;

out vec2 v_texcoord;

void main() {
    gl_Position = vec4(aPosition* projection_matrix, 1.0, 1.0);
    v_texcoord = aTexCoord;
}";

pub const FRAGMENT_SHADER_SOURCE: &CStr = c"
#version 300 es
precision highp float;
out vec4 FragColor;

in vec2 v_texcoord;

uniform vec2 textureScale;
uniform vec2 prevTextureScale;
uniform sampler2D u_prev_texture;
uniform sampler2D u_texture;

uniform float progress;
uniform float ratio;
uniform float texture_offset;

vec4 transition(vec2);

vec4 getFromColor(vec2 uv) {
    uv = (uv - texture_offset) * prevTextureScale + (texture_offset);
    return texture(u_prev_texture, uv);
}

vec4 getToColor(vec2 uv) {
    uv = (uv - texture_offset) * textureScale + (texture_offset);
    return texture(u_texture, uv);
}

void main() {
    FragColor = transition(v_texcoord);
}";

#[cfg(test)]
mod tests {
    use super::*;
    use std::{cell::RefCell, collections::BTreeMap, ffi::c_void};

    #[derive(Default)]
    struct Driver {
        context: char,
        next: BTreeMap<char, u32>,
        live: BTreeMap<(char, u32), u32>,
        deleted: Vec<(char, u32)>,
        failure: &'static str,
        error: u32,
        string_calls: usize,
    }
    thread_local! {
        static DRIVER: RefCell<Driver> = RefCell::new(Driver::default());
    }
    fn maybe_fail(step: &str) {
        DRIVER.with(|d| {
            let mut d = d.borrow_mut();
            if d.failure == step {
                d.error = gl::OUT_OF_MEMORY;
            }
        });
    }
    fn check_live(shader: u32) {
        DRIVER.with(|d| {
            let d = d.borrow();
            assert!(
                d.live.contains_key(&(d.context, shader)),
                "foreign shader namespace"
            );
        });
    }
    unsafe extern "system" fn create(stage: u32) -> u32 {
        let shader = DRIVER.with(|d| {
            let mut d = d.borrow_mut();
            if d.failure == "create-zero" {
                d.error = gl::OUT_OF_MEMORY;
                return 0;
            }
            let context = d.context;
            let next = d.next.entry(context).or_default();
            *next += 1;
            let shader = *next;
            d.live.insert((context, shader), stage);
            shader
        });
        maybe_fail("create");
        shader
    }
    unsafe extern "system" fn delete(shader: u32) {
        DRIVER.with(|d| {
            let mut d = d.borrow_mut();
            let key = (d.context, shader);
            assert!(
                d.live.remove(&key).is_some(),
                "duplicate or foreign deletion"
            );
            d.deleted.push(key);
        });
    }
    unsafe extern "system" fn source(shader: u32, _: i32, _: *const *const i8, _: *const i32) {
        check_live(shader);
        maybe_fail("source");
    }
    unsafe extern "system" fn compile(shader: u32) {
        check_live(shader);
        maybe_fail("compile");
    }
    unsafe extern "system" fn status(shader: u32, parameter: u32, value: *mut i32) {
        check_live(shader);
        let result = DRIVER.with(|d| {
            let d = d.borrow();
            match parameter {
                gl::COMPILE_STATUS => i32::from(!matches!(
                    d.failure,
                    "compile-status-zero" | "log-length" | "log" | "log-utf8"
                )),
                gl::INFO_LOG_LENGTH => 8,
                _ => panic!("unexpected shader parameter"),
            }
        });
        unsafe { *value = result };
        maybe_fail(if parameter == gl::COMPILE_STATUS {
            "status"
        } else {
            "log-length"
        });
    }
    unsafe extern "system" fn info_log(shader: u32, maximum: i32, length: *mut i32, log: *mut i8) {
        check_live(shader);
        assert_eq!(maximum, 8);
        unsafe {
            *length = 7;
            std::ptr::copy_nonoverlapping(b"failure\0".as_ptr(), log.cast(), 8);
        }
        DRIVER.with(|d| {
            if d.borrow().failure == "log-utf8" {
                unsafe { *log = -1 };
            }
        });
        maybe_fail("log");
    }
    unsafe extern "system" fn error() -> u32 {
        DRIVER.with(|d| std::mem::replace(&mut d.borrow_mut().error, gl::NO_ERROR))
    }
    unsafe extern "system" fn string(name: u32) -> *const u8 {
        DRIVER.with(|d| {
            let mut d = d.borrow_mut();
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
    fn driver() -> gl::Gl {
        gl::Gl::load_with(|name| match name {
            "glCreateShader" => create as *const c_void,
            "glDeleteShader" => delete as *const c_void,
            "glShaderSource" => source as *const c_void,
            "glCompileShader" => compile as *const c_void,
            "glGetShaderiv" => status as *const c_void,
            "glGetShaderInfoLog" => info_log as *const c_void,
            "glGetError" => error as *const c_void,
            "glGetString" => string as *const c_void,
            _ => std::ptr::null(),
        })
    }

    #[test]
    fn shader_constructor_releases_every_failure_and_transfers_only_successful_allocations() {
        DRIVER.with(|d| {
            *d.borrow_mut() = Driver {
                context: 'A',
                ..Driver::default()
            }
        });
        let gl = driver();
        let sources = [VERTEX_SHADER_SOURCE.as_ptr()];
        let vertex = unsafe { create_shader(&gl, gl::VERTEX_SHADER, &sources) }.unwrap();
        let fragment = unsafe { create_shader(&gl, gl::FRAGMENT_SHADER, &sources) }.unwrap();
        DRIVER.with(|d| d.borrow_mut().context = 'B');
        let foreign = unsafe { create_shader(&gl, gl::VERTEX_SHADER, &sources) }.unwrap();
        // The same numeric name in a different context is a separate allocation.
        assert_eq!(vertex, foreign);
        DRIVER.with(|d| d.borrow_mut().context = 'A');
        let retained = DRIVER.with(|d| d.borrow().live.clone());
        for stage in [gl::VERTEX_SHADER, gl::FRAGMENT_SHADER] {
            for step in [
                "create",
                "create-zero",
                "source",
                "compile",
                "status",
                "compile-status-zero",
                "log-length",
                "log",
                "log-utf8",
            ] {
                for _ in 0..20 {
                    DRIVER.with(|d| d.borrow_mut().failure = step);
                    assert!(
                        unsafe { create_shader(&gl, stage, &sources) }.is_err(),
                        "{stage}: {step}"
                    );
                    DRIVER.with(|d| assert_eq!(d.borrow().live, retained, "{stage}: {step}"));
                    assert_eq!(unsafe { gl.GetError() }, gl::NO_ERROR);
                    DRIVER.with(|d| d.borrow_mut().failure = "");
                    // Already-owned shaders remain valid after each rejected construction.
                    unsafe {
                        gl.CompileShader(vertex);
                        gl.CompileShader(fragment);
                    }
                    assert_eq!(unsafe { gl.GetError() }, gl::NO_ERROR);
                }
            }
            let successful = unsafe { create_shader(&gl, stage, &sources) }.unwrap();
            DRIVER.with(|d| assert_eq!(d.borrow().live[&('A', successful)], stage));
            unsafe { gl.DeleteShader(successful) };
        }
        unsafe {
            gl.DeleteShader(vertex);
            gl.DeleteShader(fragment);
        }
        DRIVER.with(|d| d.borrow_mut().context = 'B');
        unsafe {
            gl.CompileShader(foreign);
            gl.DeleteShader(foreign);
        }
        DRIVER.with(|d| {
            let d = d.borrow();
            assert!(d.live.is_empty());
            assert_eq!(d.deleted.len(), 325);
            assert_eq!(d.string_calls, 0);
        });
    }
}
