//! egui → sokol_gfx painter, the same on D3D11 and Metal (N16; measured in M0 as variant B).
//!
//! * Shader: sokol_imgui's precompiled blobs (hlsl4 / metal_macos). They fit egui: position and
//!   display size in points, fragment = texture × vertex colour. The uniform is
//!   `{ disp_size, gamma, pad }`; gamma must be 1.0 (the shader raises the colour to that power).
//! * Pipeline: egui's `Vertex` layout (pos f32×2, uv f32×2, colour u8×4, stride 20), 32-bit
//!   indices, premultiplied-alpha blending as egui_glow / egui-wgpu. Into a UNORM target this is
//!   egui-wgpu's own gamma-framebuffer path; M0 found the two pixel-identical.
//! * Buffers: `write_transient`; every mesh is written at its own offset before the first bind.
//! * Textures: sokol updates an image only whole and once per frame, so each egui texture keeps a
//!   CPU copy; partial deltas patch the copy and the whole image is uploaded once. The copies also
//!   let [`Painter::rebuild`] recreate everything on a new device.

use std::collections::HashMap;
use std::ffi::c_void;

use egui::epaint::{ImageDelta, Primitive, Vertex};
use egui::{TextureFilter, TextureId, TextureOptions, TextureWrapMode, TexturesDelta};
use sokol_gfx::gfx as sg;

const _: () = assert!(
    std::mem::size_of::<Vertex>() == 20,
    "egui vertices must keep their default layout (pos, uv, colour: 20 bytes)"
);

#[cfg(target_os = "macos")]
mod blobs {
    pub const VS: &[u8] = include_bytes!("shaders/vs_bytecode_metal_macos.bin");
    pub const FS: &[u8] = include_bytes!("shaders/fs_bytecode_metal_macos.bin");
    pub const ENTRY: Option<&std::ffi::CStr> = Some(c"main0");
}
#[cfg(not(target_os = "macos"))]
mod blobs {
    pub const VS: &[u8] = include_bytes!("shaders/vs_bytecode_hlsl4.bin");
    pub const FS: &[u8] = include_bytes!("shaders/fs_bytecode_hlsl4.bin");
    pub const ENTRY: Option<&std::ffi::CStr> = None;
}

#[repr(C)]
struct VsParams {
    disp_size: [f32; 2],
    gamma: f32,
    _pad: f32,
}

/// A sampler's identity: (mag, min, wrap).
type SamplerKey = (u8, u8, u8);

struct Texture {
    image: sg::Image,
    view: sg::View,
    sampler: SamplerKey,
    size: [usize; 2],
    /// RGBA8 premultiplied, row-major; patched by partial deltas, uploaded whole.
    pixels: Vec<u8>,
    dirty: bool,
}

struct Draw {
    texture: TextureId,
    /// Scissor in physical pixels, origin top-left.
    clip: [i32; 4],
    vertex_offset: usize,
    index_offset: usize,
    index_count: usize,
}

pub struct Painter {
    color_format: sg::PixelFormat,
    pipeline: sg::Pipeline,
    vbuf: sg::Buffer,
    ibuf: sg::Buffer,
    vcap: usize,
    icap: usize,
    textures: HashMap<TextureId, Texture>,
    samplers: HashMap<SamplerKey, sg::Sampler>,
    draws: Vec<Draw>,
}

fn range<T>(data: &[T]) -> sg::Range {
    sg::Range {
        ptr: data.as_ptr() as *const c_void,
        size: std::mem::size_of_val(data),
    }
}

fn make_buffer(size: usize, index: bool) -> sg::Buffer {
    let mut d = sg::BufferDesc::new();
    d.size = size;
    d.usage.vertex_buffer = !index;
    d.usage.index_buffer = index;
    d.usage.write_transient = true;
    sg::make_buffer(&d)
}

fn make_pipeline(color_format: sg::PixelFormat) -> sg::Pipeline {
    let mut shd = sg::ShaderDesc::new();
    shd.vertex_func.bytecode = range(blobs::VS);
    shd.fragment_func.bytecode = range(blobs::FS);
    if let Some(entry) = blobs::ENTRY {
        shd.vertex_func.entry = entry.as_ptr();
        shd.fragment_func.entry = entry.as_ptr();
    }
    for (i, name) in [c"position", c"texcoord0", c"color0"]
        .into_iter()
        .enumerate()
    {
        shd.attrs[i].glsl_name = name.as_ptr();
        shd.attrs[i].hlsl_sem_name = c"TEXCOORD".as_ptr();
        shd.attrs[i].hlsl_sem_index = i as u8;
        shd.attrs[i].base_type = sg::ShaderAttrBaseType::Float;
    }
    let ub = &mut shd.uniform_blocks[0];
    ub.stage = sg::ShaderStage::Vertex;
    ub.size = std::mem::size_of::<VsParams>() as u32;
    ub.hlsl_register_b_n = 0;
    ub.msl_buffer_n = 0;
    ub.glsl_uniforms[0].glsl_name = c"vs_params".as_ptr();
    ub.glsl_uniforms[0]._type = sg::UniformType::Float4;
    ub.glsl_uniforms[0].array_count = 1;
    let tv = &mut shd.views[0].texture;
    tv.stage = sg::ShaderStage::Fragment;
    tv.image_type = sg::ImageType::Dim2;
    tv.sample_type = sg::ImageSampleType::Float;
    tv.hlsl_register_t_n = 0;
    tv.msl_texture_n = 0;
    shd.samplers[0].stage = sg::ShaderStage::Fragment;
    shd.samplers[0].sampler_type = sg::SamplerType::Filtering;
    shd.samplers[0].hlsl_register_s_n = 0;
    shd.samplers[0].msl_sampler_n = 0;
    shd.texture_sampler_pairs[0].stage = sg::ShaderStage::Fragment;
    shd.texture_sampler_pairs[0].view_slot = 0;
    shd.texture_sampler_pairs[0].sampler_slot = 0;
    shd.texture_sampler_pairs[0].glsl_name = c"tex_smp".as_ptr();
    let shader = sg::make_shader(&shd);

    let mut pip = sg::PipelineDesc::new();
    pip.shader = shader;
    pip.layout.buffers[0].stride = 20;
    pip.layout.attrs[0].offset = 0;
    pip.layout.attrs[0].format = sg::VertexFormat::Float2;
    pip.layout.attrs[1].offset = 8;
    pip.layout.attrs[1].format = sg::VertexFormat::Float2;
    pip.layout.attrs[2].offset = 16;
    pip.layout.attrs[2].format = sg::VertexFormat::Ubyte4n;
    pip.index_type = sg::IndexType::Uint32;
    pip.depth.pixel_format = sg::PixelFormat::None;
    pip.sample_count = 1;
    pip.colors[0].pixel_format = color_format;
    let blend = &mut pip.colors[0].blend;
    blend.enabled = true;
    blend.src_factor_rgb = sg::BlendFactor::One;
    blend.dst_factor_rgb = sg::BlendFactor::OneMinusSrcAlpha;
    blend.src_factor_alpha = sg::BlendFactor::OneMinusDstAlpha;
    blend.dst_factor_alpha = sg::BlendFactor::One;
    sg::make_pipeline(&pip)
}

fn sampler_key(o: TextureOptions) -> SamplerKey {
    let filter = |f: TextureFilter| match f {
        TextureFilter::Nearest => 0u8,
        TextureFilter::Linear => 1u8,
    };
    let wrap = match o.wrap_mode {
        TextureWrapMode::ClampToEdge => 0u8,
        TextureWrapMode::Repeat => 1u8,
        TextureWrapMode::MirroredRepeat => 2u8,
    };
    (filter(o.magnification), filter(o.minification), wrap)
}

fn make_sampler(key: SamplerKey) -> sg::Sampler {
    let filter = |f: u8| {
        if f == 0 {
            sg::Filter::Nearest
        } else {
            sg::Filter::Linear
        }
    };
    let wrap = match key.2 {
        0 => sg::Wrap::ClampToEdge,
        1 => sg::Wrap::Repeat,
        _ => sg::Wrap::MirroredRepeat,
    };
    let mut d = sg::SamplerDesc::new();
    d.mag_filter = filter(key.0);
    d.min_filter = filter(key.1);
    d.wrap_u = wrap;
    d.wrap_v = wrap;
    sg::make_sampler(&d)
}

fn make_image(size: [usize; 2]) -> (sg::Image, sg::View) {
    let mut desc = sg::ImageDesc::new();
    desc.width = size[0] as i32;
    desc.height = size[1] as i32;
    desc.pixel_format = sg::PixelFormat::Rgba8;
    desc.usage.dynamic_update = true;
    let image = sg::make_image(&desc);
    let mut vd = sg::ViewDesc::new();
    vd.texture.image = image;
    (image, sg::make_view(&vd))
}

const INITIAL_BUFFER_BYTES: usize = 256 * 1024;

impl Painter {
    /// Build the shader, pipeline and buffers. Call after `sg::setup`.
    pub fn new(color_format: sg::PixelFormat) -> Painter {
        Painter {
            color_format,
            pipeline: make_pipeline(color_format),
            vbuf: make_buffer(INITIAL_BUFFER_BYTES, false),
            ibuf: make_buffer(INITIAL_BUFFER_BYTES, true),
            vcap: INITIAL_BUFFER_BYTES,
            icap: INITIAL_BUFFER_BYTES,
            textures: HashMap::new(),
            samplers: HashMap::new(),
            draws: Vec::new(),
        }
    }

    /// Recreate every GPU object after `sg::shutdown` + `sg::setup` on another device. The old
    /// handles died with the old sokol context; textures come back from their CPU copies.
    pub fn rebuild(&mut self) {
        self.pipeline = make_pipeline(self.color_format);
        self.vbuf = make_buffer(self.vcap, false);
        self.ibuf = make_buffer(self.icap, true);
        self.samplers.clear();
        for t in self.textures.values_mut() {
            let (image, view) = make_image(t.size);
            t.image = image;
            t.view = view;
            t.dirty = true;
        }
        self.upload_dirty();
    }

    fn sampler(&mut self, key: SamplerKey) -> sg::Sampler {
        *self
            .samplers
            .entry(key)
            .or_insert_with(|| make_sampler(key))
    }

    /// Apply this frame's texture deltas (CPU copies) and upload every changed image once.
    pub fn set_textures(&mut self, delta: &TexturesDelta) {
        for (id, deltas) in &delta.set {
            for d in deltas {
                self.apply_delta(*id, d);
            }
        }
        self.upload_dirty();
    }

    fn apply_delta(&mut self, id: TextureId, d: &ImageDelta) {
        let egui::ImageData::Color(img) = &d.image;
        // SAFETY: Color32 is 4 plain bytes (RGBA, premultiplied).
        let bytes: &[u8] = unsafe {
            std::slice::from_raw_parts(img.pixels.as_ptr() as *const u8, img.pixels.len() * 4)
        };
        let key = sampler_key(d.options);
        match d.pos {
            None => {
                if self.textures.get(&id).is_none_or(|t| t.size != img.size) {
                    if let Some(old) = self.textures.remove(&id) {
                        sg::destroy_view(old.view);
                        sg::destroy_image(old.image);
                    }
                    let (image, view) = make_image(img.size);
                    self.textures.insert(
                        id,
                        Texture {
                            image,
                            view,
                            sampler: key,
                            size: img.size,
                            pixels: Vec::new(),
                            dirty: true,
                        },
                    );
                }
                if let Some(t) = self.textures.get_mut(&id) {
                    t.sampler = key;
                    t.pixels.clear();
                    t.pixels.extend_from_slice(bytes);
                    t.dirty = true;
                }
            }
            Some([x0, y0]) => {
                let Some(t) = self.textures.get_mut(&id) else {
                    return;
                };
                let row = img.size[0] * 4;
                for y in 0..img.size[1] {
                    let dst = ((y0 + y) * t.size[0] + x0) * 4;
                    if dst + row <= t.pixels.len() {
                        t.pixels[dst..dst + row].copy_from_slice(&bytes[y * row..(y + 1) * row]);
                    }
                }
                t.dirty = true;
            }
        }
    }

    fn upload_dirty(&mut self) {
        for t in self.textures.values_mut().filter(|t| t.dirty) {
            let mut data = sg::ImageData::new();
            data.mip_levels[0] = range(&t.pixels);
            sg::update_image(t.image, &data);
            t.dirty = false;
        }
    }

    /// Write every mesh into the transient buffers and record the draws. Before `begin_pass`.
    pub fn prepare(&mut self, primitives: &[egui::ClippedPrimitive], ppp: f32, size_px: [u32; 2]) {
        self.draws.clear();
        let meshes = || {
            primitives.iter().filter_map(|p| match &p.primitive {
                Primitive::Mesh(m) if !m.indices.is_empty() => Some((p.clip_rect, m)),
                _ => None,
            })
        };
        let vbytes: usize = meshes().map(|(_, m)| m.vertices.len() * 20).sum();
        let ibytes: usize = meshes().map(|(_, m)| m.indices.len() * 4).sum();
        if vbytes > self.vcap {
            sg::destroy_buffer(self.vbuf);
            self.vcap = vbytes.next_power_of_two();
            self.vbuf = make_buffer(self.vcap, false);
        }
        if ibytes > self.icap {
            sg::destroy_buffer(self.ibuf);
            self.icap = ibytes.next_power_of_two();
            self.ibuf = make_buffer(self.icap, true);
        }
        let (mut voff, mut ioff) = (0usize, 0usize);
        let (w, h) = (size_px[0] as f32, size_px[1] as f32);
        for (clip, mesh) in meshes() {
            let x0 = (clip.min.x * ppp).round().clamp(0.0, w);
            let y0 = (clip.min.y * ppp).round().clamp(0.0, h);
            let x1 = (clip.max.x * ppp).round().clamp(x0, w);
            let y1 = (clip.max.y * ppp).round().clamp(y0, h);
            if x1 - x0 < 1.0 || y1 - y0 < 1.0 {
                continue;
            }
            sg::write_buffer_transient(&sg::WriteBufferDesc {
                src: sg::WriteBufferSource {
                    data: range(&mesh.vertices),
                    offset: 0,
                },
                dst: sg::BufferLocation {
                    buffer: self.vbuf,
                    offset: voff,
                },
                size: 0,
            });
            sg::write_buffer_transient(&sg::WriteBufferDesc {
                src: sg::WriteBufferSource {
                    data: range(&mesh.indices),
                    offset: 0,
                },
                dst: sg::BufferLocation {
                    buffer: self.ibuf,
                    offset: ioff,
                },
                size: 0,
            });
            self.draws.push(Draw {
                texture: mesh.texture_id,
                clip: [x0 as i32, y0 as i32, (x1 - x0) as i32, (y1 - y0) as i32],
                vertex_offset: voff,
                index_offset: ioff,
                index_count: mesh.indices.len(),
            });
            voff += mesh.vertices.len() * 20;
            ioff += mesh.indices.len() * 4;
        }
    }

    /// Record the draws. Inside the swapchain pass.
    pub fn draw(&mut self, ppp: f32, size_px: [u32; 2]) {
        sg::apply_pipeline(self.pipeline);
        let params = VsParams {
            disp_size: [size_px[0] as f32 / ppp, size_px[1] as f32 / ppp],
            gamma: 1.0,
            _pad: 0.0,
        };
        sg::apply_uniforms(0, &range(std::slice::from_ref(&params)));
        for i in 0..self.draws.len() {
            let d = &self.draws[i];
            let Some((view, key)) = self.textures.get(&d.texture).map(|t| (t.view, t.sampler))
            else {
                continue;
            };
            let (clip, voff, ioff, count) =
                (d.clip, d.vertex_offset, d.index_offset, d.index_count);
            let sampler = self.sampler(key);
            sg::apply_scissor_rect(clip[0], clip[1], clip[2], clip[3], true);
            let mut b = sg::Bindings::new();
            b.vertex_buffers[0] = self.vbuf;
            b.vertex_buffer_offsets[0] = voff as i32;
            b.index_buffer = self.ibuf;
            b.index_buffer_offset = ioff as i32;
            b.views[0] = view;
            b.samplers[0] = sampler;
            sg::apply_bindings(&b);
            sg::draw(0, count, 1);
        }
    }

    /// Release the textures egui freed this frame. After the frame is committed.
    pub fn free_textures(&mut self, delta: &TexturesDelta) {
        for id in &delta.free {
            if let Some(t) = self.textures.remove(id) {
                sg::destroy_view(t.view);
                sg::destroy_image(t.image);
            }
        }
    }
}
