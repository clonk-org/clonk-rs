use super::*;

pub(crate) const MATERIAL_NONE: i32 = -1;

pub(crate) fn default_sky_fade() -> [RgbColor; 2] {
    let settings = crate::SkySettings::default();
    [settings.fade_top, settings.fade_bottom]
}

/// FnInLiquid (C4Script.cpp:1864-1868): reads the object's CACHED
/// InLiquid flag (updated during movement, C4Movement.cpp:443-460) —
/// never the landscape at call time. Nil without an object.
pub(crate) fn in_liquid(args: &[Value]) -> Result<Value, RuntimeError> {
    let target =
        parse_object_reference_argument(args.first().unwrap_or(&Value::Nil), "InLiquid", "obj")?;
    with_host_context(Ok(Value::Nil), |context| {
        if let Some(id) = target {
            if context.object_context().map(|object| object.id()) != Some(id) {
                return Ok(context
                    .get_world_object(id)
                    .map(|object| Value::Bool(object.in_liquid()))
                    .unwrap_or(Value::Nil));
            }
        }
        Ok(context
            .object_context()
            .map(|object| Value::Bool(object.in_liquid()))
            .unwrap_or(Value::Nil))
    })
}

/// FnMaterial (C4Script.cpp:2488-2491): material number by name, -1 when
/// unknown (Game.Material.Get).
pub(crate) fn material(args: &[Value]) -> Result<Value, RuntimeError> {
    let name = parse_optional_string(args.first(), "Material", "name")?;
    let Some(name) = name else {
        return Ok(Value::Int(MATERIAL_NONE));
    };
    HOST_CONTEXT.with(|cell| {
        let borrow = cell.borrow();
        let result = borrow
            .as_ref()
            .and_then(|context| context.world.materials())
            .and_then(|materials| materials.get(&name))
            .map(|material| material.id().index() as i32)
            .unwrap_or(MATERIAL_NONE);
        Ok(Value::Int(result))
    })
}

/// FnMaterialName (C4Script.cpp:4475-4482): direct index into the loaded
/// Game.Material map, returning null for every invalid index and the exact
/// material core Name otherwise.
pub(crate) fn material_name(args: &[Value]) -> Result<Value, RuntimeError> {
    let index = value_to_i32(
        args.first().unwrap_or(&Value::Nil),
        "MaterialName",
        "material",
    )?;
    HOST_CONTEXT.with(|cell| {
        let borrow = cell.borrow();
        let name = usize::try_from(index)
            .ok()
            .and_then(crate::material::MaterialId::new)
            .and_then(|material| {
                borrow
                    .as_ref()
                    .and_then(|context| context.world.materials())
                    .and_then(|materials| materials.get_by_id(material))
            })
            .map(|material| material.name().to_string());
        Ok(name
            .map(|name| Value::String(name.into()))
            .unwrap_or(Value::Nil))
    })
}

/// FnGetMaterialColor (C4Script.cpp:4466-4473): read one channel from the
/// material core's three RGB palette entries. Invalid materials return nil.
pub(crate) fn get_material_color(args: &[Value]) -> Result<Value, RuntimeError> {
    let material_index = value_to_i32(
        args.first().unwrap_or(&Value::Nil),
        "GetMaterialColor",
        "material",
    )?;
    let number = value_to_i32(
        args.get(1).unwrap_or(&Value::Nil),
        "GetMaterialColor",
        "number",
    )?;
    let channel = value_to_i32(
        args.get(2).unwrap_or(&Value::Nil),
        "GetMaterialColor",
        "channel",
    )?;
    let Some(offset) = number
        .checked_mul(3)
        .and_then(|offset| offset.checked_add(channel))
        .and_then(|offset| usize::try_from(offset).ok())
        .filter(|offset| *offset < 9)
    else {
        return Ok(Value::Nil);
    };

    HOST_CONTEXT.with(|cell| {
        let borrow = cell.borrow();
        let color = usize::try_from(material_index)
            .ok()
            .and_then(crate::material::MaterialId::new)
            .and_then(|material| {
                borrow
                    .as_ref()
                    .and_then(|context| context.world.materials())
                    .and_then(|materials| materials.get_by_id(material))
            })
            .map(|material| material.color().get(offset).copied().unwrap_or(0));
        Ok(color.map(Value::Int).unwrap_or(Value::Nil))
    })
}

/// FnSetMaterialColor (C4Script.cpp:4451-4465): emulate recoloring the
/// material palette by installing the RGB-only modulation that transforms
/// its first color into the requested one. The other two palette arguments
/// are unused by native code, but all ten integer parameters are converted
/// before the material-validity gate.
pub(crate) fn set_material_color(args: &[Value]) -> Result<Value, RuntimeError> {
    let mut values = [0i32; 10];
    for (index, slot) in values.iter_mut().enumerate() {
        *slot = value_to_i32(
            args.get(index).unwrap_or(&Value::Nil),
            "SetMaterialColor",
            "parameter",
        )?;
    }
    let [material_index, red, green, blue, _, _, _, _, _, _] = values;
    let Some(material_id) = usize::try_from(material_index)
        .ok()
        .and_then(crate::material::MaterialId::new)
    else {
        return Ok(Value::Bool(false));
    };

    HOST_CONTEXT.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let context = borrow.as_mut().ok_or_else(|| {
            RuntimeError::new("SetMaterialColor requires an active engine context")
        })?;
        let source = context
            .world
            .materials()
            .and_then(|materials| materials.get_by_id(material_id))
            .map(|material| {
                let color = material.color();
                RgbColor::new(
                    color.first().copied().unwrap_or(0) as u8,
                    color.get(1).copied().unwrap_or(0) as u8,
                    color.get(2).copied().unwrap_or(0) as u8,
                )
            });
        let Some(source) = source else {
            return Ok(Value::Bool(false));
        };
        let target = RgbColor::new(red as u8, green as u8, blue as u8);
        let modulation =
            SkyAdjustment::from_color_modulation(source, target).modulation & 0x00ff_ffff;
        let modulation = match modulation {
            0 => 1,
            0x00ff_ffff => 0,
            modulation => modulation,
        };

        if let Some(landscape) = context.world.landscape_mut() {
            landscape.set_modulation(modulation);
        }
        context.register_landscape_operation(LandscapeOperation::MatAdjust { modulation });
        Ok(Value::Bool(true))
    })
}

/// FnGetMaterialCount (C4Script.cpp:2207-2213): invalid material indices
/// return -1. Otherwise select the raw MatCount for fReal/materials without
/// MinHeightCount, or the vertically filtered EffectiveMatCount.
pub(crate) fn get_material_count(args: &[Value]) -> Result<Value, RuntimeError> {
    let material_index = value_to_i32(
        args.first().unwrap_or(&Value::Nil),
        "GetMaterialCount",
        "material",
    )?;
    let real = value_to_bool(
        args.get(1).unwrap_or(&Value::Nil),
        "GetMaterialCount",
        "real",
    )?;
    with_host_context(Ok(Value::Int(-1)), |context| {
        let Some(material_id) = usize::try_from(material_index)
            .ok()
            .and_then(crate::material::MaterialId::new)
        else {
            return Ok(Value::Int(-1));
        };
        let Some(material) = context
            .world
            .materials()
            .and_then(|materials| materials.get_by_id(material_id))
        else {
            return Ok(Value::Int(-1));
        };
        let minimum_height =
            (!real && material.min_height_count() != 0).then_some(material.min_height_count());
        let count = context
            .landscape_ref()
            .map(|landscape| landscape.material_pixel_count(material_id, minimum_height))
            .unwrap_or(0);
        Ok(Value::Int(count as i32))
    })
}

/// FnGetMaterialVal (C4Script.cpp:4282-4300): a [Material] core entry by
/// compile name; the section must be "Material", the material is an
/// index into the loaded map, out-of-range or unknown entries are nil.
pub(crate) fn get_material_val(args: &[Value]) -> Result<Value, RuntimeError> {
    let entry = parse_optional_string(args.first(), "GetMaterialVal", "entry")?;
    let section = parse_optional_string(args.get(1), "GetMaterialVal", "section")?;
    let material = parse_optional_i32(args.get(2), "GetMaterialVal", "material")?.unwrap_or(0);
    let entry_index = parse_optional_i32(args.get(3), "GetMaterialVal", "entry_nr")?.unwrap_or(0);
    // The material core implies section "Material" (C4Script.cpp:4296).
    if section.as_deref() != Some("Material") {
        return Ok(Value::Nil);
    }
    let (Some(entry), Ok(index), Ok(entry_index)) = (
        entry,
        usize::try_from(material),
        usize::try_from(entry_index),
    ) else {
        return Ok(Value::Nil);
    };
    HOST_CONTEXT.with(|cell| {
        let borrow = cell.borrow();
        Ok(borrow
            .as_ref()
            .and_then(|context| context.world.materials())
            .and_then(|materials| {
                let id = crate::material::MaterialId::new(index)?;
                materials.get_by_id(id)
            })
            .and_then(|material| material.core_entry(&entry, entry_index))
            .map(|value| match value {
                crate::material::MaterialCoreValue::Int(value) => Value::Int(value),
                crate::material::MaterialCoreValue::Bool(value) => Value::Bool(value),
                crate::material::MaterialCoreValue::String(value) => Value::String(value.into()),
                crate::material::MaterialCoreValue::C4Id(value) => {
                    let raw = clonk_script::c4_id_parse(&value);
                    if raw == 0 {
                        Value::Nil
                    } else {
                        Value::C4Id(clonk_script::c4_id_from_raw(raw))
                    }
                }
            })
            .unwrap_or(Value::Nil))
    })
}

/// FnBubble (C4Script.cpp:2188-2192 + AddFunc :6718) -> BubbleOut
/// (C4Effect.cpp:847-857): a bubble only from semi-solid (submerged)
/// spots, capped by GetSmokeLevel (fixed at 150 in sync mode, otherwise
/// Config.Graphics.SmokeLevel; C4Effect.cpp:838-844); creates one FXU1 object.
pub(crate) fn bubble(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.len() > 2 {
        return Err(RuntimeError::new(
            "Bubble expects at most 2 arguments: x, y",
        ));
    }
    let mut x = parse_optional_i32(args.first(), "Bubble", "x")?.unwrap_or(0);
    let mut y = parse_optional_i32(args.get(1), "Bubble", "y")?.unwrap_or(0);
    with_host_context_mut(Ok(Value::Nil), |context| {
        // Local calls offset by the object position (C4Script.cpp:2190).
        if let Some(object) = context.object_context() {
            let position = object.effective_position();
            x = x.saturating_add(position.x);
            y = y.saturating_add(position.y);
        }
        // No bubbles from nowhere (C4Effect.cpp:850).
        let semi_solid = context
            .landscape_ref()
            .map(|landscape| landscape.is_semi_solid_at(x, y))
            .unwrap_or(false);
        if !semi_solid {
            return Ok(Value::Nil);
        }
        let Some(metadata) = context
            .definition_metadata(crate::BUBBLE_DEFINITION_ID)
            .cloned()
        else {
            // Unknown FXU1 def: Game.CreateObject returns nullptr.
            return Ok(Value::Nil);
        };
        // Enough bubbles out there already (C4Effect.cpp:853-854) —
        // pending same-call spawns count like the live objects.
        let bubbles = context
            .world_object_ids()
            .into_iter()
            .filter(|id| {
                context
                    .get_world_object(*id)
                    .filter(|object| object.definition_id() == crate::BUBBLE_DEFINITION_ID)
                    .filter(|object| object.status().is_active())
                    .is_some()
            })
            .count();
        if crate::bubble_cap_reached(bubbles, context.world.smoke_level()) {
            return Ok(Value::Nil);
        }
        let id = context.allocate_object_id();
        let spawn = SpawnConfig::new(crate::BUBBLE_DEFINITION_ID)
            .with_position(Vector2::new(x, y))
            .with_owner(OWNER_NONE)
            .with_category(metadata.category)
            .with_id(id);
        let preview_ocf = ocf::compute(
            metadata.ocf_base,
            metadata.crew_member,
            true,
            ObjectStatus::Normal,
            false,
            FULL_CON,
            metadata.category,
        );
        let preview = HostWorldObject::with_category(
            id,
            crate::BUBBLE_DEFINITION_ID,
            ObjectStatus::Normal,
            "Idle",
            None,
            None,
            None,
            OWNER_NONE,
            metadata.category,
            0,
            FULL_CON,
            0,
            Vector2::new(x, y),
            Vector2::ZERO,
            0,
            Vec::new(),
            0,
            0,
            0,
            None,
            None,
        )
        .with_ocf(preview_ocf)
        .with_full_state(Rc::new(crate::preview_spawn_state_with_components(
            Vector2::new(x, y),
            OWNER_NONE,
            OWNER_NONE,
            metadata.category,
            FULL_CON,
            metadata.contact_density(),
            metadata.vertices.clone(),
            metadata.components.as_slice(),
        )));
        context.register_spawn(spawn, preview);
        Ok(Value::Nil)
    })
}

/// FnSmoke (C4Script.cpp:2182-2186) -> Smoke (C4Effect.cpp:859-866): with
/// the standard particle system one Smoke particle spawns at
/// (x, y - level/2), size `level`, color `dwClr`.
pub(crate) fn smoke(args: &[Value]) -> Result<Value, RuntimeError> {
    let mut x = value_to_i32(args.first().unwrap_or(&Value::Nil), "Smoke", "x")?;
    let mut y = value_to_i32(args.get(1).unwrap_or(&Value::Nil), "Smoke", "y")?;
    let level = value_to_i32(args.get(2).unwrap_or(&Value::Nil), "Smoke", "level")?;
    let color = value_to_i32(args.get(3).unwrap_or(&Value::Nil), "Smoke", "clr")?;
    with_host_context_mut(Ok(Value::Nil), |context| {
        if let Some(object) = context.object_context() {
            let position = object.effective_position();
            x = x.saturating_add(position.x);
            y = y.saturating_add(position.y);
        }
        context.register_particle(ParticleCommand::Create(ParticleConfig {
            definition_id: "Smoke".to_string(),
            position: FloatVector2::new(x as f32, (y - level / 2) as f32),
            velocity: FloatVector2::new(0.0, 0.0),
            life: 0,
            parameter_a: level as f32,
            parameter_b: color,
            layer: ParticleLayer::Global,
        }));
        Ok(Value::Nil)
    })
}

/// FnLandscapeWidth (C4Script.cpp:3077-3080): GBackWdt.
pub(crate) fn landscape_width(_args: &[Value]) -> Result<Value, RuntimeError> {
    HOST_CONTEXT.with(|cell| {
        Ok(Value::Int(
            cell.borrow()
                .as_ref()
                .and_then(|context| context.landscape_ref())
                .map(|landscape| landscape.width() as i32)
                .unwrap_or(0),
        ))
    })
}

/// FnLandscapeHeight (C4Script.cpp:3082-3085): GBackHgt.
pub(crate) fn landscape_height(_args: &[Value]) -> Result<Value, RuntimeError> {
    HOST_CONTEXT.with(|cell| {
        Ok(Value::Int(
            cell.borrow()
                .as_ref()
                .and_then(|context| context.landscape_ref())
                .map(|landscape| landscape.estimated_height())
                .unwrap_or(0),
        ))
    })
}

/// FnLaunchLightning (C4Script.cpp:3081-3084) forwards all six integer
/// parameters and the normalized gamma flag to C4Weather::LaunchLightning.
/// Weather creates creatorless FXL1 at C4Game::CreateObject's native default
/// position (50,50), fail-safe-calls Activate, and returns true even when the
/// definition is absent or Activate errors (C4Weather.cpp:153-165;
/// C4Game.h:229-231).
pub(crate) fn launch_lightning(args: &[Value]) -> Result<Value, RuntimeError> {
    let integer = |index: usize, name: &str| {
        value_to_i32(
            args.get(index).unwrap_or(&Value::Nil),
            "LaunchLightning",
            name,
        )
    };
    let x = integer(0, "x")?;
    let y = integer(1, "y")?;
    let xdir = integer(2, "xdir")?;
    let xrange = integer(3, "xrange")?;
    let ydir = integer(4, "ydir")?;
    let yrange = integer(5, "yrange")?;
    let gamma = args.get(6).is_some_and(value_raw_truthy);

    let created = with_creatorless_object_context(|| {
        create_object(&[
            Value::C4Id("FXL1".to_string()),
            Value::Int(50),
            Value::Int(50),
            Value::Int(OWNER_NONE),
        ])
    })??;
    if let Some(target) = object_id_from_value(&created) {
        let activate_args = [
            Value::Int(x),
            Value::Int(y),
            Value::Int(xdir),
            Value::Int(xrange),
            Value::Int(ydir),
            Value::Int(yrange),
            Value::Bool(gamma),
        ];
        if let Some(Err(error)) = call_world_object_own_function(target, "Activate", &activate_args)
        {
            tracing::warn!(
                id = target.as_u64(),
                definition = "FXL1",
                function = "Activate",
                %error,
                "lightning activation failed; continuing like C++ fail-safe Call"
            );
        }
    }
    Ok(Value::Int(1))
}

/// FnLaunchVolcano (C4Script.cpp:3086-3093): the native signature consumes
/// only x (System.c4g may shadow it with a four-argument compatibility
/// wrapper). C4Weather then creates creatorless FXV1 at native position
/// (50,50) and fail-safe-calls Activate with `(x, GBackHgt - 1, bounded random size,
/// Material("Lava"))`, returning true even when FXV1 is absent or Activate
/// fails (C4Weather.cpp:178-184).
pub(crate) fn launch_volcano(args: &[Value]) -> Result<Value, RuntimeError> {
    let x = value_to_i32(args.first().unwrap_or(&Value::Nil), "LaunchVolcano", "x")?;
    let (height, lava) = try_with_host_context("LaunchVolcano requires an active engine context", |context| {
        let height = context
            .landscape_ref()
            .map(Landscape::estimated_height)
            .unwrap_or(0);
        let lava = context
            .world
            .materials()
            .and_then(|materials| materials.id_of("Lava"))
            .map(|material| material.index() as i32)
            .unwrap_or(MATERIAL_NONE);
        Ok((height, lava))
    })?;
    let size = (15 * height / 500 + draw_context_random(10)?).clamp(10, 60);

    let created = with_creatorless_object_context(|| {
        create_object(&[
            Value::C4Id("FXV1".to_string()),
            Value::Int(50),
            Value::Int(50),
            Value::Int(OWNER_NONE),
        ])
    })??;
    if let Some(target) = object_id_from_value(&created) {
        let args = [
            Value::Int(x),
            Value::Int(height - 1),
            Value::Int(size),
            Value::Int(lava),
        ];
        if let Some(Err(error)) = call_world_object_own_function(target, "Activate", &args) {
            tracing::warn!(
                id = target.as_u64(),
                definition = "FXV1",
                function = "Activate",
                %error,
                "volcano activation failed; continuing like C++ fail-safe Call"
            );
        }
    }
    Ok(Value::Int(1))
}

/// FnLaunchEarthquake (C4Script.cpp:3094-3097) discards
/// C4Weather::LaunchEarthquake's success value. Weather creates creatorless
/// FXQ1 at the exact requested position and fail-safe-calls Activate with no
/// arguments (C4Weather.cpp:196-203).
pub(crate) fn launch_earthquake(args: &[Value]) -> Result<Value, RuntimeError> {
    let x = value_to_i32(args.first().unwrap_or(&Value::Nil), "LaunchEarthquake", "x")?;
    let y = value_to_i32(args.get(1).unwrap_or(&Value::Nil), "LaunchEarthquake", "y")?;

    let created = with_creatorless_object_context(|| {
        create_object(&[
            Value::C4Id("FXQ1".to_string()),
            Value::Int(x),
            Value::Int(y),
            Value::Int(OWNER_NONE),
        ])
    })??;
    if let Some(target) = object_id_from_value(&created) {
        call_object_own_fail_safe(target, "Activate", &[]);
    }
    Ok(Value::Nil)
}

#[derive(Debug, Clone)]
#[doc(hidden)]
pub struct RetainedMapCreatorUpdate(pub(crate) crate::map_creator_s2::MapCreatorS2State);

#[derive(Debug, Clone)]
pub(crate) struct BlastRasterReplayStep {
    pub(crate) position: Vector2,
    pub(crate) original_material: Option<crate::MaterialId>,
    pub(crate) shift_byte: Option<u8>,
    pub(crate) clear: bool,
}

#[derive(Debug, Clone)]
pub(crate) enum BlastPixelReplay {
    Raster {
        steps: Vec<BlastRasterReplayStep>,
        pixel_count_by_material: HashMap<crate::MaterialId, i32>,
    },
    Column {
        shift_decisions: Vec<bool>,
    },
}

/// Random-dependent pixel decisions made synchronously by BlastFree. Object
/// creation callbacks and PXS draws also run synchronously in the host; the
/// engine replay only commits these exact terrain writes once.
#[derive(Debug, Clone)]
#[doc(hidden)]
pub struct BlastReplay {
    pub(crate) pixels: BlastPixelReplay,
}

#[derive(Debug, Clone)]
#[doc(hidden)]
pub enum LandscapeOperation {
    DigCircle {
        center: Vector2,
        radius: i32,
        requested: bool,
        by_object: Option<ObjectId>,
    },
    /// Host-previewed DigFree pixels. MaterialContents, conversion RNG, and
    /// creation callbacks already ran synchronously and copy out separately.
    DigCirclePreviewed {
        center: Vector2,
        radius: i32,
    },
    DigRect {
        origin: Vector2,
        width: i32,
        height: i32,
        requested: bool,
        by_object: Option<ObjectId>,
    },
    /// Host-previewed DigFreeRect pixels; see DigCirclePreviewed.
    DigRectPreviewed {
        origin: Vector2,
        width: i32,
        height: i32,
    },
    /// FnFreeRect -> Landscape::ClearRect (C4Script.cpp:3125-3131): the
    /// rect clears outright — no dug-out material, no PXS.
    ClearRect {
        origin: Vector2,
        width: i32,
        height: i32,
    },
    /// FnFreeRect's nonzero iFreeDensity arm ->
    /// C4Landscape::ClearRectDensity (C4Script.cpp:3119-3125).
    ClearRectDensity {
        origin: Vector2,
        width: i32,
        height: i32,
        density: i32,
    },
    /// `C4Game::CreateObjectConstruction(..., fTerrain=true)` prepares the
    /// footprint before `NewObject` and its Construction callback
    /// (C4Game.cpp:1191-1230).
    PrepareConstructionTerrain {
        center_x: i32,
        bottom_y: i32,
        width: i32,
        height: i32,
        basement: i32,
    },
    /// FnDrawMaterialQuad -> C4Landscape::DrawQuad
    /// (C4Script.cpp:5111-5115; C4Landscape.cpp:2448-2468).
    DrawMaterialQuad {
        material_texture: String,
        vertices: [Vector2; 4],
        ift: bool,
    },
    /// FnDrawMatChunks -> C4Landscape::DrawChunks
    /// (C4Script.cpp:4802-4805; C4Landscape.cpp:2419-2445). The callback
    /// resolves/allocates the texmap pair and samples every Random(1000)
    /// value synchronously; the fold replays only deterministic geometry.
    DrawMatChunks {
        origin: Vector2,
        width: i32,
        height: i32,
        count_x: i32,
        count_y: i32,
        material: String,
        byte: u8,
        map_seed: i32,
        random_offsets: Vec<i32>,
        texmap: crate::landscape::RuntimeTexMapState,
    },
    /// FnDrawVolcanoBranch's direct SetPix column interpolation
    /// (C4Script.cpp:2500-2509). This is intentionally a raw per-pixel
    /// writer, not a PrepareChange/FinishChange raster transaction.
    DrawVolcanoBranch {
        from: Vector2,
        to: Vector2,
        size: i32,
        material_byte: u8,
    },
    /// FnDrawMap -> C4Landscape::DrawMap/MapToLandscape
    /// (C4Script.cpp:4851-4855; C4Landscape.cpp:2636-2668,482-510).
    /// The callback already rendered these exact indexed bytes through the
    /// live synced RNG; the engine fold must not parse or draw RNG again.
    DrawMap {
        origin: Vector2,
        bitmap: clonk_resources::bitmap::IndexedBitmap,
        map_width: i32,
        map_height: i32,
        texmap: crate::landscape::RuntimeTexMapState,
        map_creator: Option<RetainedMapCreatorUpdate>,
    },
    /// FnDrawDefMap mutates the scenario's retained C4MapCreatorS2 before
    /// mapping its named map to the landscape (C4Landscape.cpp:2672-2696).
    /// Carry both the synchronously rendered bytes and evolved creator so
    /// the authoritative fold performs neither parsing nor RNG draws.
    DrawDefMap {
        origin: Vector2,
        bitmap: clonk_resources::bitmap::IndexedBitmap,
        map_width: i32,
        map_height: i32,
        texmap: crate::landscape::RuntimeTexMapState,
        map_creator: RetainedMapCreatorUpdate,
    },
    /// DrawMap parsing can allocate a live TextureMap entry before
    /// Render(nullptr) finds no map. C++ retains that allocation despite the
    /// false DrawMap result (C4Landscape.cpp:2659-2663; C4Texture.cpp:319-369).
    SyncRuntimeTexMap {
        texmap: crate::landscape::RuntimeTexMapState,
    },
    /// FnSetTextureIndex -> C4Landscape::SetTextureIndex. ReplaceMapColor
    /// rewrites the retained editor map before MoveIndex copies the entry;
    /// neither action refreshes Surface8's cached material tables
    /// (C4Landscape.cpp:2710-2731,2733-2808).
    SetTextureIndex {
        texmap: crate::landscape::RuntimeTexMapState,
        old_index: u8,
        new_index: u8,
    },
    /// FnRemoveUnusedTexMapEntries scans Surface8 and clears unreferenced
    /// C4TextureMap entries without refreshing the Pix2* caches. The slots
    /// captured here are for callback-local preview; the authoritative fold
    /// rescans Surface8 at this operation's ordered position
    /// (C4Landscape.cpp:2983-3007).
    RemoveUnusedTexMapEntries {
        cleared_slots: Vec<u8>,
    },
    BlastCircle {
        center: Vector2,
        radius: i32,
        controller: Option<i32>,
    },
    /// Host-previewed BlastFree pixels. Every random-dependent pixel choice,
    /// object lifecycle, and PXS velocity was already decided synchronously.
    BlastCirclePreviewed {
        center: Vector2,
        radius: i32,
        replay: BlastReplay,
    },
    ShakeCircle {
        center: Vector2,
        radius: i32,
    },
    /// FnInsertMaterial -> Landscape::InsertMaterial
    /// (C4Script.cpp:2207-2211): caller-relative coordinates, material by
    /// number.
    InsertMaterial {
        material: i32,
        position: Vector2,
        velocity: Vector2,
    },
    /// FnExtractLiquid -> Landscape::ExtractMaterial after a GBackLiquid
    /// guard (C4Script.cpp:2194-2199).
    ExtractLiquid {
        position: Vector2,
    },
    /// FnCastPXS samples its synced velocities during the script call, then
    /// the engine fold inserts them into the real C4PXSSystem in call order.
    CastPxs {
        material: crate::MaterialId,
        position: Vector2,
        velocities: Vec<FixedVec2>,
    },
    /// FnExtractMaterialAmount (C4Script.cpp:2264-2273): the count was
    /// simulated at call time; the apply reruns the REAL loop.
    ExtractMaterialAmount {
        material: i32,
        position: Vector2,
        amount: i32,
    },
    /// FnSetGamma/FnResetGamma -> C4GraphicsSystem::SetGamma
    /// (C4Script.cpp:4998-5006; C4GraphicsSystem.cpp:772-784). Gamma is
    /// presentation-only, but this existing ordered global-operation channel
    /// preserves callback write order without making it sync-relevant.
    GammaRamp {
        index: i32,
        points: [u32; 3],
    },
    /// FnSetSkyAdjust (C4Script.cpp:4620-4624) -> C4Sky::SetModulation
    /// (C4Sky.cpp:238-244).
    SkyAdjust {
        modulation: u32,
        back_color: u32,
    },
    /// FnSetMatAdjust (C4Script.cpp:4626-4630) and FnSetMaterialColor
    /// (C4Script.cpp:4451-4465) -> C4Landscape::SetModulation.
    MatAdjust {
        modulation: u32,
    },
    /// FnSetLandscapePixel (C4Script.cpp:5082-5088): a direct packed-color
    /// write into the presentation-only Surface32.
    SetLandscapePixel {
        position: Vector2,
        color: u32,
    },
    /// FnSetSkyParallax (C4Script.cpp:4955-4970) — Sky is a C4Landscape
    /// member; the raw int args carry the SkyPar_KEEP magic through to
    /// `SkyState::apply_parallax`.
    SkyParallax {
        mode: i32,
        par_x: i32,
        par_y: i32,
        xdir: i32,
        ydir: i32,
        x: i32,
        y: i32,
    },
}

pub(crate) fn blast_threshold(radius: i32) -> i64 {
    let radius = i64::from(radius.max(0));
    let size = (radius * radius * 6283) / 2000;
    let grade = i64::from(((radius as i32 / 10) - 1).clamp(1, 3));
    (size * grade) / 6
}

pub(crate) fn set_gravity(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.is_empty() {
        return Err(RuntimeError::new("SetGravity expects 1 argument: gravity"));
    }

    let gravity = match &args[0] {
        Value::Int(value) => *value,
        Value::Nil => 0,
        other => {
            return Err(RuntimeError::new(format!(
                "SetGravity: expected int or nil for gravity, got {}",
                other.type_name()
            )));
        }
    };

    PHYSICS_CONTEXT.with(|cell| {
        let context = cell
            .borrow()
            .as_ref()
            .ok_or_else(|| RuntimeError::new("SetGravity requires an active engine context"))?
            .clone();
        context.set_gravity(gravity);
        Ok(Value::Nil)
    })
}

pub(crate) fn get_gravity(args: &[Value]) -> Result<Value, RuntimeError> {
    if !args.is_empty() {
        return Err(RuntimeError::new(
            "GetGravity does not accept any arguments",
        ));
    }

    PHYSICS_CONTEXT.with(|cell| {
        let context = cell
            .borrow()
            .as_ref()
            .ok_or_else(|| RuntimeError::new("GetGravity requires an active engine context"))?
            .clone();
        Ok(Value::Int(context.gravity()))
    })
}

pub(crate) fn set_wind(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.is_empty() {
        return Err(RuntimeError::new("SetWind expects 1 argument: wind"));
    }

    let wind = match &args[0] {
        Value::Int(value) => *value,
        Value::Nil => 0,
        other => {
            return Err(RuntimeError::new(format!(
                "SetWind: expected int or nil for wind, got {}",
                other.type_name()
            )));
        }
    };

    ENVIRONMENT_CONTEXT.with(|cell| {
        let context = cell
            .borrow()
            .as_ref()
            .ok_or_else(|| RuntimeError::new("SetWind requires an active engine context"))?
            .clone();
        context.set_wind(wind);
        Ok(Value::Nil)
    })
}

pub(crate) fn get_wind(args: &[Value]) -> Result<Value, RuntimeError> {
    for (index, arg) in args.iter().take(3).enumerate() {
        match arg {
            Value::Int(_) | Value::Nil => {}
            Value::Bool(_) if index == 2 => {}
            other => {
                return Err(RuntimeError::new(format!(
                    "GetWind: unexpected argument type {} at position {}",
                    other.type_name(),
                    index + 1
                )));
            }
        }
    }

    let wind = ENVIRONMENT_CONTEXT.with(|cell| {
        let context = cell
            .borrow()
            .as_ref()
            .ok_or_else(|| RuntimeError::new("GetWind requires an active engine context"))?
            .clone();
        Ok::<i32, RuntimeError>(context.wind_force())
    })?;

    // Global form (FnGetWind, C4Script.cpp:3001-3004).
    let global = match args.get(2) {
        Some(Value::Bool(flag)) => *flag,
        Some(Value::Int(value)) => *value != 0,
        _ => false,
    };
    if global {
        return Ok(Value::Int(wind));
    }

    // Positional form: object-relative GBackWind — zero on tunnel
    // background (C4Script.cpp:3005-3007; C4Wrappers.h:189-192).
    let local_x = match args.first() {
        Some(Value::Int(value)) => *value,
        _ => 0,
    };
    let local_y = match args.get(1) {
        Some(Value::Int(value)) => *value,
        _ => 0,
    };
    with_host_context(Ok(Value::Int(wind)), |context| {
        let mut global_x = local_x;
        let mut global_y = local_y;
        if let Some(object) = context.object_context() {
            let position = object.effective_position();
            global_x = global_x.saturating_add(position.x);
            global_y = global_y.saturating_add(position.y);
        }
        let in_tunnel = context
            .landscape_ref()
            .map(|landscape| landscape.is_tunnel_at(global_x, global_y))
            .unwrap_or(false);
        Ok(Value::Int(if in_tunnel { 0 } else { wind }))
    })
}

pub(crate) fn set_temperature(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.is_empty() {
        return Err(RuntimeError::new(
            "SetTemperature expects 1 argument: temperature",
        ));
    }

    let temperature = match &args[0] {
        Value::Int(value) => *value,
        Value::Nil => 0,
        other => {
            return Err(RuntimeError::new(format!(
                "SetTemperature: expected int or nil for temperature, got {}",
                other.type_name()
            )));
        }
    };

    ENVIRONMENT_CONTEXT.with(|cell| {
        let context = cell
            .borrow()
            .as_ref()
            .ok_or_else(|| RuntimeError::new("SetTemperature requires an active engine context"))?
            .clone();
        context.set_temperature(temperature);
        Ok(Value::Nil)
    })
}

pub(crate) fn get_temperature(_args: &[Value]) -> Result<Value, RuntimeError> {
    ENVIRONMENT_CONTEXT.with(|cell| {
        let context = cell
            .borrow()
            .as_ref()
            .ok_or_else(|| RuntimeError::new("GetTemperature requires an active engine context"))?
            .clone();
        Ok(Value::Int(context.temperature()))
    })
}

pub(crate) fn set_climate(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.is_empty() {
        return Err(RuntimeError::new("SetClimate expects 1 argument: climate"));
    }

    let climate = match &args[0] {
        Value::Int(value) => *value,
        Value::Nil => 0,
        other => {
            return Err(RuntimeError::new(format!(
                "SetClimate: expected int or nil for climate, got {}",
                other.type_name()
            )));
        }
    };

    ENVIRONMENT_CONTEXT.with(|cell| {
        let context = cell
            .borrow()
            .as_ref()
            .ok_or_else(|| RuntimeError::new("SetClimate requires an active engine context"))?
            .clone();
        context.set_climate(climate);
        Ok(Value::Nil)
    })
}

pub(crate) fn get_climate(args: &[Value]) -> Result<Value, RuntimeError> {
    if !args.is_empty() {
        return Err(RuntimeError::new(
            "GetClimate does not accept any arguments",
        ));
    }

    ENVIRONMENT_CONTEXT.with(|cell| {
        let context = cell
            .borrow()
            .as_ref()
            .ok_or_else(|| RuntimeError::new("GetClimate requires an active engine context"))?
            .clone();
        Ok(Value::Int(context.climate()))
    })
}

/// FnSetSeason (C4Script.cpp:3025-3028) -> C4Weather::SetSeason.
pub(crate) fn set_season(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.is_empty() {
        return Err(RuntimeError::new("SetSeason expects 1 argument: season"));
    }

    let season = match &args[0] {
        Value::Int(value) => *value,
        Value::Nil => 0,
        other => {
            return Err(RuntimeError::new(format!(
                "SetSeason: expected int or nil for season, got {}",
                other.type_name()
            )));
        }
    };

    ENVIRONMENT_CONTEXT.with(|cell| {
        let context = cell
            .borrow()
            .as_ref()
            .ok_or_else(|| RuntimeError::new("SetSeason requires an active engine context"))?
            .clone();
        context.set_season(season);
        Ok(Value::Nil)
    })
}

/// FnGetSeason (C4Script.cpp:3030-3033) -> C4Weather::GetSeason.
pub(crate) fn get_season(args: &[Value]) -> Result<Value, RuntimeError> {
    if !args.is_empty() {
        return Err(RuntimeError::new("GetSeason does not accept any arguments"));
    }

    ENVIRONMENT_CONTEXT.with(|cell| {
        let context = cell
            .borrow()
            .as_ref()
            .ok_or_else(|| RuntimeError::new("GetSeason requires an active engine context"))?
            .clone();
        Ok(Value::Int(context.season()))
    })
}

pub(crate) fn path_free(args: &[Value]) -> Result<Value, RuntimeError> {
    let x1 = value_to_i32(args.first().unwrap_or(&Value::Nil), "PathFree", "x1")?;
    let y1 = value_to_i32(args.get(1).unwrap_or(&Value::Nil), "PathFree", "y1")?;
    let x2 = value_to_i32(args.get(2).unwrap_or(&Value::Nil), "PathFree", "x2")?;
    let y2 = value_to_i32(args.get(3).unwrap_or(&Value::Nil), "PathFree", "y2")?;

    with_host_context(Ok(Value::Bool(true)), |context| {

        let Some(landscape) = context.landscape_ref() else {
            return Ok(Value::Bool(true));
        };

        // FnPathFree → ::PathFree = the ForLine per-pixel Bresenham with
        // GBackSolid blocking (C4Landscape.cpp:1683-1738, 2052-2055) —
        // its exact stepping decides script branches (the Bison's
        // EnemyNearby flee gate). GBackSolid sees C4SolidMask's baked
        // MCVehic pixels — the plane carries them (put_solid_mask).
        let clear = match context.world.materials() {
            Some(materials) => crate::path_free_exact(landscape, materials, &[], x1, y1, x2, y2),
            None => landscape.path_is_clear(Vector2::new(x1, y1), Vector2::new(x2, y2)),
        };
        Ok(Value::Bool(clear))
    })
}

fn path_free2_native_int(value: &Value, parameter: &str) -> Result<i32, RuntimeError> {
    // Native dispatch eagerly resets falsy typed arguments to nil for callers
    // below #strict 3 before converting the C4ValueInt slots.
    let eager_falsy_conversion = match clonk_script::caller_origin_strictness() {
        clonk_script::HostCallerStrictness::NoCaller => false,
        clonk_script::HostCallerStrictness::NonStrict => true,
        clonk_script::HostCallerStrictness::Strict(level) => level < 3,
    };
    if eager_falsy_conversion && !value.as_bool() {
        return Ok(0);
    }
    value_to_i32(value, "PathFree2", parameter)
}

/// FnPathFree2 (C4Script.cpp:3132-3146): PathFree with native reference
/// parameters for the start point and blocked-pixel writeback.
pub(crate) fn path_free2(args: &[HostCallArg]) -> Result<Value, RuntimeError> {
    let x_arg = args.first().ok_or_else(|| {
        RuntimeError::new("call to \"PathFree2\" parameter 1: got \"nil\", but expected \"&\"!")
    })?;
    if !x_arg.is_reference() {
        return Err(RuntimeError::new(format!(
            "call to \"PathFree2\" parameter 1: got \"{}\", but expected \"&\"!",
            x_arg.read()?.type_name()
        )));
    }
    let y_arg = args.get(1).ok_or_else(|| {
        RuntimeError::new("call to \"PathFree2\" parameter 2: got \"nil\", but expected \"&\"!")
    })?;
    if !y_arg.is_reference() {
        return Err(RuntimeError::new(format!(
            "call to \"PathFree2\" parameter 2: got \"{}\", but expected \"&\"!",
            y_arg.read()?.type_name()
        )));
    }

    let x2_value = args
        .get(2)
        .map(HostCallArg::read)
        .transpose()?
        .unwrap_or(Value::Nil);
    let y2_value = args
        .get(3)
        .map(HostCallArg::read)
        .transpose()?
        .unwrap_or(Value::Nil);
    let x2 = path_free2_native_int(&x2_value, "x2")?;
    let y2 = path_free2_native_int(&y2_value, "y2")?;

    // Native dispatch converts x2/y2 before FnPathFree2 runs. The body then
    // uses GetRefVal().getInt(), preserving each reference while an
    // unconvertible referent contributes zero.
    let x1 = x_arg.read()?.as_c4_int().unwrap_or(0);
    let y1 = y_arg.read()?.as_c4_int().unwrap_or(0);

    let hit = HOST_CONTEXT.with(|cell| {
        let borrow = cell.borrow();
        let context = borrow.as_ref()?;
        let landscape = context.landscape_ref()?;
        Some(match context.world.materials() {
            Some(materials) => {
                crate::path_free_exact_hit(landscape, materials, &[], x1, y1, x2, y2)
            }
            None => {
                crate::for_line_first_blocker(x1, y1, x2, y2, |x, y| landscape.is_solid_at(x, y))
            }
        })
    });
    let hit = hit.flatten();

    if let Some(hit) = hit {
        let wrote_x = x_arg.write(Value::Int(hit.x))?;
        let wrote_y = y_arg.write(Value::Int(hit.y))?;
        debug_assert!(
            wrote_x && wrote_y,
            "validated PathFree2 reference disappeared"
        );
        Ok(Value::Bool(false))
    } else {
        Ok(Value::Bool(true))
    }
}

#[derive(Clone, Copy)]
pub(crate) enum LandscapeQuery {
    Solid,
    SemiSolid,
    Liquid,
    Sky,
}

pub(crate) fn g_back_solid(args: &[Value]) -> Result<Value, RuntimeError> {
    g_back_common(args, "GBackSolid", LandscapeQuery::Solid)
}

pub(crate) fn g_back_semi_solid(args: &[Value]) -> Result<Value, RuntimeError> {
    g_back_common(args, "GBackSemiSolid", LandscapeQuery::SemiSolid)
}

pub(crate) fn g_back_liquid(args: &[Value]) -> Result<Value, RuntimeError> {
    g_back_common(args, "GBackLiquid", LandscapeQuery::Liquid)
}

pub(crate) fn g_back_sky(args: &[Value]) -> Result<Value, RuntimeError> {
    g_back_common(args, "GBackSky", LandscapeQuery::Sky)
}

fn g_back_common(
    args: &[Value],
    function: &str,
    query: LandscapeQuery,
) -> Result<Value, RuntimeError> {
    // Unfilled parameter slots are nil -> 0 (C4Aul.h:104-121,
    // C4AulExec.cpp:1364-1396): GBackSolid() queries the object's position.
    let local_x = value_to_i32(args.first().unwrap_or(&Value::Nil), function, "x")?;
    let local_y = value_to_i32(args.get(1).unwrap_or(&Value::Nil), function, "y")?;

    with_host_context(Ok(Value::Bool(fallback_without_context(query))), |context| {

        let mut global_x = local_x;
        let mut global_y = local_y;
        if let Some(object) = context.object_context() {
            let position = object.effective_position();
            global_x = global_x.saturating_add(position.x);
            global_y = global_y.saturating_add(position.y);
        }

        let landscape = context.landscape_ref();
        let result = evaluate_landscape_query(landscape, query, global_x, global_y);
        Ok(Value::Bool(result))
    })
}

pub(crate) fn evaluate_landscape_query(
    landscape: Option<&Landscape>,
    query: LandscapeQuery,
    x: i32,
    y: i32,
) -> bool {
    match landscape {
        Some(landscape) => match query {
            LandscapeQuery::Solid => landscape.is_solid_at(x, y),
            // GBackSemiSolid = density >= C4M_SemiSolid(25), which liquids
            // satisfy (C4Wrappers.h:73-76, C4Material.h:202).
            LandscapeQuery::SemiSolid => landscape.is_semi_solid_at(x, y),
            LandscapeQuery::Liquid => landscape.is_liquid_at(x, y),
            // FnGBackSky is the historical background query: sky means the
            // pixel is not marked IFT, independent of material density
            // (C4Script.cpp:2252-2256).
            LandscapeQuery::Sky => !landscape.is_ift_at(x, y),
        },
        None => fallback_without_context(query),
    }
}

fn fallback_without_context(query: LandscapeQuery) -> bool {
    match query {
        LandscapeQuery::Sky => true,
        LandscapeQuery::Solid | LandscapeQuery::SemiSolid | LandscapeQuery::Liquid => false,
    }
}

pub(crate) fn get_material(args: &[Value]) -> Result<Value, RuntimeError> {
    // C++ pads missing script args with zero (FnGetMaterial(x, y),
    // C4Script.cpp:2216-2220): GetMaterial() probes the object center.
    let local_x = match args.first() {
        None | Some(Value::Nil) => 0,
        Some(value) => value_to_i32(value, "GetMaterial", "x")?,
    };
    let local_y = match args.get(1) {
        None | Some(Value::Nil) => 0,
        Some(value) => value_to_i32(value, "GetMaterial", "y")?,
    };

    with_host_context(Ok(Value::Int(MATERIAL_NONE)), |context| {

        let mut global_x = local_x;
        let mut global_y = local_y;
        if let Some(object) = context.object_context() {
            let position = object.effective_position();
            global_x = global_x.saturating_add(position.x);
            global_y = global_y.saturating_add(position.y);
        }

        let material = context
            .landscape_ref()
            // FnGetMaterial -> GBackMat -> Landscape.GetMat includes the
            // GetPix sky/MCVehic border mapping (C4Script.cpp:2216-2220;
            // C4Wrappers.h:164-167; C4Landscape.h:144-175).
            .and_then(|landscape| landscape.border_material_at(global_x, global_y));
        let result = material
            .map(|material_id| material_id.index() as i32)
            .unwrap_or(MATERIAL_NONE);
        Ok(Value::Int(result))
    })
}

/// FnGetTexture (C4Script.cpp:2222-2232): unlike GetMaterial/GBack*, x/y
/// are GLOBAL even with an object context. PixCol2Tex strips IFT, then the
/// live callback TextureMap supplies the raw entry texture name (presentation
/// may resolve liquid Smooth through the Liquid image). Parser-time allocations
/// and callback-COW terrain writes are both immediately visible.
pub(crate) fn get_texture(args: &[Value]) -> Result<Value, RuntimeError> {
    let x = value_to_i32(args.first().unwrap_or(&Value::Nil), "GetTexture", "x")?;
    let y = value_to_i32(args.get(1).unwrap_or(&Value::Nil), "GetTexture", "y")?;
    with_host_context(Ok(Value::Nil), |context| {
        let texture_index = context
            .landscape_ref()
            .and_then(|landscape| landscape.grid_byte_at(x, y))
            .map(|pixel| usize::from(pixel & 0x7f))
            .unwrap_or(0);
        if texture_index == 0 {
            return Ok(Value::Nil);
        }
        let Some(texmap) = context.runtime_texmap() else {
            return Ok(Value::Nil);
        };
        if texmap
            .material_names
            .get(texture_index)
            .and_then(Option::as_ref)
            .is_none()
        {
            return Ok(Value::Nil);
        }
        let texture = texmap
            .match_texture_names
            .get(texture_index)
            .and_then(Option::as_deref)
            .unwrap_or_default();
        Ok(Value::String(texture.to_string().into()))
    })
}

/// FnSetTextureIndex/C4Landscape::SetTextureIndex (C4Script.cpp:5071-5075;
/// C4Landscape.cpp:2733-2808). The wrapper admits 0..=255, then the landscape
/// applies its narrower texture-slot rules. MoveIndex is synchronously visible
/// through the live TextureMap even though Surface8 and Pix2* caches stay put.
pub(crate) fn set_texture_index(args: &[Value]) -> Result<Value, RuntimeError> {
    let material_texture =
        parse_optional_string(args.first(), "SetTextureIndex", "material-texture")?
            .unwrap_or_default();
    let new_index = value_to_i32(
        args.get(1).unwrap_or(&Value::Nil),
        "SetTextureIndex",
        "new-index",
    )?;
    let insert = value_to_bool(
        args.get(2).unwrap_or(&Value::Nil),
        "SetTextureIndex",
        "insert",
    )?;
    if !(0..=255).contains(&new_index) {
        return Ok(Value::Int(0));
    }

    with_host_context_mut(Ok(Value::Int(0)), |context| {
        let Some(texmap) = context.runtime_texmap_mut() else {
            return Ok(Value::Int(0));
        };
        let (succeeded, moved_indices) =
            texmap.set_texture_index(&material_texture, new_index as u8, insert);
        let texmap = texmap.clone();

        if let Some((old_index, new_index)) = moved_indices {
            let operation = LandscapeOperation::SetTextureIndex {
                texmap,
                old_index,
                new_index,
            };
            // C++ changes the live TextureMap before returning. Keep this
            // callback's COW world coherent and carry the captured state to
            // later effect callbacks and the authoritative fold.
            context
                .world
                .preview_runtime_landscape_operation(&operation);
            context.register_landscape_operation(operation);
        }
        Ok(Value::Int(i32::from(succeeded)))
    })
}

/// FnRemoveUnusedTexMapEntries/C4Landscape::RemoveUnusedTexMapEntries
/// (C4Script.cpp:5077-5080; C4Landscape.cpp:2983-3007). Usage comes from the
/// full-resolution Surface8 plane plus CrossMapMaterials' retained numeric
/// references. Entry removal is live, but the native deliberately leaves the
/// byte plane and its Pix2* lookup caches untouched.
pub(crate) fn remove_unused_texmap_entries(_args: &[Value]) -> Result<Value, RuntimeError> {
    with_host_context_mut(Ok(Value::Nil), |context| {
        let Some(texture_usage) = context
            .world
            .landscape_ref()
            .and_then(Landscape::texture_index_usage)
        else {
            return Ok(Value::Nil);
        };
        let Some(texmap) = context.runtime_texmap_mut() else {
            return Ok(Value::Nil);
        };
        let cleared_slots = texmap.remove_unused_entries(texture_usage);
        let operation = LandscapeOperation::RemoveUnusedTexMapEntries { cleared_slots };
        // Queue even an empty preview. Earlier deferred terrain operations
        // can make entries newly used or unused before the authoritative
        // fold reaches this point.
        context
            .world
            .preview_runtime_landscape_operation(&operation);
        context.register_landscape_operation(operation);
        Ok(Value::Nil)
    })
}

pub(crate) fn blast_free(args: &[Value]) -> Result<Value, RuntimeError> {
    let mut x = value_to_i32(args.first().unwrap_or(&Value::Nil), "BlastFree", "x")?;
    let mut y = value_to_i32(args.get(1).unwrap_or(&Value::Nil), "BlastFree", "y")?;
    let level = value_to_i32(args.get(2).unwrap_or(&Value::Nil), "BlastFree", "level")?;
    let caused_by_plus_one =
        value_to_i32(args.get(3).unwrap_or(&Value::Nil), "BlastFree", "caused by")?;

    let (center, controller) = try_with_host_context("BlastFree requires an active engine context", |context| {

        let mut controller = if caused_by_plus_one != 0 {
            Some(caused_by_plus_one.wrapping_sub(1))
        } else {
            None
        };

        if caused_by_plus_one == 0 {
            if let Some(object) = context.object_context() {
                let position = object.effective_position();
                x = x.saturating_add(position.x);
                y = y.saturating_add(position.y);
                controller = Some(object.controller());
            }
        }

        Ok((Vector2::new(x, y), controller))
    })?;
    native_blast_free_absolute(center, level, controller)?;
    // FnBlastFree is a void engine function; C4AulEngineFunc maps void to
    // C4VNull after performing the landscape side effect.
    Ok(Value::Nil)
}

/// Absolute C4Landscape::BlastFree entry for native engine operations.
/// Unlike the public wrapper, `OWNER_NONE` is already decoded and the point
/// must never be offset through the active script object.
pub(crate) fn native_blast_free_absolute(
    center: Vector2,
    level: i32,
    controller: Option<i32>,
) -> Result<(), RuntimeError> {
    let counts = try_with_host_context_mut("BlastFree requires an active engine context", |context| {
        let preview = context.preview_blast_circle(center, level);
        let counts = preview
            .as_ref()
            .map(|(_, counts)| counts.clone())
            .unwrap_or_default();
        let replay = preview.map(|(replay, _)| replay);
        let operation = match replay {
            Some(replay) => LandscapeOperation::BlastCirclePreviewed {
                center,
                radius: level,
                replay,
            },
            None => LandscapeOperation::BlastCircle {
                center,
                radius: level,
                controller,
            },
        };
        context.register_landscape_operation(operation);
        Ok(counts)
    })?;
    process_preview_blast_reactions(center, controller, &counts)?;
    Ok(())
}

pub(crate) fn shake_free(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.len() > 3 {
        return Err(RuntimeError::new(
            "ShakeFree expects exactly 3 arguments: x, y, radius",
        ));
    }

    let x = value_to_i32(args.first().unwrap_or(&Value::Nil), "ShakeFree", "x")?;
    let y = value_to_i32(args.get(1).unwrap_or(&Value::Nil), "ShakeFree", "y")?;
    let radius = value_to_i32(args.get(2).unwrap_or(&Value::Nil), "ShakeFree", "radius")?;
    if radius <= 0 {
        return Ok(Value::Nil);
    }

    try_with_host_context_mut("ShakeFree requires an active engine context", |context| {
        let operation = LandscapeOperation::ShakeCircle {
            center: Vector2::new(x, y),
            radius,
        };
        context
            .world
            .preview_runtime_landscape_operation(&operation);
        context.register_landscape_operation(operation);
        Ok(Value::Nil)
    })
}

/// FnSetGamma (C4Script.cpp:4998-5006) -> C4GraphicsSystem::SetGamma
/// (C4GraphicsSystem.cpp:772-784). Missing arguments are C4Aul integer zero;
/// in particular, an omitted ramp index selects the script/user slot 0.
/// Remaining app-config gap: Rust does not yet pass
/// `Config.Graphics.DisableGamma` into an engine callback. C++ skips even the
/// stored write when that flag is set (C4GraphicsSystem.cpp:774-776); until
/// that boundary exists, every otherwise-valid Rust write is retained.
pub(crate) fn set_gamma(args: &[Value]) -> Result<Value, RuntimeError> {
    let mut points = [0_u32; 3];
    for (slot, point) in points.iter_mut().enumerate() {
        *point = value_to_i32(
            args.get(slot).unwrap_or(&Value::Nil),
            "SetGamma",
            "control point",
        )? as u32;
    }
    let index = value_to_i32(args.get(3).unwrap_or(&Value::Nil), "SetGamma", "ramp index")?;

    // C4GraphicsSystem::SetGamma silently returns for every index outside
    // [0,C4MaxGammaRamps), before touching the stored controls.
    if !(0..crate::GAMMA_RAMP_COUNT as i32).contains(&index) {
        return Ok(Value::Nil);
    }
    HOST_CONTEXT.with(|cell| {
        if let Some(context) = cell.borrow_mut().as_mut() {
            context.register_landscape_operation(LandscapeOperation::GammaRamp { index, points });
        }
    });
    Ok(Value::Nil)
}

/// FnResetGamma (C4Script.cpp:5004-5006): restore the selected ramp to the
/// C4GraphicsSystem default 0x000000/0x808080/0xffffff control points.
pub(crate) fn reset_gamma(args: &[Value]) -> Result<Value, RuntimeError> {
    let index = value_to_i32(
        args.first().unwrap_or(&Value::Nil),
        "ResetGamma",
        "ramp index",
    )?;
    if !(0..crate::GAMMA_RAMP_COUNT as i32).contains(&index) {
        return Ok(Value::Nil);
    }
    HOST_CONTEXT.with(|cell| {
        if let Some(context) = cell.borrow_mut().as_mut() {
            context.register_landscape_operation(LandscapeOperation::GammaRamp {
                index,
                points: crate::GammaControlState::DEFAULT_RAMP,
            });
        }
    });
    Ok(Value::Nil)
}

/// FnSetSkyAdjust (C4Script.cpp:4620-4624) -> C4Sky::SetModulation: sky
/// blit modulation plus the alpha-gated background fill color.
pub(crate) fn set_sky_adjust(args: &[Value]) -> Result<Value, RuntimeError> {
    let modulation = value_to_i32(
        args.first().unwrap_or(&Value::Nil),
        "SetSkyAdjust",
        "adjust",
    )? as u32;
    let back_color = value_to_i32(
        args.get(1).unwrap_or(&Value::Nil),
        "SetSkyAdjust",
        "back color",
    )? as u32;

    try_with_host_context_mut("SetSkyAdjust requires an active engine context", |context| {
        context.sky_adjustment = SkyAdjustment {
            modulation,
            back_color,
        };
        context.register_landscape_operation(LandscapeOperation::SkyAdjust {
            modulation,
            back_color,
        });
        Ok(Value::Nil)
    })
}

fn apply_sky_color_modulation(function: &str, target: RgbColor) -> Result<Value, RuntimeError> {
    HOST_CONTEXT.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let context = borrow.as_mut().ok_or_else(|| {
            RuntimeError::new(format!("{function} requires an active engine context"))
        })?;
        let adjustment = SkyAdjustment::from_color_modulation(context.world.sky_fade[0], target);
        context.sky_adjustment = adjustment;
        context.register_landscape_operation(LandscapeOperation::SkyAdjust {
            modulation: adjustment.modulation,
            back_color: adjustment.back_color,
        });
        Ok(Value::Nil)
    })
}

/// FnSetSkyFade (C4Script.cpp:3039-3044): NewGfx maps only the first RGB
/// triple onto FadeClr1. The legacy destination triple remains a typed but
/// otherwise ignored compatibility parameter.
pub(crate) fn set_sky_fade(args: &[Value]) -> Result<Value, RuntimeError> {
    let target = RgbColor::new(
        value_to_i32(
            args.first().unwrap_or(&Value::Nil),
            "SetSkyFade",
            "from red",
        )? as u8,
        value_to_i32(
            args.get(1).unwrap_or(&Value::Nil),
            "SetSkyFade",
            "from green",
        )? as u8,
        value_to_i32(
            args.get(2).unwrap_or(&Value::Nil),
            "SetSkyFade",
            "from blue",
        )? as u8,
    );
    for (offset, parameter) in [(3, "to red"), (4, "to green"), (5, "to blue")] {
        value_to_i32(
            args.get(offset).unwrap_or(&Value::Nil),
            "SetSkyFade",
            parameter,
        )?;
    }
    apply_sky_color_modulation("SetSkyFade", target)
}

/// FnSetSkyColor (C4Script.cpp:3046-3054): index zero maps the requested
/// OldGfx RGB color onto FadeClr1 via GetClrModulation; other indices are
/// silent no-ops.
pub(crate) fn set_sky_color(args: &[Value]) -> Result<Value, RuntimeError> {
    let index = value_to_i32(args.first().unwrap_or(&Value::Nil), "SetSkyColor", "index")?;
    let target = RgbColor::new(
        value_to_i32(args.get(1).unwrap_or(&Value::Nil), "SetSkyColor", "red")? as u8,
        value_to_i32(args.get(2).unwrap_or(&Value::Nil), "SetSkyColor", "green")? as u8,
        value_to_i32(args.get(3).unwrap_or(&Value::Nil), "SetSkyColor", "blue")? as u8,
    );
    if index != 0 {
        return Ok(Value::Nil);
    }
    apply_sky_color_modulation("SetSkyColor", target)
}

/// FnGetSkyAdjust (C4Script.cpp:4632-4636) returns the raw sky modulation,
/// or the raw background color when its bool argument is truthy. The latter
/// is independent of `BackClrEnabled` (C4Sky.h:43-46).
pub(crate) fn get_sky_adjust(args: &[Value]) -> Result<Value, RuntimeError> {
    let back_color = args.first().is_some_and(Value::as_bool);
    try_with_host_context("GetSkyAdjust requires an active engine context", |context| {
        let raw = if back_color {
            context.sky_adjustment.back_color
        } else {
            context.sky_adjustment.modulation
        };
        Ok(Value::Int(raw as i32))
    })
}

/// FnGetSkyColor (C4Script.cpp:3056-3069), retained for OldGfx scripts.
/// Only palette index zero exists. Its alpha is zero, so C++ BltAlpha's
/// inverted-alpha `/ 256` blend returns each nonzero FadeClr2 channel minus
/// one; FadeClr1 contributes nothing.
pub(crate) fn get_sky_color(args: &[Value]) -> Result<Value, RuntimeError> {
    let index = value_to_i32(args.first().unwrap_or(&Value::Nil), "GetSkyColor", "index")?;
    let channel = value_to_i32(args.get(1).unwrap_or(&Value::Nil), "GetSkyColor", "channel")?;
    if index != 0 || !(0..=2).contains(&channel) {
        return Ok(Value::Int(0));
    }

    try_with_host_context("GetSkyColor requires an active engine context", |context| {
        let color = context.world.sky_fade[1];
        let component = match channel {
            0 => color.r,
            1 => color.g,
            2 => color.b,
            _ => unreachable!(),
        };
        Ok(Value::Int(i32::from(component.saturating_sub(1))))
    })
}

/// FnSetMatAdjust (C4Script.cpp:4626-4630): overwrite the raw landscape
/// blit modulation. Zero restores normal drawing.
pub(crate) fn set_mat_adjust(args: &[Value]) -> Result<Value, RuntimeError> {
    let modulation = value_to_i32(
        args.first().unwrap_or(&Value::Nil),
        "SetMatAdjust",
        "adjust",
    )? as u32;
    try_with_host_context_mut("SetMatAdjust requires an active engine context", |context| {
        if let Some(landscape) = context.world.landscape_mut() {
            landscape.set_modulation(modulation);
        }
        context.register_landscape_operation(LandscapeOperation::MatAdjust { modulation });
        Ok(Value::Nil)
    })
}

/// FnGetMatAdjust (C4Script.cpp:4638-4642): return the raw landscape blit
/// modulation. C4Landscape::Default initializes this to zero.
pub(crate) fn get_mat_adjust(_args: &[Value]) -> Result<Value, RuntimeError> {
    try_with_host_context("GetMatAdjust requires an active engine context", |context| {
        let modulation = context
            .world
            .landscape_ref()
            .map_or(0, |landscape| landscape.modulation());
        Ok(Value::Int(modulation as i32))
    })
}

/// FnSetLandscapePixel (C4Script.cpp:5082-5088): offset by the current
/// script object and write only the packed-color Surface32 pixel. The native
/// is void and ignores out-of-bounds or surface-lock failure.
pub(crate) fn set_landscape_pixel(args: &[Value]) -> Result<Value, RuntimeError> {
    let x = value_to_i32(
        args.first().unwrap_or(&Value::Nil),
        "SetLandscapePixel",
        "x",
    )?;
    let y = value_to_i32(args.get(1).unwrap_or(&Value::Nil), "SetLandscapePixel", "y")?;
    let color = value_to_i32(
        args.get(2).unwrap_or(&Value::Nil),
        "SetLandscapePixel",
        "color",
    )? as u32;
    HOST_CONTEXT.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let context = borrow.as_mut().ok_or_else(|| {
            RuntimeError::new("SetLandscapePixel requires an active engine context")
        })?;
        let position = context
            .caller_scope()
            .map_or(Vector2::new(x, y), |(_, base)| {
                Vector2::new(base.x.saturating_add(x), base.y.saturating_add(y))
            });
        context.register_landscape_operation(LandscapeOperation::SetLandscapePixel {
            position,
            color,
        });
        Ok(Value::Nil)
    })
}

/// FnSetSkyParallax (C4Script.cpp:4955-4970): seven plain ints; nil and
/// missing args are 0 at the C4Aul boundary (zeroing the scroll slots) —
/// only the explicit SkyPar_KEEP magic preserves a slot.
pub(crate) fn set_sky_parallax(args: &[Value]) -> Result<Value, RuntimeError> {
    let mut slots = [0i32; 7];
    for (index, slot) in slots.iter_mut().enumerate() {
        *slot = value_to_i32(
            args.get(index).unwrap_or(&Value::Nil),
            "SetSkyParallax",
            "parameter",
        )?;
    }
    let [mode, par_x, par_y, xdir, ydir, x, y] = slots;

    try_with_host_context_mut("SetSkyParallax requires an active engine context", |context| {
        context.register_landscape_operation(LandscapeOperation::SkyParallax {
            mode,
            par_x,
            par_y,
            xdir,
            ydir,
            x,
            y,
        });
        Ok(Value::Nil)
    })
}

pub(crate) fn dig_free(args: &[Value]) -> Result<Value, RuntimeError> {
    let x = value_to_i32(args.first().unwrap_or(&Value::Nil), "DigFree", "x")?;
    let y = value_to_i32(args.get(1).unwrap_or(&Value::Nil), "DigFree", "y")?;
    let radius = value_to_i32(args.get(2).unwrap_or(&Value::Nil), "DigFree", "radius")?;
    if radius < 0 {
        return Ok(Value::Nil);
    }

    let requested = if let Some(arg) = args.get(3) {
        value_to_bool(arg, "DigFree", "requested")?
    } else {
        false
    };

    let (by_object, counts) = try_with_host_context_mut("DigFree requires an active engine context", |context| {
        let by_object = context.object_context().map(|object| object.id());
        let center = Vector2::new(x, y);
        let counts = context.preview_dig_circle(center, radius);
        context.register_landscape_operation(LandscapeOperation::DigCirclePreviewed {
            center,
            radius,
        });
        Ok((by_object, counts))
    })?;
    process_preview_dig_reactions(by_object, &counts, requested)?;
    Ok(Value::Nil)
}

/// FnDrawMatChunks/C4Landscape::DrawChunks (C4Script.cpp:4802-4805;
/// C4Landscape.cpp:2419-2445): resolve the separate material/texture pair,
/// then sample one synchronized Random(1000) value per chunk in x-major,
/// y-minor order. The resulting offsets are carried to the deferred fold so
/// authoritative drawing cannot consume the global RNG a second time.
pub(crate) fn draw_mat_chunks(args: &[Value]) -> Result<Value, RuntimeError> {
    let x = value_to_i32(args.first().unwrap_or(&Value::Nil), "DrawMatChunks", "x")?;
    let y = value_to_i32(args.get(1).unwrap_or(&Value::Nil), "DrawMatChunks", "y")?;
    let width = value_to_i32(args.get(2).unwrap_or(&Value::Nil), "DrawMatChunks", "wdt")?;
    let height = value_to_i32(args.get(3).unwrap_or(&Value::Nil), "DrawMatChunks", "hgt")?;
    let count_x = value_to_i32(
        args.get(4).unwrap_or(&Value::Nil),
        "DrawMatChunks",
        "count-x",
    )?;
    let count_y = value_to_i32(
        args.get(5).unwrap_or(&Value::Nil),
        "DrawMatChunks",
        "count-y",
    )?;
    // FnStringPar maps null string parameters to "". Passing None to
    // get_index would instead wildcard-match the first material slot.
    let material =
        parse_optional_string(args.get(6), "DrawMatChunks", "material")?.unwrap_or_default();
    let texture =
        parse_optional_string(args.get(7), "DrawMatChunks", "texture")?.unwrap_or_default();
    let ift = value_to_bool(args.get(8).unwrap_or(&Value::Nil), "DrawMatChunks", "ift")?;

    with_host_context_mut(Ok(Value::Int(0)), |context| {
        let Some(map_seed) = context
            .world
            .landscape_ref()
            .and_then(Landscape::raster_state)
            .map(crate::landscape::LandscapeRasterState::map_seed)
        else {
            return Ok(Value::Int(0));
        };
        let Some(texmap) = context.runtime_texmap_mut() else {
            return Ok(Value::Int(0));
        };

        // GetMapColorIndex special-cases exact-case "Sky" to byte zero,
        // then DrawChunks still performs the independent material lookup.
        let slot = if material == "Sky" {
            0
        } else {
            texmap.get_index(&material, Some(&texture), true)
        };
        if (material != "Sky" && slot == 0) || texmap.material(&material).is_none() {
            return Ok(Value::Int(0));
        }
        let byte = if material == "Sky" {
            0
        } else {
            slot | if ift { 0x80 } else { 0 }
        };

        let mut random_offsets = Vec::new();
        for _ in 0..count_x {
            for _ in 0..count_y {
                random_offsets.push(draw_context_random(1_000)?);
            }
        }
        let texmap = texmap.clone();
        let operation = LandscapeOperation::DrawMatChunks {
            origin: Vector2::new(x, y),
            width,
            height,
            count_x,
            count_y,
            material,
            byte,
            map_seed,
            random_offsets,
            texmap,
        };
        context.preview_draw_mat_chunks(&operation);
        context.register_landscape_operation(operation);
        Ok(Value::Int(1))
    })
}

/// FnDrawVolcanoBranch (C4Script.cpp:2500-2509): draw the numeric material's
/// DefaultMatTex through raw, GLOBAL SetPix calls. The callback captures the
/// live default byte and previews the synchronous Surface8 mutation; the
/// authoritative fold repeats only the deterministic pixel loop.
pub(crate) fn draw_volcano_branch(args: &[Value]) -> Result<Value, RuntimeError> {
    let material = value_to_i32(
        args.first().unwrap_or(&Value::Nil),
        "DrawVolcanoBranch",
        "material",
    )?;
    let from_x = value_to_i32(
        args.get(1).unwrap_or(&Value::Nil),
        "DrawVolcanoBranch",
        "from-x",
    )?;
    let from_y = value_to_i32(
        args.get(2).unwrap_or(&Value::Nil),
        "DrawVolcanoBranch",
        "from-y",
    )?;
    let to_x = value_to_i32(
        args.get(3).unwrap_or(&Value::Nil),
        "DrawVolcanoBranch",
        "to-x",
    )?;
    let to_y = value_to_i32(
        args.get(4).unwrap_or(&Value::Nil),
        "DrawVolcanoBranch",
        "to-y",
    )?;
    let size = value_to_i32(
        args.get(5).unwrap_or(&Value::Nil),
        "DrawVolcanoBranch",
        "size",
    )?;

    with_host_context_mut(Ok(Value::Nil), |context| {
        let Some(material_byte) = context
            .runtime_texmap()
            .and_then(|texmap| texmap.default_material_entry_by_index(material))
        else {
            // C++'s direct material-map indexing is undefined for an invalid
            // index. Keep script execution safe without inventing a success
            // or failure value: the native is void in every case.
            return Ok(Value::Nil);
        };
        let operation = LandscapeOperation::DrawVolcanoBranch {
            from: Vector2::new(from_x, from_y),
            to: Vector2::new(to_x, to_y),
            size,
            material_byte,
        };
        context.preview_draw_volcano_branch(&operation);
        context.register_landscape_operation(operation);
        Ok(Value::Nil)
    })
}

/// FnDrawMaterialQuad (C4Script.cpp:5111-5115): all coordinates are GLOBAL
/// (there is no cthr->Obj offset), and the bool is the Surface8 IFT bit.
/// GetIndexMatTex runs synchronously so an unresolved material returns false
/// without queuing a landscape change (C4Landscape.cpp:2448-2452).
pub(crate) fn draw_material_quad(args: &[Value]) -> Result<Value, RuntimeError> {
    let material_texture =
        parse_optional_string(args.first(), "DrawMaterialQuad", "material-texture")?
            .unwrap_or_default();
    let coordinate_names = ["x1", "y1", "x2", "y2", "x3", "y3", "x4", "y4"];
    let mut coordinates = [0; 8];
    for (index, coordinate) in coordinates.iter_mut().enumerate() {
        *coordinate = value_to_i32(
            args.get(index + 1).unwrap_or(&Value::Nil),
            "DrawMaterialQuad",
            coordinate_names[index],
        )?;
    }
    let ift = value_to_bool(
        args.get(9).unwrap_or(&Value::Nil),
        "DrawMaterialQuad",
        "sub",
    )?;
    let vertices = [
        Vector2::new(coordinates[0], coordinates[1]),
        Vector2::new(coordinates[2], coordinates[3]),
        Vector2::new(coordinates[4], coordinates[5]),
        Vector2::new(coordinates[6], coordinates[7]),
    ];

    with_host_context_mut(Ok(Value::Bool(false)), |context| {
        if !context.resolve_runtime_material_texture(&material_texture) {
            return Ok(Value::Bool(false));
        }
        let operation = LandscapeOperation::DrawMaterialQuad {
            material_texture,
            vertices,
            ift,
        };
        context.preview_draw_material_quad(&operation);
        context.register_landscape_operation(operation);
        Ok(Value::Bool(true))
    })
}

fn runtime_map_random_context(function: &str) -> Result<Rc<RandomContext>, RuntimeError> {
    RANDOM_CONTEXT
        .with(|cell| cell.borrow().as_ref().cloned())
        .ok_or_else(|| RuntimeError::new(format!("{function}: random context unavailable")))
}

/// Run one AlgoScript callback without retaining either thread-local borrow.
/// The map renderer owns a detached copy of the live RANDOM_CONTEXT ledger;
/// publish it immediately before entering the nested scenario VM, then pull
/// back every Random() draw before continuing the native pixel traversal.
fn call_runtime_map_script_algo(
    random_context: &Rc<RandomContext>,
    scenario_script: Option<&Arc<ScriptEngine>>,
    rng: &mut LcgRng,
    function: &str,
    args: [i32; 4],
) -> bool {
    *random_context.rng.borrow_mut() = rng.clone();
    let args = args.map(Value::Int);
    let result = scenario_script
        .and_then(|script| call_scoped_scenario_function(Arc::clone(script), function, &args))
        .and_then(Result::ok)
        .is_some_and(|value| value.as_bool());
    *rng = random_context.rng.borrow().clone();
    result
}

/// FnDrawMap/C4Landscape::DrawMap (C4Script.cpp:4851-4855;
/// C4Landscape.cpp:2636-2668): clip the GLOBAL destination first, then make
/// an exact-size temporary S2 map through the callback's live synced RNG.
/// The resulting indexed bytes are queued verbatim so the later engine fold
/// cannot consume the map's size/range/seed draws a second time.
pub(crate) fn draw_map(args: &[Value]) -> Result<Value, RuntimeError> {
    let x = value_to_i32(args.first().unwrap_or(&Value::Nil), "DrawMap", "x")?;
    let y = value_to_i32(args.get(1).unwrap_or(&Value::Nil), "DrawMap", "y")?;
    let width = value_to_i32(args.get(2).unwrap_or(&Value::Nil), "DrawMap", "wdt")?;
    let height = value_to_i32(args.get(3).unwrap_or(&Value::Nil), "DrawMap", "hgt")?;
    let Some(source) = parse_optional_string(args.get(4), "DrawMap", "map-definition")? else {
        return Ok(Value::Int(0));
    };

    let preflight = HOST_CONTEXT.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let context = borrow.as_mut()?;
        let Some((landscape_width, landscape_height, map_zoom, retained_creator)) =
            context.world.landscape_ref().and_then(|landscape| {
                let (landscape_width, landscape_height) = landscape.grid_dimensions()?;
                let raster = landscape.raster_state()?;
                Some((
                    landscape_width,
                    landscape_height,
                    raster.map_zoom(),
                    raster.map_creator().cloned(),
                ))
            })
        else {
            return None;
        };
        if map_zoom <= 0 {
            return None;
        }

        // C4Landscape::ClipRect (C4Landscape.cpp:2698-2707) is the half-open
        // intersection with the landscape. i64 intermediates retain those
        // semantics without overflowing on hostile script integers.
        let left = i64::from(x).max(0);
        let top = i64::from(y).max(0);
        let right = (i64::from(x) + i64::from(width)).min(i64::from(landscape_width));
        let bottom = (i64::from(y) + i64::from(height)).min(i64::from(landscape_height));
        if right <= left || bottom <= top {
            return None;
        }
        let clipped_x = left as i32;
        let clipped_y = top as i32;
        let clipped_width = (right - left) as i32;
        let clipped_height = (bottom - top) as i32;
        let map_width = (clipped_width - 1) / map_zoom + 1;
        let map_height = (clipped_height - 1) / map_zoom + 1;

        let texmap = context.runtime_texmap()?.clone();
        let scenario_script = context.world.scenario_script().cloned();
        Some((
            clipped_x,
            clipped_y,
            map_width,
            map_height,
            retained_creator,
            texmap,
            scenario_script,
        ))
    });
    let Some((
        clipped_x,
        clipped_y,
        map_width,
        map_height,
        mut retained_creator,
        texmap,
        scenario_script,
    )) = preflight
    else {
        return Ok(Value::Int(0));
    };

    let texmap_before = texmap.clone();
    let mut classifier = crate::scenario::MapPixelClassifier::from_runtime_state(texmap);
    let script_functions = scenario_script
        .as_ref()
        .map(|script| {
            script
                .functions()
                .keys()
                .filter(|name| script.has_local_function(name))
                .cloned()
                .collect::<HashSet<_>>()
        })
        .unwrap_or_default();
    let random_context = runtime_map_random_context("DrawMap")?;
    let mut rng = random_context.rng.borrow().clone();
    let rendered = {
        let mut script_algo = |rng: &mut LcgRng, function: &str, args: [i32; 4]| {
            call_runtime_map_script_algo(
                &random_context,
                scenario_script.as_ref(),
                rng,
                function,
                args,
            )
        };
        crate::map_creator_s2::render_runtime_s2_map_with_script_algo(
            retained_creator.as_mut(),
            &source,
            &mut classifier,
            map_width,
            map_height,
            &mut rng,
            &script_functions,
            &mut script_algo,
        )
    };
    // Parsing/evaluation draws also precede the caller's next script opcode,
    // even when no ScriptAlgo function exists or Render finds no map.
    *random_context.rng.borrow_mut() = rng;

    let texmap = classifier.into_runtime_state();
    with_host_context_mut(Ok(Value::Int(0)), |context| {
        // Parser-side texture allocations are live to later calls in this
        // VM session even if Render found no map.
        context.set_runtime_texmap(texmap.clone());
        let Some(bitmap) = rendered else {
            if texmap != texmap_before {
                let operation = LandscapeOperation::SyncRuntimeTexMap { texmap };
                context
                    .world
                    .preview_runtime_landscape_operation(&operation);
                context.register_landscape_operation(operation);
            }
            return Ok(Value::Int(0));
        };
        let map_creator = retained_creator.map(RetainedMapCreatorUpdate);
        let operation = LandscapeOperation::DrawMap {
            origin: Vector2::new(clipped_x, clipped_y),
            bitmap,
            map_width,
            map_height,
            texmap,
            map_creator,
        };
        context.preview_draw_indexed_map(&operation);
        context.register_landscape_operation(operation);
        Ok(Value::Int(1))
    })
}

/// FnDrawDefMap/C4Landscape::DrawDefMap (C4Script.cpp:4857-4861;
/// C4Landscape.cpp:2672-2696): clip the GLOBAL destination, resize the named
/// map in the retained scenario creator, re-evaluate that complete creator
/// through the live synced RNG, and queue the rendered indexed bytes.
pub(crate) fn draw_def_map(args: &[Value]) -> Result<Value, RuntimeError> {
    let x = value_to_i32(args.first().unwrap_or(&Value::Nil), "DrawDefMap", "x")?;
    let y = value_to_i32(args.get(1).unwrap_or(&Value::Nil), "DrawDefMap", "y")?;
    let width = value_to_i32(args.get(2).unwrap_or(&Value::Nil), "DrawDefMap", "wdt")?;
    let height = value_to_i32(args.get(3).unwrap_or(&Value::Nil), "DrawDefMap", "hgt")?;
    let Some(map_name) = parse_optional_string(args.get(4), "DrawDefMap", "map-definition")? else {
        return Ok(Value::Int(0));
    };

    let preflight = HOST_CONTEXT.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let context = borrow.as_mut()?;
        let Some((landscape_width, landscape_height, map_zoom, map_creator)) =
            context.world.landscape_ref().and_then(|landscape| {
                let (landscape_width, landscape_height) = landscape.grid_dimensions()?;
                let raster = landscape.raster_state()?;
                Some((
                    landscape_width,
                    landscape_height,
                    raster.map_zoom(),
                    raster.map_creator()?.clone(),
                ))
            })
        else {
            return None;
        };
        if map_zoom <= 0 {
            return None;
        }

        // C4Landscape::ClipRect runs before GetMap/SetSize. In particular,
        // an empty clipped destination cannot re-evaluate the retained tree.
        let left = i64::from(x).max(0);
        let top = i64::from(y).max(0);
        let right = (i64::from(x) + i64::from(width)).min(i64::from(landscape_width));
        let bottom = (i64::from(y) + i64::from(height)).min(i64::from(landscape_height));
        if right <= left || bottom <= top {
            return None;
        }
        let clipped_x = left as i32;
        let clipped_y = top as i32;
        let clipped_width = (right - left) as i32;
        let clipped_height = (bottom - top) as i32;
        let map_width = (clipped_width - 1) / map_zoom + 1;
        let map_height = (clipped_height - 1) / map_zoom + 1;

        let texmap = context.runtime_texmap()?.clone();
        let scenario_script = context.world.scenario_script().cloned();
        Some((
            clipped_x,
            clipped_y,
            map_width,
            map_height,
            map_creator,
            texmap,
            scenario_script,
        ))
    });
    let Some((
        clipped_x,
        clipped_y,
        map_width,
        map_height,
        mut map_creator,
        texmap,
        scenario_script,
    )) = preflight
    else {
        return Ok(Value::Int(0));
    };

    let mut classifier = crate::scenario::MapPixelClassifier::from_runtime_state(texmap);
    let random_context = runtime_map_random_context("DrawDefMap")?;
    let mut rng = random_context.rng.borrow().clone();
    let rendered = {
        let mut script_algo = |rng: &mut LcgRng, function: &str, args: [i32; 4]| {
            call_runtime_map_script_algo(
                &random_context,
                scenario_script.as_ref(),
                rng,
                function,
                args,
            )
        };
        crate::map_creator_s2::render_named_s2_map_with_script_algo(
            &mut map_creator,
            &map_name,
            &mut classifier,
            map_width,
            map_height,
            &mut rng,
            &mut script_algo,
        )
    };
    *random_context.rng.borrow_mut() = rng;

    let texmap = classifier.into_runtime_state();
    with_host_context_mut(Ok(Value::Int(0)), |context| {
        context.set_runtime_texmap(texmap.clone());
        let Some(bitmap) = rendered else {
            return Ok(Value::Int(0));
        };

        // C++ mutates pMapCreator synchronously. Update the callback's COW
        // world now so a later DrawDefMap/DrawMap in this same VM call sees
        // the resized and re-evaluated tree; the operation carries the same
        // state into the authoritative engine fold.
        context.preview_runtime_map_creator(map_creator.clone());
        let operation = LandscapeOperation::DrawDefMap {
            origin: Vector2::new(clipped_x, clipped_y),
            bitmap,
            map_width,
            map_height,
            texmap,
            map_creator: RetainedMapCreatorUpdate(map_creator),
        };
        context.preview_draw_indexed_map(&operation);
        context.register_landscape_operation(operation);
        Ok(Value::Int(1))
    })
}

pub(crate) fn free_rect(args: &[Value]) -> Result<Value, RuntimeError> {
    let x = value_to_i32(args.first().unwrap_or(&Value::Nil), "FreeRect", "x")?;
    let y = value_to_i32(args.get(1).unwrap_or(&Value::Nil), "FreeRect", "y")?;
    let width = value_to_i32(args.get(2).unwrap_or(&Value::Nil), "FreeRect", "wdt")?;
    let height = value_to_i32(args.get(3).unwrap_or(&Value::Nil), "FreeRect", "hgt")?;
    let density = value_to_i32(
        args.get(4).unwrap_or(&Value::Nil),
        "FreeRect",
        "free_density",
    )?;
    with_host_context_mut(Ok(Value::Nil), |context| {
        context.preview_clear_rect(
            Vector2::new(x, y),
            width,
            height,
            (density != 0).then_some(density),
        )?;
        Ok(Value::Nil)
    })
}

pub(crate) fn dig_free_rect(args: &[Value]) -> Result<Value, RuntimeError> {
    let x = value_to_i32(args.first().unwrap_or(&Value::Nil), "DigFreeRect", "x")?;
    let y = value_to_i32(args.get(1).unwrap_or(&Value::Nil), "DigFreeRect", "y")?;
    let width = value_to_i32(args.get(2).unwrap_or(&Value::Nil), "DigFreeRect", "width")?;
    let height = value_to_i32(args.get(3).unwrap_or(&Value::Nil), "DigFreeRect", "height")?;
    if width <= 0 || height <= 0 {
        return Ok(Value::Nil);
    }

    let requested = if let Some(arg) = args.get(4) {
        value_to_bool(arg, "DigFreeRect", "requested")?
    } else {
        false
    };

    let (by_object, counts) = try_with_host_context_mut("DigFreeRect requires an active engine context", |context| {
        let by_object = context.object_context().map(|object| object.id());
        let origin = Vector2::new(x, y);
        let counts = context.preview_dig_rect(origin, width, height);
        context.register_landscape_operation(LandscapeOperation::DigRectPreviewed {
            origin,
            width,
            height,
        });
        Ok((by_object, counts))
    })?;
    process_preview_dig_reactions(by_object, &counts, requested)?;
    Ok(Value::Nil)
}

pub(crate) fn cast_pxs(args: &[Value]) -> Result<Value, RuntimeError> {
    // FnCastPXS -> C4PXSSystem::Cast (C4Script.cpp:2470-2474,
    // C4PXS.cpp:309-321): resolve the material once, offset local x/y by
    // the caller, then consume r2/r1 for every attempt even when the
    // material lookup failed. The engine function is void.
    let material_name =
        parse_optional_string(args.first(), "CastPXS", "material")?.unwrap_or_default();
    let amount = value_to_i32(args.get(1).unwrap_or(&Value::Nil), "CastPXS", "amount")?;
    let level = value_to_i32(args.get(2).unwrap_or(&Value::Nil), "CastPXS", "level")?;
    let x_offset = value_to_i32(args.get(3).unwrap_or(&Value::Nil), "CastPXS", "x")?;
    let y_offset = value_to_i32(args.get(4).unwrap_or(&Value::Nil), "CastPXS", "y")?;

    let (material, position) = try_with_host_context("CastPXS requires an active engine context", |context| {
        let material = context
            .world
            .materials()
            .and_then(|materials| materials.id_of(&material_name));
        let base = context
            .object_context()
            .map(ObjectScopeContext::effective_position)
            .unwrap_or(Vector2::ZERO);
        Ok((
            material,
            Vector2::new(
                base.x.saturating_add(x_offset),
                base.y.saturating_add(y_offset),
            ),
        ))
    })?;

    let velocities = RANDOM_CONTEXT.with(|cell| {
        let context = cell
            .borrow()
            .as_ref()
            .ok_or_else(|| RuntimeError::new("random context unavailable"))?
            .clone();
        let mut rng = context.rng.borrow_mut();
        Ok::<_, RuntimeError>(
            (0..amount)
                .map(|_| crate::pxs::PxsSystem::sample_cast_velocity(&mut rng, level))
                .collect::<Vec<_>>(),
        )
    })?;

    if let Some(material) = material {
        try_with_host_context_mut("CastPXS requires an active engine context", |context| {
            context.register_landscape_operation(LandscapeOperation::CastPxs {
                material,
                position,
                velocities,
            });
            Ok::<_, RuntimeError>(())
        })?;
    }

    Ok(Value::Nil)
}

/// FindObject container sentinels (C4Object.h:83-84): `NoContainer()` = 124,
/// `AnyContainer()` = 123 (FnNoContainer/FnAnyContainer,
/// C4Script.cpp:6731-6732).
/// FnInsertMaterial (C4Script.cpp:2207-2211): insert one material pixel
/// at caller-relative coordinates. The authoritative fold runs the full
/// landscape/PXS/reaction path with vx/vy converted through FIXED10.
pub(crate) fn insert_material(args: &[Value]) -> Result<Value, RuntimeError> {
    let material = value_to_i32(args.first().unwrap_or(&Value::Nil), "InsertMaterial", "mat")?;
    let x = value_to_i32(args.get(1).unwrap_or(&Value::Nil), "InsertMaterial", "x")?;
    let y = value_to_i32(args.get(2).unwrap_or(&Value::Nil), "InsertMaterial", "y")?;
    // FnInsertMaterial (C4Script.cpp:2207-2211): vx/vy ride into
    // C4Landscape::InsertMaterial (FIXED10 there).
    let vx = value_to_i32(args.get(3).unwrap_or(&Value::Nil), "InsertMaterial", "vx")?;
    let vy = value_to_i32(args.get(4).unwrap_or(&Value::Nil), "InsertMaterial", "vy")?;
    with_host_context_mut(Ok(Value::Bool(false)), |context| {
        let Some(material_id) = usize::try_from(material)
            .ok()
            .and_then(crate::material::MaterialId::new)
        else {
            return Ok(Value::Bool(false));
        };
        let Some(materials) = context.world.materials() else {
            return Ok(Value::Bool(false));
        };
        let Some(definition) = materials.get_by_id(material_id) else {
            return Ok(Value::Bool(false));
        };
        let density = definition.density();
        let max_slide = definition.max_slide();
        let instable = definition.instable();
        let position = context
            .caller_scope()
            .map_or(Vector2::new(x, y), |(_, base)| {
                Vector2::new(base.x.saturating_add(x), base.y.saturating_add(y))
            });
        // C4Landscape::InsertMaterial returns true before checking bounds for
        // density-zero materials, and performs no landscape operation.
        if density == 0 {
            return Ok(Value::Bool(true));
        }
        let can_insert = context.world.landscape_ref().is_some_and(|landscape| {
            landscape
                .insert_material_destination(
                    position.x,
                    position.y,
                    density,
                    context.world.landscape_push_pull(),
                    max_slide,
                    instable,
                    materials,
                )
                .is_some()
        });
        if !can_insert {
            return Ok(Value::Bool(false));
        }
        context.register_landscape_operation(LandscapeOperation::InsertMaterial {
            material,
            position,
            velocity: Vector2::new(vx, vy),
        });
        Ok(Value::Bool(true))
    })
}

/// FnExtractLiquid (C4Script.cpp:2194-2199): caller-relative coordinates,
/// MNone for a non-liquid pixel, otherwise the material number returned by
/// C4Landscape::ExtractMaterial. A callback-local COW preview supplies C++'s
/// synchronous visibility; the matching authoritative mutation folds after
/// the callback and runs instability side effects exactly once.
pub(crate) fn extract_liquid(args: &[Value]) -> Result<Value, RuntimeError> {
    let x = value_to_i32(args.first().unwrap_or(&Value::Nil), "ExtractLiquid", "x")?;
    let y = value_to_i32(args.get(1).unwrap_or(&Value::Nil), "ExtractLiquid", "y")?;
    with_host_context_mut(Ok(Value::Int(MATERIAL_NONE)), |context| {
        let position = context
            .caller_scope()
            .map_or(Vector2::new(x, y), |(_, base)| {
                Vector2::new(base.x.saturating_add(x), base.y.saturating_add(y))
            });
        let Some(material) = context.preview_extract_liquid(position) else {
            return Ok(Value::Int(MATERIAL_NONE));
        };
        Ok(Value::Int(material.index() as i32))
    })
}

/// FnExtractMaterialAmount (C4Script.cpp:2264-2273): extract up to
/// `amount` pixels while `GBackMat(x,y) == mat`, each through
/// ExtractMaterial (FindMatTop + clear). The count is computed by an
/// overlay simulation on the read view and the mutation staged as an
/// operation applied on the same state.
pub(crate) fn extract_material_amount(args: &[Value]) -> Result<Value, RuntimeError> {
    let x = value_to_i32(
        args.first().unwrap_or(&Value::Nil),
        "ExtractMaterialAmount",
        "x",
    )?;
    let y = value_to_i32(
        args.get(1).unwrap_or(&Value::Nil),
        "ExtractMaterialAmount",
        "y",
    )?;
    let material = value_to_i32(
        args.get(2).unwrap_or(&Value::Nil),
        "ExtractMaterialAmount",
        "mat",
    )?;
    let amount = value_to_i32(
        args.get(3).unwrap_or(&Value::Nil),
        "ExtractMaterialAmount",
        "amount",
    )?;
    with_host_context_mut(Ok(Value::Int(0)), |context| {
        let mut position = Vector2::new(x, y);
        if let Some(object) = context.object_context() {
            let base = object.current_position;
            position = Vector2::new(base.x + x, base.y + y);
        }
        let Some(material_id) = usize::try_from(material)
            .ok()
            .and_then(crate::material::MaterialId::new)
        else {
            return Ok(Value::Int(0));
        };
        let Some(materials) = context.world.materials() else {
            return Ok(Value::Int(0));
        };
        let extracted = context
            .world
            .landscape_ref()
            .map(|landscape| {
                landscape.simulate_extract_material_amount(
                    materials,
                    position.x,
                    position.y,
                    material_id,
                    amount,
                )
            })
            .unwrap_or(0);
        if extracted > 0 {
            context.register_landscape_operation(LandscapeOperation::ExtractMaterialAmount {
                material,
                position,
                amount: extracted,
            });
        }
        Ok(Value::Int(extracted))
    })
}
