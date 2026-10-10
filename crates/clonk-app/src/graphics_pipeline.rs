//! Execute owned graphics projections while the application owns simulation.
//!
//! One presentation can be in flight. Engine, controls and synchronized RNG
//! remain on the application thread; the worker holds no landscape store.

use crate::*;
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::thread;

type Job<R> = Box<dyn FnOnce() -> R + Send>;

/// A single presentation in flight. Completed work still occupies the slot
/// until the application consumes it, so stale frames cannot queue up.
pub(crate) struct FrameWorker<R> {
    jobs: Option<SyncSender<Job<R>>>,
    results: Receiver<thread::Result<R>>,
    thread: Option<thread::JoinHandle<()>>,
    pending: bool,
}
impl<R: Send + 'static> FrameWorker<R> {
    pub(crate) fn new(wake: impl Fn() + Send + 'static) -> std::io::Result<Self> {
        let (jobs, input) = mpsc::sync_channel::<Job<R>>(1);
        let (output, results) = mpsc::sync_channel(1);
        let thread = thread::Builder::new()
            .name("clonk-graphics".into())
            .spawn(move || {
                while let Ok(job) = input.recv() {
                    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(job));
                    if output.send(result).is_err() {
                        break;
                    }
                    wake();
                }
            })?;
        Ok(Self {
            jobs: Some(jobs),
            results,
            thread: Some(thread),
            pending: false,
        })
    }
    pub(crate) fn pending(&self) -> bool {
        self.pending
    }
    pub(crate) fn submit(
        &mut self,
        render: impl FnOnce() -> R + Send + 'static,
    ) -> std::io::Result<()> {
        if self.pending {
            return Err(std::io::Error::new(
                std::io::ErrorKind::WouldBlock,
                "a presentation is already in flight",
            ));
        }
        self.jobs
            .as_ref()
            .ok_or_else(|| std::io::Error::other("graphics worker stopped"))?
            .send(Box::new(render))
            .map_err(|_| std::io::Error::other("graphics worker disconnected"))?;
        self.pending = true;
        Ok(())
    }
    pub(crate) fn try_finish(&mut self) -> std::io::Result<Option<R>> {
        if !self.pending {
            return Ok(None);
        }
        match self.results.try_recv() {
            Ok(result) => Ok(Some(self.complete(result))),
            Err(mpsc::TryRecvError::Empty) => Ok(None),
            Err(mpsc::TryRecvError::Disconnected) => {
                Err(std::io::Error::other("graphics worker disconnected"))
            }
        }
    }
    pub(crate) fn finish(&mut self) -> std::io::Result<Option<R>> {
        if !self.pending {
            return Ok(None);
        }
        let result = self
            .results
            .recv()
            .map_err(|_| std::io::Error::other("graphics worker disconnected"))?;
        Ok(Some(self.complete(result)))
    }
    fn complete(&mut self, result: thread::Result<R>) -> R {
        self.pending = false;
        result.unwrap_or_else(|payload| std::panic::resume_unwind(payload))
    }
}
impl<R> Drop for FrameWorker<R> {
    fn drop(&mut self) {
        self.jobs.take();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

#[cfg(all(
    test,
    any(not(feature = "app-test-shard-mode"), feature = "app-test-shard-5"),
))]
mod tests {
    use super::*;
    #[test]
    fn graphics_executes_on_another_thread() {
        let mut worker = FrameWorker::new(|| {}).unwrap();
        worker.submit(|| thread::current().id()).unwrap();
        assert_ne!(worker.finish().unwrap().unwrap(), thread::current().id());
    }
    #[test]
    fn slow_graphics_does_not_block_input_and_cannot_queue_another_frame() {
        let (started_tx, started_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let (wake_tx, wake_rx) = mpsc::channel();
        let mut worker = FrameWorker::new(move || {
            wake_tx.send(()).unwrap();
        })
        .unwrap();
        worker
            .submit(move || {
                started_tx.send(()).unwrap();
                release_rx.recv().unwrap();
                7
            })
            .unwrap();
        started_rx
            .recv_timeout(std::time::Duration::from_secs(5))
            .unwrap();
        assert!(worker.pending());
        assert_eq!(worker.try_finish().unwrap(), None);
        assert_eq!(
            worker.submit(|| 8).unwrap_err().kind(),
            std::io::ErrorKind::WouldBlock
        );
        release_tx.send(()).unwrap();
        wake_rx
            .recv_timeout(std::time::Duration::from_secs(5))
            .unwrap();
        // A finished but unconsumed frame still occupies the sole slot.
        assert_eq!(
            worker.submit(|| 9).unwrap_err().kind(),
            std::io::ErrorKind::WouldBlock
        );
        assert_eq!(worker.try_finish().unwrap(), Some(7));
        assert!(!worker.pending());
        worker.submit(|| 10).unwrap();
        assert_eq!(worker.finish().unwrap(), Some(10));
    }

    #[test]
    fn dropping_worker_joins_its_last_presentation() {
        let completed = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let written = completed.clone();
        let mut worker = FrameWorker::new(|| {}).unwrap();
        worker
            .submit(move || written.store(true, std::sync::atomic::Ordering::Release))
            .unwrap();
        drop(worker);
        assert!(completed.load(std::sync::atomic::Ordering::Acquire));
    }
}

/// Drawing projects process-local audio against the frame it captured, even
/// when the next simulation tick has finished before rasterization/present.
pub(crate) struct DrawFeedback {
    snapshot: SimulationSnapshot,
    viewports: Vec<ActiveViewportProjection>,
    calls: RenderedObjectAudibilityCalls,
}
impl DrawFeedback {
    pub(crate) fn capture(app: &GameApp) -> Self {
        // Audio reduction and attached-channel mixing read only objects and
        // listener players. Retain neither terrain nor the script/surface data
        // from the full simulation projection across the next tick.
        let snapshot = SimulationSnapshot {
            frame: app.snapshot.frame,
            objects: app.snapshot.objects.clone(),
            players: app.snapshot.players.clone(),
            ..SimulationSnapshot::default()
        };
        Self {
            snapshot,
            viewports: app.rendering.graphics.active_viewport_projections(),
            calls: app
                .rendering
                .graphics
                .rendered_object_audibility_calls()
                .clone(),
        }
    }
    pub(crate) fn apply(self, app: &mut GameApp) {
        if app.mode != AppMode::Running
            || app.console_session.enabled
            || app.snapshot.frame < self.snapshot.frame
        {
            return;
        }
        if let Some(audio) = app.sound.context.as_ref() {
            let mut audio = audio.borrow_mut();
            audio.cache_rendered_object_audibility(&self.calls, &self.snapshot, &self.viewports);
            audio.refresh_attached_channel_mix_after_render(&self.snapshot, &self.viewports);
        }
    }
}

pub(crate) struct CpuFrameState {
    pub(crate) presenter: clonk_scaling::FramePresenter,
    pub(crate) rgba: Vec<u8>,
}
impl CpuFrameState {
    pub(crate) fn for_presenter(
        previous: Option<Self>,
        presenter: &clonk_scaling::FramePresenter,
    ) -> Self {
        let (width, height) = presenter.physical_size();
        previous
            .filter(|state| {
                state.presenter.physical_size() == (width, height)
                    && state.presenter.scale() == presenter.scale()
            })
            .unwrap_or_else(|| Self {
                presenter: clonk_scaling::FramePresenter::new(presenter.scale(), width, height),
                rgba: vec![0; width as usize * height as usize * 4],
            })
    }
}

pub(crate) enum GraphicsResult {
    Gpu(Result<RetainedGpuProfiledOutcome>),
    Cpu(Result<bool>),
}
pub(crate) enum GraphicsOutput {
    Gpu {
        surface: WindowSurface,
        renderer: gpu_renderer::RetainedGpuRenderer,
        result: Result<RetainedGpuProfiledOutcome>,
        execution: Duration,
    },
    Cpu {
        state: CpuFrameState,
        renderers: Vec<clonk_graphics::CpuSceneRenderer>,
        result: Result<bool>,
        execution: Duration,
    },
}
pub(crate) struct PendingGraphicsPass {
    pub(crate) started: Instant,
    pub(crate) preparation: Duration,
    pub(crate) feedback: Option<DrawFeedback>,
}
pub(crate) struct CompletedGraphicsPass {
    pub(crate) result: GraphicsResult,
    pub(crate) duration: Duration,
    pub(crate) pass: PendingGraphicsPass,
}

pub(crate) fn execute_cpu_frame(
    retained: RetainedGpuFrame,
    mut state: CpuFrameState,
    mut renderers: Vec<clonk_graphics::CpuSceneRenderer>,
) -> GraphicsOutput {
    let started = Instant::now();
    let result = retained.render_cpu(&mut renderers, &mut state.presenter, &mut state.rgba);
    GraphicsOutput::Cpu {
        state,
        renderers,
        result,
        execution: started.elapsed(),
    }
}

pub(crate) fn execute_gpu_frame(
    surface: WindowSurface,
    mut renderer: gpu_renderer::RetainedGpuRenderer,
    retained: RetainedGpuFrame,
    shader_landscape: Option<(
        clonk_graphics::GpuTextureId,
        clonk_graphics::ShaderLandscapePlan,
    )>,
    context: RetainedGpuFrameContext,
    preparation: Duration,
) -> GraphicsOutput {
    let started = Instant::now();
    let result = submit_retained_gpu_frame_profiled(
        &surface,
        &mut renderer,
        context,
        false,
        false,
        false,
        || Ok((retained, shader_landscape, preparation)),
    )
    .map(|(outcome, _, _, _)| outcome);
    GraphicsOutput::Gpu {
        surface,
        renderer,
        result,
        execution: started.elapsed(),
    }
}
