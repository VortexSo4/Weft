//! The sky: a full screen triangle drawn first in the world pass, so everything else covers it.
//! `sky.wgsl` paints the gradient from the fog's color at the horizon to the dimension's sky
//! color overhead, the sun and moon, stars at night and clouds; a shader pack may replace it.

use weft_render::*;

use crate::gpu_layout::{Sky, bind};

pub struct SkyRenderer {
    pipeline: Option<PipelineId>,
    /// One `Sky` per frame in flight.
    bufs: Vec<BufferId>,
}

impl SkyRenderer {
    pub fn new(gpu: &mut dyn Backend, assets: &crate::assets::Assets) -> SkyRenderer {
        let mut s = SkyRenderer { pipeline: None, bufs: Vec::new() };
        s.reload(gpu, assets);
        s
    }

    pub fn reload(&mut self, gpu: &mut dyn Backend, assets: &crate::assets::Assets) {
        let made = crate::packs::compose(assets, "sky.wgsl", &[]).map_err(|e| e.0).and_then(|(s, _)| {
            // Drawn under everything: never tested against the depth, never written to it.
            let depth = DepthState { write: false, compare: Compare::Always };
            gpu.create_pipeline(&PipelineDesc { label: "sky", shader: &s, vertex: "vs", fragment: Some("fs"), color: Some(Format::Surface), depth: Some(depth), blend: Blend::None, cull_back: false, topology: Topology::Triangles })
                .map_err(|e| e.0)
        });
        match made {
            Ok(p) => {
                if let Some(old) = self.pipeline.replace(p) {
                    gpu.destroy_pipeline(old);
                }
            }
            Err(e) => log::error!("sky shader: {e}"),
        }
    }

    /// Into the open world pass, before anything else. Without a pipeline (a broken shader)
    /// the pass's clear color, the fog's, shows as before.
    pub fn draw(&mut self, gpu: &mut dyn Backend, list: &mut CommandList, frame: &FrameInfo, sky: &Sky, scene: BufferId) {
        let Some(pipe) = self.pipeline else { return };
        while self.bufs.len() < gpu.frames_in_flight() {
            self.bufs.push(gpu.create_buffer(size_of::<Sky>() as u64, usage::STORAGE | usage::HOST, "sky"));
        }
        let buf = self.bufs[frame.slot];
        gpu.mapped(buf)[..size_of::<Sky>()].copy_from_slice(bytemuck::bytes_of(sky));
        list.pipeline(pipe);
        list.bind(bind::sky::SKY, buf);
        list.bind(bind::sky::SCENE, scene);
        list.push(Cmd::Draw { vertices: 3, instances: 1, first_vertex: 0, first_instance: 0 });
    }

    pub fn destroy(&mut self, gpu: &mut dyn Backend) {
        if let Some(p) = self.pipeline.take() {
            gpu.destroy_pipeline(p);
        }
        for b in self.bufs.drain(..) {
            gpu.destroy_buffer(b);
        }
    }
}
