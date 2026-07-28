//! `impl Engine` — global scripts, definition accessors and object spawning.
//!
//! Moved verbatim from the root `impl Engine` block in `lib.rs`.
//! Structural only: same crate, same type, same method bodies.

use super::*;

impl Engine {
    pub fn global_effects(&self) -> &[EffectState] {
        &self.global_effects
    }

    pub fn register_definition(&mut self, definition: Definition) -> Result<(), EngineError> {
        let id = definition.id().to_string();
        if self.definitions.contains_key(&id) {
            return Err(EngineError::DefinitionAlreadyExists(id));
        }

        let mut definition = definition;
        let game_script_name = self
            .scenario_script
            .as_ref()
            .map(|scenario| scenario.script.script_name())
            .unwrap_or("Script.c")
            .to_owned();
        definition.set_game_script_name(game_script_name);
        // C4Game runs the one-shot definition colorization after materials
        // and definitions are loaded. Production Rust loaders establish the
        // final MaterialSet before registering definitions, so applying at
        // this insertion boundary covers legacy, network, and sandbox paths
        // without making the non-idempotent palette replacement repeat.
        definition.colorize_by_material(&self.materials);
        // Preparse constants precede the function-body Hold pass in C4Aul.
        if let Err(diagnostic) = clonk_script::register_global_declarations_with_strings(
            definition.base_script.var_decls(),
            &self.script_globals,
            Some(&self.script_global_consts),
            &self.script_string_registrations,
        ) {
            tracing::warn!(
                definition = %id,
                %diagnostic,
                "definition static-constant link diagnostic; continuing like C++"
            );
        }
        {
            // One GlobalNamed table for every script host: `static`
            // declarations compiled into the definition move to it.
            let script = Arc::make_mut(&mut definition.script);
            script.set_global_variables(self.script_globals.clone());
            script.set_global_slots(self.script_global_slots.clone());
            script.set_global_constants(self.script_global_consts.clone());
            script.set_string_registrations_deferred(self.script_string_registrations.clone());
            script.adopt_statics_into_globals();
        }

        // A def script's `global func`s land on the ENGINE scope like any
        // System.c4g global (C4AulParse "global" storage — the GoldRush
        // Talker's StartMovie is called from the scenario script). C4Aul
        // also leaves a FnLink in the declaring definition; repoint the
        // local copy at the engine-chained function so inherited() follows
        // OwnerOverloaded without later definitions stealing this link.
        let def_globals: Vec<(String, clonk_script::Function)> = definition
            .script
            .global_access_functions()
            .map(|(name, function)| (name.clone(), function.clone()))
            .collect();
        if !def_globals.is_empty() {
            let def_global_order = definition
                .script
                .global_function_names_in_link_order()
                .map(str::to_owned)
                .collect::<Vec<_>>();
            let mut functions: rustc_hash::FxHashMap<String, clonk_script::Function> = self
                .global_script_functions
                .as_deref()
                .cloned()
                .unwrap_or_default();
            for (function_name, mut function) in def_globals {
                if let Some(previous) = functions.remove(&function_name) {
                    function.push_overload(previous);
                }
                Arc::make_mut(&mut definition.script)
                    .link_global_access_function(&function_name, function.clone());
                functions.insert(function_name, function);
            }
            let table = Some(Arc::new(functions));
            let mut function_order = self.global_script_function_order.clone();
            function_order.extend(def_global_order);
            self.distribute_global_script_functions(table, function_order);
        }
        definition.set_global_functions(self.global_script_functions.clone());
        let definition_id = DefinitionId::from(id.as_str());
        self.script_link_sources
            .push(ScriptLinkSource::Definition(definition_id.clone()));
        self.definition_load_order.push(definition_id.clone());
        let runtime_order = Rc::make_mut(&mut self.runtime_definition_order);
        runtime_order.push(definition_id);
        runtime_order.sort_unstable_by_key(|id| {
            definition_id_to_c4id(id.as_str())
                .map(|id| id as u32)
                .unwrap_or_default()
        });
        self.definitions.insert(id, definition);
        self.definition_metadata_cache.borrow_mut().take();
        self.command_definition_snapshot_cache.borrow_mut().take();
        self.invalidate_host_definition_tables();
        self.solid_mask_metadata_cache.borrow_mut().take();
        Ok(())
    }

    /// Installs the engine-global script functions — the System.c4g
    /// `global func`s that C++ owns on `Game.ScriptEngine`
    /// (resolution per FindSameNameFunc: own-def script first, engine-owned
    /// fallback, C4Aul.cpp:130-148). Scripts that fail to compile log and
    /// are skipped like C++. The table is shared into every registered
    /// (and future) script host.
    pub(crate) fn distribute_global_script_functions(
        &mut self,
        table: Option<Arc<rustc_hash::FxHashMap<String, clonk_script::Function>>>,
        function_order: Vec<String>,
    ) {
        self.global_script_functions = table.clone();
        self.global_script_function_order = function_order;
        for definition in self.definitions.values_mut() {
            definition.set_global_functions(table.clone());
        }
        if let Some(scenario) = self.scenario_script.as_mut() {
            scenario.set_global_functions(table.clone());
        }
        for source in &mut self.script_link_sources {
            if let ScriptLinkSource::Script { script, .. } = source {
                Arc::make_mut(script).set_global_functions(table.clone());
            }
        }
        self.invalidate_host_definition_tables();
    }

    pub(crate) fn global_menu_callback_script(
        &self,
        function: &str,
    ) -> Option<(String, Arc<ScriptEngine>)> {
        self.script_link_sources.iter().find_map(|source| {
            let (name, script) = match source {
                ScriptLinkSource::Script { name, script, .. } => (name.clone(), Arc::clone(script)),
                ScriptLinkSource::Definition(id) => {
                    let definition = self.definitions.get(id.as_str())?;
                    (id.to_string(), definition.script_arc())
                }
                ScriptLinkSource::Scenario => {
                    let scenario = self.scenario_script.as_ref()?;
                    (scenario.name.clone(), scenario.script_arc())
                }
            };
            script
                .resolve_function(function, true)
                .is_some_and(|resolution| {
                    resolution.scope == clonk_script::ScriptFunctionScope::Global
                        && resolution.host_identity == script.host_identity()
                })
                .then_some((name, script))
        })
    }

    pub(crate) fn global_menu_condition_resolves(&self, function: &str, condition: &str) -> bool {
        self.global_menu_callback_script(function)
            .is_some_and(|(_, script)| script.resolve_function(condition, true).is_some())
            || self
                .global_script_functions
                .as_deref()
                .is_some_and(|functions| functions.contains_key(condition))
    }

    pub fn install_global_scripts(&mut self, sources: &[(String, String)]) -> usize {
        self.global_script_functions = None;
        self.global_script_function_order.clear();
        self.script_link_sources
            .retain(|source| !matches!(source, ScriptLinkSource::Script { .. }));
        self.install_additional_global_scripts(sources)
    }

    /// Adds global scripts ON TOP of the installed table: later definitions
    /// overload earlier ones and `inherited` reaches them (C++ link order —
    /// the scenario's System.c4g joins the engine's, C4Game.cpp:3317-3343).
    pub fn install_additional_global_scripts(&mut self, sources: &[(String, String)]) -> usize {
        self.install_global_scripts_at(sources)
    }

    /// Installs scenario-local System.c4g after all definitions, matching the
    /// explicit C++ overload-priority phase (C4Game.cpp:2606-2617).
    pub fn install_scenario_global_scripts(&mut self, sources: &[(String, String)]) -> usize {
        self.install_global_scripts_at(sources)
    }

    fn install_global_scripts_at(&mut self, sources: &[(String, String)]) -> usize {
        let game_script_name = self
            .scenario_script
            .as_ref()
            .map(|scenario| scenario.script.script_name())
            .unwrap_or("Script.c")
            .to_owned();
        let mut functions: rustc_hash::FxHashMap<String, clonk_script::Function> = self
            .global_script_functions
            .as_deref()
            .cloned()
            .unwrap_or_default();
        let mut function_order = self.global_script_function_order.clone();
        let mut loaded = 0usize;
        for (name, source) in sources {
            match clonk_script::Script::compile_global_c4_string(source) {
                Ok(compiled) => {
                    for diagnostic in compiled.parse_diagnostics() {
                        tracing::warn!(
                            script = %name,
                            %diagnostic,
                            "global script compile diagnostic; continuing like C++"
                        );
                    }
                    // System/scenario System.c4g declarations participate in
                    // the same engine-global GlobalNamed/GlobalConsts tables
                    // as definition scripts (C4Aul preparser and
                    // RegisterGlobalConstant, C4Aul.cpp:484-492).
                    if let Err(diagnostic) = clonk_script::register_global_declarations_with_strings(
                        compiled.var_decls(),
                        &self.script_globals,
                        Some(&self.script_global_consts),
                        &self.script_string_registrations,
                    ) {
                        tracing::warn!(
                            script = %name,
                            %diagnostic,
                            "global script static-constant link diagnostic; continuing like C++"
                        );
                    }
                    let mut script = ScriptEngine::new();
                    script.set_script_name(name.clone());
                    script.set_game_script_name(game_script_name.clone());
                    script.add_script(compiled.clone().without_static_declarations());
                    script.set_global_variables(self.script_globals.clone());
                    script.set_global_slots(self.script_global_slots.clone());
                    script.set_global_constants(self.script_global_consts.clone());
                    script.set_string_registrations_deferred(
                        self.script_string_registrations.clone(),
                    );
                    script.set_global_functions(self.global_script_functions.clone());
                    compat::register_host_functions(&mut script);
                    let declarations = script
                        .global_access_functions()
                        .map(|(function_name, function)| (function_name.clone(), function.clone()))
                        .collect::<Vec<_>>();
                    function_order.extend(
                        script
                            .global_function_names_in_link_order()
                            .map(str::to_owned),
                    );
                    for (function_name, mut function) in declarations {
                        if let Some(previous) = functions.remove(&function_name) {
                            function.push_overload(previous);
                        }
                        script.link_global_access_function(&function_name, function.clone());
                        functions.insert(function_name, function);
                    }
                    #[allow(clippy::arc_with_non_send_sync)] // single-threaded sharing
                    let script = Arc::new(script);
                    self.script_link_sources.push(ScriptLinkSource::Script {
                        name: name.clone(),
                        base_script: compiled,
                        script,
                    });
                    loaded += 1;
                }
                Err(error) => {
                    tracing::warn!(
                        script = %name,
                        %error,
                        "global script failed to compile; skipping like C++"
                    );
                }
            }
        }
        let table = (!functions.is_empty()).then(|| Arc::new(functions));
        self.distribute_global_script_functions(table, function_order);
        loaded
    }

    /// `C4AulScript::ResolveAppends` (C4AulLink.cpp:29-64): every script's
    /// `#appendto` targets receive a COPY of its non-global functions as
    /// overrides (AppendTo with bHighPrio, :114-141). MUST run before
    /// include resolution (":27-28 ResolveAppends has to be called
    /// first!"). Sources follow the single script-host registration order:
    /// definition-pack System hosts stay interleaved with definitions, then
    /// scenario Script.c and scenario System.c4g follow (C4Def.cpp:927-968;
    /// C4Game.cpp:2606-2617). That order decides the overload chain when
    /// several appends hit the same target function. Unknown targets warn and skip
    /// (:42-49); `#appendto *` reaches every definition except the source
    /// (:53-60).
    pub fn resolve_appends(&mut self) {
        if self.script_link_sources.is_empty() {
            return;
        }

        let ordered_ids = self.definition_load_order.clone();
        for source in self.script_link_sources.clone() {
            let (source_script, source_id, targets) = match source {
                ScriptLinkSource::Script {
                    base_script,
                    script,
                    ..
                } => {
                    let targets = base_script.appends().to_vec();
                    if targets.is_empty() {
                        continue;
                    }
                    (script, None, targets)
                }
                ScriptLinkSource::Definition(id) => match self.definitions.get(&id) {
                    Some(definition) => {
                        let targets = definition.appends.clone();
                        if targets.is_empty() {
                            continue;
                        }
                        (definition.script.clone(), Some(id), targets)
                    }
                    None => continue,
                },
                ScriptLinkSource::Scenario => {
                    let Some((script, script_name)) = self
                        .scenario_script
                        .as_ref()
                        .map(|scenario| (scenario.base_script.clone(), scenario.name.clone()))
                    else {
                        continue;
                    };
                    let targets = script.appends().to_vec();
                    if targets.is_empty() {
                        continue;
                    }
                    let mut engine = ScriptEngine::new();
                    engine.set_script_name(script_name);
                    engine.add_script(script.without_static_declarations());
                    engine.set_global_variables(self.script_globals.clone());
                    engine.set_global_slots(self.script_global_slots.clone());
                    engine.set_global_constants(self.script_global_consts.clone());
                    engine.set_string_registrations_deferred(
                        self.script_string_registrations.clone(),
                    );
                    #[allow(clippy::arc_with_non_send_sync)] // single-threaded sharing
                    (Arc::new(engine), None, targets)
                }
            };
            let source_definition = source_id
                .as_ref()
                .and_then(|id| self.definitions.get(id))
                .cloned();
            for target in targets {
                let resolved: Vec<DefinitionId> = match &target {
                    clonk_script::AppendTo::Id { id: token, nowarn } => {
                        let id = DefinitionId::from(token.as_str());
                        if self.definitions.contains_key(&id) {
                            vec![id]
                        } else if !*nowarn {
                            // "script to #appendto not found"
                            // (C4AulLink.cpp:42-49) — a warning, never an
                            // error.
                            tracing::warn!(target = %token, "script to #appendto not found");
                            Vec::new()
                        } else {
                            Vec::new()
                        }
                    }
                    clonk_script::AppendTo::Wildcard => ordered_ids
                        .iter()
                        .filter(|id| Some(*id) != source_id.as_ref())
                        .cloned()
                        .collect(),
                };
                for target_id in resolved {
                    if let Some(definition) = self.definitions.get_mut(&target_id) {
                        if let Some(source) = source_definition.as_ref() {
                            definition.include_definition_metadata(source);
                        }
                        definition.mark_callbacks_unlinked();
                        Arc::make_mut(&mut definition.script).append_overrides_from(&source_script);
                        definition.refresh_script_flags();
                    }
                }
            }
        }
        self.definition_metadata_cache.borrow_mut().take();
        self.command_definition_snapshot_cache.borrow_mut().take();
        self.invalidate_host_definition_tables();
        self.solid_mask_metadata_cache.borrow_mut().take();
    }

    fn rebuild_global_script_functions(&mut self) {
        fn chain_function(
            functions: &mut rustc_hash::FxHashMap<String, clonk_script::Function>,
            name: String,
            mut function: clonk_script::Function,
        ) -> clonk_script::Function {
            if let Some(previous) = functions.remove(&name) {
                function.push_overload(previous);
            }
            functions.insert(name, function.clone());
            function
        }

        let reloaded = self
            .reloaded_global_definitions
            .iter()
            .cloned()
            .collect::<HashSet<_>>();
        let mut sources = self.script_link_sources.clone();
        sources.retain(
            |source| !matches!(source, ScriptLinkSource::Definition(id) if reloaded.contains(id)),
        );
        sources.extend(
            self.reloaded_global_definitions
                .iter()
                .cloned()
                .map(ScriptLinkSource::Definition),
        );

        let mut functions = rustc_hash::FxHashMap::default();
        let mut function_order = Vec::new();
        for source in sources {
            match source {
                ScriptLinkSource::Script { script, .. } => {
                    let host_identity = script.host_identity();
                    function_order.extend(
                        script
                            .global_function_names_in_link_order()
                            .map(str::to_owned),
                    );
                    let declarations = script
                        .global_access_functions()
                        .map(|(name, function)| (name.clone(), function.clone()))
                        .collect::<Vec<_>>();
                    drop(script);
                    for (name, function) in declarations {
                        let linked = chain_function(&mut functions, name.clone(), function);
                        if let Some(retained) =
                            self.script_link_sources
                                .iter_mut()
                                .find_map(|source| match source {
                                    ScriptLinkSource::Script { script, .. }
                                        if script.host_identity() == host_identity =>
                                    {
                                        Some(script)
                                    }
                                    _ => None,
                                })
                        {
                            Arc::make_mut(retained).link_global_access_function(&name, linked);
                        }
                    }
                }
                ScriptLinkSource::Definition(id) => {
                    let Some((declarations, declaration_order)) =
                        self.definitions.get(&id).map(|definition| {
                            (
                                definition
                                    .script
                                    .global_access_functions()
                                    .map(|(name, function)| (name.clone(), function.clone()))
                                    .collect::<Vec<_>>(),
                                definition
                                    .script
                                    .global_function_names_in_link_order()
                                    .map(str::to_owned)
                                    .collect::<Vec<_>>(),
                            )
                        })
                    else {
                        continue;
                    };
                    function_order.extend(declaration_order);
                    for (name, function) in declarations {
                        let linked = chain_function(&mut functions, name.clone(), function);
                        if let Some(definition) = self.definitions.get_mut(&id) {
                            Arc::make_mut(&mut definition.script)
                                .link_global_access_function(&name, linked);
                        }
                    }
                }
                ScriptLinkSource::Scenario => {
                    let Some((declarations, declaration_order)) =
                        self.scenario_script.as_ref().map(|scenario| {
                            (
                                scenario
                                    .script
                                    .global_access_functions()
                                    .map(|(name, function)| (name.clone(), function.clone()))
                                    .collect::<Vec<_>>(),
                                scenario
                                    .script
                                    .global_function_names_in_link_order()
                                    .map(str::to_owned)
                                    .collect::<Vec<_>>(),
                            )
                        })
                    else {
                        continue;
                    };
                    function_order.extend(declaration_order);
                    for (name, function) in declarations {
                        let linked = chain_function(&mut functions, name.clone(), function);
                        if let Some(scenario) = self.scenario_script.as_mut() {
                            Arc::make_mut(&mut scenario.script)
                                .link_global_access_function(&name, linked);
                        }
                    }
                }
            }
        }

        let table = (!functions.is_empty()).then(|| Arc::new(functions));
        self.distribute_global_script_functions(table, function_order);
        self.definition_metadata_cache.borrow_mut().take();
        self.solid_mask_metadata_cache.borrow_mut().take();
    }

    /// Rebuilds the complete script tree from its preparsed hosts. Shared
    /// engine-global static and constant cells deliberately remain intact;
    /// only linked function copies and dependency state are discarded.
    pub fn relink_scripts(&mut self) -> Result<(), EngineError> {
        clonk_script::clear_c4_string_holds(&self.script_string_registrations);
        // UnLink restores every preparsed host without reacquiring function-
        // body Holds. The one global Parse pass after append/include linking
        // below reacquires them in engine child-list order.
        for index in 0..self.script_link_sources.len() {
            match self.script_link_sources[index].clone() {
                ScriptLinkSource::Script { base_script, .. } => {
                    let ScriptLinkSource::Script { script, .. } =
                        &mut self.script_link_sources[index]
                    else {
                        unreachable!()
                    };
                    Arc::make_mut(script).replace_script_deferred(base_script, false);
                }
                ScriptLinkSource::Definition(id) => {
                    if let Some(definition) = self.definitions.get_mut(&id) {
                        definition.reset_script_links();
                    }
                }
                ScriptLinkSource::Scenario => {
                    if let Some(scenario) = self.scenario_script.as_mut() {
                        scenario.reset_script_links();
                    }
                }
            }
        }

        self.rebuild_global_script_functions();
        self.resolve_appends();
        self.resolve_includes()?;
        self.definition_metadata_cache.borrow_mut().take();
        self.solid_mask_metadata_cache.borrow_mut().take();
        Ok(())
    }

    /// Replaces one definition's Script.c preparsed body and performs a full
    /// ReLink. This is the source-backed core used by future disk/file-watch
    /// reload frontends; an unknown definition mirrors ReloadScript's false
    /// result without mutating the engine.
    pub fn reload_definition_script(
        &mut self,
        definition_id: &str,
        source: &str,
    ) -> Result<bool, EngineError> {
        if !self.definitions.contains_key(definition_id) {
            return Ok(false);
        }
        let script = clonk_script::Script::compile_c4_string(source).map_err(|parse_error| {
            EngineError::Script {
                definition: definition_id.to_owned(),
                function: "reload".to_owned(),
                source: parse_error.into(),
                recovery: None,
            }
        })?;
        for diagnostic in script.parse_diagnostics() {
            tracing::warn!(
                definition = %definition_id,
                %diagnostic,
                "definition script reload parse error quarantined; continuing like C++"
            );
        }

        // A reloaded preparser registers only this host's declarations.
        // Existing static cells keep their values; constants reuse and
        // overwrite their cells. Pure ReLink below must not re-register the
        // unchanged hosts.
        if let Err(diagnostic) = clonk_script::register_global_declarations_with_strings(
            script.var_decls(),
            &self.script_globals,
            Some(&self.script_global_consts),
            &self.script_string_registrations,
        ) {
            tracing::warn!(
                definition = %definition_id,
                %diagnostic,
                "definition reload static-constant link diagnostic; continuing like C++"
            );
        }
        self.definitions
            .get_mut(definition_id)
            .expect("definition existence checked")
            .replace_base_script(source, script);
        let definition_id = DefinitionId::from(definition_id);
        self.reloaded_global_definitions
            .retain(|candidate| candidate != &definition_id);
        self.reloaded_global_definitions.push(definition_id);
        self.relink_scripts()?;
        Ok(true)
    }

    /// C4AulScriptEngine::Link's final recursive Parse pass. Every host has
    /// already preparsed its declarations, so static-constant string Refs are
    /// present before the first function-body operand receives Hold.
    fn acquire_script_string_holds(&mut self) {
        for index in 0..self.script_link_sources.len() {
            match self.script_link_sources[index].clone() {
                ScriptLinkSource::Script { .. } => {
                    let ScriptLinkSource::Script { script, .. } =
                        &mut self.script_link_sources[index]
                    else {
                        unreachable!()
                    };
                    Arc::make_mut(script).acquire_string_literal_holds();
                }
                ScriptLinkSource::Definition(id) => {
                    if let Some(definition) = self.definitions.get_mut(&id) {
                        Arc::make_mut(&mut definition.script).acquire_string_literal_holds();
                    }
                }
                ScriptLinkSource::Scenario => {
                    if let Some(scenario) = self.scenario_script.as_mut() {
                        Arc::make_mut(&mut scenario.script).acquire_string_literal_holds();
                    }
                }
            }
        }
    }

    /// Recollects every persistent host's `global func` declarations in
    /// script-tree order. Kept as the public linking seam used by loaders.
    pub fn collect_definition_global_functions(&mut self) {
        self.rebuild_global_script_functions();
    }

    pub fn resolve_includes(&mut self) -> Result<(), EngineError> {
        self.definition_metadata_cache.borrow_mut().take();
        self.command_definition_snapshot_cache.borrow_mut().take();
        self.invalidate_host_definition_tables();
        self.solid_mask_metadata_cache.borrow_mut().take();
        fn resolve_definition(
            engine: &mut Engine,
            child_id: &str,
            resolving: &mut HashSet<String>,
            resolved: &mut HashSet<String>,
        ) -> Result<bool, EngineError> {
            if engine
                .definitions
                .get(child_id)
                .is_some_and(|definition| definition.includes_resolved)
            {
                resolved.insert(child_id.to_string());
                return Ok(true);
            }
            if resolved.contains(child_id) {
                return Ok(true);
            }
            // C4AulScript::ResolveIncludes marks the recursive edge failed
            // and lets its caller skip that include (C4AulLink.cpp:72-97).
            if !resolving.insert(child_id.to_string()) {
                tracing::warn!(
                    definition = %child_id,
                    "Circular include chain detected - ignoring all includes!"
                );
                if let Some(definition) = engine.definitions.get_mut(child_id) {
                    definition.includes_resolved = true;
                }
                return Ok(false);
            }
            let includes = engine
                .definitions
                .get(child_id)
                .map(|definition| definition.includes().to_vec())
                .unwrap_or_default();

            // C4AulParseState::Parse_Script pushes each declaration to the
            // FRONT, so ResolveIncludes sees sibling includes last-declared
            // first (C4AulParse.cpp:1456; C4AulLink.cpp:86-96).
            for parent_id in includes.iter().rev() {
                if !engine.definitions.contains_key(parent_id) {
                    tracing::warn!(
                        target = %parent_id,
                        definition = %child_id,
                        "script to #include not found"
                    );
                    continue;
                }
                if !resolve_definition(engine, parent_id, resolving, resolved)? {
                    continue;
                }
                let parent = engine
                    .definitions
                    .get(parent_id)
                    .expect("checked include exists")
                    .clone();
                if let Some(child) = engine.definitions.get_mut(child_id) {
                    child.merge_from(&parent);
                }
            }
            resolving.remove(child_id);
            if let Some(definition) = engine.definitions.get_mut(child_id) {
                definition.includes_resolved = true;
            }
            resolved.insert(child_id.to_string());
            Ok(true)
        }

        // C4AulScriptEngine resolves child scripts in registration order;
        // this also makes the skipped edge deterministic for include cycles.
        let definition_ids = self.definition_load_order.clone();
        let mut resolving = HashSet::new();
        let mut resolved = HashSet::new();
        for definition_id in definition_ids {
            let _ = resolve_definition(self, &definition_id, &mut resolving, &mut resolved)?;
        }

        // Game.Script is a regular C4AulScript host too. Resolve its
        // definition includes only after every definition has received
        // #appendto copies, so callback lookup and execution see the exact
        // append-before-include symbol set (C4AulLink.cpp:27-29,83-95).
        let scenario_includes = self
            .scenario_script
            .as_ref()
            .filter(|scenario| !scenario.includes_resolved)
            .map(|scenario| scenario.base_script.includes().to_vec());
        if let Some(includes) = scenario_includes {
            for parent_id in includes.iter().rev() {
                if !self.definitions.contains_key(parent_id.as_str()) {
                    tracing::warn!(
                        target = %parent_id,
                        definition = "Scenario",
                        "script to #include not found"
                    );
                    continue;
                }
                if !resolve_definition(self, parent_id, &mut resolving, &mut resolved)? {
                    continue;
                }
                let parent_script = self
                    .definitions
                    .get(parent_id.as_str())
                    .expect("checked scenario include exists")
                    .script
                    .clone();
                if let Some(scenario) = self.scenario_script.as_mut() {
                    Arc::make_mut(&mut scenario.script).merge_from(&parent_script);
                }
            }
            if let Some(scenario) = self.scenario_script.as_mut() {
                scenario.includes_resolved = true;
                scenario.refresh_script_flags();
            }
        }

        // Native Link performs Parse only after every append/include has been
        // resolved. This is also the first point where initial-load function
        // literals may acquire Hold; all hosts' constants were preparsed while
        // they were installed.
        self.acquire_script_string_holds();

        // Native AfterLink resolves these only once the complete function
        // tree exists. UnLink/reload clears the cache before rebuilding it.
        for definition_id in self.definition_load_order.clone() {
            if let Some(definition) = self.definitions.get_mut(&definition_id) {
                definition.link_callbacks();
            }
        }
        self.definition_metadata_cache.borrow_mut().take();

        Ok(())
    }

    /// Read-only definition access. Keeping mutation behind engine methods
    /// ensures definition-derived runtime caches cannot become stale.
    pub fn definition(&self, definition_id: &str) -> Option<&Definition> {
        self.definitions.get(definition_id)
    }

    pub fn definition_name(&self, definition_id: &str) -> Option<&str> {
        self.definition(definition_id).map(Definition::name)
    }

    pub fn definition_description(&self, definition_id: &str) -> Option<&str> {
        self.definitions
            .get(definition_id)
            .and_then(Definition::description)
    }

    /// Whether the definition's script defines `function`
    /// (C4AulScript::GetSFunc; used by the presentation-side
    /// C4Object::DrawCommands port, src/C4ScriptHost.cpp:100-120).
    pub fn definition_script_has_function(&self, definition_id: &str, function: &str) -> bool {
        self.definitions
            .get(definition_id)
            .is_some_and(|definition| definition.has_function(function))
    }

    /// The first caption segment returned by `C4ScriptHost::GetControlDesc`.
    /// A nonempty raw descriptor may intentionally begin with `|`, yielding
    /// an empty caption rather than falling back to the receiver's name.
    pub fn definition_control_description(
        &self,
        definition_id: &str,
        function: &str,
    ) -> Option<String> {
        let definition = self.definitions.get(definition_id)?;
        let resolution = definition.script.resolve_function(function, false)?;
        let description = resolution
            .function
            .description
            .as_deref()
            .filter(|description| !description.is_empty())?;
        Some(description.split('|').next().unwrap_or_default().to_owned())
    }

    /// The definition's raw script source — presentation-side descriptor
    /// extraction (GetControlDesc, src/C4ScriptHost.cpp:151-172).
    pub fn definition_script_source(&self, definition_id: &str) -> Option<&str> {
        self.definitions
            .get(definition_id)
            .map(|definition| definition.script_source.as_str())
    }

    /// The definition's `#include` chain ids (C4Def script includes).
    pub fn definition_includes(&self, definition_id: &str) -> Option<&[String]> {
        self.definitions
            .get(definition_id)
            .map(|definition| definition.includes())
    }

    /// `Def->GrabPutGet` (src/C4Def.cpp:364-373).
    pub fn definition_grab_put_get(&self, definition_id: &str) -> i32 {
        self.definitions
            .get(definition_id)
            .map(|definition| definition.grab_put_get())
            .unwrap_or(0)
    }

    /// Raw DefCore `HideHUDBars`; missing definitions use C++'s zero default.
    pub fn definition_hide_hud_bars(&self, definition_id: &str) -> i32 {
        self.definitions
            .get(definition_id)
            .map(Definition::hide_hud_bars)
            .unwrap_or(0)
    }

    /// Raw DefCore `HideHUDElements`; missing definitions use C++'s zero default.
    pub fn definition_hide_hud_elements(&self, definition_id: &str) -> i32 {
        self.definitions
            .get(definition_id)
            .map(Definition::hide_hud_elements)
            .unwrap_or(0)
    }

    /// The definition's DefCore Category bits (C4Def::Category), e.g. for
    /// filtering C4D_Goal/C4D_Rule objects like `C4MainMenu::ActivateRules`
    /// (C4MainMenu.cpp:392-400).
    pub fn definition_category(&self, definition_id: &str) -> Option<i32> {
        self.definitions
            .get(definition_id)
            .map(|definition| definition.category())
    }

    pub fn definition_value(&self, definition_id: &str) -> Option<i32> {
        self.definitions
            .get(definition_id)
            .map(|definition| definition.value())
    }

    /// `C4Def::GetValue`: run the definition's `CalcDefValue(base, player)`
    /// override and the optional base object's `CalcBuyValue(definition,
    /// value)` adjustment. Unlike [`Self::definition_value`], this executes
    /// script callbacks and folds their host-side effects.
    pub fn calculated_definition_value(
        &mut self,
        definition_id: &str,
        base: Option<ObjectId>,
        player: i32,
    ) -> Result<Option<i32>, EngineError> {
        let Some(definition) = self.definitions.get(definition_id) else {
            return Ok(None);
        };
        let base_has_override = base
            .and_then(|base| self.find_object_index(base))
            .and_then(|index| self.definitions.get(&self.objects[index].definition_id))
            .is_some_and(|definition| definition.has_function("CalcBuyValue"));
        if !definition.has_function("CalcDefValue") && !base_has_override {
            return Ok(Some(definition.value()));
        }

        let world = self.host_world_context();
        let (value, _args, batch, audio_state, rng, script_error) =
            ScenarioScript::execute_value_for_script(
                definition_id,
                Some(DefinitionId::from(definition_id)),
                "GetValue",
                &[],
                world,
                self.rng.clone(),
                self.frame,
                &self.global_effects.clone(),
                self.physics,
                self.environment,
                self.audio_registry.clone(),
                self.game_over_triggered,
                || {
                    compat::calculated_definition_value(definition_id, base, player)
                        .map(|value| (value.map(Value::Int).unwrap_or(Value::Nil), Vec::new()))
                        .map_err(Into::into)
                },
            );
        self.rng = rng;
        self.audio_registry = audio_state;
        self.apply_scenario_batch(batch)?;
        if let Some(error) = script_error {
            return match error {
                EngineError::Script { .. } => Ok(Some(0)),
                other => Err(other),
            };
        }
        Ok(Some(value.and_then(|value| value.as_c4_int()).unwrap_or(0)))
    }

    pub fn definition_mass(&self, definition_id: &str) -> Option<i32> {
        self.definitions
            .get(definition_id)
            .map(|definition| definition.mass())
    }

    pub fn definition_picture(&self, definition_id: &str) -> Option<DefinitionPicture> {
        self.definitions
            .get(definition_id)
            .and_then(|definition| definition.picture())
    }

    pub fn definition_picture_image(&self, definition_id: &str) -> Option<DefinitionPictureImage> {
        self.definitions
            .get(definition_id)
            .and_then(|definition| definition.picture_image().cloned())
    }

    /// Checked menu-facing lookup: a loaded definition may legitimately have
    /// no drawable Picture facet, while an unknown ID violates the caller's
    /// definition-list invariant.
    pub fn try_definition_picture_image(
        &self,
        definition_id: &str,
    ) -> Result<Option<DefinitionPictureImage>, EngineError> {
        let definition = self
            .definitions
            .get(definition_id)
            .ok_or_else(|| EngineError::UnknownDefinition(definition_id.to_string()))?;
        Ok(definition.picture_image().cloned())
    }

    /// `C4MouseControl::CreateDragImage`: use the definition picture when
    /// `DragImagePicture` is set, otherwise use the main Graphics facet at
    /// `(0, 0, Shape.Wdt, Shape.Hgt)` in its raw facet dimensions.
    pub fn definition_construction_drag_image(
        &self,
        definition_id: &str,
    ) -> Option<DefinitionPictureImage> {
        let definition = self.definitions.get(definition_id)?;
        if definition.drag_image_picture != 0 {
            let picture = definition.picture()?;
            return DefinitionPictureImage::from_sprite_rect_clipped(
                definition.sprite_image()?,
                DefinitionRect::new(picture.x, picture.y, picture.width, picture.height),
            );
        }
        let shape = definition.shape_rect()?;
        DefinitionPictureImage::from_sprite_rect_clipped(
            definition.sprite_image()?,
            DefinitionRect::new(0, 0, shape.width, shape.height),
        )
    }

    /// `C4Def::Picture2Facet` with an explicit horizontal phase. The phase
    /// selection is fixed by AddMenuItem; even an out-of-range phase remains
    /// a valid (clipped/transparent) facet instead of falling back to zero.
    pub fn definition_picture_phase_image(
        &self,
        definition_id: &str,
        phase: i32,
    ) -> Option<DefinitionPictureImage> {
        let definition = self.definitions.get(definition_id)?;
        let picture = definition.picture()?;
        let phase_x = picture
            .width
            .saturating_mul(phase)
            .saturating_add(picture.x);
        let scale = definition.graphics_scale();
        let scaled = |value: i32| (value as f32 * scale) as i32;
        let rect = DefinitionRect::new(
            scaled(phase_x),
            scaled(picture.y),
            scaled(picture.width),
            scaled(picture.height),
        );
        DefinitionPictureImage::from_sprite_rect_clipped(definition.sprite_image()?, rect)
    }

    /// `C4Object::CanConcatPictureWith` (src/C4Object.cpp:6173-6213),
    /// including every `AllowPictureStack` exception and the asymmetric
    /// picture-overlay walk.
    pub fn can_concat_picture_with(&self, object: &ObjectSnapshot, other: &ObjectSnapshot) -> bool {
        if object.definition_id != other.definition_id {
            return false;
        }
        let Some(definition) = self.definitions.get(&object.definition_id) else {
            return false;
        };
        let allowed = definition.allow_picture_stack();
        if allowed & APS_COLOR == 0 {
            if definition.color_by_owner() && object.color != other.color {
                return false;
            }
            if object.color_modulation != other.color_modulation
                || object.blit_mode != other.blit_mode
            {
                return false;
            }
        }
        if allowed & APS_GRAPHICS == 0 {
            fn graphics_key(snapshot: &ObjectSnapshot) -> (&str, Option<&str>) {
                snapshot
                    .base_graphics
                    .as_ref()
                    .map(|graphics| {
                        (
                            graphics.definition.as_str(),
                            graphics.graphics_name.as_deref(),
                        )
                    })
                    .unwrap_or((snapshot.definition_id.as_str(), None))
            }
            let (object_definition, object_name) = graphics_key(object);
            let (other_definition, other_name) = graphics_key(other);
            if !resolved_graphics_equal(
                Some(object_definition),
                object_name,
                Some(other_definition),
                other_name,
            ) || object.picture_rect != other.picture_rect
            {
                return false;
            }
        }
        if allowed & APS_NAME == 0 {
            let object_name = object
                .custom_name
                .as_deref()
                .filter(|name| !name.is_empty())
                .or_else(|| {
                    self.crew_object_infos
                        .get(&object.id)
                        .map(|info| info.name.as_str())
                })
                .unwrap_or(definition.name());
            let other_name = other
                .custom_name
                .as_deref()
                .filter(|name| !name.is_empty())
                .or_else(|| {
                    self.crew_object_infos
                        .get(&other.id)
                        .map(|info| info.name.as_str())
                })
                .unwrap_or(definition.name());
            if object_name != other_name {
                return false;
            }
        }
        if allowed & APS_OVERLAY == 0 {
            // C4GraphicsOverlay::operator== intentionally ignores animation
            // phase (C4DefGraphics.cpp:868-878).
            for overlay in object
                .graphics_overlays
                .iter()
                .filter(|overlay| overlay.mode == GraphicsOverlayMode::Picture)
            {
                let Some(other_overlay) = other
                    .graphics_overlays
                    .iter()
                    .find(|candidate| candidate.id == overlay.id)
                else {
                    return false;
                };
                if !picture_overlays_equal(other_overlay, overlay) {
                    return false;
                }
            }
            for overlay in other
                .graphics_overlays
                .iter()
                .filter(|overlay| overlay.mode == GraphicsOverlayMode::Picture)
            {
                if !object
                    .graphics_overlays
                    .iter()
                    .any(|candidate| candidate.id == overlay.id)
                {
                    return false;
                }
            }
        }
        true
    }

    /// `C4Object::Picture2Facet` source selection before ColorMod/overlay
    /// compositing (src/C4Object.cpp:3123-3129): use the object's own rect
    /// when its width is nonzero, otherwise the definition rect, against the
    /// currently selected base graphics bitmap.
    pub fn object_picture_image(&self, object: &ObjectSnapshot) -> Option<DefinitionPictureImage> {
        let rect = if object.picture_rect.width != 0 {
            object.picture_rect
        } else {
            let picture = self.definition_picture(&object.definition_id)?;
            DefinitionRect::new(picture.x, picture.y, picture.width, picture.height)
        };
        let scale = self
            .definitions
            .get(&object.definition_id)
            .map_or(1.0, Definition::graphics_scale);
        let scaled = |value: i32| (value as f32 * scale) as i32;
        let rect = DefinitionRect::new(
            scaled(rect.x),
            scaled(rect.y),
            scaled(rect.width),
            scaled(rect.height),
        );
        let (graphics_definition, graphics_name) = object
            .base_graphics
            .as_ref()
            .map(|graphics| {
                (
                    graphics.definition.as_str(),
                    graphics.graphics_name.as_deref(),
                )
            })
            .unwrap_or((object.definition_id.as_str(), None));
        let sprite = self.definition_sprite_image(graphics_definition, graphics_name)?;
        DefinitionPictureImage::from_sprite_rect_clipped(&sprite, rect)
    }

    /// Base image for an Object/ObjectRank menu recipe captured at add time.
    pub fn object_menu_picture_image(
        &self,
        object: &ObjectMenuPictureSnapshot,
    ) -> Option<DefinitionPictureImage> {
        let rect = if object.picture_rect.width != 0 {
            object.picture_rect
        } else {
            let picture = self.definition_picture(&object.definition_id)?;
            DefinitionRect::new(picture.x, picture.y, picture.width, picture.height)
        };
        let scale = self
            .definitions
            .get(&object.definition_id)
            .map_or(1.0, Definition::graphics_scale);
        let scaled = |value: i32| (value as f32 * scale) as i32;
        let rect = DefinitionRect::new(
            scaled(rect.x),
            scaled(rect.y),
            scaled(rect.width),
            scaled(rect.height),
        );
        let (graphics_definition, graphics_name) = object
            .base_graphics
            .as_ref()
            .map(|graphics| {
                (
                    graphics.definition.as_str(),
                    graphics.graphics_name.as_deref(),
                )
            })
            .unwrap_or((object.definition_id.as_str(), None));
        let sprite = self.definition_sprite_image(graphics_definition, graphics_name)?;
        DefinitionPictureImage::from_sprite_rect_clipped(&sprite, rect)
    }

    /// Picture-mode overlay sources in C4GraphicsOverlay list order
    /// (`C4Object::Picture2Facet`, src/C4Object.cpp:3147-3151). Each source
    /// uses its definition picture, animation phase and definition graphics
    /// scale before the app composites its transform/blit state.
    pub fn object_picture_overlay_images(
        &self,
        object: &ObjectSnapshot,
    ) -> Vec<(ObjectGraphicsOverlay, DefinitionPictureImage)> {
        object
            .graphics_overlays
            .iter()
            .filter(|overlay| overlay.mode == GraphicsOverlayMode::Picture)
            .filter_map(|overlay| {
                let definition_id = overlay
                    .definition
                    .as_deref()
                    .unwrap_or(object.definition_id.as_str());
                let definition = self.definitions.get(definition_id)?;
                let picture = definition.picture()?;
                let phase_x = picture
                    .width
                    .saturating_mul(overlay.phase)
                    .saturating_add(picture.x);
                let scale = definition.graphics_scale();
                let scaled = |value: i32| (value as f32 * scale) as i32;
                let rect = DefinitionRect::new(
                    scaled(phase_x),
                    scaled(picture.y),
                    scaled(picture.width),
                    scaled(picture.height),
                );
                let sprite = definition.sprite_image_variant(overlay.graphics_name.as_deref())?;
                DefinitionPictureImage::from_sprite_rect_clipped(sprite, rect)
                    .map(|image| (overlay.clone(), image))
            })
            .collect()
    }

    /// Picture-mode overlay sources retained by an add-time object-menu
    /// snapshot, in the original overlay order.
    pub fn object_menu_picture_overlay_images(
        &self,
        object: &ObjectMenuPictureSnapshot,
    ) -> Vec<(ObjectGraphicsOverlay, DefinitionPictureImage)> {
        object
            .graphics_overlays
            .iter()
            .filter(|overlay| overlay.mode == GraphicsOverlayMode::Picture)
            .filter_map(|overlay| {
                let definition_id = overlay
                    .definition
                    .as_deref()
                    .unwrap_or(object.definition_id.as_str());
                let definition = self.definitions.get(definition_id)?;
                let picture = definition.picture()?;
                let phase_x = picture
                    .width
                    .saturating_mul(overlay.phase)
                    .saturating_add(picture.x);
                let scale = definition.graphics_scale();
                let scaled = |value: i32| (value as f32 * scale) as i32;
                let rect = DefinitionRect::new(
                    scaled(phase_x),
                    scaled(picture.y),
                    scaled(picture.width),
                    scaled(picture.height),
                );
                let sprite = definition.sprite_image_variant(overlay.graphics_name.as_deref())?;
                DefinitionPictureImage::from_sprite_rect_clipped(sprite, rect)
                    .map(|image| (overlay.clone(), image))
            })
            .collect()
    }

    /// The def's first portrait for the HUD cursor info
    /// (C4ObjectInfo::Draw, src/C4ObjectInfo.cpp:308-320). Read-only
    /// presentation data.
    pub fn definition_portrait_image(&self, definition_id: &str) -> Option<DefinitionPictureImage> {
        self.definitions
            .get(definition_id)
            .and_then(|definition| definition.portrait_image().cloned())
    }

    pub fn definition_portrait_graphics_image(
        &self,
        definition_id: &str,
    ) -> Option<DefinitionPictureImage> {
        self.definitions
            .get(definition_id)
            .and_then(|definition| definition.portrait_graphics_image().cloned())
    }

    pub fn definition_named_portrait_graphics_image(
        &self,
        definition_id: &str,
        portrait_name: &str,
    ) -> Option<DefinitionPictureImage> {
        self.definitions
            .get(definition_id)
            .and_then(|definition| definition.portrait_graphics(portrait_name).cloned())
    }

    /// The def's own rank symbol strip (`pDef->pRankSymbols`,
    /// src/C4ObjectInfo.cpp:334-341). Read-only presentation data.
    pub fn definition_rank_symbols_image(
        &self,
        definition_id: &str,
    ) -> Option<DefinitionPictureImage> {
        self.definitions
            .get(definition_id)
            .and_then(|definition| definition.rank_symbols_image().cloned())
    }

    pub fn definition_rank_symbol_count(&self, definition_id: &str) -> Option<u32> {
        self.definitions
            .get(definition_id)
            .and_then(Definition::rank_symbol_count)
    }

    pub fn definition_sprite_image(
        &self,
        definition_id: &str,
        graphics_name: Option<&str>,
    ) -> Option<DefinitionSpriteImage> {
        self.definitions
            .get(definition_id)
            .and_then(|definition| definition.sprite_image_variant(graphics_name).cloned())
    }

    /// C4Def graphics scale (`DefCore Scale / 100.0`) used to map logical
    /// facet coordinates into the selected definition bitmap.
    pub fn definition_graphics_scale(&self, definition_id: &str) -> f32 {
        self.definitions
            .get(definition_id)
            .map_or(1.0, Definition::graphics_scale)
    }

    pub fn definition_sprite_variant_names(&self, definition_id: &str) -> Vec<String> {
        self.definitions
            .get(definition_id)
            .map(|definition| definition.sprite_variant_keys())
            .unwrap_or_default()
    }

    /// The definition Shape rect (frontend idle-facet sizing:
    /// C4Object::DrawFace draws Shape.Wdt x Shape.Hgt from the graphics
    /// origin, C4Object.cpp:438-460).
    pub fn definition_shape_rect(&self, definition_id: &str) -> Option<DefinitionRect> {
        self.definitions
            .get(definition_id)
            .and_then(|definition| definition.shape_rect())
    }

    pub fn definition_entrance_rect(&self, definition_id: &str) -> Option<DefinitionRect> {
        self.definitions
            .get(definition_id)
            .and_then(Definition::entrance_rect)
    }

    pub fn definition_collection_rect(&self, definition_id: &str) -> Option<DefinitionRect> {
        self.definitions
            .get(definition_id)
            .and_then(Definition::collection_rect)
    }

    pub fn definition_solid_mask(&self, definition_id: &str) -> Option<DefinitionTargetRect> {
        self.definitions
            .get(definition_id)
            .and_then(Definition::solid_mask)
    }

    /// C4Shape::FireTop copied from DefCore and scaled with the live shape
    /// before the burning-object facet is drawn (src/C4Shape.cpp:103-127;
    /// src/C4Object.cpp:2388-2408).
    pub fn definition_fire_top(&self, definition_id: &str) -> i32 {
        self.definitions
            .get(definition_id)
            .map_or(0, Definition::fire_top)
    }

    /// DefCore Rotateable. Positive values make UpdateShape rotate vertices
    /// and enlarge the live shape rectangle whenever raw r is nonzero.
    pub fn definition_rotateable(&self, definition_id: &str) -> i32 {
        self.definitions
            .get(definition_id)
            .map_or(0, Definition::rotateable)
    }

    pub fn definition_line(&self, definition_id: &str) -> i32 {
        self.definitions
            .get(definition_id)
            .map_or(0, Definition::line)
    }

    /// The live object-local C4Shape rectangle after per-instance shape,
    /// construction, stretch-growth, and rotation updates.
    pub fn object_current_shape_rect(&self, object_id: ObjectId) -> Option<DefinitionRect> {
        self.find_object_index(object_id)
            .and_then(|index| self.objects[index].current_shape_rect())
    }

    /// DefCore `TopFace` presentation metadata (src/C4Def.cpp:306), used by
    /// the frontend's second object rendering pass (src/C4ObjectList.cpp:390-396).
    pub fn definition_top_face(&self, definition_id: &str) -> Option<DefinitionTargetRect> {
        self.definitions
            .get(definition_id)
            .and_then(|definition| definition.top_face())
    }

    /// DefCore `StretchGrowth` → C4Def::GrowthType (src/C4Def.cpp:387):
    /// selects the DrawFace/UpdateShape con-scaling mode (C4Shape::Stretch
    /// vs ::Jolt, src/C4Object.cpp:329-333).
    pub fn definition_stretch_growth(&self, definition_id: &str) -> bool {
        self.definitions
            .get(definition_id)
            .is_some_and(|definition| definition.stretch_growth())
    }

    pub fn definition_action_graphics(
        &self,
        definition_id: &str,
    ) -> Option<HashMap<String, DefinitionActionGraphics>> {
        self.definitions
            .get(definition_id)
            .map(|definition| definition.action_graphics().clone())
    }

    pub fn definition_ids(&self) -> impl Iterator<Item = &str> {
        self.definitions.keys().map(|id| id.as_str())
    }

    pub fn spawn_object(&mut self, config: SpawnConfig) -> Result<ObjectId, EngineError> {
        self.spawn_object_inner(config, None)
    }

    fn spawn_object_inner(
        &mut self,
        config: SpawnConfig,
        initial_info_physical: Option<PhysicalInfo>,
    ) -> Result<ObjectId, EngineError> {
        let was_deferred = self.solid_mask_staging.defer_solid_mask_updates;
        let result = (|| {
            let (id, additional, nested_outcomes) =
                self.spawn_single_inner(config, initial_info_physical)?;
            self.process_spawn_queue_with_outcomes(additional, nested_outcomes)?;
            self.refresh_elimination_state();
            self.check_game_over()?;
            Ok(id)
        })();
        let outermost = !was_deferred && self.solid_mask_staging.defer_solid_mask_updates;
        self.finish_host_solid_mask_operations(outermost, result)
    }

    /// `C4Game::CreateInfoObject`: attach the exact roster node before the
    /// ordinary creation callbacks. Native assigns the object's enumeration
    /// number only after `C4Object::Init`; a first fair-crew projection fill
    /// inside Init must therefore precede number reservation as well.
    pub(crate) fn spawn_object_with_crew_info(
        &mut self,
        mut config: SpawnConfig,
        info: CrewObjectInfo,
        link: CrewInfoLink,
        physical: PhysicalInfo,
    ) -> Result<ObjectId, EngineError> {
        if self.use_fair_crew && !config.loaded {
            let object_definition_id = config.definition_id.clone();
            let info_definition_id = if self.definitions.contains_key(&info.definition_id) {
                info.definition_id.clone()
            } else {
                object_definition_id.clone()
            };
            let projection_source = self.definitions.get(&info_definition_id).map(|definition| {
                (
                    *definition.physical(),
                    definition.rank_base().unwrap_or(1_000),
                    definition.script_arc(),
                )
            });
            match projection_source {
                Some((definition_physical, rank_base, script)) => {
                    self.fill_fair_crew_projection(
                        info_definition_id,
                        definition_physical,
                        rank_base,
                        script,
                    );
                }
                None => {
                    let definition_physical = self
                        .definitions
                        .get(&object_definition_id)
                        .map(|definition| *definition.physical())
                        .unwrap_or_default();
                    fair_crew_physical_cached(
                        definition_physical,
                        self.fair_crew_strength,
                        1_000,
                        &info_definition_id,
                        &self.fair_crew_physical_cache,
                    );
                }
            }
        }
        let id = self.next_object_id();
        config = config.with_id(id);
        let rank = info.rank;
        Rc::make_mut(&mut self.crew_object_infos).insert(id, info);
        Rc::make_mut(&mut self.crew_info_links).insert(id, link);
        Rc::make_mut(&mut self.crew_ranks).insert(id.as_u64(), rank);

        let result = self.spawn_object_inner(config, Some(physical));
        if result.is_err() && self.find_object_index(id).is_none() {
            // A failed materialization cannot retain an Info pointer. If a
            // callback moved the info first, its destination is a different
            // key and remains untouched.
            Rc::make_mut(&mut self.crew_object_infos).remove(&id);
            Rc::make_mut(&mut self.crew_info_links).remove(&id);
            Rc::make_mut(&mut self.crew_ranks).remove(&id.as_u64());
        }
        result
    }

    /// `C4Game::NewObject` for engine-owned creation sites which do not run
    /// inside a script host context (notably the `Init*` placement pass): make
    /// the raw Con=0 object live, call Construction(creator), apply the initial
    /// DoCon, then call Completion/Initialize only on a FullCon crossing
    /// (C4Game.cpp:1102-1146; C4Object.cpp:1428-1515).
    pub(crate) fn spawn_object_with_initial_lifecycle(
        &mut self,
        mut config: SpawnConfig,
        creator: Option<ObjectId>,
    ) -> Result<Option<ObjectId>, EngineError> {
        // C4Object::Init copies both pCreator->pLayer and the wrapper's
        // independently retained enumerated number. The latter may be a
        // stale/unresolved word and therefore cannot be reconstructed from
        // the live pointer (C4Object.cpp:153-170; C4Object.h:310-331).
        if let Some(creator_index) = creator.and_then(|id| self.find_object_index(id)) {
            config.layer = self.objects[creator_index].state.layer;
            config.compiler_cache.layer = self.objects[creator_index].compiler_cache.layer;
        }
        let initial_construction = config.construction;
        config.construction = 0;
        // The callbacks and initial DoCon run below against the now-live
        // object; spawn materialization must do neither itself.
        config.initialized = true;
        config.position_adjusted = true;

        let object_id = self.spawn_object(config)?;
        let Some(index) = self.find_object_index(object_id) else {
            return Ok(None);
        };
        let creator = creator
            .map(|id| Value::Object(id.as_u64()))
            .unwrap_or(Value::Nil);
        let construction = self.call_object_function(index, "Construction", vec![creator]);
        tolerate_script_error(construction)?;
        if !self.object_survives_creation(index) {
            return Ok(None);
        }

        let crossed_full_con = self.do_initial_con(index, initial_construction);
        if !self.object_survives_creation(index) {
            return Ok(None);
        }
        if crossed_full_con {
            let completion = self.call_object_function(index, "Completion", Vec::new());
            tolerate_script_error(completion)?;
            if self.object_survives_creation(index) {
                let initialize = self.call_object_function(index, "Initialize", Vec::new());
                tolerate_script_error(initialize)?;
            }
        }

        Ok(self.object_survives_creation(index).then_some(object_id))
    }

    /// Shared `CreateLine` helper (C4ObjectCom.cpp:364-377). Object creation
    /// and callbacks finish first; only then are the two live endpoint
    /// vertices and action targets overwritten from the endpoints' current
    /// state.
    pub(crate) fn create_line_object(
        &mut self,
        definition_id: &str,
        owner: i32,
        from: ObjectId,
        to: ObjectId,
    ) -> Result<Option<ObjectId>, EngineError> {
        if !self.definitions.contains_key(definition_id) {
            return Ok(None);
        }
        let Some(from_index) = self.find_object_index(from) else {
            return Ok(None);
        };
        if self.find_object_index(to).is_none() {
            return Ok(None);
        }

        let mut config = SpawnConfig::new(definition_id)
            .with_position(Vector2::ZERO)
            .with_owner(owner)
            .with_action(ActionState::new("Idle"));
        if let Some(layer) = self.objects[from_index].state.layer {
            config = config.with_layer(layer);
        }
        let Some(line_id) = self.spawn_object_with_initial_lifecycle(config, Some(from))? else {
            return Ok(None);
        };

        let endpoint = |engine: &Self, object_id: ObjectId| {
            engine.find_object_index(object_id).map(|index| {
                let object = &engine.objects[index];
                let height = object
                    .current_shape_rect()
                    .map(|shape| shape.height)
                    .unwrap_or(0);
                Vector2::new(
                    object.state.position.x,
                    object.state.position.y.wrapping_add(height / 4),
                )
            })
        };
        let (Some(from_point), Some(to_point)) = (endpoint(self, from), endpoint(self, to)) else {
            return Ok(None);
        };
        let Some(line_index) = self.find_object_index(line_id) else {
            return Ok(None);
        };
        // C++ writes only VtxNum and the first two X/Y slots. Construction
        // may have reduced the active prefix, but dormant CNAT/friction
        // bytes in the fixed C4Shape buffer survive and become active again.
        let mut vertices = self.objects[line_index].state.shape_vertices.clone();
        vertices.count = 2;
        vertices.slots[0].x = from_point.x;
        vertices.slots[0].y = from_point.y;
        vertices.slots[1].x = to_point.x;
        vertices.slots[1].y = to_point.y;
        self.objects[line_index].set_shape_vertex_buffer(vertices);
        self.objects[line_index].state.action.target = Some(from);
        self.objects[line_index].state.action.target2 = Some(to);
        Ok(Some(line_id))
    }

    pub(crate) fn object_survives_creation(&self, index: usize) -> bool {
        self.objects.get(index).is_some_and(|object| {
            !object.destroyed && !matches!(object.state.status, ObjectStatus::Deleted)
        })
    }

    /// The initial (`fInitial=true`) DoCon half of NewObject. Unlike later
    /// DoCon calls, it preserves the pre-growth shape bottom even for rotated
    /// objects and deliberately leaves the fixed position at the raw Init
    /// coordinates (C4Object.cpp:1428-1515).
    pub(crate) fn do_initial_con(&mut self, index: usize, change: i32) -> bool {
        let before = self.objects[index].state.construction;
        let was_full = before >= FULL_CON;
        let oversize = self
            .definitions
            .get(&self.objects[index].definition_id)
            .is_some_and(Definition::oversize);
        let mut after = before.saturating_add(change).max(0);
        if !oversize {
            after = after.min(FULL_CON);
        }
        let previous_rect = self.objects[index].current_shape_rect();
        let stale_fixed_position = self.objects[index].fixed_position;

        {
            let object = &mut self.objects[index];
            object.state.construction = after;
            if docon_refreshes_construction(before, after) && object.shape_template.line == 0 {
                object.refresh_shape_geometry();
                let current_rect = object.current_shape_rect();
                if let (Some(previous), Some(current)) = (previous_rect, current_rect) {
                    if previous.height != current.height || previous.y != current.y {
                        let bottom = object
                            .state
                            .position
                            .y
                            .saturating_add(previous.y)
                            .saturating_add(previous.height);
                        object.state.position.y = bottom
                            .saturating_sub(current.height)
                            .saturating_sub(current.y);
                    }
                }
            }
            object.fixed_position = stale_fixed_position;
        }

        // ComponentConGain follows the Con update and precedes Completion
        // (C4Object.cpp:1454-1458,1506-1511).
        let definition_components = self
            .definitions
            .get(&self.objects[index].definition_id)
            .map(|definition| {
                definition
                    .components()
                    .iter()
                    .map(|component| (component.id.clone(), component.count))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let components = docon_component_counts(
            &self.objects[index].state.components,
            &self.objects[index].state.component_order,
            &definition_components,
            after,
            change,
        );
        self.objects[index].state.components = components;

        self.refresh_object_ocf(index);
        self.update_sector_for_index(index);
        self.update_solid_mask(index);
        if after <= 0 {
            self.remove_solid_mask(index);
            self.objects[index].mark_destroyed();
            self.clear_destroyed_object_layers();
            self.update_sector_for_index(index);
        }

        !was_full && after >= FULL_CON
    }
}
