//! `scenario` — moved verbatim from the parent module.
//!
//! Structural only: same crate, same items, same bodies.

use super::*;

enum DefinitionDiagnostic {
    ParticleRejected { group: PathBuf, error: String },
    InvalidId { group: PathBuf, id: String },
    DefinitionRejected { group: PathBuf, error: String },
    Resource(ResourceLoadDiagnostic),
}

enum DefinitionLoadEvent {
    Diagnostic(DefinitionDiagnostic),
    Progress {
        child_path: Vec<usize>,
        line: &'static str,
    },
}

impl DefinitionDiagnostic {
    fn emit(self) {
        match self {
            Self::ParticleRejected { group, error } => tracing::warn!(
                group = %group.display(),
                %error,
                "particle definition failed to load; skipping"
            ),
            Self::InvalidId { group, id } => tracing::warn!(
                %id,
                group = %group.display(),
                "skipping definition with invalid C4ID"
            ),
            Self::DefinitionRejected { group, error } => tracing::warn!(
                group = %group.display(),
                %error,
                "definition failed to load; skipping"
            ),
            Self::Resource(diagnostic) => diagnostic.emit(),
        }
    }
}

enum OrderedParallelOutcome<R> {
    Completed(R),
    Cancelled,
    Panicked(Box<dyn std::any::Any + Send + 'static>),
}

fn ordered_parallel_map_until<T, R, F, S, C>(
    items: &[T],
    worker_count: usize,
    map: F,
    is_terminal: S,
    mut on_ordered: C,
) -> Vec<OrderedParallelOutcome<R>>
where
    T: Sync,
    R: Send,
    F: Fn(&T) -> R + Sync,
    S: Fn(&R) -> bool + Sync,
    C: FnMut(usize, &mut R),
{
    let worker_count = worker_count.max(1).min(items.len().max(1));
    let earliest_terminal = std::sync::atomic::AtomicUsize::new(usize::MAX);
    let run_item = |index: usize, item: &T| {
        use std::sync::atomic::Ordering;

        if index > earliest_terminal.load(Ordering::Acquire) {
            return OrderedParallelOutcome::Cancelled;
        }
        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| map(item))) {
            Ok(result) => {
                if is_terminal(&result) {
                    earliest_terminal.fetch_min(index, Ordering::AcqRel);
                }
                OrderedParallelOutcome::Completed(result)
            }
            Err(payload) => {
                earliest_terminal.fetch_min(index, Ordering::AcqRel);
                OrderedParallelOutcome::Panicked(payload)
            }
        }
    };
    let report_ordered =
        |index: usize, outcome: &mut OrderedParallelOutcome<R>, on_ordered: &mut C| {
            use std::sync::atomic::Ordering;

            if index <= earliest_terminal.load(Ordering::Acquire) {
                if let OrderedParallelOutcome::Completed(result) = outcome {
                    on_ordered(index, result);
                }
            }
        };
    if worker_count == 1 || items.len() <= 1 {
        return items
            .iter()
            .enumerate()
            .map(|(index, item)| {
                let mut outcome = run_item(index, item);
                report_ordered(index, &mut outcome, &mut on_ordered);
                outcome
            })
            .collect();
    }
    let chunk_size = items.len().div_ceil(worker_count);
    std::thread::scope(|scope| {
        let chunks = items
            .chunks(chunk_size)
            .enumerate()
            .map(|(chunk_index, chunk)| {
                let start = chunk_index * chunk_size;
                let run_item = &run_item;
                let worker_chunk = chunk;
                std::thread::Builder::new()
                    .name("definition-loader".to_owned())
                    .spawn_scoped(scope, move || {
                        worker_chunk
                            .iter()
                            .enumerate()
                            .map(|(offset, item)| run_item(start + offset, item))
                            .collect::<Vec<_>>()
                    })
                    .map_err(|_| (start, chunk))
            })
            .collect::<Vec<_>>();
        let mut output = Vec::with_capacity(items.len());
        for chunk in chunks {
            let chunk = match chunk {
                Ok(handle) => match handle.join() {
                    Ok(chunk) => chunk,
                    Err(payload) => std::panic::resume_unwind(payload),
                },
                Err((start, chunk)) => chunk
                    .iter()
                    .enumerate()
                    .map(|(offset, item)| run_item(start + offset, item))
                    .collect(),
            };
            for mut outcome in chunk {
                report_ordered(output.len(), &mut outcome, &mut on_ordered);
                output.push(outcome);
            }
        }
        output
    })
}

// Scenario definition loading returns `ScenarioError`, whose `EngineError`
// variant can carry a live script continuation. That error is intentionally
// not `Send`: moving a suspended VM frame to a worker would make its ownership
// boundary implicit. Preserve the typed error and native child order by using
// this serial fold for the recursive loader; the generic parallel helper
// remains available for payloads that are safe to move between workers.
fn ordered_map_until<T, R, F, S, C>(
    items: &[T],
    map: F,
    is_terminal: S,
    mut on_ordered: C,
) -> Vec<OrderedParallelOutcome<R>>
where
    T: Sync,
    F: Fn(&T) -> R,
    S: Fn(&R) -> bool,
    C: FnMut(usize, &mut R),
{
    let mut terminal = false;
    items
        .iter()
        .enumerate()
        .map_while(|(index, item)| {
            if terminal {
                return None;
            }
            let mut outcome =
                match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| map(item))) {
                    Ok(result) => OrderedParallelOutcome::Completed(result),
                    Err(payload) => return Some(OrderedParallelOutcome::Panicked(payload)),
                };
            if let OrderedParallelOutcome::Completed(result) = &mut outcome {
                terminal = is_terminal(result);
                on_ordered(index, result);
            }
            Some(outcome)
        })
        .collect()
}

fn emit_definition_event(
    buffered: &mut Vec<DefinitionLoadEvent>,
    live: &mut Option<&mut dyn FnMut(DefinitionLoadEvent)>,
    event: DefinitionLoadEvent,
) {
    if let Some(report) = live.as_deref_mut() {
        report(event);
    } else {
        buffered.push(event);
    }
}

fn emit_definition_diagnostic(
    buffered: &mut Vec<DefinitionLoadEvent>,
    live: &mut Option<&mut dyn FnMut(DefinitionLoadEvent)>,
    diagnostic: DefinitionDiagnostic,
) {
    emit_definition_event(buffered, live, DefinitionLoadEvent::Diagnostic(diagnostic));
}

fn emit_definition_progress(
    buffered: &mut Vec<DefinitionLoadEvent>,
    live: &mut Option<&mut dyn FnMut(DefinitionLoadEvent)>,
    line: &'static str,
) {
    emit_definition_event(
        buffered,
        live,
        DefinitionLoadEvent::Progress {
            child_path: Vec::new(),
            line,
        },
    );
}

fn resolve_definition_progress(
    mut min_progress: i32,
    mut max_progress: i32,
    child_path: &[usize],
) -> i32 {
    debug_assert!(min_progress <= max_progress);
    for &child_index in child_path {
        let parent_min = i64::from(min_progress);
        let parent_max = i64::from(max_progress);
        let progress_span = parent_max - parent_min;
        // C4DefList caps every child range at its parent's maximum. Once the
        // sixteenth child is reached, all remaining children therefore remain
        // at that maximum as well (src/C4Def.cpp:939-950).
        let child_index = child_index.min(16) as i64;
        let child_min = parent_max.min(parent_min + progress_span * child_index / 16);
        let child_max = parent_max.min(parent_min + progress_span * (child_index + 1) / 16);
        min_progress = child_min as i32;
        max_progress = child_max as i32;
    }
    max_progress
}

#[allow(clippy::too_many_arguments)]
fn report_resolved_definition_progress(
    child_path: &[usize],
    line: &'static str,
    min_progress: i32,
    max_progress: i32,
    last_progress: &mut i32,
    report_progress: &mut dyn FnMut(i32, &str),
) {
    let progress = resolve_definition_progress(min_progress, max_progress, child_path);
    if progress > *last_progress || (progress == *last_progress && !line.is_empty()) {
        *last_progress = (*last_progress).max(progress);
        report_progress(progress, line);
    }
}

fn emit_ordered_child_events<T, E>(
    failed: &mut bool,
    successful_child_index: &mut usize,
    opened: bool,
    result: &Result<T, E>,
    child_events: &mut Vec<DefinitionLoadEvent>,
    buffered: &mut Vec<DefinitionLoadEvent>,
    live: &mut Option<&mut dyn FnMut(DefinitionLoadEvent)>,
) {
    if *failed {
        child_events.clear();
        return;
    }
    if !opened {
        child_events.clear();
        return;
    }
    let child_index = *successful_child_index;
    *successful_child_index += 1;
    for mut event in child_events.drain(..) {
        match &mut event {
            DefinitionLoadEvent::Progress { child_path, .. } => {
                child_path.insert(0, child_index);
            }
            DefinitionLoadEvent::Diagnostic(_) => {}
        }
        emit_definition_event(buffered, live, event);
    }
    *failed = result.is_err();
}

pub(in crate::scenario) fn collect_definitions_from_group<S: AsRef<str> + Sync>(
    group: &Group,
    load_system_groups: bool,
    skip_ids: &HashSet<String>,
    languages: &[S],
    language_packs: &LanguagePacks,
    scenario: &Group,
    scenario_origin: Option<&str>,
    sound_effect_groups: &mut Vec<Group>,
    output: &mut Vec<CollectedDefinition>,
) -> Result<(), ScenarioError> {
    let mut ignore_progress = |_: i32, _: &str| {};
    collect_definitions_from_group_with_progress(
        group,
        load_system_groups,
        skip_ids,
        languages,
        language_packs,
        scenario,
        scenario_origin,
        sound_effect_groups,
        output,
        0,
        0,
        "",
        &mut ignore_progress,
    )
}

#[allow(clippy::too_many_arguments)]
pub(in crate::scenario) fn collect_definitions_from_group_with_progress<S: AsRef<str> + Sync>(
    group: &Group,
    load_system_groups: bool,
    skip_ids: &HashSet<String>,
    languages: &[S],
    language_packs: &LanguagePacks,
    scenario: &Group,
    scenario_origin: Option<&str>,
    sound_effect_groups: &mut Vec<Group>,
    output: &mut Vec<CollectedDefinition>,
    min_progress: i32,
    max_progress: i32,
    completion_line: &'static str,
    report_progress: &mut dyn FnMut(i32, &str),
) -> Result<(), ScenarioError> {
    let mut buffered_events = Vec::new();
    let mut last_progress = min_progress;
    let mut handle_event = |event| match event {
        DefinitionLoadEvent::Diagnostic(diagnostic) => diagnostic.emit(),
        DefinitionLoadEvent::Progress { child_path, line } => report_resolved_definition_progress(
            &child_path,
            line,
            min_progress,
            max_progress,
            &mut last_progress,
            report_progress,
        ),
    };
    let mut live_events: Option<&mut dyn FnMut(DefinitionLoadEvent)> = Some(&mut handle_event);
    collect_definitions_from_group_inner(
        group,
        load_system_groups,
        skip_ids,
        languages,
        language_packs,
        scenario,
        scenario_origin,
        sound_effect_groups,
        output,
        min_progress != max_progress,
        completion_line,
        &mut buffered_events,
        &mut live_events,
        true,
    )
}

#[allow(clippy::too_many_arguments)]
fn collect_definitions_from_group_inner<S: AsRef<str> + Sync>(
    group: &Group,
    load_system_groups: bool,
    skip_ids: &HashSet<String>,
    languages: &[S],
    language_packs: &LanguagePacks,
    scenario: &Group,
    scenario_origin: Option<&str>,
    sound_effect_groups: &mut Vec<Group>,
    output: &mut Vec<CollectedDefinition>,
    track_progress: bool,
    completion_line: &'static str,
    buffered_events: &mut Vec<DefinitionLoadEvent>,
    live_events: &mut Option<&mut dyn FnMut(DefinitionLoadEvent)>,
    _parallel_children: bool,
) -> Result<(), ScenarioError> {
    let indexed_group = group.is_directory().then(|| group.indexed()).transpose()?;
    let group = indexed_group.as_ref().unwrap_or(group);
    let mut primary_definition = false;
    // C4Def::Load diverts Particle.txt groups into C4ParticleDef before it
    // even attempts DefCore; they never become object definitions.
    if group.exists("Particle.txt") {
        // C4Def::Load marks particle groups as non-definitions, loads the
        // particle metadata, and then still runs the invalid-definition
        // LoadEffects path regardless of whether that metadata succeeded.
        sound_effect_groups.push(group.clone());
        match ResourceParticleDefinition::load(group) {
            Ok(definition) => output.push(CollectedDefinition::Particle(definition)),
            Err(error) => emit_definition_diagnostic(
                buffered_events,
                live_events,
                DefinitionDiagnostic::ParticleRejected {
                    group: group.root().to_path_buf(),
                    error: error.to_string(),
                },
            ),
        }
    } else if group.exists("DefCore.txt") {
        // C4Def::Load checks SkipDefs immediately after DefCore, before
        // scripts, ActMap, graphics, sounds, or localized auxiliary data.
        // Probe the ID first so malformed data in a skipped definition is
        // never observed.
        let core = match ResourceDefCore::load_with_diagnostics(group, |diagnostic| {
            emit_definition_diagnostic(
                buffered_events,
                live_events,
                DefinitionDiagnostic::Resource(diagnostic),
            );
        }) {
            Ok(core) => Some(core),
            Err(ResourceDefinitionError::DefCoreMissing) => {
                sound_effect_groups.push(group.clone());
                None
            }
            Err(error) if is_rejected_definition_error(&error) => {
                queue_rejected_definition(buffered_events, live_events, group, &error);
                // A failed C4DefCore::Load deliberately turns the group into
                // a pure sound container before C4DefList visits children.
                sound_effect_groups.push(group.clone());
                None
            }
            Err(error) => return Err(error.into()),
        };
        if let Some(core) = core {
            if !core.has_valid_id() {
                emit_definition_diagnostic(
                    buffered_events,
                    live_events,
                    DefinitionDiagnostic::InvalidId {
                        group: group.root().to_path_buf(),
                        id: core.id.clone(),
                    },
                );
                // NeededGfxMode is checked even after an invalid ID made the
                // definition unsuccessful. OLDGFX therefore suppresses the
                // otherwise intentional pure-sound fallback.
                if core.needed_gfx_mode != 2 {
                    sound_effect_groups.push(group.clone());
                }
            } else if skip_ids.contains(&core.id.to_ascii_uppercase()) {
                // C4Def::Load checks SkipDefs before the graphics-mode gate.
            } else if core.needed_gfx_mode == 2 {
                // C4DGFXMODE_OLDGFX is no longer supported. Native returns
                // false here without a dedicated diagnostic.
            } else {
                let components =
                    language_packs.component_groups(group, Some(scenario), scenario_origin);
                match ResourceDefinitionData::load_with_core_and_languages_and_components(
                    group,
                    core,
                    languages,
                    &components,
                ) {
                    Ok(resource) => {
                        if resource.graphics_image.is_none() {
                            queue_rejected_definition(
                                buffered_events,
                                live_events,
                                group,
                                &"required Graphics.png/Graphics.bmp is missing or invalid",
                            );
                        } else {
                            primary_definition = true;
                            // Valid definitions reach LoadEffects only after
                            // bitmap, portrait and ActMap/resource loading has
                            // succeeded. Retain the event before descending
                            // into child definitions.
                            sound_effect_groups.push(group.clone());
                            let mut definition =
                                scenario_definition_from_resource(resource, Some(group.clone()));
                            definition.script =
                                localize_script_source_with_components_and_diagnostics(
                                    &components,
                                    &definition.script,
                                    languages,
                                    |diagnostic| {
                                        emit_definition_diagnostic(
                                            buffered_events,
                                            live_events,
                                            DefinitionDiagnostic::Resource(diagnostic),
                                        );
                                    },
                                )?;
                            output.push(CollectedDefinition::Definition(definition));
                        }
                    }
                    Err(error) if is_rejected_definition_error(&error) => {
                        queue_rejected_definition(buffered_events, live_events, group, &error);
                    }
                    Err(error) => return Err(error.into()),
                }
            }
        }
    } else {
        // Missing DefCore is the canonical pure `.c4d` sound-folder case.
        sound_effect_groups.push(group.clone());
    }

    // C4DefList::Load recursively visits only *.c4d children. Independent
    // root subtrees may decode concurrently, but each subtree retains native
    // traversal order and the results are folded in entry order below.
    let child_entries = group
        .entries()?
        .into_iter()
        .filter(|entry| legacy_group_wildcard_match(b"*.c4d", &entry.name_bytes))
        .collect::<Vec<_>>();
    let mut event_failed = false;
    let mut successful_child_index = 0;
    let child_results = ordered_map_until(
        &child_entries,
        |entry| {
            // Open inside the bounded worker set so all candidates are not
            // preopened serially. Loaded group images remain retained by the
            // resulting definitions; the cap bounds concurrent I/O and
            // decoder scratch rather than total scenario resource memory.
            let Ok(child) = group.open_child_entry_exact(entry) else {
                return (
                    false,
                    Ok::<(), ScenarioError>(()),
                    Vec::new(),
                    Vec::new(),
                    Vec::new(),
                );
            };
            let mut child_sounds = Vec::new();
            let mut child_output = Vec::new();
            let mut child_events = Vec::new();
            let mut no_live_events = None;
            // The recursive call omits fLoadSysGroups in C++, so its default
            // true applies even when only the scenario root suppressed System
            // loading.
            let result = collect_definitions_from_group_inner(
                &child,
                true,
                skip_ids,
                languages,
                language_packs,
                scenario,
                scenario_origin,
                &mut child_sounds,
                &mut child_output,
                track_progress,
                "",
                &mut child_events,
                &mut no_live_events,
                false,
            );
            (true, result, child_sounds, child_output, child_events)
        },
        |(_, result, _, _, _)| result.is_err(),
        |_, (opened, result, _, _, child_events)| {
            emit_ordered_child_events(
                &mut event_failed,
                &mut successful_child_index,
                *opened,
                result,
                child_events,
                buffered_events,
                live_events,
            );
        },
    );
    for child_result in child_results {
        let (_, result, mut child_sounds, mut child_output, _) = match child_result {
            OrderedParallelOutcome::Completed(result) => result,
            OrderedParallelOutcome::Cancelled => continue,
            OrderedParallelOutcome::Panicked(payload) => std::panic::resume_unwind(payload),
        };
        result?;
        sound_effect_groups.append(&mut child_sounds);
        output.append(&mut child_output);
    }

    // A non-primary definition root loads its System.c4g only AFTER all child
    // definitions (C4Def.cpp:927-968). Direct primary definitions suppress
    // their own System group, as does the scenario-file InitDefs pass.
    if !primary_definition && load_system_groups {
        if let Ok(system) = group.open_child(Path::new("System.c4g")) {
            let components =
                language_packs.component_groups(&system, Some(scenario), scenario_origin);
            if let Ok(sources) = load_system_scripts_with_components_and_diagnostics(
                &system,
                &components,
                languages,
                |diagnostic| {
                    emit_definition_diagnostic(
                        buffered_events,
                        live_events,
                        DefinitionDiagnostic::Resource(diagnostic),
                    );
                },
            ) {
                output.push(CollectedDefinition::SystemScripts(sources));
            }
        }
    }
    if track_progress {
        emit_definition_progress(buffered_events, live_events, completion_line);
    }
    Ok(())
}

fn is_rejected_definition_error(error: &ResourceDefinitionError) -> bool {
    matches!(
        error,
        ResourceDefinitionError::MissingDefCoreField(_)
            | ResourceDefinitionError::InvalidCategoryValue(_)
            | ResourceDefinitionError::DefCoreParse(_)
            | ResourceDefinitionError::ActMapParse(_)
            | ResourceDefinitionError::Graphics { .. }
            | ResourceDefinitionError::ColorByOwnerOverlay { .. }
    )
}

fn queue_rejected_definition(
    buffered_events: &mut Vec<DefinitionLoadEvent>,
    live_events: &mut Option<&mut dyn FnMut(DefinitionLoadEvent)>,
    group: &Group,
    error: &impl fmt::Display,
) {
    emit_definition_diagnostic(
        buffered_events,
        live_events,
        DefinitionDiagnostic::DefinitionRejected {
            group: group.root().to_path_buf(),
            error: error.to_string(),
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ordered_parallel_map_until_preserves_order_across_worker_chunks() {
        let values = [3, 1, 2];
        let mapped = ordered_parallel_map_until(&values, 2, |value| *value, |_| false, |_, _| {})
            .into_iter()
            .map(|outcome| match outcome {
                OrderedParallelOutcome::Completed(value) => value,
                OrderedParallelOutcome::Cancelled => panic!("no work should be cancelled"),
                OrderedParallelOutcome::Panicked(payload) => std::panic::resume_unwind(payload),
            })
            .collect::<Vec<_>>();

        assert_eq!(mapped, values);
    }

    #[test]
    fn ordered_parallel_map_until_cancels_after_the_first_terminal_index() {
        let visited = std::sync::atomic::AtomicUsize::new(0);
        let values = [0, 1, 2, 3];

        let mapped = ordered_parallel_map_until(
            &values,
            1,
            |value| {
                visited.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                *value
            },
            |value| *value == 1,
            |_, _| {},
        );

        assert_eq!(visited.load(std::sync::atomic::Ordering::Relaxed), 2);
        assert!(matches!(mapped[0], OrderedParallelOutcome::Completed(0)));
        assert!(matches!(mapped[1], OrderedParallelOutcome::Completed(1)));
        assert!(matches!(mapped[2], OrderedParallelOutcome::Cancelled));
        assert!(matches!(mapped[3], OrderedParallelOutcome::Cancelled));
    }

    #[test]
    fn ordered_parallel_map_until_keeps_a_later_panic_behind_the_first_terminal() {
        // C4DefList::Load visits child groups strictly in FindNextEntry order
        // (src/C4Def.cpp:930-949), so a concurrent later outcome must never
        // replace the first terminal outcome in that order.
        let barrier = std::sync::Barrier::new(2);
        let values = [0, 1];
        let mut reported = Vec::new();

        let mapped = ordered_parallel_map_until(
            &values,
            2,
            |value| {
                barrier.wait();
                if *value == 0 {
                    Err::<(), _>("first terminal")
                } else {
                    panic!("later panic")
                }
            },
            Result::is_err,
            |index, _| reported.push(index),
        );

        assert!(matches!(
            mapped[0],
            OrderedParallelOutcome::Completed(Err("first terminal"))
        ));
        assert!(matches!(mapped[1], OrderedParallelOutcome::Panicked(_)));
        assert_eq!(reported, [0]);
    }

    #[test]
    fn ordered_child_events_stop_after_the_first_fatal_result() {
        let mut failed = false;
        let mut successful_child_index = 0;
        let mut buffered = Vec::new();
        let mut reported = Vec::new();
        let mut report = |event| {
            if let DefinitionLoadEvent::Progress { child_path, .. } = event {
                reported.push(resolve_definition_progress(0, 16, &child_path));
            }
        };
        let mut live: Option<&mut dyn FnMut(DefinitionLoadEvent)> = Some(&mut report);
        let mut first = vec![DefinitionLoadEvent::Progress {
            child_path: Vec::new(),
            line: "first",
        }];
        let mut fatal = vec![DefinitionLoadEvent::Progress {
            child_path: Vec::new(),
            line: "fatal child",
        }];
        let mut later = vec![DefinitionLoadEvent::Progress {
            child_path: Vec::new(),
            line: "later child",
        }];

        emit_ordered_child_events(
            &mut failed,
            &mut successful_child_index,
            true,
            &Ok::<_, &str>(()),
            &mut first,
            &mut buffered,
            &mut live,
        );
        emit_ordered_child_events(
            &mut failed,
            &mut successful_child_index,
            true,
            &Err::<(), _>("fatal"),
            &mut fatal,
            &mut buffered,
            &mut live,
        );
        emit_ordered_child_events(
            &mut failed,
            &mut successful_child_index,
            true,
            &Ok::<_, &str>(()),
            &mut later,
            &mut buffered,
            &mut live,
        );

        assert_eq!(reported, [1, 2]);
    }

    #[test]
    fn ordered_child_events_preserve_diagnostic_and_progress_interleaving() {
        let mut events = vec![
            DefinitionLoadEvent::Progress {
                child_path: Vec::new(),
                line: "before warning",
            },
            DefinitionLoadEvent::Diagnostic(DefinitionDiagnostic::InvalidId {
                group: PathBuf::from("Broken.c4d"),
                id: "TOOLONG".to_owned(),
            }),
            DefinitionLoadEvent::Progress {
                child_path: Vec::new(),
                line: "after warning",
            },
        ];
        let mut failed = false;
        let mut successful_child_index = 0;
        let mut buffered = Vec::new();
        let mut observed = Vec::new();
        let mut report = |event| {
            observed.push(match event {
                DefinitionLoadEvent::Progress { line, .. } => line,
                DefinitionLoadEvent::Diagnostic(_) => "warning",
            });
        };
        let mut live: Option<&mut dyn FnMut(DefinitionLoadEvent)> = Some(&mut report);

        emit_ordered_child_events(
            &mut failed,
            &mut successful_child_index,
            true,
            &Ok::<_, &str>(()),
            &mut events,
            &mut buffered,
            &mut live,
        );

        assert_eq!(observed, ["before warning", "warning", "after warning"]);
    }

    #[test]
    fn failed_child_open_does_not_consume_a_progress_slot() {
        let mut failed = false;
        let mut successful_child_index = 0;
        let mut buffered = Vec::new();
        let mut reported = Vec::new();
        let mut report = |event| {
            if let DefinitionLoadEvent::Progress { child_path, .. } = event {
                reported.push(resolve_definition_progress(10, 35, &child_path));
            }
        };
        let mut live: Option<&mut dyn FnMut(DefinitionLoadEvent)> = Some(&mut report);
        let mut skipped_events = Vec::new();
        let mut first_opened_events = vec![DefinitionLoadEvent::Progress {
            child_path: Vec::new(),
            line: "complete",
        }];

        emit_ordered_child_events(
            &mut failed,
            &mut successful_child_index,
            false,
            &Ok::<_, &str>(()),
            &mut skipped_events,
            &mut buffered,
            &mut live,
        );
        emit_ordered_child_events(
            &mut failed,
            &mut successful_child_index,
            true,
            &Ok::<_, &str>(()),
            &mut first_opened_events,
            &mut buffered,
            &mut live,
        );

        assert_eq!(successful_child_index, 1);
        assert_eq!(reported, [11]);
    }

    #[test]
    fn deeply_nested_child_progress_preserves_each_integer_division() {
        // C4DefList::Load derives every nested range from its already-rounded
        // parent range (src/C4Def.cpp:939-950). Collapsing a descendant back
        // into one canonical numeric range loses that composition.
        let mut buffered = Vec::new();
        let mut no_live = None;
        let mut failed = false;
        let mut child_index = 7;
        let mut events = vec![DefinitionLoadEvent::Progress {
            child_path: Vec::new(),
            line: "deep child",
        }];

        emit_ordered_child_events(
            &mut failed,
            &mut child_index,
            true,
            &Ok::<_, &str>(()),
            &mut events,
            &mut buffered,
            &mut no_live,
        );

        let mut parent_events = Vec::new();
        let mut parent_index = 15;
        emit_ordered_child_events(
            &mut failed,
            &mut parent_index,
            true,
            &Ok::<_, &str>(()),
            &mut buffered,
            &mut parent_events,
            &mut no_live,
        );

        let mut reported = Vec::new();
        let mut report = |event| {
            if let DefinitionLoadEvent::Progress { child_path, .. } = event {
                reported.push(resolve_definition_progress(10, 267, &child_path));
            }
        };
        let mut live: Option<&mut dyn FnMut(DefinitionLoadEvent)> = Some(&mut report);
        let mut root_index = 15;
        emit_ordered_child_events(
            &mut failed,
            &mut root_index,
            true,
            &Ok::<_, &str>(()),
            &mut parent_events,
            &mut Vec::new(),
            &mut live,
        );

        assert_eq!(reported, [266]);
    }

    #[test]
    fn nested_child_progress_preserves_each_integer_division() {
        // C4DefList::Load derives the grandchild range from the rounded child
        // range (src/C4Def.cpp:939-950): child 1 then grandchild 8 within
        // 10..35 resolves to 12.
        let mut failed = false;
        let mut successful_child_index = 1;
        let mut child_events = vec![DefinitionLoadEvent::Progress {
            child_path: vec![8],
            line: "grandchild",
        }];
        let mut buffered = Vec::new();
        let mut reported = Vec::new();
        let mut report = |event| {
            if let DefinitionLoadEvent::Progress { child_path, .. } = event {
                reported.push(resolve_definition_progress(10, 35, &child_path));
            }
        };
        let mut live: Option<&mut dyn FnMut(DefinitionLoadEvent)> = Some(&mut report);

        emit_ordered_child_events(
            &mut failed,
            &mut successful_child_index,
            true,
            &Ok::<_, &str>(()),
            &mut child_events,
            &mut buffered,
            &mut live,
        );

        assert_eq!(reported, [12]);
    }

    #[test]
    fn equal_progress_keeps_the_root_completion_line() {
        // C4DefList::Load logs the completed definition count before reporting
        // the root maximum (src/C4Def.cpp:979-982). A sixteenth child's empty
        // progress event must not suppress that nonempty completion line.
        let mut last_progress = 10;
        let mut reported = Vec::new();

        report_resolved_definition_progress(
            &[15],
            "",
            10,
            40,
            &mut last_progress,
            &mut |progress, line| reported.push((progress, line.to_owned())),
        );
        report_resolved_definition_progress(
            &[],
            "Definition metadata and sources collected",
            10,
            40,
            &mut last_progress,
            &mut |progress, line| reported.push((progress, line.to_owned())),
        );

        assert_eq!(
            reported,
            [
                (40, String::new()),
                (40, "Definition metadata and sources collected".to_owned())
            ]
        );
    }
}

pub(in crate::scenario) fn scenario_definition_from_resource(
    resource: ResourceDefinitionData,
    source_group: Option<Group>,
) -> ScenarioDefinition {
    let script_name = source_group
        .as_ref()
        .map(|group| group.root().join("Script.c").to_string_lossy().into_owned());
    let description = resource.description().map(str::to_owned);
    let ResourceDefinitionData {
        core,
        script,
        action_map,
        picture_image,
        picture_color_by_owner_mask,
        graphics_image,
        color_by_owner_mask,
        additional_graphics,
        portrait_image,
        portrait_graphics_image,
        portrait_color_by_owner_mask,
        portrait_graphics,
        rank_symbols_image,
        rank_names,
        rank_base,
        rank_symbol_count,
        clonk_names,
    } = resource;
    let actions = action_map.map(|map| convert_action_map(&map));
    let full_core = core.clone();

    ScenarioDefinition {
        id: core.id,
        name: core.name,
        description,
        clonk_names,
        script: script.combined().to_string(),
        script_name,
        actions,
        crew_member: core.crew_member != 0,
        can_be_base: core.can_be_base,
        movement: MovementProfile::default(),
        movement_manifest: false,
        category: core.category,
        value: core.value,
        mass: core.mass,
        picture: core.picture.map(DefinitionPicture::from),
        picture_image,
        picture_color_by_owner_mask,
        graphics_image,
        color_by_owner_mask,
        additional_graphics,
        portrait_image,
        portrait_graphics_image,
        portrait_color_by_owner_mask,
        portrait_graphics,
        rank_symbols_image,
        rank_names,
        rank_base,
        rank_symbol_count,
        resource_group: source_group,
        components: core
            .components
            .into_iter()
            .map(|component| DefinitionComponent {
                id: component.id,
                count: component.count,
            })
            .collect(),
        line_connect: core.line_connect,
        vertices: core.vertices,
        shape: core.shape,
        core: Some(full_core),
    }
}

pub(in crate::scenario) fn convert_action_map(map: &ResourceActionMap) -> DefinitionActions {
    let mut specs = HashMap::new();
    let mut physical = Vec::with_capacity(map.actions.len());
    let mut graphics = HashMap::new();
    graphics.insert(
        crate::PHYSICAL_ACTION_GRAPHICS_MARKER.to_string(),
        DefinitionActionGraphics::default(),
    );
    let mut reflections = HashMap::new();
    for (index, (name, definition)) in map.actions.iter().enumerate() {
        let (spec, visuals) = convert_action_definition(definition);
        physical.push((name.clone(), spec.clone()));
        // SetActionByName and FnGetActMapVal both scan the physical ActMap
        // forward, so the first duplicate name wins.
        specs.entry(name.clone()).or_insert(spec);
        graphics
            .entry(name.clone())
            .or_insert_with(|| visuals.clone());
        graphics.insert(
            crate::physical_action_graphics_key(index.min(u32::MAX as usize) as u32),
            visuals,
        );
        reflections
            .entry(name.clone())
            .or_insert_with(|| crate::action::C4ActionReflection::from_resource(name, definition));
    }
    DefinitionActions {
        default_action: map.default_action.clone(),
        specs,
        physical,
        graphics,
        reflections,
    }
}

pub(crate) fn convert_action_definition(
    action: &ResourceActionDefinition,
) -> (ActionSpec, DefinitionActionGraphics) {
    let mut spec = ActionSpec::default();
    if let Some(length) = action.length {
        spec = spec.with_length(length);
    }
    if let Some(next) = &action.next_action {
        spec = spec.with_next(next.clone());
    }
    spec = spec.with_next_index(action.next_action_index);
    if let Some(procedure) = action.procedure.as_deref().and_then(|procedure| {
        clonk_resources::definition::PROCEDURE_NAMES
            .iter()
            .find(|candidate| **candidate == procedure)
    }) {
        spec = spec.with_procedure(*procedure);
    }
    if let Some(delay) = action.delay {
        spec = spec.with_delay(delay);
    }
    if let Some(step) = action.step {
        spec = spec.with_step(step);
    }
    if let Some(phase_call) = &action.phase_call {
        spec = spec.with_phase_call(phase_call.clone());
    }
    if let Some(start_call) = &action.start_call {
        spec = spec.with_start_call(start_call.clone());
    }
    if let Some(end_call) = &action.end_call {
        spec = spec.with_end_call(end_call.clone());
    }
    if let Some(abort_call) = &action.abort_call {
        spec = spec.with_abort_call(abort_call.clone());
    }
    if action.no_other_action {
        spec = spec.with_no_other_action(true);
    }
    if action.disabled {
        spec = spec.with_disabled(true);
    }
    if action.energy_usage != 0 {
        spec = spec.with_energy_usage(action.energy_usage);
    }
    if let Some(in_liquid_action) = &action.in_liquid_action {
        spec = spec.with_in_liquid_action(in_liquid_action.clone());
    }
    if let Some(directions) = action.directions {
        spec = spec.with_directions(directions);
    }
    if let Some(flip_dir) = action.flip_dir {
        spec = spec.with_flip_dir(flip_dir);
    }
    if let Some(turn_action) = &action.turn_action {
        spec = spec.with_turn_action(turn_action.clone());
    }
    if let Some(sound) = &action.sound {
        spec = spec.with_sound(sound.clone());
    }
    if let Some(dig_free) = action.dig_free {
        spec = spec.with_dig_free(dig_free);
    }
    // ActMap Attach: the ExecAction default case zeroes dirs and
    // mobilizes instead of applying gravity (C4Object.cpp:5426-5437) —
    // dropping it made every NONE-procedure aimer/rider free-fall.
    if action.attach != 0 {
        spec = spec.with_attach(action.attach);
    }
    let mut graphics = DefinitionActionGraphics::default();
    graphics.length = action.length;
    graphics.directions = action.directions.unwrap_or(1);
    graphics.flip_dir = action.flip_dir;
    graphics.reverse = action.reverse;
    graphics.facet_base = action.facet_base;
    graphics.facet_top_face = action.facet_top_face;
    graphics.facet_target_stretch = action.facet_target_stretch;
    graphics.facet = action.facet.as_ref().map(convert_action_facet);
    (spec, graphics)
}

pub(crate) fn convert_action_facet(facet: &ResourceActionFacet) -> DefinitionActionFacet {
    DefinitionActionFacet {
        x: facet.x,
        y: facet.y,
        width: facet.width,
        height: facet.height,
        target_x: facet.target_x,
        target_y: facet.target_y,
    }
}

pub(in crate::scenario) fn read_group_file_bytes(
    group: &Group,
    path: &Path,
) -> Result<Vec<u8>, ScenarioError> {
    match group.read_file(path) {
        Ok(bytes) => Ok(bytes),
        Err(GroupError::EntryNotFound(_)) => read_file_from_fs(group, path),
        Err(GroupError::Io(error)) if error.kind() == io::ErrorKind::NotFound => {
            read_file_from_fs(group, path)
        }
        Err(error) => Err(ScenarioError::Resources(error)),
    }
}

fn read_file_from_fs(group: &Group, path: &Path) -> Result<Vec<u8>, ScenarioError> {
    let fallback = group.root().join(path);
    fs::read(&fallback).map_err(|_| ScenarioError::MissingScript {
        path: PathBuf::from(path),
    })
}

#[derive(Debug, Deserialize)]
pub(in crate::scenario) struct ScenarioManifest {
    #[serde(default)]
    pub(in crate::scenario) name: Option<String>,
    #[serde(default)]
    pub(in crate::scenario) description: Option<String>,
    #[serde(default)]
    pub(in crate::scenario) ticks: Option<u32>,
    #[serde(default)]
    pub(in crate::scenario) ground_height: Option<i32>,
    #[serde(default)]
    pub(in crate::scenario) definitions: Vec<DefinitionManifest>,
    #[serde(default)]
    pub(in crate::scenario) initial_objects: Vec<ObjectManifest>,
    #[serde(default)]
    pub(in crate::scenario) landscape: Option<LandscapeManifest>,
    #[serde(default)]
    pub(in crate::scenario) physics: Option<PhysicsManifest>,
    #[serde(default)]
    pub(in crate::scenario) environment: Option<EnvironmentManifest>,
    #[serde(default)]
    pub(in crate::scenario) sky: Option<SkyManifest>,
    #[serde(default)]
    pub(in crate::scenario) script: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(in crate::scenario) struct DefinitionManifest {
    pub(in crate::scenario) id: String,
    #[serde(default)]
    pub(in crate::scenario) name: Option<String>,
    pub(in crate::scenario) script: String,
    #[serde(default)]
    pub(in crate::scenario) default_action: Option<String>,
    #[serde(default)]
    pub(in crate::scenario) actions: HashMap<String, ActionSpec>,
    #[serde(default)]
    pub(in crate::scenario) crew_member: bool,
    /// Synthetic fixture movement is opt-in; omission keeps C++'s native
    /// DFA_FLOAT physical bounds, including the zero default.
    #[serde(default)]
    pub(in crate::scenario) movement: Option<MovementManifest>,
    #[serde(default)]
    pub(in crate::scenario) category: Option<i32>,
}

#[derive(Debug, Deserialize, Default)]
pub(in crate::scenario) struct MovementManifest {
    #[serde(default)]
    float: Option<FloatMovementManifest>,
    #[serde(default)]
    swim: Option<SwimMovementManifest>,
    #[serde(default)]
    walk: Option<WalkMovementManifest>,
    #[serde(default)]
    scale: Option<ScaleMovementManifest>,
    #[serde(default)]
    hangle: Option<HangleMovementManifest>,
    #[serde(default)]
    dig: Option<DigMovementManifest>,
}

#[derive(Debug, Deserialize, Default)]
struct FloatMovementManifest {
    #[serde(default)]
    speed: Option<i32>,
    #[serde(default)]
    acceleration: Option<i32>,
}

#[derive(Debug, Deserialize, Default)]
struct SwimMovementManifest {
    #[serde(default)]
    speed: Option<i32>,
    #[serde(default)]
    acceleration: Option<i32>,
}

#[derive(Debug, Deserialize, Default)]
struct WalkMovementManifest {
    #[serde(default)]
    speed: Option<i32>,
    #[serde(default)]
    acceleration: Option<i32>,
}

#[derive(Debug, Deserialize, Default)]
struct ScaleMovementManifest {
    #[serde(default)]
    speed: Option<i32>,
    #[serde(default)]
    acceleration: Option<i32>,
}

#[derive(Debug, Deserialize, Default)]
struct HangleMovementManifest {
    #[serde(default)]
    speed: Option<i32>,
    #[serde(default)]
    acceleration: Option<i32>,
}

#[derive(Debug, Deserialize, Default)]
struct DigMovementManifest {
    #[serde(default)]
    speed: Option<i32>,
}

impl MovementManifest {
    pub(in crate::scenario) fn into_profile(
        self,
        id: &str,
    ) -> Result<MovementProfile, ScenarioError> {
        let mut profile = MovementProfile::default();
        if let Some(float) = self.float {
            if let Some(speed) = float.speed {
                if speed < 0 {
                    return Err(ScenarioError::InvalidMovement {
                        id: id.to_string(),
                        detail: format!("float.speed must be >= 0 (got {speed})"),
                    });
                }
                profile.float_speed = speed;
            }
            if let Some(acceleration) = float.acceleration {
                if acceleration < 0 {
                    return Err(ScenarioError::InvalidMovement {
                        id: id.to_string(),
                        detail: format!("float.acceleration must be >= 0 (got {acceleration})"),
                    });
                }
                profile.float_acceleration = acceleration;
            }
        }
        if let Some(swim) = self.swim {
            if let Some(speed) = swim.speed {
                if speed < 0 {
                    return Err(ScenarioError::InvalidMovement {
                        id: id.to_string(),
                        detail: format!("swim.speed must be >= 0 (got {speed})"),
                    });
                }
                profile.swim_speed = speed;
            }
            if let Some(acceleration) = swim.acceleration {
                if acceleration < 0 {
                    return Err(ScenarioError::InvalidMovement {
                        id: id.to_string(),
                        detail: format!("swim.acceleration must be >= 0 (got {acceleration})"),
                    });
                }
                profile.swim_acceleration = acceleration;
            }
        }
        if let Some(walk) = self.walk {
            if let Some(speed) = walk.speed {
                if speed < 0 {
                    return Err(ScenarioError::InvalidMovement {
                        id: id.to_string(),
                        detail: format!("walk.speed must be >= 0 (got {speed})"),
                    });
                }
                profile.walk_speed = speed;
            }
            if let Some(acceleration) = walk.acceleration {
                if acceleration < 0 {
                    return Err(ScenarioError::InvalidMovement {
                        id: id.to_string(),
                        detail: format!("walk.acceleration must be >= 0 (got {acceleration})"),
                    });
                }
                profile.walk_acceleration = acceleration;
            }
        }
        if let Some(scale) = self.scale {
            if let Some(speed) = scale.speed {
                if speed < 0 {
                    return Err(ScenarioError::InvalidMovement {
                        id: id.to_string(),
                        detail: format!("scale.speed must be >= 0 (got {speed})"),
                    });
                }
                profile.scale_speed = speed;
            }
            if let Some(acceleration) = scale.acceleration {
                if acceleration < 0 {
                    return Err(ScenarioError::InvalidMovement {
                        id: id.to_string(),
                        detail: format!("scale.acceleration must be >= 0 (got {acceleration})"),
                    });
                }
                profile.scale_acceleration = acceleration;
            }
        }
        if let Some(hangle) = self.hangle {
            if let Some(speed) = hangle.speed {
                if speed < 0 {
                    return Err(ScenarioError::InvalidMovement {
                        id: id.to_string(),
                        detail: format!("hangle.speed must be >= 0 (got {speed})"),
                    });
                }
                profile.hangle_speed = speed;
            }
            if let Some(acceleration) = hangle.acceleration {
                if acceleration < 0 {
                    return Err(ScenarioError::InvalidMovement {
                        id: id.to_string(),
                        detail: format!("hangle.acceleration must be >= 0 (got {acceleration})"),
                    });
                }
                profile.hangle_acceleration = acceleration;
            }
        }
        if let Some(dig) = self.dig {
            if let Some(speed) = dig.speed {
                if speed < 0 {
                    return Err(ScenarioError::InvalidMovement {
                        id: id.to_string(),
                        detail: format!("dig.speed must be >= 0 (got {speed})"),
                    });
                }
                profile.dig_speed = speed;
            }
        }
        Ok(profile)
    }
}

#[derive(Debug, Deserialize)]
pub(in crate::scenario) struct ObjectManifest {
    pub(in crate::scenario) definition: String,
    #[serde(default)]
    pub(in crate::scenario) position: Option<[i32; 2]>,
    #[serde(default)]
    pub(in crate::scenario) velocity: Option<[i32; 2]>,
    #[serde(default)]
    pub(in crate::scenario) energy: Option<i32>,
    #[serde(default)]
    pub(in crate::scenario) owner: Option<i32>,
    #[serde(default)]
    pub(in crate::scenario) action: Option<ActionManifest>,
    #[serde(default)]
    pub(in crate::scenario) effects: Vec<EffectManifest>,
    #[serde(default)]
    pub(in crate::scenario) crew_member: Option<bool>,
    #[serde(default)]
    pub(in crate::scenario) alive: Option<bool>,
    #[serde(default)]
    pub(in crate::scenario) status: Option<ObjectStatusSpec>,
    #[serde(default)]
    pub(in crate::scenario) handle: Option<String>,
    #[serde(default)]
    pub(in crate::scenario) container: Option<String>,
    #[serde(default)]
    pub(in crate::scenario) category: Option<i32>,
}

#[derive(Debug)]
pub(in crate::scenario) struct ObjectStatusSpec(ObjectStatus);

impl ObjectStatusSpec {
    fn from_name(name: &str) -> Option<ObjectStatus> {
        if name.eq_ignore_ascii_case("deleted") {
            Some(ObjectStatus::Deleted)
        } else if name.eq_ignore_ascii_case("normal") {
            Some(ObjectStatus::Normal)
        } else if name.eq_ignore_ascii_case("inactive") {
            Some(ObjectStatus::Inactive)
        } else {
            None
        }
    }

    fn from_code(code: i64) -> Option<ObjectStatus> {
        match code {
            0 => Some(ObjectStatus::Deleted),
            1 => Some(ObjectStatus::Normal),
            2 => Some(ObjectStatus::Inactive),
            _ => None,
        }
    }
}

impl From<ObjectStatusSpec> for ObjectStatus {
    fn from(spec: ObjectStatusSpec) -> Self {
        spec.0
    }
}

impl<'de> Deserialize<'de> for ObjectStatusSpec {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct StatusVisitor;

        impl<'de> Visitor<'de> for StatusVisitor {
            type Value = ObjectStatusSpec;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(
                    "an object status (\"deleted\", \"normal\", \"inactive\") or numeric code 0/1/2",
                )
            }

            fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                ObjectStatusSpec::from_name(value)
                    .map(ObjectStatusSpec)
                    .ok_or_else(|| E::custom(format!("unknown object status `{value}`")))
            }

            fn visit_string<E>(self, value: String) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                self.visit_str(&value)
            }

            fn visit_i64<E>(self, value: i64) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                ObjectStatusSpec::from_code(value)
                    .map(ObjectStatusSpec)
                    .ok_or_else(|| E::custom(format!("unsupported object status code {value}")))
            }

            fn visit_u64<E>(self, value: u64) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                if value > i64::MAX as u64 {
                    return Err(E::custom(format!("unsupported object status code {value}")));
                }
                self.visit_i64(value as i64)
            }
        }

        deserializer.deserialize_any(StatusVisitor)
    }
}

#[derive(Debug, Deserialize)]
pub(in crate::scenario) struct ActionManifest {
    name: String,
    #[serde(default)]
    phase: Option<i32>,
    #[serde(default)]
    ticks: Option<i32>,
    #[serde(default)]
    data: Option<i32>,
}

impl ActionManifest {
    pub(in crate::scenario) fn into_state(self) -> ActionState {
        let mut state = ActionState::new(self.name);
        if let Some(phase) = self.phase {
            state.phase = phase;
        }
        if let Some(ticks) = self.ticks {
            state.ticks = ticks;
        }
        if let Some(data) = self.data {
            state.data = data;
        }
        state
    }
}

#[derive(Debug, Deserialize)]
pub(in crate::scenario) struct EffectManifest {
    name: String,
    #[serde(default = "EffectManifest::default_priority")]
    priority: i32,
    #[serde(default = "EffectManifest::default_interval")]
    interval: i32,
    #[serde(default)]
    timer: i32,
}

impl EffectManifest {
    fn default_priority() -> i32 {
        100
    }

    fn default_interval() -> i32 {
        1
    }

    pub(in crate::scenario) fn into_state(self) -> EffectState {
        EffectState::new(self.name)
            .with_priority(self.priority)
            .with_interval(self.interval)
            .with_timer(self.timer)
    }
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(in crate::scenario) enum LandscapeManifest {
    Flat { width: u32, height: i32 },
    HeightMap { width: u32, heights: Vec<i32> },
}

impl LandscapeManifest {
    pub(in crate::scenario) fn into_landscape(self) -> Result<Landscape, ScenarioError> {
        match self {
            LandscapeManifest::Flat { width, height } => Ok(Landscape::flat(width, height)),
            LandscapeManifest::HeightMap { width, heights } => Landscape::new(width, heights)
                .map_err(|error| ScenarioError::InvalidLandscape(error.to_string())),
        }
    }
}

#[derive(Debug, Deserialize)]
pub(in crate::scenario) struct PhysicsManifest {
    #[serde(default)]
    pub(in crate::scenario) gravity: Option<i32>,
    #[serde(default)]
    pub(in crate::scenario) max_fall_speed: Option<i32>,
    #[serde(default)]
    pub(in crate::scenario) max_rise_speed: Option<i32>,
    #[serde(default)]
    pub(in crate::scenario) max_horizontal_speed: Option<i32>,
}

impl PhysicsManifest {
    pub(in crate::scenario) fn into_settings(self) -> Result<PhysicsSettings, ScenarioError> {
        // An unset bound means unbounded, matching the pinned engine, which has
        // no terminal-speed limit at all. Only a fixture that names one gets
        // one (clonk-org/clonk-rs#1112).
        PhysicsSettings::from_optional(
            self.gravity.unwrap_or(PhysicsSettings::default().gravity),
            self.max_fall_speed,
            self.max_rise_speed,
            self.max_horizontal_speed,
        )
        .map_err(|detail| ScenarioError::InvalidPhysics(detail.to_string()))
    }
}

#[derive(Debug, Deserialize)]
pub(in crate::scenario) struct EnvironmentManifest {
    #[serde(default)]
    wind: Option<i32>,
    #[serde(default)]
    wind_variation: Option<i32>,
    #[serde(default)]
    wind_period: Option<u32>,
    #[serde(default)]
    temperature: Option<i32>,
    #[serde(default)]
    climate: Option<i32>,
    #[serde(default)]
    temperature_variation: Option<i32>,
    #[serde(default)]
    temperature_period: Option<u32>,
    #[serde(default)]
    temperature_phase: Option<u32>,
    #[serde(default)]
    time_of_day: Option<i32>,
    #[serde(default)]
    time_speed: Option<i32>,
    #[serde(default)]
    precipitation: Option<i32>,
    #[serde(default)]
    sky_color: Option<ColorSpec>,
    #[serde(default)]
    season: Option<i32>,
    #[serde(default)]
    year_speed: Option<i32>,
    #[serde(default)]
    temperature_range: Option<i32>,
    #[serde(default)]
    lightning: Option<i32>,
    #[serde(default)]
    meteorite: Option<i32>,
    #[serde(default)]
    volcano: Option<i32>,
    #[serde(default)]
    earthquake: Option<i32>,
    #[serde(default)]
    precipitation_strength: Option<i32>,
    #[serde(default)]
    gamma_enabled: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub(in crate::scenario) struct SkyManifest {
    #[serde(default)]
    surface: Option<String>,
    #[serde(default)]
    fade_top: Option<ColorSpec>,
    #[serde(default)]
    fade_bottom: Option<ColorSpec>,
    #[serde(default)]
    scroll_mode: Option<String>,
    #[serde(default)]
    parallax_x: Option<i32>,
    #[serde(default)]
    parallax_y: Option<i32>,
    #[serde(default)]
    xdir: Option<f32>,
    #[serde(default)]
    ydir: Option<f32>,
    #[serde(default)]
    modulation: Option<ColorSpec>,
    #[serde(default)]
    back_color: Option<ColorSpec>,
}

#[derive(Debug)]
struct ColorSpec(RgbColor);

impl ColorSpec {
    fn into_color(self) -> RgbColor {
        self.0
    }
}

impl<'de> Deserialize<'de> for ColorSpec {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct ColorVisitor;

        impl<'de> Visitor<'de> for ColorVisitor {
            type Value = ColorSpec;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("a hex string #RRGGBB or an array [r, g, b]")
            }

            fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
            where
                A: SeqAccess<'de>,
            {
                let mut components = Vec::with_capacity(3);
                while let Some(value) = seq.next_element::<i32>()? {
                    if !(0..=255).contains(&value) {
                        return Err(A::Error::custom(format!(
                            "color components must be between 0 and 255 (got {value})"
                        )));
                    }
                    components.push(value as u8);
                }

                if components.len() != 3 {
                    return Err(A::Error::invalid_length(
                        components.len(),
                        &"array with exactly three entries [r, g, b]",
                    ));
                }

                Ok(ColorSpec(RgbColor::new(
                    components[0],
                    components[1],
                    components[2],
                )))
            }

            fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                parse_hex_color(value).map(ColorSpec).map_err(E::custom)
            }

            fn visit_string<E>(self, value: String) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                self.visit_str(&value)
            }
        }

        fn parse_hex_color(value: &str) -> Result<RgbColor, String> {
            let trimmed = value.trim();
            let hex = trimmed.strip_prefix('#').unwrap_or(trimmed);
            if hex.len() != 6 || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
                return Err(format!(
                    "expected hex color in RRGGBB format, got `{}`",
                    value
                ));
            }

            let parse_component = |segment: &str| -> Result<u8, String> {
                u8::from_str_radix(segment, 16)
                    .map_err(|_| format!("invalid hex component `{segment}`"))
            };

            let r = parse_component(&hex[0..2])?;
            let g = parse_component(&hex[2..4])?;
            let b = parse_component(&hex[4..6])?;
            Ok(RgbColor::new(r, g, b))
        }

        deserializer.deserialize_any(ColorVisitor)
    }
}

impl EnvironmentManifest {
    pub(in crate::scenario) fn into_settings(self) -> EnvironmentSettings {
        let mut settings = EnvironmentSettings::new(self.wind.unwrap_or(0));
        if let Some(variation) = self.wind_variation {
            let period = self.wind_period.unwrap_or(120);
            settings = settings.with_wind_variation(variation, period);
        }
        if let Some(climate) = self.climate {
            settings = settings.with_climate(climate);
        }
        if let Some(temperature) = self.temperature {
            settings = settings.with_temperature(temperature);
        }
        if self.temperature_variation.is_some()
            || self.temperature_period.is_some()
            || self.temperature_phase.is_some()
        {
            let variation = self.temperature_variation.unwrap_or(0);
            let period = self.temperature_period.unwrap_or(600);
            let phase = self.temperature_phase.unwrap_or(0);
            settings = settings.with_temperature_cycle(variation, period, phase);
        }
        if let Some(time_of_day) = self.time_of_day {
            settings = settings.with_time_of_day(time_of_day);
        }
        if let Some(time_speed) = self.time_speed {
            settings = settings.with_time_speed(time_speed);
        }
        if let Some(precipitation) = self.precipitation {
            settings = settings.with_precipitation(precipitation);
            if self.precipitation_strength.is_none() {
                settings = settings.with_precipitation_strength(precipitation);
            }
        }
        if let Some(color) = self.sky_color {
            settings = settings.with_sky_color(color.into_color());
        }
        if let Some(season) = self.season {
            settings = settings.with_season(season);
        }
        if let Some(year_speed) = self.year_speed {
            settings = settings.with_year_speed(year_speed);
        }
        if let Some(range) = self.temperature_range {
            settings = settings.with_temperature_range(range);
        }
        if let Some(lightning) = self.lightning {
            settings = settings.with_lightning(lightning);
        }
        if let Some(meteorite) = self.meteorite {
            settings = settings.with_meteorite(meteorite);
        }
        if let Some(volcano) = self.volcano {
            settings = settings.with_volcano(volcano);
        }
        if let Some(earthquake) = self.earthquake {
            settings = settings.with_earthquake(earthquake);
        }
        if let Some(strength) = self.precipitation_strength {
            settings = settings.with_precipitation_strength(strength);
        }
        if let Some(enabled) = self.gamma_enabled {
            settings = if enabled {
                settings.with_gamma_enabled()
            } else {
                settings.with_gamma_disabled()
            };
        }
        settings
    }
}

impl SkyManifest {
    pub(in crate::scenario) fn into_config(
        self,
        group: &Group,
    ) -> Result<SkyConfig, ScenarioError> {
        let mut settings = SkySettings::default();
        let mut surface_image = None;

        if let Some(surface_name) = self.surface {
            let path = PathBuf::from(&surface_name);
            let bytes = match group.read_file(&path) {
                Ok(bytes) => bytes,
                Err(GroupError::EntryNotFound(_)) => {
                    return Err(ScenarioError::SkySurfaceMissing { path });
                }
                Err(GroupError::Io(error)) if error.kind() == io::ErrorKind::NotFound => {
                    return Err(ScenarioError::SkySurfaceMissing { path });
                }
                Err(error) => return Err(ScenarioError::Resources(error)),
            };

            let decoded = load_image_from_memory(&bytes).map_err(|source| {
                ScenarioError::SkySurfaceDecode {
                    path: path.clone(),
                    source,
                }
            })?;
            let rgba = decoded.to_rgba8();
            let (width, height) = rgba.dimensions();
            let pixels = rgba.into_raw();
            settings = settings.with_surface(width, height);
            surface_image = Some(Arc::new(GraphicsImage::new(width, height, pixels)));
        }

        if let Some(color) = self.fade_top {
            settings.fade_top = color.into_color();
        }
        if let Some(color) = self.fade_bottom {
            settings.fade_bottom = color.into_color();
        }
        if let Some(mode) = self.scroll_mode {
            settings.parallax_mode = parse_scroll_mode(&mode)?;
        }
        if let Some(value) = self.parallax_x {
            settings.parallax_x = value;
        }
        if let Some(value) = self.parallax_y {
            settings.parallax_y = value;
        }
        if let Some(value) = self.xdir {
            settings.base_xdir = value;
        }
        if let Some(value) = self.ydir {
            settings.base_ydir = value;
        }
        if let Some(color) = self.modulation {
            settings.modulation = Some(rgb_to_bgr_u32(color.into_color()));
        }
        if let Some(color) = self.back_color {
            let back_color = rgb_to_bgr_u32(color.into_color());
            settings.back_color = Some(back_color);
            settings.back_color_raw = back_color;
        }

        Ok(SkyConfig {
            settings,
            surface: surface_image,
        })
    }
}

fn parse_scroll_mode(value: &str) -> Result<SkyParallaxMode, ScenarioError> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Ok(SkyParallaxMode::Fixed);
    }
    if let Ok(code) = trimmed.parse::<i32>() {
        return match code {
            0 => Ok(SkyParallaxMode::Fixed),
            1 => Ok(SkyParallaxMode::Wind),
            2 => Ok(SkyParallaxMode::Parallax),
            other => Err(ScenarioError::InvalidSky(format!(
                "unknown sky scroll mode code {other}"
            ))),
        };
    }
    match trimmed.to_ascii_lowercase().as_str() {
        "fixed" => Ok(SkyParallaxMode::Fixed),
        "wind" => Ok(SkyParallaxMode::Wind),
        "parallax" => Ok(SkyParallaxMode::Parallax),
        other => Err(ScenarioError::InvalidSky(format!(
            "unknown sky scroll mode `{other}`"
        ))),
    }
}

fn rgb_to_bgr_u32(color: RgbColor) -> u32 {
    u32::from(color.b) | (u32::from(color.g) << 8) | (u32::from(color.r) << 16)
}
