//! The GPU side of the renderer (Blueprint section 14): one atlas texture and one instanced-quad pipeline.
//!
//! [`GpuScene`] uploads the atlas once, takes the quads [`crate::scene::build_scene`] produced, and draws
//! them in a single draw call into whatever render pass it is given. The app hands it egui's pass through a
//! paint callback, so the map is drawn in the middle of the egui frame, clipped to the view's rectangle.
//! Nothing here decides what to draw; it is deliberately thin.

use crate::atlas::Atlas;
use crate::scene::Quad;

const SHADER: &str = r#"
struct View {
    size: vec2<f32>,
    pad: vec2<f32>,
};
@group(0) @binding(0) var<uniform> view: View;
@group(1) @binding(0) var atlas: texture_2d<f32>;
@group(1) @binding(1) var samp: sampler;

struct Vs {
    @builtin(vertex_index) vi: u32,
    @location(0) pos: vec2<f32>,
    @location(1) size: vec2<f32>,
    @location(2) uv: vec4<f32>,
    @location(3) tint: vec4<f32>,
};
struct Fs {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) tint: vec4<f32>,
};

@vertex
fn vs_main(in: Vs) -> Fs {
    var corners = array<vec2<f32>, 6>(
        vec2<f32>(0.0, 0.0), vec2<f32>(1.0, 0.0), vec2<f32>(0.0, 1.0),
        vec2<f32>(1.0, 0.0), vec2<f32>(1.0, 1.0), vec2<f32>(0.0, 1.0),
    );
    let c = corners[in.vi];
    let p = in.pos + c * in.size;
    var out: Fs;
    out.clip = vec4<f32>(p.x / view.size.x * 2.0 - 1.0, 1.0 - p.y / view.size.y * 2.0, 0.0, 1.0);
    out.uv = mix(in.uv.xy, in.uv.zw, c);
    out.tint = in.tint;
    return out;
}

@fragment
fn fs_main(in: Fs) -> @location(0) vec4<f32> {
    let t = textureSample(atlas, samp, in.uv);
    let a = t.a * in.tint.a;
    return vec4<f32>(t.rgb * in.tint.rgb * a, a);
}
"#;

/// Bytes per quad instance: position, size, four UV numbers, an RGBA tint.
const INSTANCE_BYTES: usize = 8 + 8 + 16 + 4;

pub struct GpuScene {
    pipeline: wgpu::RenderPipeline,
    atlas_group: wgpu::BindGroup,
    view_buffer: wgpu::Buffer,
    view_group: wgpu::BindGroup,
    instances: wgpu::Buffer,
    capacity: usize,
    count: u32,
    staging: Vec<u8>,
    /// Draw calls and quads of the last prepared frame, for the overlay's renderer stats.
    pub last_quads: usize,
}

impl GpuScene {
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        target_format: wgpu::TextureFormat,
        atlas: &Atlas,
    ) -> GpuScene {
        let size = wgpu::Extent3d {
            width: atlas.width as u32,
            height: atlas.height as u32,
            depth_or_array_layers: 1,
        };
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("pg_atlas"),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[wgpu::TextureFormat::Rgba8Unorm],
        });
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &atlas.rgba,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(4 * atlas.width as u32),
                rows_per_image: Some(atlas.height as u32),
            },
            size,
        );
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("pg_atlas_sampler"),
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            ..Default::default()
        });
        let view_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("pg_view_layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let atlas_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("pg_atlas_layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        multisampled: false,
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let view_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("pg_view"),
            size: 16,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let view_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("pg_view_group"),
            layout: &view_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: view_buffer.as_entire_binding(),
            }],
        });
        let atlas_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("pg_atlas_group"),
            layout: &atlas_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(
                        &texture.create_view(&wgpu::TextureViewDescriptor::default()),
                    ),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
            ],
        });
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("pg_scene_shader"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("pg_scene_layout"),
            bind_group_layouts: &[Some(&view_layout), Some(&atlas_layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("pg_scene_pipeline"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vs_main"),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: INSTANCE_BYTES as u64,
                    step_mode: wgpu::VertexStepMode::Instance,
                    attributes: &wgpu::vertex_attr_array![
                        0 => Float32x2, 1 => Float32x2, 2 => Float32x4, 3 => Unorm8x4
                    ],
                })],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: target_format,
                    blend: Some(wgpu::BlendState {
                        color: wgpu::BlendComponent {
                            src_factor: wgpu::BlendFactor::One,
                            dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                            operation: wgpu::BlendOperation::Add,
                        },
                        alpha: wgpu::BlendComponent {
                            src_factor: wgpu::BlendFactor::OneMinusDstAlpha,
                            dst_factor: wgpu::BlendFactor::One,
                            operation: wgpu::BlendOperation::Add,
                        },
                    }),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            }),
            multiview_mask: None,
            cache: None,
        });
        let capacity = 4096;
        GpuScene {
            pipeline,
            atlas_group,
            view_buffer,
            view_group,
            instances: instance_buffer(device, capacity),
            capacity,
            count: 0,
            staging: Vec::new(),
            last_quads: 0,
        }
    }

    /// Uploads this frame's quads. `scale` converts the quads' view pixels to physical pixels (the display's
    /// pixels per point); `view_px` is the view's size in physical pixels.
    pub fn prepare(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        quads: &[Quad],
        scale: f32,
        view_px: [f32; 2],
    ) {
        if quads.len() > self.capacity {
            self.capacity = quads.len().next_power_of_two();
            self.instances = instance_buffer(device, self.capacity);
        }
        self.staging.clear();
        self.staging.reserve(quads.len() * INSTANCE_BYTES);
        for q in quads {
            for v in [
                q.x * scale,
                q.y * scale,
                q.w * scale,
                q.h * scale,
                q.uv.u0,
                q.uv.v0,
                q.uv.u1,
                q.uv.v1,
            ] {
                self.staging.extend_from_slice(&v.to_le_bytes());
            }
            self.staging.extend_from_slice(&q.tint);
        }
        if !self.staging.is_empty() {
            queue.write_buffer(&self.instances, 0, &self.staging);
        }
        let mut view = Vec::with_capacity(16);
        for v in [view_px[0], view_px[1], 0.0, 0.0] {
            view.extend_from_slice(&v.to_le_bytes());
        }
        queue.write_buffer(&self.view_buffer, 0, &view);
        self.count = quads.len() as u32;
        self.last_quads = quads.len();
    }

    /// Draws the prepared quads into `pass` (one draw call).
    pub fn paint(&self, pass: &mut wgpu::RenderPass<'_>) {
        if self.count == 0 {
            return;
        }
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.view_group, &[]);
        pass.set_bind_group(1, &self.atlas_group, &[]);
        pass.set_vertex_buffer(0, self.instances.slice(..));
        pass.draw(0..6, 0..self.count);
    }
}

fn instance_buffer(device: &wgpu::Device, capacity: usize) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("pg_instances"),
        size: (capacity * INSTANCE_BYTES) as u64,
        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}
