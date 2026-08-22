use crate::{AudioCommand, ObjectId, Vector2};
use std::cell::RefCell;
use std::collections::HashMap;
use std::fmt;
use std::marker::PhantomData;
use std::rc::{Rc, Weak};
use std::sync::atomic::{AtomicU64, Ordering};

thread_local! {
    static SYNCHRONOUS_SOUND_HOSTS: RefCell<
        HashMap<u64, Weak<RefCell<dyn SynchronousSoundHost>>>,
    > = RefCell::new(HashMap::new());
}

static NEXT_SYNCHRONOUS_SOUND_HOST_ID: AtomicU64 = AtomicU64::new(1);

/// The player fields used by the client-local sound audibility calculation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LocalAudioPlayerView {
    pub cursor: Option<ObjectId>,
    pub view_cursor: Option<ObjectId>,
    pub view_target: Option<ObjectId>,
}

/// Synchronous access to the live object geometry visible to a native call.
///
/// This remains an ephemeral query rather than copied engine state: C++
/// `NewInstance` reads object positions at the exact call site, including
/// writes made earlier in the same script invocation.
pub trait LocalAudioWorld {
    fn object_position(&self, object: ObjectId) -> Option<Vector2>;
    /// C++ tests the raw Status word when a fresh instance attaches. Inactive
    /// (2) remains present; only deleted/zero rejects a looping attachment.
    fn object_status_present(&self, object: ObjectId) -> bool;
    fn player_view(&self, player: i32) -> Option<LocalAudioPlayerView>;
}

/// One `C4SoundSystem::NewInstance` request at its call-time world state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalSoundStart {
    pub name: String,
    pub target: Option<ObjectId>,
    pub volume: i32,
    pub looped: bool,
    pub multiple: bool,
    pub custom_falloff: Option<i32>,
    /// Last live position of an attached emitter that may already have left
    /// the world by the time a parallel callback outcome folds back.
    pub target_position: Option<Vector2>,
    /// `StartSoundEffectAt` has no object anchor but computes its initial mix
    /// from this fixed landscape position.
    pub position: Option<Vector2>,
}

/// App-owned, process-local half of synchronous C4SoundSystem operations.
///
/// Implementations run on the game thread. They may reserve a predecoded
/// mixer channel, but must never wait for the realtime output callback.
pub trait SynchronousSoundHost {
    /// Complete `NewInstance`, returning whether the logical instance exists.
    fn start_sound(&mut self, request: &LocalSoundStart, world: &dyn LocalAudioWorld) -> bool;

    /// Erase the first `FindInst` match in native sample/instance order.
    fn stop_sound(&mut self, name: &str, target: Option<ObjectId>);

    /// Update the first match or create C++'s looping SoundLevel fallback.
    fn set_sound_volume(
        &mut self,
        name: &str,
        target: Option<ObjectId>,
        volume: i32,
        world: &dyn LocalAudioWorld,
    );

    /// Apply `Instance::DetachObj` to every instance owned by `target`.
    fn detach_object_sounds(
        &mut self,
        target: ObjectId,
        position: Vector2,
        world: &dyn LocalAudioWorld,
    );

    /// Remove every logical sound instance at a sound-system reset boundary.
    fn clear_sound_instances(&mut self);
}

/// Send-safe identity retained by the engine's client-local audio registry.
/// The actual `Rc` endpoint never leaves its owning app thread.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SynchronousSoundHostHandle(u64);

/// Thread-local registration owned by the app audio context.
pub struct SynchronousSoundHostRegistration {
    handle: SynchronousSoundHostHandle,
    /// The endpoint is deliberately single-threaded even though its numeric
    /// handle may travel inside an Engine error recovery value.
    _local: PhantomData<Rc<()>>,
}

impl SynchronousSoundHostRegistration {
    pub fn new<T>(host: &Rc<RefCell<T>>) -> Self
    where
        T: SynchronousSoundHost + 'static,
    {
        let erased: Rc<RefCell<dyn SynchronousSoundHost>> = host.clone();
        let mut id = NEXT_SYNCHRONOUS_SOUND_HOST_ID.fetch_add(1, Ordering::Relaxed);
        if id == 0 {
            id = NEXT_SYNCHRONOUS_SOUND_HOST_ID.fetch_add(1, Ordering::Relaxed);
        }
        SYNCHRONOUS_SOUND_HOSTS.with(|hosts| {
            let replaced = hosts.borrow_mut().insert(id, Rc::downgrade(&erased));
            assert!(replaced.is_none(), "sound host ids must be unique");
        });
        Self {
            handle: SynchronousSoundHostHandle(id),
            _local: PhantomData,
        }
    }

    pub fn handle(&self) -> SynchronousSoundHostHandle {
        self.handle
    }
}

impl Drop for SynchronousSoundHostRegistration {
    fn drop(&mut self) {
        SYNCHRONOUS_SOUND_HOSTS.with(|hosts| {
            hosts.borrow_mut().remove(&self.handle.0);
        });
    }
}

impl fmt::Debug for SynchronousSoundHostRegistration {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SynchronousSoundHostRegistration")
            .field("handle", &self.handle)
            .finish()
    }
}

impl SynchronousSoundHostHandle {
    pub(crate) fn with_mut<R>(
        &self,
        operation: impl FnOnce(&mut dyn SynchronousSoundHost) -> R,
    ) -> Option<R> {
        let host = SYNCHRONOUS_SOUND_HOSTS
            .with(|hosts| hosts.borrow().get(&self.0).and_then(Weak::upgrade))?;
        let mut host = host.try_borrow_mut().ok()?;
        Some(operation(&mut *host))
    }
}

impl LocalAudioWorld for crate::Engine {
    fn object_position(&self, object: ObjectId) -> Option<Vector2> {
        self.find_object_index(object)
            .map(|index| self.objects[index].position_pixels())
    }

    fn object_status_present(&self, object: ObjectId) -> bool {
        self.find_object_index(object).is_some_and(|index| {
            !self.objects[index].destroyed
                && !matches!(
                    self.objects[index].state.status,
                    crate::ObjectStatus::Deleted
                )
        })
    }

    fn player_view(&self, player: i32) -> Option<LocalAudioPlayerView> {
        self.player(player).map(|state| LocalAudioPlayerView {
            cursor: state.cursor(),
            view_cursor: state.view_cursor(),
            view_target: state.raw_view_target(),
        })
    }
}

impl crate::Engine {
    pub(crate) fn emit_audio_command(&mut self, command: AudioCommand) {
        if !self.audio_registry.synchronous_host_configured() {
            self.pending_audio.push(command);
            return;
        }
        let host = self.audio_registry.synchronous_host();
        let handled = match &command {
            AudioCommand::PlaySound {
                name,
                target,
                volume,
                looped,
                multiple,
                custom_falloff,
                target_position,
            } => {
                let request = LocalSoundStart {
                    name: name.clone(),
                    target: *target,
                    volume: i32::from(*volume),
                    looped: *looped,
                    multiple: *multiple,
                    custom_falloff: *custom_falloff,
                    target_position: *target_position,
                    position: None,
                };
                if let Some(host) = host {
                    host.with_mut(|host| host.start_sound(&request, self))
                        .map(|started| {
                            if started {
                                if let Some(target) = target {
                                    self.audio_registry.note_attached_sound(*target);
                                }
                            }
                        })
                        .is_some()
                } else {
                    true
                }
            }
            AudioCommand::PlaySoundAt { name, position } => {
                let request = LocalSoundStart {
                    name: name.clone(),
                    target: None,
                    volume: 100,
                    looped: false,
                    multiple: true,
                    custom_falloff: None,
                    target_position: None,
                    position: Some(*position),
                };
                if let Some(host) = host {
                    host.with_mut(|host| host.start_sound(&request, self))
                        .is_some()
                } else {
                    true
                }
            }
            AudioCommand::StopSound { name, target } => {
                if let Some(host) = host {
                    host.with_mut(|host| host.stop_sound(name, *target))
                        .is_some()
                } else {
                    true
                }
            }
            AudioCommand::SetSoundVolume {
                name,
                target,
                volume,
            } => {
                if let Some(host) = host {
                    let handled = host
                        .with_mut(|host| {
                            if *volume <= 0 {
                                host.stop_sound(name, *target);
                            } else {
                                host.set_sound_volume(name, *target, *volume, self);
                            }
                        })
                        .is_some();
                    if handled && *volume > 0 {
                        if let Some(target) = target {
                            self.audio_registry.note_attached_sound(*target);
                        }
                    }
                    handled
                } else {
                    true
                }
            }
            AudioCommand::DetachObjectSounds { target, position } => {
                if let Some(host) = host {
                    host.with_mut(|host| host.detach_object_sounds(*target, *position, self))
                        .map(|()| self.audio_registry.note_detached_sounds(*target))
                        .is_some()
                } else {
                    true
                }
            }
            AudioCommand::PlaySpeech { .. }
            | AudioCommand::PlayMusic { .. }
            | AudioCommand::StopMusic
            | AudioCommand::SetMusicLevel { .. }
            | AudioCommand::SetMusicPlaylist { .. } => false,
        };
        if !handled {
            self.pending_audio.push(command);
        }
    }

    pub(crate) fn emit_audio_commands(&mut self, commands: impl IntoIterator<Item = AudioCommand>) {
        for command in commands {
            self.emit_audio_command(command);
        }
    }

    pub(crate) fn clear_local_sound_instances(&mut self) {
        if let Some(host) = self.audio_registry.synchronous_host() {
            let _ = host.with_mut(|host| host.clear_sound_instances());
        }
        self.audio_registry.clear_sound_instances();
    }
}
