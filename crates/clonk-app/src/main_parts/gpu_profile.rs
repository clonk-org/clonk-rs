//! Durable retained-renderer benchmark evidence.

use super::*;
use std::collections::HashSet;

const RETAINED_GPU_PROFILE_PREFIX: &str = "LC_APP_RETAINED_GPU_PROFILE";

fn duration_ns(duration: Duration, field: &str) -> Result<u64> {
    u64::try_from(duration.as_nanos())
        .with_context(|| format!("{field} exceeds the retained GPU profile range"))
}

#[derive(Clone, Debug, serde::Serialize)]
pub(crate) struct RetainedGpuCpuStagesRecord {
    pub(crate) frame_preparation_ns: u64,
    pub(crate) validation_ns: u64,
    pub(crate) texture_synchronization_ns: u64,
    pub(crate) stream_packing_upload_ns: u64,
    pub(crate) command_encoding_ns: u64,
    pub(crate) drawable_acquisition_ns: u64,
    pub(crate) queue_submission_ns: u64,
    pub(crate) presentation_ns: u64,
    pub(crate) named_total_ns: u64,
    pub(crate) unclassified_ns: u64,
    pub(crate) overrun_ns: u64,
}

#[derive(Clone, Debug, serde::Serialize)]
pub(crate) struct RetainedGpuRendererStatsRecord {
    pub(crate) resident_source_textures: usize,
    pub(crate) created_source_textures: usize,
    pub(crate) full_upload_calls: usize,
    pub(crate) full_upload_bytes: u64,
    pub(crate) dirty_upload_calls: usize,
    pub(crate) dirty_upload_bytes: u64,
    pub(crate) draw_calls: usize,
    pub(crate) quad_draw_calls: usize,
    pub(crate) sprite_draw_calls: usize,
    pub(crate) object_sprite_draw_calls: usize,
    pub(crate) landscape_draw_calls: usize,
    pub(crate) shader_landscape_draw_calls: usize,
    pub(crate) solid_draw_calls: usize,
    pub(crate) solid_rect_draw_calls: usize,
    pub(crate) monitor_gamma_draw_calls: usize,
    pub(crate) presentation_draw_calls: usize,
    pub(crate) total_draw_calls: usize,
    pub(crate) compatible_resource_runs: usize,
    pub(crate) generic_vertices: usize,
    pub(crate) generic_vertex_upload_bytes: usize,
    pub(crate) quad_instances: usize,
    pub(crate) sprite_instances: usize,
    pub(crate) object_sprite_instances: usize,
    pub(crate) solid_rect_instances: usize,
    pub(crate) quad_instance_upload_bytes: usize,
    pub(crate) sprite_instance_upload_bytes: usize,
    pub(crate) object_sprite_upload_bytes: usize,
    pub(crate) solid_rect_upload_bytes: usize,
    pub(crate) composition_recreated: bool,
}

impl From<gpu_renderer::GpuRendererStats> for RetainedGpuRendererStatsRecord {
    fn from(stats: gpu_renderer::GpuRendererStats) -> Self {
        Self {
            resident_source_textures: stats.resident_source_textures,
            created_source_textures: stats.created_source_textures,
            full_upload_calls: stats.full_upload_calls,
            full_upload_bytes: stats.full_upload_bytes,
            dirty_upload_calls: stats.dirty_upload_calls,
            dirty_upload_bytes: stats.dirty_upload_bytes,
            draw_calls: stats.draw_calls,
            quad_draw_calls: stats.quad_draw_calls,
            sprite_draw_calls: stats.sprite_draw_calls,
            object_sprite_draw_calls: stats.object_sprite_draw_calls,
            landscape_draw_calls: stats.landscape_draw_calls,
            shader_landscape_draw_calls: stats.shader_landscape_draw_calls,
            solid_draw_calls: stats.solid_draw_calls,
            solid_rect_draw_calls: stats.solid_rect_draw_calls,
            monitor_gamma_draw_calls: stats.monitor_gamma_draw_calls,
            presentation_draw_calls: stats.presentation_draw_calls,
            total_draw_calls: stats.total_draw_calls,
            compatible_resource_runs: stats.compatible_resource_runs,
            generic_vertices: stats.generic_vertices,
            generic_vertex_upload_bytes: stats.generic_vertex_upload_bytes,
            quad_instances: stats.quad_instances,
            sprite_instances: stats.sprite_instances,
            object_sprite_instances: stats.object_sprite_instances,
            solid_rect_instances: stats.solid_rect_instances,
            quad_instance_upload_bytes: stats.quad_instance_upload_bytes,
            sprite_instance_upload_bytes: stats.sprite_instance_upload_bytes,
            object_sprite_upload_bytes: stats.object_sprite_upload_bytes,
            solid_rect_upload_bytes: stats.solid_rect_upload_bytes,
            composition_recreated: stats.composition_recreated,
        }
    }
}

#[derive(Clone, Debug, serde::Serialize)]
pub(crate) struct RetainedGpuCaptureStatsRecord {
    pub(crate) generic_sprite_fallbacks: usize,
    pub(crate) spatial_fog_fallbacks: usize,
    pub(crate) precomputed_fog_modulation_fallbacks: usize,
    pub(crate) texture_indent_fallbacks: usize,
    pub(crate) owner_mask_fallbacks: usize,
    pub(crate) physical_texture_tile_fallbacks: usize,
    pub(crate) fog_expanded_chunks: usize,
}

impl From<clonk_graphics::GpuSceneCaptureStats> for RetainedGpuCaptureStatsRecord {
    fn from(stats: clonk_graphics::GpuSceneCaptureStats) -> Self {
        Self {
            generic_sprite_fallbacks: stats.generic_sprite_fallbacks,
            spatial_fog_fallbacks: stats.spatial_fog_fallbacks,
            precomputed_fog_modulation_fallbacks: stats.precomputed_fog_modulation_fallbacks,
            texture_indent_fallbacks: stats.texture_indent_fallbacks,
            owner_mask_fallbacks: stats.owner_mask_fallbacks,
            physical_texture_tile_fallbacks: stats.physical_texture_tile_fallbacks,
            fog_expanded_chunks: stats.fog_expanded_chunks,
        }
    }
}

#[derive(Clone, Debug, serde::Serialize)]
pub(crate) struct RetainedGpuProfileFrame {
    pub(crate) sample_index: usize,
    pub(crate) end_to_end_ns: u64,
    pub(crate) timestamp_frame_id: Option<u64>,
    pub(crate) cpu: RetainedGpuCpuStagesRecord,
    pub(crate) renderer: RetainedGpuRendererStatsRecord,
    pub(crate) frontend_capture: RetainedGpuCaptureStatsRecord,
}

impl RetainedGpuProfileFrame {
    pub(crate) fn from_reconciled(
        sample_index: usize,
        profile: ReconciledRetainedGpuFrameProfile,
    ) -> Result<Self> {
        anyhow::ensure!(
            profile.has_exact_reconciliation(),
            "retained GPU CPU stages do not reconcile with the graphics duration"
        );
        let renderer = profile.raw.renderer;
        anyhow::ensure!(
            renderer.has_exact_draw_call_counts(),
            "retained GPU draw-call counters do not reconcile"
        );
        Ok(Self {
            sample_index,
            end_to_end_ns: duration_ns(profile.graphics_duration, "graphics duration")?,
            timestamp_frame_id: renderer.timestamp_frame_id,
            cpu: RetainedGpuCpuStagesRecord {
                frame_preparation_ns: duration_ns(
                    profile.raw.frame_preparation,
                    "frame preparation",
                )?,
                validation_ns: duration_ns(renderer.cpu_stages.validation, "validation")?,
                texture_synchronization_ns: duration_ns(
                    renderer.cpu_stages.texture_synchronization,
                    "texture synchronization",
                )?,
                stream_packing_upload_ns: duration_ns(
                    renderer.cpu_stages.stream_packing_upload,
                    "stream packing/upload",
                )?,
                command_encoding_ns: duration_ns(
                    renderer
                        .cpu_stages
                        .command_encoding
                        .saturating_add(profile.raw.surface.command_encoder_finalization),
                    "command encoding",
                )?,
                drawable_acquisition_ns: duration_ns(
                    profile.raw.surface.drawable_acquisition,
                    "drawable acquisition",
                )?,
                queue_submission_ns: duration_ns(
                    profile.raw.surface.queue_submission,
                    "queue submission",
                )?,
                presentation_ns: duration_ns(profile.raw.surface.presentation, "presentation")?,
                named_total_ns: duration_ns(profile.named_cpu, "named CPU total")?,
                unclassified_ns: duration_ns(profile.unclassified_cpu, "unclassified CPU")?,
                overrun_ns: duration_ns(profile.overrun_cpu, "CPU overrun")?,
            },
            renderer: renderer.into(),
            frontend_capture: profile.raw.capture.into(),
        })
    }
}

pub(crate) fn retained_gpu_profile_context_is_stable(
    profiles: &[ReconciledRetainedGpuFrameProfile],
    boundary: RetainedGpuFrameContext,
) -> bool {
    profiles
        .iter()
        .all(|profile| profile.raw.context == boundary)
}

pub(crate) fn retained_gpu_profile_machine_line<T: serde::Serialize>(
    profile: &T,
) -> Result<String> {
    Ok(format!(
        "{RETAINED_GPU_PROFILE_PREFIX} {}",
        serde_json::to_string(profile).context("serialize retained GPU profile")?
    ))
}

#[derive(Clone, Debug, serde::Serialize)]
pub(crate) struct RetainedGpuAdapterRecord {
    pub(crate) name: String,
    pub(crate) vendor_id: u32,
    pub(crate) device_id: u32,
    pub(crate) device_type: &'static str,
    pub(crate) pci_bus_id: Option<String>,
    pub(crate) driver: String,
    pub(crate) driver_info: String,
    pub(crate) backend: &'static str,
    pub(crate) subgroup_min_size: u32,
    pub(crate) subgroup_max_size: u32,
    pub(crate) transient_saves_memory: bool,
}

impl From<wgpu::AdapterInfo> for RetainedGpuAdapterRecord {
    fn from(info: wgpu::AdapterInfo) -> Self {
        let wgpu::AdapterInfo {
            name,
            vendor,
            device,
            device_type,
            device_pci_bus_id,
            driver,
            driver_info,
            backend,
            subgroup_min_size,
            subgroup_max_size,
            transient_saves_memory,
        } = info;
        let device_type = match device_type {
            wgpu::DeviceType::Other => "other",
            wgpu::DeviceType::IntegratedGpu => "integrated_gpu",
            wgpu::DeviceType::DiscreteGpu => "discrete_gpu",
            wgpu::DeviceType::VirtualGpu => "virtual_gpu",
            wgpu::DeviceType::Cpu => "cpu",
        };
        Self {
            name,
            vendor_id: vendor,
            device_id: device,
            device_type,
            pci_bus_id: (!device_pci_bus_id.is_empty()).then_some(device_pci_bus_id),
            driver,
            driver_info,
            backend: backend.to_str(),
            subgroup_min_size,
            subgroup_max_size,
            transient_saves_memory,
        }
    }
}

#[derive(Clone, Debug, serde::Serialize)]
pub(crate) struct RetainedGpuDeviceRecord {
    pub(crate) feature_bits: [u64; 2],
    pub(crate) limits_debug: String,
    pub(crate) max_texture_dimension_2d: u32,
    pub(crate) timestamp_period_ns: f32,
}

#[derive(Clone, Debug, serde::Serialize)]
pub(crate) struct RetainedGpuSurfaceRecord {
    pub(crate) format: String,
    pub(crate) present_mode: String,
    pub(crate) alpha_mode: String,
    pub(crate) surface_extent: [u32; 2],
    pub(crate) buffer_extent: [u32; 2],
}

#[derive(Clone, Debug, serde::Serialize)]
pub(crate) struct RetainedGpuRendererConfigRecord {
    pub(crate) mipmaps: bool,
    pub(crate) smooth_landscape: bool,
    pub(crate) shader_landscape: bool,
    pub(crate) landscape_detail: u32,
    pub(crate) surface_format: String,
}

#[derive(Clone, Copy, Debug, serde::Serialize)]
pub(crate) struct RetainedGpuFrontendConfigRecord {
    pub(crate) no_alpha_add: bool,
    pub(crate) no_box_fades: bool,
    pub(crate) tex_indent: i32,
    pub(crate) blit_offset: i32,
    pub(crate) allowed_blit_modes: u32,
    pub(crate) shader: bool,
    pub(crate) use_shader_gamma: bool,
    pub(crate) disable_gamma: bool,
}

impl From<clonk_frontend::AdvancedRendererConfig> for RetainedGpuFrontendConfigRecord {
    fn from(config: clonk_frontend::AdvancedRendererConfig) -> Self {
        Self {
            no_alpha_add: config.no_alpha_add,
            no_box_fades: config.no_box_fades,
            tex_indent: config.tex_indent,
            blit_offset: config.blit_offset,
            allowed_blit_modes: config.allowed_blit_modes,
            shader: config.shader,
            use_shader_gamma: config.use_shader_gamma,
            disable_gamma: config.disable_gamma,
        }
    }
}

#[derive(Clone, Copy, Debug, serde::Serialize)]
pub(crate) struct RetainedGpuPresentationRecord {
    pub(crate) physical_extent: [u32; 2],
    pub(crate) scale: f32,
    pub(crate) crop_top: u32,
}

#[derive(Clone, Debug, serde::Serialize)]
pub(crate) struct RetainedGpuFingerprintRecord {
    pub(crate) adapter: RetainedGpuAdapterRecord,
    pub(crate) adapter_feature_bits: [u64; 2],
    pub(crate) device: RetainedGpuDeviceRecord,
    pub(crate) surface: RetainedGpuSurfaceRecord,
    pub(crate) renderer: RetainedGpuRendererConfigRecord,
    pub(crate) frontend: RetainedGpuFrontendConfigRecord,
    pub(crate) presentation: RetainedGpuPresentationRecord,
}

#[derive(Clone, Copy, Debug, serde::Serialize)]
pub(crate) struct RetainedGpuTimestampStatusRecord {
    pub(crate) requested: bool,
    pub(crate) supported: bool,
    pub(crate) enabled: bool,
    pub(crate) dropped_frames: u64,
    pub(crate) readback_errors: u64,
    pub(crate) device_discontinuities: u64,
}

#[derive(Clone, Copy, Debug, serde::Serialize)]
pub(crate) struct RetainedGpuTimestampPassRecord {
    pub(crate) pass: &'static str,
    pub(crate) begin_tick: u64,
    pub(crate) end_tick: u64,
    pub(crate) duration_ns: Option<f64>,
    pub(crate) validity: &'static str,
}

impl From<gpu_renderer::GpuTimestampSample> for RetainedGpuTimestampPassRecord {
    fn from(sample: gpu_renderer::GpuTimestampSample) -> Self {
        let pass = match sample.pass {
            gpu_renderer::GpuTimestampPass::ShaderLandscape => "shader_landscape",
            gpu_renderer::GpuTimestampPass::Scene => "scene",
            gpu_renderer::GpuTimestampPass::MonitorGamma => "monitor_gamma",
            gpu_renderer::GpuTimestampPass::Presentation => "presentation",
        };
        let validity = match sample.validity {
            gpu_renderer::GpuTimestampSampleValidity::Valid => "valid",
            gpu_renderer::GpuTimestampSampleValidity::InvalidPeriod => "invalid_period",
            gpu_renderer::GpuTimestampSampleValidity::CounterRollover => "counter_rollover",
            gpu_renderer::GpuTimestampSampleValidity::InvalidDuration => "invalid_duration",
        };
        Self {
            pass,
            begin_tick: sample.begin_tick,
            end_tick: sample.end_tick,
            duration_ns: sample.duration_ns,
            validity,
        }
    }
}

#[derive(Clone, Debug, serde::Serialize)]
pub(crate) struct RetainedGpuTimestampFrameRecord {
    pub(crate) frame_id: u64,
    pub(crate) renderer_generation: u64,
    pub(crate) timestamp_period_ns: f32,
    pub(crate) passes: Vec<RetainedGpuTimestampPassRecord>,
}

impl From<gpu_renderer::GpuTimestampFrame> for RetainedGpuTimestampFrameRecord {
    fn from(frame: gpu_renderer::GpuTimestampFrame) -> Self {
        Self {
            frame_id: frame.frame_id,
            renderer_generation: frame.renderer_generation,
            timestamp_period_ns: frame.timestamp_period_ns,
            passes: frame.passes.into_iter().map(Into::into).collect(),
        }
    }
}

#[derive(Clone, Debug, serde::Serialize)]
pub(crate) struct RetainedGpuProfileArtifact {
    pub(crate) schema_version: u32,
    pub(crate) fingerprint: RetainedGpuFingerprintRecord,
    pub(crate) timestamp_queries: RetainedGpuTimestampStatusRecord,
    pub(crate) frames: Vec<RetainedGpuProfileFrame>,
    pub(crate) gpu_timestamp_frames: Vec<RetainedGpuTimestampFrameRecord>,
}

impl RetainedGpuProfileArtifact {
    pub(crate) fn from_runtime(
        report: &PresentationBenchmarkReport,
        pixels: &WindowSurface,
        renderer: &gpu_renderer::RetainedGpuRenderer,
        frontend: clonk_frontend::AdvancedRendererConfig,
        presentation: clonk_scaling::PresentationGeometry,
    ) -> Result<Self> {
        let retained_submission_count = usize::try_from(report.retained_gpu_submissions)
            .context("retained GPU submission count exceeds usize")?;
        let boundary_context =
            RetainedGpuFrameContext::capture(pixels, renderer, frontend, &presentation);
        anyhow::ensure!(
            report.cpu_submissions == 0
                && report.submissions == report.retained_gpu_submissions
                && report.retained_gpu_profiles.len() == retained_submission_count,
            "retained GPU profile requires one raw profile for every successful submission"
        );
        anyhow::ensure!(
            report.graphics_samples.len() == report.retained_gpu_profiles.len()
                && report
                    .graphics_samples
                    .iter()
                    .zip(&report.retained_gpu_profiles)
                    .all(|(duration, profile)| *duration == profile.graphics_duration),
            "retained GPU profiles disagree with the benchmark graphics samples"
        );
        anyhow::ensure!(
            retained_gpu_profile_context_is_stable(&report.retained_gpu_profiles, boundary_context,),
            "retained GPU surface, renderer, or presentation context changed during measurement"
        );
        let frames = report
            .retained_gpu_profiles
            .iter()
            .copied()
            .enumerate()
            .map(|(sample_index, profile)| {
                RetainedGpuProfileFrame::from_reconciled(sample_index, profile)
            })
            .collect::<Result<Vec<_>>>()?;
        let expected_timestamp_ids = frames
            .iter()
            .filter_map(|frame| frame.timestamp_frame_id)
            .collect::<HashSet<_>>();
        let timestamp_id_count = frames
            .iter()
            .filter(|frame| frame.timestamp_frame_id.is_some())
            .count();
        anyhow::ensure!(
            expected_timestamp_ids.len() == timestamp_id_count,
            "retained GPU profile contains duplicate timestamp frame IDs"
        );
        let mut gpu_timestamp_frames = report
            .gpu_timestamp_frames
            .iter()
            .filter(|frame| expected_timestamp_ids.contains(&frame.frame_id))
            .cloned()
            .collect::<Vec<_>>();
        gpu_timestamp_frames.sort_by_key(|frame| frame.frame_id);

        let status = pixels.timestamp_query_status();
        anyhow::ensure!(
            status.enabled == renderer.timestamp_queries_enabled(),
            "surface and retained renderer disagree about GPU timestamp enablement"
        );
        anyhow::ensure!(
            if status.enabled {
                timestamp_id_count == frames.len()
            } else {
                timestamp_id_count == 0
            },
            "retained GPU frame IDs disagree with GPU timestamp enablement"
        );
        let completed_timestamp_ids = gpu_timestamp_frames
            .iter()
            .map(|frame| frame.frame_id)
            .collect::<HashSet<_>>();
        anyhow::ensure!(
            completed_timestamp_ids.len() == gpu_timestamp_frames.len(),
            "retained GPU timestamp results contain duplicate frame IDs"
        );
        anyhow::ensure!(
            !status.enabled || completed_timestamp_ids == expected_timestamp_ids,
            "retained GPU timestamp results do not cover every measured frame"
        );
        let telemetry = renderer.timestamp_telemetry();
        let device = pixels.device();
        let adapter_feature_bits = pixels.adapter_features().bits().0;
        let device_feature_bits = device.features().bits().0;
        let presentation = RetainedGpuPresentationRecord {
            physical_extent: boundary_context.presentation_physical_extent,
            scale: f32::from_bits(boundary_context.presentation_scale_bits),
            crop_top: boundary_context.presentation_crop_top,
        };
        Ok(Self {
            schema_version: 1,
            fingerprint: RetainedGpuFingerprintRecord {
                adapter: device.adapter_info().into(),
                adapter_feature_bits,
                device: RetainedGpuDeviceRecord {
                    feature_bits: device_feature_bits,
                    limits_debug: format!("{:?}", device.limits()),
                    max_texture_dimension_2d: pixels.max_texture_dimension_2d(),
                    timestamp_period_ns: pixels.queue().get_timestamp_period(),
                },
                surface: RetainedGpuSurfaceRecord {
                    format: format!("{:?}", boundary_context.surface_format),
                    present_mode: format!("{:?}", boundary_context.present_mode),
                    alpha_mode: format!("{:?}", boundary_context.alpha_mode),
                    surface_extent: boundary_context.surface_extent,
                    buffer_extent: boundary_context.buffer_extent,
                },
                renderer: RetainedGpuRendererConfigRecord {
                    mipmaps: boundary_context.mipmaps,
                    smooth_landscape: boundary_context.smooth_landscape,
                    shader_landscape: boundary_context.shader_landscape,
                    landscape_detail: boundary_context.landscape_detail,
                    surface_format: format!("{:?}", boundary_context.renderer_surface_format),
                },
                frontend: boundary_context.frontend.into(),
                presentation,
            },
            timestamp_queries: RetainedGpuTimestampStatusRecord {
                requested: status.requested,
                supported: status.supported,
                enabled: status.enabled,
                dropped_frames: telemetry.dropped_frames,
                readback_errors: telemetry.readback_errors,
                device_discontinuities: telemetry.device_discontinuities,
            },
            frames,
            gpu_timestamp_frames: gpu_timestamp_frames.into_iter().map(Into::into).collect(),
        })
    }

    pub(crate) fn machine_line(&self) -> Result<String> {
        retained_gpu_profile_machine_line(self)
    }
}

pub(crate) fn finish_retained_gpu_profile_artifact(
    report: &mut PresentationBenchmarkReport,
    pixels: &WindowSurface,
    renderer: &mut gpu_renderer::RetainedGpuRenderer,
    frontend: clonk_frontend::AdvancedRendererConfig,
    presentation: clonk_scaling::PresentationGeometry,
) -> Result<String> {
    report
        .gpu_timestamp_frames
        .extend(renderer.drain_timestamp_frames(pixels.device())?);
    RetainedGpuProfileArtifact::from_runtime(report, pixels, renderer, frontend, presentation)?
        .machine_line()
}
