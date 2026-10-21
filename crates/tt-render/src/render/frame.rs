//! Drawing a view: the scene passes, the post path and the upscale.

use super::dynamic::Layer;
use super::meshes::ShapeMesh;
use super::shadows::atlas_tile;
use super::view::Globals;
use super::{detail_pixel_scale, Camera, Renderer, Target, View, DEPTH_FORMAT, MAX_POINT_LIGHTS};
use crate::shapes::{Blend, IFL_TICKS_PER_SECOND};
use glam::{Mat4, Vec3};
use std::path::PathBuf;

/// HDR extra: emissive surfaces (self-illuminated, additive) and sprites
/// are this much brighter, so they reach the bloom threshold.
const HDR_EMISSIVE_BOOST: f32 = 1.8;

/// Vertical field of view used by [`Renderer::render`].
pub const DEFAULT_FOV_DEG: f32 = 60.0;

impl Renderer {
    /// Screen rectangle `[x, y, w, h]` (of a `size` view) that covers every
    /// visible glass surface, for the glass copy; `None` without glass in
    /// view. Static glass (or glass around the camera) takes the whole view.
    fn glass_rect(&self, view_proj: Mat4, size: (u32, u32)) -> Option<[u32; 4]> {
        let full = Some([0, 0, size.0, size.1]);
        if let Some((mesh, _)) = self.shapes.as_ref().filter(|_| self.world) {
            if self.static_draws.iter().any(|(b, _)| mesh.batches[*b].glass) {
                return full;
            }
        }
        let (mut lo, mut hi) = (glam::Vec2::splat(f32::MAX), glam::Vec2::splat(f32::MIN));
        for l in self.dynamic.layers.iter().filter(|l| l.instances.is_some()) {
            for (path, range) in &l.draws {
                let Some(Some((mesh, _))) = self.dynamic.meshes.get(path) else { continue };
                if !mesh.batches.iter().any(|b| b.glass) {
                    continue;
                }
                for i in range.clone() {
                    let Some(m) = l.transforms.get(i as usize) else { return full };
                    let c = m.transform_point3(mesh.center);
                    let r = mesh.bounds_radius * m.x_axis.truncate().length().max(m.z_axis.truncate().length()) * 1.25 + 0.1;
                    for k in 0..8 {
                        let corner = c + Vec3::new(if k & 1 == 0 { -r } else { r }, if k & 2 == 0 { -r } else { r }, if k & 4 == 0 { -r } else { r });
                        let q = view_proj * corner.extend(1.0);
                        if q.w <= 1e-3 {
                            // Behind or around the camera: no usable bounds.
                            return full;
                        }
                        let ndc = glam::Vec2::new(q.x, q.y) / q.w;
                        lo = lo.min(ndc);
                        hi = hi.max(ndc);
                    }
                }
            }
        }
        if lo.x > hi.x || hi.x < -1.0 || lo.x > 1.0 || hi.y < -1.0 || lo.y > 1.0 {
            return None;
        }
        // NDC to pixels (y down), a few pixels of margin for the refraction.
        let (w, h) = (size.0 as f32, size.1 as f32);
        let x0 = ((lo.x.max(-1.0) * 0.5 + 0.5) * w - 4.0).floor().max(0.0) as u32;
        let x1 = ((hi.x.min(1.0) * 0.5 + 0.5) * w + 4.0).ceil().min(w) as u32;
        let y0 = ((0.5 - hi.y.min(1.0) * 0.5) * h - 4.0).floor().max(0.0) as u32;
        let y1 = ((0.5 - lo.y.max(-1.0) * 0.5) * h + 4.0).ceil().min(h) as u32;
        (x1 > x0 && y1 > y0).then_some([x0, y0, x1 - x0, y1 - y0])
    }

    /// Draws the whole target with a 60° camera, clearing it first.
    /// `time`: seconds since the scene started (drives texture animation).
    pub fn render(&mut self, target: Target, camera: &Camera, time: f32) {
        let view = View { camera: *camera, fov_deg: DEFAULT_FOV_DEG, viewport: None, clear: true };
        self.render_view(target, &view, time);
    }

    /// Draws one view into `target`.
    pub fn render_view(&mut self, target: Target, view: &View, time: f32) {
        let Target { device, queue, view: target, size: (width, height) } = target;
        if width == 0 || height == 0 {
            return;
        }
        let [vx, vy, vw, vh] = match view.viewport {
            Some([x, y, w, h]) => {
                let (x, y) = (x.min(width), y.min(height));
                [x, y, w.min(width - x), h.min(height - y)]
            }
            None => [0, 0, width, height],
        };
        if vw == 0 || vh == 0 {
            return;
        }
        let scale = self.settings.render_scale;
        let post = self.post.is_some();
        // The post path always draws into its own texture (of the scaled size).
        let scaled = scale < 0.999 && !post;
        let (sw, sh) = if scale < 0.999 { (((vw as f32 * scale).round() as u32).max(1), ((vh as f32 * scale).round() as u32).max(1)) } else { (vw, vh) };
        if let Some(p) = self.post.as_mut() {
            p.ensure(device, (sw, sh), DEPTH_FORMAT, (&self.behind_layout, &self.behind_sampler));
        }
        // Where the scene pass draws: the target's viewport, or a whole
        // offscreen texture of the scaled size.
        let (pw, ph, [px, py, pvw, pvh]) = if post {
            (sw, sh, [0, 0, sw, sh])
        } else if scaled {
            if self.scaled.as_ref().is_none_or(|s| (s.1, s.2) != (sw, sh)) {
                let tex = device.create_texture(&wgpu::TextureDescriptor {
                    label: Some("scaled view"),
                    size: wgpu::Extent3d { width: sw, height: sh, depth_or_array_layers: 1 },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: self.format,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                });
                let tv = tex.create_view(&Default::default());
                let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("blit"),
                    layout: &self.blit.2,
                    entries: &[
                        wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&tv) },
                        wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&self.blit.3) },
                    ],
                });
                self.scaled = Some((tv, sw, sh, group));
            }
            (sw, sh, [0, 0, sw, sh])
        } else {
            (width, height, [vx, vy, vw, vh])
        };
        let full = post || scaled || [vx, vy, vw, vh] == [0, 0, width, height];
        let depth_index = if post {
            0
        } else {
            match self.depth.iter().position(|d| (d.1, d.2) == (pw, ph)) {
                Some(i) => i,
                None => {
                    let tex = device.create_texture(&wgpu::TextureDescriptor {
                        label: Some("depth"),
                        size: wgpu::Extent3d { width: pw, height: ph, depth_or_array_layers: 1 },
                        mip_level_count: 1,
                        sample_count: 1,
                        dimension: wgpu::TextureDimension::D2,
                        format: DEPTH_FORMAT,
                        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
                        view_formats: &[],
                    });
                    if self.depth.len() >= 3 {
                        self.depth.remove(0);
                    }
                    let sample = tex.create_view(&wgpu::TextureViewDescriptor { aspect: wgpu::TextureAspect::DepthOnly, ..Default::default() });
                    self.depth.push((tex.create_view(&Default::default()), pw, ph, sample));
                    self.depth.len() - 1
                }
            }
        };
        let camera = &view.camera;
        let pixel_scale = detail_pixel_scale((vw, vh), view.fov_deg);
        if self.world {
            self.select_static_details(queue, camera.pos, pixel_scale);
        }
        let forward = camera.forward();
        let right = forward.cross(Vec3::Z).try_normalize().unwrap_or(Vec3::X);
        let up = right.cross(forward).normalize();
        let e = self.settings.extras;
        let g = Globals {
            view_proj: camera.view_proj_near(vw as f32 / vh as f32, self.near, self.far, view.fov_deg).to_cols_array_2d(),
            camera: camera.pos.extend(1.0).into(),
            sun_dir: self.sun_dir,
            sun_color: self.sun_color,
            ambient: self.ambient,
            fog_color: self.fog_color,
            params: self.scene_params,
            time: [time, IFL_TICKS_PER_SECOND, self.dynamic.layers[0].casters.len() as f32, self.near.max(1e-3)],
            terrain: self.terrain_params,
            fog_volumes: self.fog_volumes,
            cam_right: right.extend(0.0).into(),
            cam_up: up.extend(0.0).into(),
            cam_forward: forward.extend(0.0).into(),
            extras: [
                if e.normal_maps { e.normal_strength.clamp(0.0, 2.0) } else { 0.0 },
                if e.glass && post { 1.0 } else { 0.0 },
                if e.hdr && post { HDR_EMISSIVE_BOOST } else { 1.0 },
                if e.hdr && post { 1.0 } else { 0.0 },
            ],
            light_info: [self.point_lights.len() as f32, atlas_tile(e.shadows) as f32, e.shadows.min(2) as f32, self.far],
            lights: {
                let mut l = [[0.0; 4]; 2 * MAX_POINT_LIGHTS];
                for (k, p) in self.point_lights.iter().enumerate() {
                    l[2 * k] = [p.pos.x, p.pos.y, p.pos.z, p.radius.max(0.1)];
                    l[2 * k + 1] = [p.color[0], p.color[1], p.color[2], 0.0];
                }
                l
            },
        };
        queue.write_buffer(&self.globals, 0, bytemuck::bytes_of(&g));
        let sprites = self.sprite_layers.iter().flatten();
        self.sprites.prepare(device, queue, sprites, camera.pos, forward);

        let load = if view.clear && full {
            wgpu::LoadOp::Clear(self.clear)
        } else if scaled || post {
            // Kept background: render over transparency, blend in the blit.
            wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT)
        } else {
            wgpu::LoadOp::Load
        };
        let mut enc = device.create_command_encoder(&Default::default());
        if let Some(t) = self.timer.get_mut().as_mut() {
            t.begin(device, &mut enc);
        }
        let mark = |enc: &mut wgpu::CommandEncoder, label: &'static str| {
            if let Some(t) = self.timer.borrow_mut().as_mut() {
                t.mark(enc, label);
            }
        };
        self.draw_shadow_atlas(&mut enc);
        mark(&mut enc, "shadows");
        let (color_view, depth_view, depth_sample) = match self.post.as_ref().and_then(|p| p.targets.as_ref()) {
            Some(t) => (&t.color, &t.depth, &t.depth_sample),
            None => (if scaled { &self.scaled.as_ref().unwrap().0 } else { target }, &self.depth[depth_index].0, &self.depth[depth_index].3),
        };
        // Glass needs the opaque scene in a texture: the blended surfaces
        // then go into a second pass.
        // Only when some glass is in view, and only its screen rectangle is
        // copied.
        let glass = if post && e.glass { self.glass_rect(Mat4::from_cols_array_2d(&g.view_proj), (pw, ph)) } else { None };
        // Soft sprites sample the opaque scene's depth: then the blended
        // surfaces go into a second pass with the depth read-only.
        let soft = self.sprites.len() > 0;
        if soft {
            self.sprites.set_scene_depth(device, depth_sample);
        }
        let split = glass.is_some() || soft;
        let begin = |enc: &mut wgpu::CommandEncoder, first: bool| -> wgpu::RenderPass<'static> {
            let (cl, dl, sl) = if first { (load, wgpu::LoadOp::Clear(1.0), wgpu::LoadOp::Clear(0)) } else { (wgpu::LoadOp::Load, wgpu::LoadOp::Load, wgpu::LoadOp::Load) };
            enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some(if first { "scene" } else { "scene (blended)" }),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: color_view, resolve_target: None, ops: wgpu::Operations { load: cl, store: wgpu::StoreOp::Store } })],
                // The depth buffer is private to the renderer, so clearing all
                // of it is fine for a viewport too. The blended pass only
                // tests it (read-only, so sprites can sample it).
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: depth_view,
                    depth_ops: first.then_some(wgpu::Operations { load: dl, store: wgpu::StoreOp::Store }),
                    stencil_ops: first.then_some(wgpu::Operations { load: sl, store: wgpu::StoreOp::Store }),
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
            })
            .forget_lifetime()
        };
        let p = &self.shape_pipelines;
        let layers = &self.dynamic.layers;
        let bind_mesh = |pass: &mut wgpu::RenderPass, l: &Layer, mesh: &ShapeMesh| {
            pass.set_bind_group(2, l.poses.as_ref().map_or(&self.empty_poses, |p| &p.1), &[]);
            pass.set_bind_group(1, &mesh.group, &[]);
            pass.set_vertex_buffer(0, mesh.vb.slice(..));
            pass.set_vertex_buffer(1, l.instances.as_ref().unwrap().slice(..));
            pass.set_index_buffer(mesh.ib.slice(..), wgpu::IndexFormat::Uint32);
        };
        let draw_static = |pass: &mut wgpu::RenderPass, blend: Blend| {
            if let Some((mesh, instances)) = self.shapes.as_ref().filter(|_| self.world) {
                pass.set_bind_group(2, &self.empty_poses, &[]);
                pass.set_bind_group(1, &mesh.group, &[]);
                pass.set_vertex_buffer(0, mesh.vb.slice(..));
                pass.set_vertex_buffer(1, instances.slice(..));
                pass.set_index_buffer(mesh.ib.slice(..), wgpu::IndexFormat::Uint32);
                for (b, range) in &self.static_draws {
                    let b = &mesh.batches[*b];
                    if b.blend == blend {
                        pass.draw_indexed(b.indices.clone(), 0, range.clone());
                    }
                }
            }
        };
        let setup = |pass: &mut wgpu::RenderPass, behind: &wgpu::BindGroup| {
            pass.set_viewport(px as f32, py as f32, pvw as f32, pvh as f32, 0.0, 1.0);
            pass.set_scissor_rect(px, py, pvw, pvh);
            pass.set_bind_group(0, &self.bind_group, &[]);
            pass.set_bind_group(3, behind, &[]);
        };
        // Blended static surfaces, then blended dynamic shapes and sprites
        // together, far to near.
        let blended = |pass: &mut wgpu::RenderPass| {
            pass.set_pipeline(&p.translucent);
            draw_static(pass, Blend::Translucent);
            pass.set_pipeline(&p.additive);
            draw_static(pass, Blend::Additive);
            let mut items: Vec<(f32, usize, &PathBuf, u32)> = Vec::new();
            for (li, l) in layers.iter().enumerate().filter(|(_, l)| l.instances.is_some()) {
                for (path, range) in &l.draws {
                    let Some(Some((mesh, _))) = self.dynamic.meshes.get(path) else { continue };
                    if mesh.batches.iter().any(|b| b.blend != Blend::Opaque) {
                        for i in range.clone() {
                            let d = l.positions.get(i as usize).map_or(0.0, |p| p.distance(camera.pos));
                            items.push((d, li, path, i));
                        }
                    }
                }
            }
            items.sort_by(|a, b| b.0.total_cmp(&a.0));
            let mut next_sprite = 0;
            for (d, li, path, i) in items {
                let mut end = next_sprite;
                while end < self.sprites.len() && self.sprites.distance(end) > d {
                    end += 1;
                }
                self.sprites.draw_range(pass, next_sprite..end);
                next_sprite = end;
                let l = &layers[li];
                let Some(Some((mesh, _))) = self.dynamic.meshes.get(path) else { continue };
                bind_mesh(pass, l, mesh);
                let level = self.dynamic_detail(mesh, l, i, camera.pos, pixel_scale);
                for (blend, pipeline) in [(Blend::Translucent, &p.translucent), (Blend::Additive, &p.additive)] {
                    let mut batches = mesh.batches.iter().filter(|b| b.blend == blend && b.detail == level).peekable();
                    if batches.peek().is_some() {
                        pass.set_pipeline(pipeline);
                        for b in batches {
                            pass.draw_indexed(b.indices.clone(), 0, i..i + 1);
                        }
                    }
                }
            }
            self.sprites.draw_range(pass, next_sprite..self.sprites.len());
        };
        {
            let mut pass = begin(&mut enc, true);
            setup(&mut pass, &self.behind_dummy);
            if view.clear && !full {
                pass.set_pipeline(&self.clear_pipeline);
                pass.draw(0..3, 0..1);
            }
            // The terrain's depth, then the opaque shapes (tested against
            // it), and only then the sky and the shaded terrain where they
            // are still the nearest: the same picture as drawing sky,
            // terrain and shapes in that order (ties too: the terrain's
            // depth is first), without shading hidden terrain or sky.
            if self.world {
                pass.set_pipeline(&self.terrain_depth_pipeline);
                pass.set_vertex_buffer(0, self.terrain_vb.slice(..));
                pass.set_index_buffer(self.terrain_ib.slice(..), wgpu::IndexFormat::Uint32);
                pass.draw_indexed(0..self.terrain_index_count, 0, 0..1);
            }
            // Opaque geometry of all shapes (dynamic shadows are part of the
            // terrain shader).
            pass.set_pipeline(&p.opaque);
            draw_static(&mut pass, Blend::Opaque);
            for l in layers.iter().filter(|l| l.instances.is_some()) {
                for (path, range) in &l.draws {
                    let Some(Some((mesh, _))) = self.dynamic.meshes.get(path) else { continue };
                    if mesh.batches.iter().any(|b| b.blend == Blend::Opaque) {
                        bind_mesh(&mut pass, l, mesh);
                        for i in range.clone() {
                            let level = self.dynamic_detail(mesh, l, i, camera.pos, pixel_scale);
                            for b in mesh.batches.iter().filter(|b| b.blend == Blend::Opaque && b.detail == level) {
                                pass.draw_indexed(b.indices.clone(), 0, i..i + 1);
                            }
                        }
                    }
                }
            }
            if let Some((pipeline, group, vb, count)) = self.sky.as_ref().filter(|_| self.world) {
                pass.set_pipeline(pipeline);
                pass.set_bind_group(1, group, &[]);
                pass.set_vertex_buffer(0, vb.slice(..));
                pass.draw(0..*count, 0..1);
            }
            if self.world {
                pass.set_pipeline(&self.terrain_pipeline);
                pass.set_vertex_buffer(0, self.terrain_vb.slice(..));
                pass.set_index_buffer(self.terrain_ib.slice(..), wgpu::IndexFormat::Uint32);
                pass.draw_indexed(0..self.terrain_index_count, 0, 0..1);
            }
            if self.world && !self.dynamic.layers[0].shadow_draws.is_empty() {
                pass.set_pipeline(&self.terrain_shadow_pipeline);
                for (caster, range) in &self.dynamic.layers[0].shadow_draws {
                    pass.draw_indexed(range.clone(), 0, *caster..*caster + 1);
                }
            }
            if !split {
                blended(&mut pass);
            }
        }
        mark(&mut enc, "scene");
        if split {
            let mut behind = &self.behind_dummy;
            if let (Some(post), Some(rect)) = (self.post.as_ref(), glass) {
                post.copy_behind(&mut enc, rect);
                mark(&mut enc, "glass copy");
                behind = post.targets.as_ref().and_then(|t| t.behind_group.as_ref()).unwrap_or(&self.behind_dummy);
            }
            {
                let mut pass = begin(&mut enc, false);
                setup(&mut pass, behind);
                blended(&mut pass);
            }
            mark(&mut enc, "blended");
        }
        if let Some(post) = self.post.as_ref() {
            let tan_y = (view.fov_deg.to_radians() * 0.5).tan();
            let params = crate::post::ViewParams {
                near: self.near,
                far: self.far,
                tan_half_x: tan_y * vw as f32 / vh as f32,
                tan_half_y: tan_y,
                visible: self.scene_params[1],
                fog_distance: self.scene_params[0],
                cam_z: view.camera.pos.z,
                up: [right.z, up.z, -forward.z],
                vols: self.fog_volumes,
            };
            post.run(queue, &mut enc, crate::post::Output { target, viewport: [vx, vy, vw, vh], blend: !view.clear }, params, &mark);
        }
        if scaled {
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("upscale"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target,
                    resolve_target: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            pass.set_viewport(vx as f32, vy as f32, vw as f32, vh as f32, 0.0, 1.0);
            pass.set_scissor_rect(vx, vy, vw, vh);
            pass.set_pipeline(if view.clear { &self.blit.0 } else { &self.blit.1 });
            pass.set_bind_group(0, &self.scaled.as_ref().unwrap().3, &[]);
            pass.draw(0..3, 0..1);
            drop(pass);
            mark(&mut enc, "upscale");
        }
        let mut timer = self.timer.borrow_mut();
        if let Some(t) = timer.as_mut() {
            t.end(&mut enc);
        }
        queue.submit([enc.finish()]);
        if let Some(t) = timer.as_mut() {
            t.submitted();
        }
    }
}
