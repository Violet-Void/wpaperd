use super::{gl, renderer::projection_matrix, shader::create_shader, wallpaper::Wallpaper};
use crate::{
    display_info::DisplayInfo,
    gl_check,
    scene::{Background, Frame, Scene},
};
use color_eyre::{
    eyre::{ensure, eyre},
    Result,
};
use image::DynamicImage;
use std::{collections::BTreeMap, ffi::CStr, rc::Rc};

const VERTEX: &CStr = c"#version 300 es
precision highp float;
layout (location = 1) in vec2 aTexCoord;
uniform vec4 rect;
uniform vec4 crop;
uniform mat2 projection_matrix;
out vec2 uv;
void main() {
    vec2 p=(rect.xy+aTexCoord*rect.zw-crop.xy)/crop.zw;
    gl_Position=vec4(vec2(p.x*2.0-1.0,1.0-p.y*2.0)*projection_matrix,0.0,1.0);
    uv=aTexCoord;
}";
const FRAGMENT: &CStr = c"#version 300 es
precision highp float;
in vec2 uv;
uniform sampler2D sprite;
out vec4 color;
void main() {
    color=texture(sprite,uv);
    color.rgb*=color.a;
}";

pub struct MotionRenderer {
    gl: Rc<gl::Gl>,
    pub scene_sha256: String,
    program: u32,
    textures: BTreeMap<String, Wallpaper>,
}
impl MotionRenderer {
    pub fn new(gl: Rc<gl::Gl>, scene: &Scene) -> Result<Self> {
        let program = unsafe { gl.CreateProgram() };
        let mut result = Self {
            gl,
            scene_sha256: scene.sha256.clone(),
            program,
            textures: BTreeMap::new(),
        };
        unsafe {
            let v = create_shader(&result.gl, gl::VERTEX_SHADER, &[VERTEX.as_ptr()])?;
            let f = match create_shader(&result.gl, gl::FRAGMENT_SHADER, &[FRAGMENT.as_ptr()]) {
                Ok(s) => s,
                Err(e) => {
                    result.gl.DeleteShader(v);
                    return Err(e);
                }
            };
            result.gl.AttachShader(program, v);
            result.gl.AttachShader(program, f);
            result.gl.LinkProgram(program);
            result.gl.DeleteShader(v);
            result.gl.DeleteShader(f);
            let mut linked = 0;
            result
                .gl
                .GetProgramiv(program, gl::LINK_STATUS, &mut linked);
            ensure!(linked != 0, "Failed to link motion shader");
            let mut maximum = 0;
            result.gl.GetIntegerv(gl::MAX_TEXTURE_SIZE, &mut maximum);
            // Validate the entire render set before allocating any texture.
            let mut keys = scene
                .objects
                .iter()
                .map(|o| o.asset.as_str())
                .collect::<Vec<_>>();
            if let Background::Asset(key) = &scene.background {
                keys.push(key);
            }
            for key in &keys {
                let image = &scene.images[*key];
                ensure!(
                    image.width() <= maximum as u32 && image.height() <= maximum as u32,
                    "Scene asset exceeds GPU texture limit"
                );
            }
            for key in keys {
                let texture = Wallpaper::new(
                    result.gl.clone(),
                    DynamicImage::ImageRgba8((*scene.images[key]).clone()),
                    true,
                )?;
                // No additional opacity, tint, glow, color conversion or master redraw.
                result.gl.TexParameteri(
                    gl::TEXTURE_2D,
                    gl::TEXTURE_WRAP_S,
                    gl::CLAMP_TO_EDGE as i32,
                );
                result.gl.TexParameteri(
                    gl::TEXTURE_2D,
                    gl::TEXTURE_WRAP_T,
                    gl::CLAMP_TO_EDGE as i32,
                );
                result.textures.insert(key.into(), texture);
            }
            gl_check!(result.gl, "Failed to initialize scene textures");
        }
        Ok(result)
    }
    pub unsafe fn draw(&self, frame: &Frame, display: &DisplayInfo) -> Result<()> {
        self.gl.UseProgram(self.program);
        let crop = [
            frame.crop[0] as f32,
            frame.crop[1] as f32,
            (frame.crop[2] - frame.crop[0]) as f32,
            (frame.crop[3] - frame.crop[1]) as f32,
        ];
        self.gl.Uniform4fv(self.uniform(c"crop")?, 1, crop.as_ptr());
        self.gl.UniformMatrix2fv(
            self.uniform(c"projection_matrix")?,
            1,
            0,
            projection_matrix(display.transform).as_ptr(),
        );
        self.gl.Uniform1i(self.uniform(c"sprite")?, 1);
        self.gl.Disable(gl::BLEND);
        let bg = match frame.scene.background {
            Background::Solid(color) => color,
            Background::Asset(_) => [0; 4],
        };
        self.gl.ClearColor(
            bg[0] as f32 / 255.0 * (bg[3] as f32 / 255.0),
            bg[1] as f32 / 255.0 * (bg[3] as f32 / 255.0),
            bg[2] as f32 / 255.0 * (bg[3] as f32 / 255.0),
            1.0,
        );
        self.gl.Clear(gl::COLOR_BUFFER_BIT);
        // The wallpaper presents scene alpha-over over an explicit opaque black matte.
        // Source texture channels stay straight RGBA; only shader output is premultiplied.
        self.gl.Enable(gl::BLEND);
        self.gl.BlendFuncSeparate(
            gl::ONE,
            gl::ONE_MINUS_SRC_ALPHA,
            gl::ONE,
            gl::ONE_MINUS_SRC_ALPHA,
        );
        if let Background::Asset(key) = &frame.scene.background {
            self.quad(
                key,
                [
                    0.0,
                    0.0,
                    frame.scene.canvas[0] as f32,
                    frame.scene.canvas[1] as f32,
                ],
            )?;
        }
        for (obj, center) in frame.scene.objects.iter().zip(&frame.centers) {
            let rect = [
                (center[0] - obj.size[0] as f64 / 2.0).round_ties_even() as f32,
                (center[1] - obj.size[1] as f64 / 2.0).round_ties_even() as f32,
                obj.size[0] as f32,
                obj.size[1] as f32,
            ];
            self.quad(&obj.asset, rect)?;
        }
        self.gl.Disable(gl::BLEND);
        self.gl.ClearColor(0.0, 0.0, 0.0, 1.0);
        gl_check!(self.gl, "Failed to draw motion scene");
        Ok(())
    }
    unsafe fn uniform(&self, name: &CStr) -> Result<i32> {
        let location = self.gl.GetUniformLocation(self.program, name.as_ptr());
        ensure!(location >= 0, "Missing scene shader uniform");
        Ok(location)
    }
    unsafe fn quad(&self, key: &str, rect: [f32; 4]) -> Result<()> {
        self.gl.ActiveTexture(gl::TEXTURE1);
        self.textures
            .get(key)
            .ok_or_else(|| eyre!("Missing validated scene texture"))?
            .bind()?;
        self.gl.Uniform4fv(self.uniform(c"rect")?, 1, rect.as_ptr());
        self.gl
            .DrawElements(gl::TRIANGLES, 6, gl::UNSIGNED_INT, std::ptr::null());
        Ok(())
    }

    pub fn disarm(&mut self) {
        self.program = 0;
        for texture in self.textures.values_mut() {
            texture.disarm();
        }
    }
    #[cfg(test)]
    pub(super) fn test_resources(gl: Rc<gl::Gl>) -> Self {
        let texture = Wallpaper::test_texture(gl.clone(), 10);
        Self {
            gl,
            scene_sha256: "test".into(),
            program: 9,
            textures: BTreeMap::from([("test".into(), texture)]),
        }
    }
}
impl Drop for MotionRenderer {
    fn drop(&mut self) {
        if self.program != 0 {
            unsafe {
                self.gl
                    .DeleteProgram(std::mem::replace(&mut self.program, 0));
            }
        }
    }
}
