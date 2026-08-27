use super::*;
use clonk_core::log_target::SCRIPT_LOG_TARGET;

/// Which post-name script argument layout a command function uses.
#[derive(Clone, Copy)]
pub(crate) enum CommandArgLayout {
    /// FnSetCommand: (name, target, Tx, Ty, target2, data, retries) — no
    /// update-interval slot, and the resulting command is always pushed
    /// with C4CMD_Mode_Base (C4Script.cpp:840-867, C4Object.cpp:3949).
    Set,
    /// FnAddCommand/FnAppendCommand: (name, target, Tx, Ty, target2,
    /// interval, data, retries, base_mode); an unfilled base_mode is int 0
    /// = C4CMD_Mode_SilentSub (C4Script.cpp:870-916, C4Command.h:62).
    Add,
}

pub(crate) fn parse_command_request(
    id: CommandId,
    args: &[Value],
    layout: CommandArgLayout,
    function: &str,
) -> Result<CommandRequest, RuntimeError> {
    let target = if args.len() > 1 {
        parse_object_reference_argument(&args[1], function, "target")?
    } else {
        None
    };

    let (tx, tx_definition, tx_value) = if args.len() > 2 {
        match &args[2] {
            Value::Nil => (None, None, (id == CommandId::Call).then_some(Value::Nil)),
            // Tx is a C4Value rather than C4ValueInt in all three C++
            // wrappers. Call uniquely skips ConvertTo(C4V_Int) and forwards
            // arbitrary tagged values to its script callback. Legacy
            // conversion also leaves a C4ID's tag intact.
            Value::C4Id(id) => {
                let raw = cast_c4id_payload(id);
                (
                    Some(raw as i32),
                    (raw != 0).then(|| clonk_script::c4_id_from_raw(raw)),
                    Some(Value::C4Id(id.clone())),
                )
            }
            other if id == CommandId::Call => (
                match other {
                    Value::Int(value) => Some(*value),
                    _ => None,
                },
                None,
                Some(other.clone()),
            ),
            other => {
                let value = value_to_i32(other, function, "Tx")?;
                (Some(value), None, Some(Value::Int(value)))
            }
        }
    } else {
        (None, None, (id == CommandId::Call).then_some(Value::Nil))
    };

    let ty = if args.len() > 3 {
        match &args[3] {
            Value::Nil => None,
            other => Some(value_to_i32(other, function, "Ty")?),
        }
    } else {
        None
    };

    let target2 = if args.len() > 4 {
        parse_object_reference_argument(&args[4], function, "target2")?
    } else {
        None
    };

    let (interval_slot, data_slot, retries_slot, mode_slot) = match layout {
        CommandArgLayout::Set => (None, 5usize, 6usize, None),
        CommandArgLayout::Add => (Some(5usize), 6, 7, Some(8usize)),
    };

    let update_interval = interval_slot
        .and_then(|slot| args.get(slot))
        .map(|value| value_to_i32(value, function, "update_interval"))
        .transpose()?
        .unwrap_or(0);

    let data_value = args.get(data_slot).unwrap_or(&Value::Nil);
    let data = match (id, data_value) {
        (CommandId::Call, Value::String(text)) => {
            let text = text.as_ref();
            let nul = text.as_bytes().iter().position(|byte| *byte == 0);
            CommandData::Text(text[..nul.unwrap_or(text.len())].to_owned())
        }
        // C4Value::getStr() returns null for every failed strict conversion,
        // and FnStringPar maps that null to an empty function name. Native
        // still queues the Call; C4Command::Call fails it during execution.
        (CommandId::Call, _) => CommandData::Text(String::new()),
        (_, Value::Nil) => CommandData::Integer(0),
        (_, Value::C4Id(id)) => CommandData::Integer(cast_c4id_payload(id) as i32),
        (_, other) => CommandData::Integer(value_to_i32(other, function, "data")?),
    };

    let retries = args
        .get(retries_slot)
        .map(|value| value_to_i32(value, function, "retries"))
        .transpose()?
        .unwrap_or(0);

    let mode = mode_slot
        .and_then(|slot| args.get(slot))
        .map(|value| value_to_i32(value, function, "mode"))
        .transpose()?
        .map(|raw| CommandMode::from_i32(raw).unwrap_or(CommandMode::Base))
        .unwrap_or(match layout {
            CommandArgLayout::Set => CommandMode::Base,
            CommandArgLayout::Add => CommandMode::SilentSub,
        });

    let mut request = CommandRequest::new(id)
        .with_target(target)
        .with_target2(target2)
        .with_tx(tx)
        .with_ty(ty)
        .with_data(data)
        .with_update_interval(update_interval)
        .with_retries(retries)
        .with_mode(mode);
    request.tx_definition = tx_definition;
    request.tx_value = tx_value;
    Ok(request)
}

/// Presence check for ordered command previews: once a same-call scope
/// exists, its removal state supersedes the immutable frame snapshot.
fn preview_object_is_present(target: ObjectId) -> bool {
    with_host_context(false, |context| match context.object_scope(target) {
        Some(scope) => !scope.destroy && scope.status() != ObjectStatus::Deleted,
        None => context
            .get_world_object(target)
            .is_some_and(|object| object.is_present()),
    })
}

/// Script callbacks run against a copied world view, so construction terrain
/// needs a matching live preview in addition to the operation folded into the
/// real engine afterward. This mirrors the pixel mutations relevant to script
/// queries; instability/mass-mover side effects remain on the authoritative
/// `Engine::prepare_construction_terrain` fold.
pub(crate) fn preview_construction_terrain(
    landscape: &mut Landscape,
    materials: &MaterialSet,
    center_x: i32,
    bottom_y: i32,
    width: i32,
    height: i32,
    basement: i32,
) {
    let x = center_x.saturating_sub(width / 2);
    let y = bottom_y.saturating_sub(height);

    if width.saturating_mul(height) < 12_000 {
        if let Some((grid_width, _)) = landscape.grid_dimensions() {
            for column in x..x.saturating_add(width) {
                for row in y..y.saturating_add(height) {
                    let _ = landscape.dig_free_pix(column, row, materials);
                }
            }
            let start = x.max(0).min(grid_width) as usize;
            let end = x.saturating_add(width).max(0).min(grid_width) as usize;
            landscape.refresh_raster_columns(start..end);
        } else {
            for column in x..x.saturating_add(width) {
                let Some(previous_height) = landscape.surface_height(column) else {
                    continue;
                };
                let Some(material_id) = landscape.solid_material_at(column) else {
                    continue;
                };
                if materials
                    .get_by_id(material_id)
                    .is_none_or(|material| !material.dig_free())
                {
                    continue;
                }
                let clamped_bottom = bottom_y.max(0);
                let desired_bottom = if clamped_bottom <= previous_height {
                    if clamped_bottom.saturating_add(1) <= previous_height {
                        continue;
                    }
                    previous_height.saturating_add(1)
                } else {
                    clamped_bottom
                };
                landscape.ensure_surface_at_least(column, desired_bottom);
            }
        }
    }

    let vehicle = materials.id_of("Vehicle");
    if let Some((grid_width, grid_height)) = landscape.grid_dimensions() {
        for column in x..x.saturating_add(width) {
            let mut target_y = bottom_y;
            while target_y + 1 < grid_height
                && landscape.density_at(column, target_y + 1, materials) < crate::C4M_SOLID
            {
                target_y += 1;
            }
            if target_y + 1 >= grid_height || target_y - bottom_y >= 20 {
                continue;
            }
            let Some(pixel) = landscape.grid_byte_at(column, target_y + 1) else {
                continue;
            };
            if vehicle.is_some_and(|vehicle| {
                landscape.border_material_at(column, target_y + 1) == Some(vehicle)
            }) {
                continue;
            }
            while target_y >= bottom_y {
                landscape.grid_set_byte(column, target_y, pixel);
                target_y -= 1;
            }
        }
        let start = x.max(0).min(grid_width) as usize;
        let end = x.saturating_add(width).max(0).min(grid_width) as usize;
        landscape.refresh_raster_columns(start..end);
    } else {
        for column in x..x.saturating_add(width) {
            let Ok(column_index) = u32::try_from(column) else {
                continue;
            };
            let Some(surface) = landscape.surface_height(column) else {
                continue;
            };
            if surface.saturating_sub(1).saturating_sub(bottom_y) < 20 {
                landscape.set_height(column_index, bottom_y);
            }
        }
    }

    let Some(granite) = materials.id_of("Granite") else {
        return;
    };
    let draw_rect = |landscape: &mut Landscape, x: i32, width: i32| {
        let Some(granite_definition) = materials.get_by_id(granite) else {
            return;
        };
        let granite_density = granite_definition.density();
        let granite_dig_free = i32::from(granite_definition.dig_free());
        for row in bottom_y..bottom_y.saturating_add(8) {
            for column in x..x.saturating_add(width) {
                let current_density = landscape.density_at(column, row, materials);
                let current_dig_free = landscape
                    .border_material_at(column, row)
                    .and_then(|material| materials.get_by_id(material))
                    .map(|material| i32::from(material.dig_free()))
                    .unwrap_or(1);
                if granite_density > current_density
                    || (granite_density == current_density && granite_dig_free <= current_dig_free)
                {
                    let _ = landscape.insert_material_pix(column, row, granite);
                }
            }
        }
    };
    if basement > 1 {
        let border_width = basement.min(width);
        draw_rect(landscape, x, border_width);
        draw_rect(
            landscape,
            x.saturating_add(width).saturating_sub(border_width),
            border_width,
        );
    } else if basement != 0 {
        draw_rect(landscape, x, width);
    }
}

/// Side effects a nested script call (Find_Func/GameCall reentrancy) made to
/// an object other than the outer call's `this`. Folded out of the nested
/// scope in first-call order; the engine applies them after the outer
/// object's update.
fn preview_dig_single_pixel(
    landscape: &mut Landscape,
    materials: &MaterialSet,
    x: i32,
    y: i32,
    dx: i32,
    dy: i32,
) {
    if landscape.density_at(x, y, materials) > landscape.density_at(x + dx, y + dy, materials) {
        let _ = landscape.dig_free_pix(x, y, materials);
    }
}

fn preview_dig_column(
    landscape: &mut Landscape,
    materials: &MaterialSet,
    column: i32,
    target_height: i32,
) -> Option<(crate::MaterialId, i32)> {
    if column < 0 || column >= landscape.width() as i32 {
        return None;
    }
    if materials.is_empty() {
        landscape.ensure_surface_at_least(column, target_height);
        return None;
    }
    let previous = landscape.surface_height(column).unwrap_or(0);
    let material_id = landscape.solid_material_at(column)?;
    let material = materials.get_by_id(material_id)?;
    if !material.dig_free() {
        return None;
    }
    let target = target_height.max(0);
    let desired = if target <= previous {
        if target.saturating_add(1) <= previous {
            return None;
        }
        previous.saturating_add(1)
    } else {
        target
    };
    landscape.ensure_surface_at_least(column, desired);
    let removed = landscape
        .surface_height(column)
        .unwrap_or(previous)
        .saturating_sub(previous);
    (removed > 0).then_some((material_id, removed))
}

pub(crate) fn preview_dig_circle_pixels(
    landscape: &mut Landscape,
    materials: &MaterialSet,
    center: Vector2,
    radius: i32,
) -> HashMap<crate::MaterialId, i32> {
    let mut counts = HashMap::new();
    if radius <= 0 {
        return counts;
    }
    if landscape.pixel_grid().is_some() {
        let mut line_width = 0;
        for ycnt in -radius..radius {
            let remaining =
                i64::from(radius) * i64::from(radius) - i64::from(ycnt) * i64::from(ycnt);
            line_width = (remaining as f64).sqrt() as i32;
            let y = center.y.saturating_add(ycnt);
            let extend = i32::from(line_width == 0);
            for xcnt in -line_width..line_width + extend {
                if let Some(material) =
                    landscape.dig_free_pix(center.x.saturating_add(xcnt), y, materials)
                {
                    *counts.entry(material).or_insert(0) += 1;
                }
            }
            preview_dig_single_pixel(
                landscape,
                materials,
                center.x.saturating_sub(line_width).saturating_sub(1),
                y,
                -1,
                0,
            );
            preview_dig_single_pixel(
                landscape,
                materials,
                center.x.saturating_add(line_width).saturating_add(extend),
                y,
                1,
                0,
            );
        }
        preview_dig_single_pixel(
            landscape,
            materials,
            center.x,
            center.y.saturating_sub(radius).saturating_sub(1),
            0,
            -1,
        );
        let extend = i32::from(line_width == 0);
        for xcnt in -line_width..line_width + extend {
            preview_dig_single_pixel(
                landscape,
                materials,
                center.x.saturating_add(xcnt),
                center.y.saturating_add(radius),
                0,
                1,
            );
        }
        if let Some((width, _)) = landscape.grid_dimensions() {
            let start = center
                .x
                .saturating_sub(radius)
                .saturating_sub(1)
                .clamp(0, width) as usize;
            let end = center
                .x
                .saturating_add(radius)
                .saturating_add(2)
                .clamp(0, width) as usize;
            landscape.refresh_raster_columns(start..end);
        }
        return counts;
    }
    let radius_sq = i64::from(radius) * i64::from(radius);
    for dx in -radius..=radius {
        let remaining = radius_sq - i64::from(dx) * i64::from(dx);
        if remaining >= 0 {
            if let Some((material, removed)) = preview_dig_column(
                landscape,
                materials,
                center.x.saturating_add(dx),
                center
                    .y
                    .saturating_add((remaining as f64).sqrt().floor() as i32),
            ) {
                *counts.entry(material).or_insert(0) += removed;
            }
        }
    }
    counts
}

pub(crate) fn preview_dig_rect_pixels(
    landscape: &mut Landscape,
    materials: &MaterialSet,
    origin: Vector2,
    width: i32,
    height: i32,
) -> HashMap<crate::MaterialId, i32> {
    let mut counts = HashMap::new();
    if width <= 0 || height <= 0 {
        return counts;
    }
    if landscape.pixel_grid().is_some() {
        for x in origin.x..origin.x.saturating_add(width) {
            for y in origin.y..origin.y.saturating_add(height) {
                if let Some(material) = landscape.dig_free_pix(x, y, materials) {
                    *counts.entry(material).or_insert(0) += 1;
                }
            }
        }
        if let Some((grid_width, _)) = landscape.grid_dimensions() {
            landscape.refresh_raster_columns(
                origin.x.clamp(0, grid_width) as usize
                    ..origin.x.saturating_add(width).clamp(0, grid_width) as usize,
            );
        }
        return counts;
    }
    let bottom = origin.y.saturating_add(height);
    for offset in 0..width {
        if let Some((material, removed)) = preview_dig_column(
            landscape,
            materials,
            origin.x.saturating_add(offset),
            bottom,
        ) {
            *counts.entry(material).or_insert(0) += removed;
        }
    }
    counts
}

pub(crate) fn preview_shake_circle_pixels(
    landscape: &mut Landscape,
    materials: &MaterialSet,
    center: Vector2,
    radius: i32,
) {
    if radius <= 0 {
        return;
    }
    if landscape.pixel_grid().is_some() {
        let mut cleared_solid_pixels = Vec::new();
        for ycnt in (-radius..radius).rev() {
            let remaining =
                i64::from(radius) * i64::from(radius) - i64::from(ycnt) * i64::from(ycnt);
            let line_width = (remaining as f64).sqrt() as i32;
            let y = center.y.saturating_add(ycnt);
            for xcnt in -line_width..line_width + i32::from(line_width == 0) {
                let x = center.x.saturating_add(xcnt);
                if let Some(material) = landscape
                    .dig_free_pix(x, y, materials)
                    .and_then(|material| materials.get_by_id(material))
                {
                    if material.dig_free() && material.is_solid() {
                        cleared_solid_pixels.push(Vector2::new(x, y));
                    }
                }
            }
        }
        let fragments = landscape.shake_free_fragments(&cleared_solid_pixels, materials);
        let mut first_changed_column = center.x.saturating_sub(radius);
        let mut last_changed_column = center.x.saturating_add(radius);
        for (position, _) in fragments {
            first_changed_column = first_changed_column.min(position.x);
            last_changed_column = last_changed_column.max(position.x);
            let _ = landscape.dig_free_pix(position.x, position.y, materials);
        }
        if let Some((width, _)) = landscape.grid_dimensions() {
            landscape.refresh_raster_columns(
                first_changed_column.clamp(0, width) as usize
                    ..last_changed_column.saturating_add(1).clamp(0, width) as usize,
            );
        }
        return;
    }
    if materials.is_empty() {
        return;
    }
    let radius_sq = i64::from(radius) * i64::from(radius);
    for dx in -radius..=radius {
        let column = center.x.saturating_add(dx);
        if column < 0 || column >= landscape.width() as i32 {
            continue;
        }
        let remaining = radius_sq - i64::from(dx) * i64::from(dx);
        if remaining < 0 {
            continue;
        }
        let previous = landscape.surface_height(column).unwrap_or(0);
        let mut target = center
            .y
            .saturating_add((remaining as f64).sqrt().floor() as i32);
        if target <= previous {
            if previous.saturating_sub(target) > radius {
                continue;
            }
            target = previous.saturating_add(1);
        }
        let _ = preview_dig_column(landscape, materials, column, target);
    }
}

pub(crate) fn preview_raster_blast(
    landscape: &mut Landscape,
    materials: &MaterialSet,
    center: Vector2,
    radius: i32,
    rng: &mut LcgRng,
) -> (BlastPixelReplay, HashMap<crate::MaterialId, i32>) {
    let mut counts = HashMap::new();
    for ycnt in -radius..=radius {
        let remaining = i64::from(radius) * i64::from(radius) - i64::from(ycnt) * i64::from(ycnt);
        let line_width = (remaining.max(0) as f64).sqrt() as i32;
        let y = center.y.saturating_add(ycnt);
        for xcnt in -line_width..line_width + i32::from(line_width == 0) {
            if let Some(material) = landscape.border_material_at(center.x.saturating_add(xcnt), y) {
                *counts.entry(material).or_insert(0) += 1;
            }
        }
    }

    let threshold = blast_threshold(radius);
    let mut steps = Vec::new();
    for ycnt in -radius..=radius {
        let remaining = i64::from(radius) * i64::from(radius) - i64::from(ycnt) * i64::from(ycnt);
        let line_width = (remaining.max(0) as f64).sqrt() as i32;
        let y = center.y.saturating_add(ycnt);
        for xcnt in -line_width..line_width + i32::from(line_width == 0) {
            let x = center.x.saturating_add(xcnt);
            let original_material = landscape.border_material_at(x, y);
            let mut shift_byte = None;
            let mut clear = false;
            if let Some((material_id, material)) =
                original_material.and_then(|id| materials.get_by_id(id).map(|entry| (id, entry)))
            {
                clear = material.blast_free();
                if let Some(byte) = material
                    .blast_shift_to_spec()
                    .zip(material.blast_shift_to_target())
                    .and_then(|(spec, fallback)| {
                        landscape.crossmapped_material_texture_byte(
                            spec,
                            material_id,
                            materials,
                            fallback,
                        )
                    })
                {
                    let total = counts.get(&material_id).copied().unwrap_or(0);
                    if i64::from(rng.random(total)) < threshold {
                        shift_byte = Some(byte);
                    }
                }
            }
            if let Some(byte) = shift_byte {
                let _ = landscape.insert_material_texture_pix(x, y, byte);
            }
            if clear {
                let _ = landscape.clear_pix(x, y);
            }
            steps.push(BlastRasterReplayStep {
                position: Vector2::new(x, y),
                original_material,
                shift_byte,
                clear,
            });
        }
    }
    if let Some((width, _)) = landscape.grid_dimensions() {
        landscape.refresh_raster_columns(
            center.x.saturating_sub(radius).clamp(0, width) as usize
                ..center
                    .x
                    .saturating_add(radius)
                    .saturating_add(1)
                    .clamp(0, width) as usize,
        );
    }
    (
        BlastPixelReplay::Raster {
            steps,
            pixel_count_by_material: counts.clone(),
        },
        counts,
    )
}

pub(crate) fn preview_column_blast(
    landscape: &mut Landscape,
    materials: &MaterialSet,
    center: Vector2,
    radius: i32,
    rng: &mut LcgRng,
) -> (BlastPixelReplay, HashMap<crate::MaterialId, i32>) {
    let result = landscape.blast_circle(center, radius, materials);
    let threshold = blast_threshold(radius);
    let mut shift_decisions = Vec::with_capacity(result.shift_candidates.len());
    for candidate in &result.shift_candidates {
        let total = result
            .pixel_count_by_material
            .get(&candidate.material)
            .copied()
            .unwrap_or(0);
        let mut should_shift = false;
        if total > 0 {
            for _ in 0..candidate.pixel_count.max(0) {
                if i64::from(rng.random(total)) < threshold {
                    should_shift = true;
                }
            }
        }
        shift_decisions.push(should_shift);
        if should_shift && candidate.apply_column_shift && candidate.column >= 0 {
            landscape.set_solid_material(candidate.column as u32, Some(candidate.target));
        }
    }
    let counts = result.pixel_count_by_material;
    (BlastPixelReplay::Column { shift_decisions }, counts)
}

pub(crate) fn preview_captured_blast_pixels(
    landscape: &mut Landscape,
    materials: &MaterialSet,
    center: Vector2,
    radius: i32,
    replay: &BlastPixelReplay,
) {
    match replay {
        BlastPixelReplay::Raster { steps, .. } => {
            for step in steps {
                if let Some(byte) = step.shift_byte {
                    let _ = landscape.insert_material_texture_pix(
                        step.position.x,
                        step.position.y,
                        byte,
                    );
                }
                if step.clear {
                    let _ = landscape.clear_pix(step.position.x, step.position.y);
                }
            }
            if let Some((width, _)) = landscape.grid_dimensions() {
                landscape.refresh_raster_columns(
                    center.x.saturating_sub(radius).clamp(0, width) as usize
                        ..center
                            .x
                            .saturating_add(radius)
                            .saturating_add(1)
                            .clamp(0, width) as usize,
                );
            }
        }
        BlastPixelReplay::Column { shift_decisions } => {
            let result = landscape.blast_circle(center, radius, materials);
            for (candidate, should_shift) in result.shift_candidates.iter().zip(shift_decisions) {
                if *should_shift && candidate.apply_column_shift && candidate.column >= 0 {
                    landscape.set_solid_material(candidate.column as u32, Some(candidate.target));
                }
            }
        }
    }
}

/// FnGetCommand (C4Script.cpp:918-945): walk the command stack to entry
/// iCommandNum and return the requested element. Reads use the staged live
/// stack when script has changed or executed commands in this call.
pub(crate) fn get_command(args: &[Value]) -> Result<Value, RuntimeError> {
    let mut index = 0;
    let mut target_id: Option<ObjectId> = None;
    if let Some(arg) = args.get(index) {
        if matches!(
            arg,
            Value::Object(_) | Value::Proplist(_) | Value::Nil | Value::Int(0)
        ) {
            target_id = parse_object_reference_argument(arg, "GetCommand", "target")?;
            index += 1;
        }
    }
    let element = args
        .get(index)
        .map(|value| value_to_i32(value, "GetCommand", "element"))
        .transpose()?
        .unwrap_or(0);
    index += 1;
    let command_num = args
        .get(index)
        .map(|value| value_to_i32(value, "GetCommand", "command number"))
        .transpose()?
        .unwrap_or(0);

    with_host_context(Ok(Value::Nil), |context| {
        let resolved = target_id.or_else(|| context.object_context().map(|object| object.id()));
        let Some(resolved) = resolved else {
            return Ok(Value::Nil);
        };
        // `while (Command && iCommandNum--)` (C4Script.cpp:924): a negative
        // count walks off the list end -> nil.
        if command_num < 0 {
            return Ok(Value::Nil);
        }
        let view = context
            .object_scope(resolved)
            .and_then(|scope| {
                scope
                    .live_commands
                    .command_views()
                    .get(command_num as usize)
                    .cloned()
            })
            .or_else(|| {
                context
                    .get_world_object(resolved)
                    .and_then(|object| object.commands.get(command_num as usize).cloned())
            });
        let Some(view) = view else {
            return Ok(Value::Nil);
        };
        // Element map (C4Script.cpp:926-945): 0 name, 1 Target, 2 Tx,
        // 3 C4VInt(Ty), 4 Target2, 5 C4Value(Data, C4V_Any) — a zero
        // int Data reads nil in C++.
        match element {
            0 => Ok(Value::String(view.name.clone().into())),
            1 => Ok(view
                .target
                .map(object_reference_value)
                .unwrap_or(Value::Nil)),
            2 => Ok(view
                .tx_value
                .or_else(|| view.tx_definition.map(Value::C4Id))
                .or_else(|| view.tx.map(Value::Int))
                .unwrap_or(Value::Nil)),
            3 => Ok(Value::Int(view.ty.unwrap_or(0))),
            4 => Ok(view
                .target2
                .map(object_reference_value)
                .unwrap_or(Value::Nil)),
            5 => Ok(view
                .legacy_data
                .or(match view.data {
                    CommandData::Integer(data) => Some(data),
                    CommandData::Text(_) | CommandData::None => None,
                })
                .map(command_data_any_value)
                .unwrap_or(Value::Nil)),
            _ => Ok(Value::Nil),
        }
    })
}

/// FnFinishCommand (C4Script.cpp:947-957): mark the iCommandNum-th
/// command of the target finished (success) or bump its failures.
pub(crate) fn finish_command(args: &[Value]) -> Result<Value, RuntimeError> {
    let target = args
        .first()
        .map(|arg| parse_object_reference_argument(arg, "FinishCommand", "obj"))
        .transpose()?
        .flatten();
    let success = value_to_bool(
        args.get(1).unwrap_or(&Value::Nil),
        "FinishCommand",
        "success",
    )?;
    let index = parse_optional_i32(args.get(2), "FinishCommand", "command")?.unwrap_or(0);
    let active = with_host_context(None, |context| {
        context.object_context().map(|object| object.id())
    });
    if let Some(target) = target {
        if Some(target) != active {
            return match call_world_object_function(
                target,
                "FinishCommand",
                &[Value::Int(0), Value::Bool(success), Value::Int(index)],
            ) {
                Some(result) => result,
                None => Ok(Value::Bool(false)),
            };
        }
    }
    with_host_context_mut(Ok(Value::Bool(false)), |context| {
        let Some(object) = context.object_context_mut() else {
            return Ok(Value::Bool(false));
        };
        if !object.live_commands.finish_entry_public(index, success) {
            return Ok(Value::Bool(false));
        }
        object
            .command_operations
            .push(CommandOperation::Finish { index, success });
        Ok(Value::Bool(true))
    })
}

pub(crate) fn get_path(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.len() != 4 {
        return Err(RuntimeError::new(
            "GetPath expects 4 arguments: from_x, from_y, to_x, to_y",
        ));
    }

    let from_x = value_to_i32(&args[0], "GetPath", "from_x")?;
    let from_y = value_to_i32(&args[1], "GetPath", "from_y")?;
    let to_x = value_to_i32(&args[2], "GetPath", "to_x")?;
    let to_y = value_to_i32(&args[3], "GetPath", "to_y")?;

    with_host_context(Ok(Value::Nil), |context| {
        let landscape = match context.landscape_ref() {
            Some(landscape) => landscape,
            None => return Ok(Value::Nil),
        };
        let mut finder = PathFinder::new(landscape, context.world.transfer_zones());
        let (level, transfer_zones_enabled) = context.world.pathfinder_settings();
        finder.set_level(level);
        finder.enable_transfer_zones(transfer_zones_enabled);
        let path = finder.find(Vector2::new(from_x, from_y), Vector2::new(to_x, to_y));
        *context.world.pathfinder_debug.borrow_mut() = finder.debug_snapshot().clone();
        let path = match path {
            Some(path) => path,
            None => return Ok(Value::Nil),
        };
        let mut result = ValueMap::new();
        result.insert("Length".into(), Value::Int(path.length));
        let mut waypoints = Vec::with_capacity(path.waypoints.len());
        for waypoint in path.waypoints {
            let mut map = ValueMap::new();
            map.insert("X".into(), Value::Int(waypoint.x));
            map.insert("Y".into(), Value::Int(waypoint.y));
            if let Some(target) = waypoint.transfer_target {
                map.insert("TransferTarget".into(), object_reference_value(target));
            }
            waypoints.push(Value::Proplist(map));
        }
        result.insert("Waypoints".into(), Value::Array(waypoints));
        Ok(Value::Proplist(result))
    })
}

pub(crate) fn set_transfer_zone(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.len() > 5 {
        return Err(RuntimeError::new(
            "SetTransferZone expects at most 5 arguments: x, y, width, height, [object]",
        ));
    }

    let x = value_to_i32(args.first().unwrap_or(&Value::Nil), "SetTransferZone", "x")?;
    let y = value_to_i32(args.get(1).unwrap_or(&Value::Nil), "SetTransferZone", "y")?;
    let width = value_to_i32(
        args.get(2).unwrap_or(&Value::Nil),
        "SetTransferZone",
        "width",
    )?;
    let height = value_to_i32(
        args.get(3).unwrap_or(&Value::Nil),
        "SetTransferZone",
        "height",
    )?;
    let explicit_object = args
        .get(4)
        .map(|value| parse_object_reference_argument(value, "SetTransferZone", "object"))
        .transpose()?;

    HOST_CONTEXT.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let context = borrow.as_mut().ok_or_else(|| {
            RuntimeError::new("SetTransferZone requires an active engine context")
        })?;

        let owner = match explicit_object.flatten() {
            Some(id) => id,
            None => context
                .object_context()
                .map(|ctx| ctx.id())
                .ok_or_else(|| {
                    RuntimeError::new(
                        "SetTransferZone requires an object argument or active object context",
                    )
                })?,
        };

        // pObj->x/y off the LIVE object (C4Script.cpp:3154): the executing
        // scope resolves the object even while its own Initialize runs
        // before the world snapshot knows it (C4Object::Init fires the
        // callbacks on the constructed object, C4Object.cpp:215+ — the
        // WZKP homebase placed at player join).
        let position = context
            .object_context()
            .filter(|object| object.id() == owner)
            .map(|object| object.effective_position())
            .or_else(|| {
                context
                    .get_world_object(owner)
                    .map(|object| object.position())
            })
            .ok_or_else(|| {
                RuntimeError::new(format!(
                    "SetTransferZone: object {} not found in current engine context",
                    owner
                ))
            })?;

        if width == 0 || height == 0 {
            context.register_transfer_zone_command(TransferZoneCommand::clear(owner));
            return Ok(Value::Bool(true));
        }

        let abs_x = position.x.saturating_add(x);
        let abs_y = position.y.saturating_add(y);
        let rect = TransferZoneRect {
            x: abs_x,
            y: abs_y,
            width,
            height,
        };
        context.register_transfer_zone_command(TransferZoneCommand::set(owner, rect));
        Ok(Value::Bool(true))
    })
}

/// The post-pixel evaluate loop of C4Landscape::BlastFree. Object creation
/// and its lifecycle callbacks happen between consecutive groups of four
/// random draws; PXS draws follow that material's objects before the next
/// material. Keeping this in the live host context makes callbacks and the
/// outer script observe the same order as C++ while the queued operations
/// only commit already-previewed terrain/PXS storage.
pub(crate) fn process_preview_blast_reactions(
    center: Vector2,
    controller: Option<i32>,
    counts: &HashMap<crate::MaterialId, i32>,
) -> Result<(), RuntimeError> {
    let materials =
        try_with_host_context("BlastFree requires an active engine context", |context| {
            Ok::<_, RuntimeError>(
                context
                    .world
                    .materials()
                    .map(|materials| {
                        materials
                            .iter()
                            .map(|material| {
                                (
                                    material.id(),
                                    material.blast_to_object_name().map(str::to_string),
                                    material.blast_to_object_ratio(),
                                    material.blast_to_pxs_ratio(),
                                )
                            })
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default(),
            )
        })?;

    for (material, definition, object_ratio, pxs_ratio) in materials {
        let count = counts.get(&material).copied().unwrap_or(0);
        if count == 0 {
            continue;
        }
        if let (Some(definition), Some(ratio)) = (definition, object_ratio) {
            if ratio != 0 {
                for _ in 0..count / ratio {
                    let rotation_velocity = itofix(draw_context_random(3)? + 1);
                    let ydir = fixed10(draw_context_random(61)? - 40);
                    let xdir = fixed10(draw_context_random(61)? - 30);
                    let rotation = draw_context_random(360)?;
                    let _ = create_native_object(NativeObjectCreation {
                        definition: definition.clone(),
                        creator: None,
                        owner: OWNER_NONE,
                        controller: controller.unwrap_or(OWNER_NONE),
                        construction: FULL_CON,
                        position: center,
                        rotation,
                        velocity: FixedVec2::new(xdir, ydir),
                        rotation_velocity,
                    })?;
                }
            }
        }
        if let Some(ratio) = pxs_ratio {
            if ratio != 0 {
                let velocities = RANDOM_CONTEXT.with(|cell| {
                    let random = cell
                        .borrow()
                        .as_ref()
                        .ok_or_else(|| RuntimeError::new("random context unavailable"))?
                        .clone();
                    let mut rng = random.rng.borrow_mut();
                    Ok::<_, RuntimeError>(
                        (0..count / ratio)
                            .map(|_| crate::pxs::PxsSystem::sample_cast_velocity(&mut rng, 60))
                            .collect::<Vec<_>>(),
                    )
                })?;
                HOST_CONTEXT.with(|cell| {
                    let mut borrow = cell.borrow_mut();
                    let context = borrow.as_mut().ok_or_else(|| {
                        RuntimeError::new("BlastFree requires an active engine context")
                    })?;
                    context.register_landscape_operation(LandscapeOperation::CastPxs {
                        material,
                        position: center,
                        velocities,
                    });
                    Ok::<_, RuntimeError>(())
                })?;
            }
        }
    }
    Ok(())
}

/// C4Object::DigOutMaterialCast at the end of DigFree/DigFreeRect. The
/// creator's MaterialContents stay live through each spawned object's
/// lifecycle callbacks and reset only after CreateObject returns.
pub(crate) fn process_preview_dig_reactions(
    by_object: Option<ObjectId>,
    counts: &HashMap<crate::MaterialId, i32>,
    requested: bool,
) -> Result<(), RuntimeError> {
    let Some(target) = by_object else {
        return Ok(());
    };
    let (frame, materials) =
        try_with_host_context_mut("DigFree requires an active engine context", |context| {
            if !context.add_dig_material_counts(target, counts) {
                return Ok((context.world.frame, Vec::new()));
            }
            let materials = context
                .world
                .materials()
                .map(|materials| {
                    materials
                        .iter()
                        .map(|material| {
                            (
                                material.id(),
                                material.dig_to_object_name().map(str::to_string),
                                material.dig_to_object_ratio(),
                                material.dig_to_object_on_request_only(),
                            )
                        })
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            Ok::<_, RuntimeError>((context.world.frame, materials))
        })?;
    if frame % 5 != 0 {
        return Ok(());
    }

    for (material, definition, ratio, on_request_only) in materials {
        let (Some(definition), Some(ratio)) = (definition, ratio) else {
            continue;
        };
        if ratio == 0 || (on_request_only && !requested) {
            continue;
        }
        let position = HOST_CONTEXT.with(|cell| {
            let mut borrow = cell.borrow_mut();
            let context = borrow.as_mut()?;
            let content = context.dig_material_content(target, material);
            if content == 0 || content < ratio {
                return None;
            }
            let object = context.get_world_object(target)?;
            let position = context
                .object_scope(target)
                .map(ObjectScopeContext::effective_position)
                .unwrap_or(object.position);
            let bottom = live_object_shape(context, target).map_or(position.y, |shape| {
                position
                    .y
                    .saturating_add(shape.y)
                    .saturating_add(shape.height)
            });
            Some(Vector2::new(position.x, bottom))
        });
        let Some(position) = position else {
            continue;
        };
        let rotation = draw_context_random(360)?;
        let _ = create_native_object(NativeObjectCreation {
            definition,
            creator: Some(target),
            owner: OWNER_NONE,
            controller: OWNER_NONE,
            construction: FULL_CON,
            position,
            rotation,
            velocity: FixedVec2::ZERO,
            rotation_velocity: C4Fixed::ZERO,
        })?;
        HOST_CONTEXT.with(|cell| {
            if let Some(context) = cell.borrow_mut().as_mut() {
                context.reset_dig_material_content(target, material);
            }
        });
    }
    Ok(())
}

pub(crate) fn command_data_any_value(value: i32) -> Value {
    if value == 0 {
        return Value::Nil;
    }
    // C4Value(Data, C4V_Any) lazily runs GuessType. Packed four-byte IDs
    // win over the integer fallback; 1..9999 deliberately stay integers
    // despite LooksLikeID accepting their decimal-ID representation.
    let raw = value as u32;
    if raw >= 10_000
        && raw
            .to_le_bytes()
            .iter()
            .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || *byte == b'_')
    {
        Value::C4Id(clonk_script::c4_id_from_raw(raw as usize))
    } else {
        Value::Int(value)
    }
}

fn command_data_value(data: &CommandData) -> Value {
    match data {
        CommandData::Integer(value) => command_data_any_value(*value),
        CommandData::Text(value) => Value::String(value.clone().into()),
        CommandData::None => Value::Nil,
    }
}

fn command_view_tx_value(command: &CommandView) -> Value {
    command
        .tx_value
        .clone()
        .or_else(|| command.tx_definition.clone().map(Value::C4Id))
        .or_else(|| command.tx.map(Value::Int))
        .unwrap_or(Value::Nil)
}

fn command_view_data_value(command: &CommandView) -> Value {
    match command.legacy_data {
        Some(value) => command_data_any_value(value),
        None => command_data_value(&command.data),
    }
}

/// Host-preview twin of C4Command::Fail's ExecFail tail. ExecuteCommand runs
/// inside a script VM call, so CallFailed/BuildNeedsMaterial and the ComDir
/// stop must be visible before the next script instruction and before
/// ControlCommandFinished.
fn preview_command_failure_feedback(
    actor: ObjectId,
    feedback: CommandFailureFeedback,
) -> Result<(), RuntimeError> {
    let crew = with_host_context(false, |context| {
        context
            .object_scope(actor)
            .map(|scope| scope.ocf() & ocf::CREW_MEMBER != 0)
            .or_else(|| {
                context
                    .get_world_object(actor)
                    .map(|object| object.ocf & ocf::CREW_MEMBER != 0)
            })
            .unwrap_or(false)
    });
    if !crew {
        return Ok(());
    }

    let failure_reason = feedback.reason;
    let mut fail_message = (failure_reason == Some(CommandFailureReason::CannotBuild))
        .then(|| {
            HOST_CONTEXT.with(|cell| {
                let borrow = cell.borrow();
                let context = borrow.as_ref()?;
                let name = context
                    .object_custom_name(actor)
                    .filter(|name| !name.is_empty())
                    .or_else(|| match context.object_scope(actor) {
                        Some(scope) => scope.info_core().map(|info| info.name.clone()),
                        None => context
                            .world
                            .crew_infos
                            .get(&actor)
                            .map(|info| info.name.clone()),
                    })
                    .or_else(|| {
                        context
                            .object_effective_definition_id(actor)
                            .and_then(|definition| {
                                context
                                    .definition_metadata(&definition)
                                    .map(|metadata| metadata.name.clone())
                                    .or(Some(definition))
                            })
                    })?;
                Some(format!("{name} can't build."))
            })
        })
        .flatten();
    let command = feedback.command;
    match command.name.as_str() {
        "Call" => {
            if let (Some(target), CommandData::Text(text)) = (command.target, &command.data) {
                if !text.is_empty() {
                    let args = [
                        object_reference_value(actor),
                        command_view_tx_value(&command),
                        Value::Int(command.ty.unwrap_or(0)),
                        command
                            .target2
                            .map(object_reference_value)
                            .unwrap_or(Value::Nil),
                    ];
                    let handled =
                        call_object_own_fail_safe(target, &format!("{text}Failed"), &args)
                            .as_bool();
                    if handled {
                        return Ok(());
                    }
                }
            }
        }
        "Build" => {
            if let Some(target) = command.target {
                let (component, count) = with_host_context((None, 0), |context| {
                    let scope = context.object_scope(target);
                    let id = scope
                        .and_then(|scope| scope.pending_update.component_order.as_ref())
                        .and_then(|order| order.first())
                        .cloned()
                        .or_else(|| {
                            context.get_world_object(target).and_then(|object| {
                                object
                                    .full_state()
                                    .and_then(|state| state.component_order.first())
                                    .cloned()
                            })
                        });
                    let Some(id) = id else {
                        return (None, 0);
                    };
                    let count = scope
                        .and_then(|scope| scope.pending_update.components.as_ref())
                        .and_then(|components| components.get(&id))
                        .or_else(|| {
                            context.get_world_object(target).and_then(|object| {
                                object
                                    .full_state()
                                    .and_then(|state| state.components.get(&id))
                            })
                        })
                        .unwrap_or(0);
                    (Some(id), count)
                });
                // A truthy result suppresses only the generated material
                // message; the common Stop still runs.
                let handled = call_object_own_fail_safe(
                    actor,
                    "BuildNeedsMaterial",
                    &[
                        component.map(Value::C4Id).unwrap_or(Value::Nil),
                        Value::Int(count),
                    ],
                )
                .as_bool();
                if !handled && failure_reason.is_none() {
                    // Even when presentation is deferred, constructing the
                    // message runs target GetCustomComponents synchronously.
                    fail_message = match get_needed_mat_str(&[object_reference_value(target)])? {
                        Value::String(text) => Some(text.into_string()),
                        _ => None,
                    };
                }
            }
        }
        _ => {}
    }

    if !object_has_status(actor) {
        return Ok(());
    }
    let silent_commands = with_host_context(false, |context| {
        context
            .object_effective_definition_id(actor)
            .and_then(|id| context.definition_metadata(&id))
            .is_some_and(|metadata| metadata.silent_commands)
    });
    if silent_commands {
        return Ok(());
    }
    with_host_context_mut((), |context| {
        if context.ensure_object_scope(actor) {
            if let Some(scope) = context.object_scope_mut(actor) {
                scope.set_command_direction(CommandDirection::Stop);
            }
        }
        if let Some(text) = fail_message {
            context.register_message(MessageCommand::Append {
                spec: MessageSpec::target(text, actor),
                no_duplicates: true,
            });
        }
    });
    Ok(())
}

/// Host-preview form of ObjectActionJump used by C4Command::Grab's
/// scale/hangle let-go. ExecuteCommand runs inside the VM, so both the
/// OnActionJump hook and Jump action callbacks must complete before the
/// caller's next script instruction (C4ObjectCom.cpp:48-61,310-314).
fn preview_object_action_jump(target: ObjectId, velocity: FixedVec2) -> Result<bool, RuntimeError> {
    let handled = call_object_own_fail_safe(
        target,
        "OnActionJump",
        &[
            Value::Int(fixtoi_prec(velocity.x, 100)),
            Value::Int(fixtoi_prec(velocity.y, 100)),
            Value::Bool(true),
        ],
    );
    if value_raw_truthy(&handled) {
        return Ok(true);
    }
    if !native_set_action_by_name(target, "Jump")? {
        return Ok(false);
    }
    HOST_CONTEXT.with(|cell| {
        if let Some(object) = cell
            .borrow_mut()
            .as_mut()
            .and_then(|context| context.object_scope_mut(target))
        {
            object.set_fixed_velocity(velocity);
            object.set_mobile(true);
            object.set_t_attach(object.t_attach() & !CNAT_BOTTOM);
        }
    });
    Ok(true)
}

/// Synchronous host twin of ObjectComUnGrab. ObjectComDrop invokes this
/// after its Exit callbacks and NoCollectDelay update, before ExecuteCommand
/// may run ControlCommandFinished (C4ObjectCom.cpp:261-278,640-676).
fn preview_object_com_ungrab(
    actor: ObjectId,
    restore_fight_ready: bool,
) -> Result<bool, RuntimeError> {
    let target = HOST_CONTEXT.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let context = borrow.as_mut()?;
        if !context.ensure_object_scope(actor) {
            return None;
        }
        let scope = context.object_scope_mut(actor)?;
        if scope.effective_action_procedure() != ActionProcedure::Push {
            return None;
        }
        let target = scope.effective_action_target(0);
        scope.set_command_direction(CommandDirection::Stop);
        Some(target)
    });
    let Some(target) = target else {
        return Ok(false);
    };

    // SetActionByName("Walk") performs a full SetOCF before its action
    // callbacks. Preserve the pre-Drop FightReady capability underneath a
    // callback-installed ObjectDisabled Push action so changing to Walk can
    // expose it again; NoCollectDelay still keeps Collection disabled.
    let cached_ocf_before = HOST_CONTEXT.with(|cell| {
        let mut cached_before = None;
        if let Some(scope) = cell
            .borrow_mut()
            .as_mut()
            .and_then(|context| context.object_scope_mut(actor))
        {
            cached_before = scope.cached_ocf;
            if restore_fight_ready && scope.ocf() & ocf::ALIVE != 0 {
                let cached = scope.cached_ocf.unwrap_or_else(|| scope.ocf());
                scope.cached_ocf = Some(cached | ocf::FIGHT_READY);
            }
        }
        cached_before
    });
    if !native_set_action_by_name(actor, "Walk")? {
        HOST_CONTEXT.with(|cell| {
            if let Some(scope) = cell
                .borrow_mut()
                .as_mut()
                .and_then(|context| context.object_scope_mut(actor))
            {
                scope.cached_ocf = cached_ocf_before;
            }
        });
        return Ok(false);
    }
    HOST_CONTEXT.with(|cell| {
        if let Some(scope) = cell
            .borrow_mut()
            .as_mut()
            .and_then(|context| context.object_scope_mut(actor))
        {
            scope.set_fixed_velocity(FixedVec2::ZERO);
        }
    });
    if !close_object_menu(actor, false) {
        return Ok(false);
    }
    let target_value = target.map(object_reference_value).unwrap_or(Value::Nil);
    let _ = call_object_own_fail_safe(actor, "Grab", &[target_value, Value::Bool(false)]);
    if object_has_status(actor) {
        if let Some(target) = target.filter(|target| object_has_status(*target)) {
            let _ = call_object_own_fail_safe(
                target,
                "Grabbed",
                &[object_reference_value(actor), Value::Bool(false)],
            );
        }
    }
    Ok(true)
}

/// Synchronous ObjectActionThrow twin for script ExecuteCommand. Force and
/// facing are frozen before SetAction callbacks; position/shape and the Exit
/// callbacks are observed afterward (C4ObjectCom.cpp:120-137).
fn preview_object_action_throw(actor: ObjectId, object: ObjectId) -> Result<bool, RuntimeError> {
    let physical = HOST_CONTEXT.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let context = borrow.as_mut()?;
        if !context.ensure_object_scope(actor) || !context.ensure_object_scope(object) {
            return None;
        }
        let actor_state = context.get_world_object(actor)?;
        if !actor_state.is_present() {
            return None;
        }
        context.prepare_object_physical(actor, false)
    });
    let Some(physical) = physical else {
        return Ok(false);
    };
    let throw_force = crate::math::val_by_physical(400, physical.resolve().throw);
    let prepared = HOST_CONTEXT.with(|cell| {
        let borrow = cell.borrow();
        let context = borrow.as_ref()?;
        let actor_scope = context.object_scope(actor)?;
        let direction = if actor_scope.current_direction == Direction::Left {
            -1
        } else {
            1
        };
        Some(direction)
    });
    let Some(direction) = prepared else {
        return Ok(false);
    };
    if !native_set_action_by_name(actor, "Throw")? {
        return Ok(false);
    }

    // ObjectActionThrow consumes the synced rotation only after SetAction
    // succeeds, even if callbacks removed the carried object before Exit.
    let rotation = draw_context_random(360)?;
    let exit = HOST_CONTEXT.with(|cell| {
        let borrow = cell.borrow();
        let context = borrow.as_ref()?;
        let actor_position = context.object_scope(actor)?.effective_position();
        let shape_top = live_object_bounds_shape(context, actor)
            .map(|shape| shape.y)
            .unwrap_or(0);
        Some(Vector2::new(
            actor_position.x,
            actor_position.y.wrapping_add(shape_top).wrapping_sub(1),
        ))
    });
    if let Some(position) = exit {
        let _ = exit_object_at_position_with_full_motion_and_calls(
            object,
            position,
            rotation,
            FixedVec2::new(throw_force * direction, -throw_force),
            throw_force * direction,
            true,
        )?;
    }
    Ok(true)
}

/// Synchronous host preview of ObjectComDrop for script ExecuteCommand.
/// Item/actor state, callbacks, delay and cached OCF are staged here in C++
/// order before the outer script call folds back into the engine.
fn preview_object_com_drop(actor: ObjectId, object: ObjectId) -> Result<bool, RuntimeError> {
    let physical = HOST_CONTEXT.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let context = borrow.as_mut()?;
        if !context.ensure_object_scope(actor) || !context.ensure_object_scope(object) {
            return None;
        }
        context.prepare_object_physical(actor, false)
    });
    let Some(physical) = physical else {
        return Ok(false);
    };
    let throw_force = crate::math::val_by_physical(400, physical.resolve().throw);
    let prepared = HOST_CONTEXT.with(|cell| {
        let borrow = cell.borrow();
        let context = borrow.as_ref()?;
        let actor_scope = context.object_scope(actor)?;
        let procedure = actor_scope.effective_action_procedure();
        let command_direction = actor_scope.command_direction();
        let actor_xdir = actor_scope.fixed_velocity().x;
        let actor_position = actor_scope.effective_position();
        let actor_shape = live_object_shape(context, actor).unwrap_or_default();
        let object_shape = live_object_shape(context, object).unwrap_or_default();
        let restore_fight_ready = actor_scope.ocf() & ocf::FIGHT_READY != 0;
        Some((
            throw_force,
            procedure,
            command_direction,
            actor_xdir,
            actor_position,
            actor_shape,
            object_shape,
            restore_fight_ready,
        ))
    });
    let Some((
        throw_force,
        procedure,
        command_direction,
        actor_xdir,
        actor_position,
        actor_shape,
        object_shape,
        restore_fight_ready,
    )) = prepared
    else {
        return Ok(false);
    };

    let com_dir_like = |sample: CommandDirection| {
        let com = command_direction.to_script_value();
        let sample = sample.to_script_value();
        com == sample || com % 8 + 1 == sample || com == sample % 8 + 1
    };
    let hangling_or_swimming = matches!(procedure, ActionProcedure::Hang | ActionProcedure::Swim);
    let mut throw_direction = 0;
    let mut right = 0;
    let mut outpos_reduction = 1;
    if procedure != ActionProcedure::Scale {
        if com_dir_like(CommandDirection::Left) {
            throw_direction = -1;
            if actor_xdir < fixed10(15) && !hangling_or_swimming {
                outpos_reduction -= 1;
            }
        }
        if com_dir_like(CommandDirection::Right) {
            throw_direction = 1;
            right = 1;
            if actor_xdir > -fixed10(15) && !hangling_or_swimming {
                outpos_reduction -= 1;
            }
        }
    }
    let edge = actor_shape
        .x
        .wrapping_add(actor_shape.width.wrapping_mul(right));
    let exit_position = Vector2::new(
        actor_position.x.wrapping_add(
            edge.wrapping_mul(i32::from(throw_direction != 0))
                .wrapping_mul(outpos_reduction),
        ),
        actor_position
            .y
            .wrapping_add(actor_shape.y)
            .wrapping_add(actor_shape.height)
            .wrapping_sub(object_shape.y.wrapping_add(object_shape.height)),
    );
    let velocity = FixedVec2::new(throw_force * throw_direction, C4Fixed::ZERO);
    let _ = exit_object_at_position_with_motion_and_calls(
        object,
        exit_position,
        velocity,
        C4Fixed::ZERO,
        true,
    )?;

    // The first SetOCF happened in Exit before Ejection. This explicit
    // ObjectComDrop update happens after Departure and before UnGrab.
    with_host_context_mut((), |context| {
        if !context.ensure_object_scope(actor) {
            return;
        }
        if let Some(scope) = context.object_scope_mut(actor) {
            scope.set_no_collect_delay(2);
        }
        let _ = refresh_live_object_ocf(context, actor);
        if let Some(scope) = context.object_scope_mut(actor) {
            scope.cached_ocf = Some(scope.ocf() & !ocf::COLLECTION);
            scope.record_no_collect_delay_assignment();
            scope.persist_final_ocf = true;
        }
    });
    let _ = preview_object_com_ungrab(actor, restore_fight_ready)?;
    Ok(true)
}

enum PreviewObjectComPutGate {
    Reject,
    Drop,
    Put,
}

/// Synchronous ObjectComPut twin for script ExecuteCommand. The Enter
/// callbacks and Put/Collection tail must be visible before the next script
/// instruction, and its result must not resolve an unrelated outer Put.
fn preview_object_com_put(
    actor: ObjectId,
    target: ObjectId,
    object: ObjectId,
) -> Result<bool, RuntimeError> {
    let gate = with_host_context(PreviewObjectComPutGate::Reject, |context| {
        let Some(actor_state) = context.get_world_object(actor) else {
            return PreviewObjectComPutGate::Reject;
        };
        let Some(target_state) = context.get_world_object(target) else {
            return PreviewObjectComPutGate::Reject;
        };
        let Some(_object_state) = context.get_world_object(object) else {
            return PreviewObjectComPutGate::Reject;
        };
        let target_metadata = context
            .object_effective_definition_id(target)
            .and_then(|id| context.definition_metadata(&id));
        let actor_container = actor_state.container();
        let grab_put = target_metadata
            .is_some_and(|metadata| metadata.grab_put_get & crate::GRAB_PUT_GET_PUT != 0);
        if actor_container != Some(target) && !grab_put {
            let down_double = context
                .player_state(actor_state.owner)
                .is_some_and(|player| player.control.last_com_down_double != 0);
            return if down_double {
                PreviewObjectComPutGate::Drop
            } else {
                PreviewObjectComPutGate::Reject
            };
        }
        let target_ocf = context
            .object_scope(target)
            .map(ObjectScopeContext::ocf)
            .unwrap_or_else(|| target_state.ocf());
        if target_ocf & ocf::FULL_CON == 0 {
            return PreviewObjectComPutGate::Reject;
        }
        let collection_limit = target_metadata.map_or(0, |metadata| metadata.collection_limit);
        let contents_count = target_state
            .contents()
            .iter()
            .filter(|object_id| {
                context
                    .get_world_object(**object_id)
                    .is_some_and(|object| object.is_present())
            })
            .count();
        if crate::collection_limit_reached(collection_limit, contents_count) {
            return PreviewObjectComPutGate::Reject;
        }
        PreviewObjectComPutGate::Put
    });

    match gate {
        PreviewObjectComPutGate::Reject => Ok(false),
        PreviewObjectComPutGate::Drop => preview_object_com_drop(actor, object),
        PreviewObjectComPutGate::Put => {
            if !enter_object_live_with_reject_collect(object, target)? {
                return Ok(false);
            }
            let _ = call_object_own_fail_safe(actor, "Put", &[]);
            let _ = call_object_own_fail_safe(
                target,
                "Collection",
                &[object_reference_value(object), Value::Bool(true)],
            );
            Ok(true)
        }
    }
}

pub(crate) struct PreviewInternalObjectMenuSource;

impl crate::direct_com::InternalObjectMenuSource for PreviewInternalObjectMenuSource {
    type Error = RuntimeError;

    fn current_menu(&self, object: ObjectId) -> Option<crate::ObjectMenuState> {
        with_host_context(None, |context| context.object_menu(object))
    }

    fn object(&self, object: ObjectId) -> Option<crate::direct_com::InternalObjectMenuObject> {
        HOST_CONTEXT.with(|cell| {
            let borrow = cell.borrow();
            let context = borrow.as_ref()?;
            let object_state = context.get_world_object(object)?;
            let definition_id = object_state.definition_id().to_string();
            let name = context
                .object_custom_name(object)
                .or_else(|| {
                    context
                        .object_scope(object)
                        .and_then(ObjectScopeContext::info_core)
                        .map(|info| info.name.clone())
                })
                .or_else(|| {
                    context
                        .world
                        .crew_infos
                        .get(&object)
                        .map(|info| info.name.clone())
                })
                .or_else(|| {
                    context
                        .definition_metadata(&definition_id)
                        .map(|definition| definition.name.clone())
                })
                .unwrap_or_else(|| definition_id.clone());
            let contents_link_generation = context
                .object_scope(object)
                .map(|scope| scope.current_contents_link_generation)
                .or_else(|| {
                    object_state
                        .full_state()
                        .map(|state| state.contents_link_generation)
                })
                .unwrap_or(0);
            Some(crate::direct_com::InternalObjectMenuObject {
                id: object,
                contents_link_generation,
                definition_id,
                name,
                category: object_state.category(),
                ocf: object_state.ocf(),
                contents: object_state.contents().to_vec(),
                active: object_state.status() != ObjectStatus::Deleted,
            })
        })
    }

    fn definition(
        &self,
        definition: &str,
    ) -> Option<crate::direct_com::InternalObjectMenuDefinition> {
        HOST_CONTEXT.with(|cell| {
            let borrow = cell.borrow();
            let context = borrow.as_ref()?;
            let metadata = context.definition_metadata(definition)?;
            Some(crate::direct_com::InternalObjectMenuDefinition {
                description: context
                    .world
                    .definition_description(definition)
                    .unwrap_or_default()
                    .to_string(),
                no_get: metadata.fire.no_get,
                collection_limit: metadata.collection_limit,
            })
        })
    }

    fn object_menu_picture_snapshot(
        &self,
        object: ObjectId,
    ) -> Option<crate::ObjectMenuPictureSnapshot> {
        with_host_context(None, |context| {
            context.object_menu_picture_snapshot(object, false, 35)
        })
    }

    fn can_concat_picture_with(&self, object: ObjectId, other: ObjectId) -> bool {
        HOST_CONTEXT.with(|cell| {
            cell.borrow()
                .as_ref()
                .is_some_and(|context| context.object_can_concat_picture_with(object, other))
        })
    }

    fn activate_value(
        &mut self,
        command_object: ObjectId,
        object: ObjectId,
        container: ObjectId,
        menu_before_value: &crate::ObjectMenuState,
    ) -> Result<i32, Self::Error> {
        HOST_CONTEXT.with(|cell| {
            if let Some(context) = cell.borrow_mut().as_mut() {
                context.set_object_menu(command_object, Some(menu_before_value.clone()));
            }
        });
        let value = get_value(&[
            object_reference_value(object),
            Value::Int(0),
            object_reference_value(container),
            Value::Int(OWNER_NONE),
        ]);
        match value {
            Ok(value) => Ok(value.as_c4_int().unwrap_or(0)),
            Err(error) => {
                tracing::error!(%error, "script error in Activate-menu GetValue; continuing");
                Ok(0)
            }
        }
    }

    fn reject_collection(
        &mut self,
        command_object: ObjectId,
        object: ObjectId,
        menu_before_call: &crate::ObjectMenuState,
    ) -> Result<bool, Self::Error> {
        HOST_CONTEXT.with(|cell| {
            if let Some(context) = cell.borrow_mut().as_mut() {
                context.set_object_menu(command_object, Some(menu_before_call.clone()));
            }
        });
        let definition_id = HOST_CONTEXT.with(|cell| {
            cell.borrow()
                .as_ref()
                .and_then(|context| context.get_world_object(object))
                .map(|object| object.definition_id().to_string())
        });
        let Some(definition_id) = definition_id else {
            return Ok(false);
        };
        Ok(call_object_own_fail_safe(
            command_object,
            "RejectCollect",
            &[Value::C4Id(definition_id), object_reference_value(object)],
        )
        .as_bool())
    }
}

enum PreviewObjectComPutTakeOutcome {
    Finished,
    NeedsGet(ObjectId),
}

/// Synchronous script-host half of ObjectComPutTake. Item transfers run
/// entirely in the live VM scope. Internal menu construction continues
/// through the established menu-request fold, while command completion is
/// still resolved before ExecuteCommand returns.
fn preview_object_com_put_take(
    actor: ObjectId,
    target: ObjectId,
    requested_item: Option<ObjectId>,
) -> Result<PreviewObjectComPutTakeOutcome, RuntimeError> {
    let prepared = HOST_CONTEXT.with(|cell| {
        let borrow = cell.borrow();
        let context = borrow.as_ref()?;
        let actor_state = context.get_world_object(actor)?;
        let _target_state = context.get_world_object(target)?;
        let is_resolved = |item| context.get_world_object(item).is_some();
        let (item, needs_get) = match requested_item {
            Some(item) if actor_state.contents().contains(&item) && is_resolved(item) => {
                (Some(item), None)
            }
            Some(item) if is_resolved(item) => (None, Some(item)),
            Some(_) | None => (
                actor_state.contents().iter().copied().find(|item| {
                    context
                        .get_world_object(*item)
                        .is_some_and(|object| object.is_present())
                }),
                None,
            ),
        };
        let grab_get = context
            .object_effective_definition_id(target)
            .and_then(|id| context.definition_metadata(&id))
            .is_some_and(|metadata| metadata.grab_put_get & crate::GRAB_PUT_GET_GET != 0);
        Some((
            item,
            actor_state.container(),
            actor_state.controller(),
            actor_state.owner,
            grab_get,
            needs_get,
        ))
    });
    let Some((item, container, controller, owner, grab_get, needs_get)) = prepared else {
        return Ok(PreviewObjectComPutTakeOutcome::Finished);
    };
    if let Some(item) = needs_get {
        return Ok(PreviewObjectComPutTakeOutcome::NeedsGet(item));
    };
    if let Some(item) = item {
        let _ = preview_object_com_put(actor, target, item)?;
        return Ok(PreviewObjectComPutTakeOutcome::Finished);
    }

    let request = if container == Some(target) {
        Some(MenuRequest {
            crew_id: actor,
            owner: controller,
            kind: MenuRequestKind::Activate,
        })
    } else if grab_get {
        Some(MenuRequest {
            crew_id: actor,
            owner,
            kind: MenuRequestKind::Get { container: target },
        })
    } else {
        None
    };
    if let Some(request) = request {
        let _ = preview_prepare_put_take_menu(request);
    }
    Ok(PreviewObjectComPutTakeOutcome::Finished)
}

fn preview_object_com_stop(actor: ObjectId) -> Result<(), RuntimeError> {
    let _ = native_set_action_by_name(actor, "Idle")?;
    HOST_CONTEXT.with(|cell| {
        if let Some(scope) = cell
            .borrow_mut()
            .as_mut()
            .and_then(|context| context.object_scope_mut(actor))
        {
            scope.set_command_direction(CommandDirection::Stop);
        }
    });
    if native_set_action_by_name(actor, "Walk")? {
        HOST_CONTEXT.with(|cell| {
            if let Some(scope) = cell
                .borrow_mut()
                .as_mut()
                .and_then(|context| context.object_scope_mut(actor))
            {
                scope.set_fixed_velocity(FixedVec2::ZERO);
            }
        });
    }
    Ok(())
}

/// Synchronous host twin of ObjectComDig for script ExecuteCommand. The
/// physical gate, callbackful SetAction, localized GameMsgObject failure and
/// Action.Data reset all complete before the caller's next script instruction.
fn preview_object_com_dig(actor: ObjectId) -> Result<bool, RuntimeError> {
    let physical = HOST_CONTEXT.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let context = borrow.as_mut()?;
        if !context.ensure_object_scope(actor) {
            return None;
        }
        context.prepare_object_physical(actor, false)
    });
    let can_dig = physical
        .map(PhysicalResolution::resolve)
        .map(|physical| physical.can_dig != 0);
    let succeeded = can_dig == Some(true) && native_set_action_by_name(actor, "Dig")?;
    if succeeded {
        HOST_CONTEXT.with(|cell| {
            if let Some(scope) = cell
                .borrow_mut()
                .as_mut()
                .and_then(|context| context.object_scope_mut(actor))
            {
                scope.reset_action_data();
            }
        });
        return Ok(true);
    }

    with_host_context_mut((), |context| {
        let name = context.object_effective_name(actor).unwrap_or_default();
        let text = if context
            .get_world_object(actor)
            .is_some_and(|object| object.is_present())
        {
            context
                .world
                .object_no_dig_resource_string
                .replacen("%s", &name, 1)
        } else {
            // An empty non-Multiple target message performs the native
            // replacement clear without leaving a message behind.
            String::new()
        };
        context.register_message(MessageCommand::Add(MessageSpec::target(text, actor)));
    });
    Ok(false)
}

pub(crate) type PreparedCommandRuntimeData = (
    CommandObjectSnapshots,
    HashMap<i32, CommandPlayerSnapshot>,
    HashMap<DefinitionId, CommandDefinitionSnapshot>,
    TransferZoneTable,
);

#[derive(Default)]
pub(crate) struct CommandPreviewOutcome {
    pub(crate) finished: Option<CommandView>,
    pub(crate) buy_attempts: Vec<(ObjectId, ObjectId, DefinitionId, i32, i32, i32)>,
    pub(crate) sell_attempts: Vec<(ObjectId, ObjectId, DefinitionId, Option<ObjectId>, i32)>,
    pub(crate) grab_attempts: Vec<(ObjectId, ObjectId)>,
    pub(crate) put_attempts: Vec<(ObjectId, ObjectId, ObjectId, bool, u64)>,
    pub(crate) drop_attempts: Vec<(ObjectId, ObjectId, u64)>,
    pub(crate) ungrab_attempts: Vec<(ObjectId, u64)>,
    pub(crate) put_take_attempts: Vec<(ObjectId, ObjectId, Option<ObjectId>, CommandId, u64)>,
    pub(crate) throw_attempts: Vec<(ObjectId, ObjectId, bool, u64)>,
    pub(crate) throw_preludes: Vec<CommandEvent>,
    pub(crate) entrance_attempts: Vec<(ObjectId, ObjectId, Option<CallResultAction>, u64)>,
    pub(crate) control_transfers: Vec<(ObjectId, ObjectId, Value, i32, u64)>,
    pub(crate) call_attempts: Vec<CommandEvent>,
    pub(crate) exit_attempts: Vec<CommandEvent>,
    pub(crate) failure_feedback: Vec<(ObjectId, CommandFailureFeedback)>,
    pub(crate) move_to_stops: Vec<ObjectId>,
    pub(crate) build_stops: Vec<(ObjectId, u64)>,
    pub(crate) build_actions: Vec<(ObjectId, ObjectId, bool)>,
    pub(crate) dig_attempts: Vec<(ObjectId, bool, Option<CommandDirection>, u64)>,
    pub(crate) physical_reads: Vec<(ObjectId, u8, u64)>,
}

impl CommandPreviewOutcome {
    fn append(&mut self, mut other: Self) {
        if other.finished.is_some() {
            self.finished = other.finished.take();
        }
        self.buy_attempts.append(&mut other.buy_attempts);
        self.sell_attempts.append(&mut other.sell_attempts);
        self.grab_attempts.append(&mut other.grab_attempts);
        self.put_attempts.append(&mut other.put_attempts);
        self.drop_attempts.append(&mut other.drop_attempts);
        self.ungrab_attempts.append(&mut other.ungrab_attempts);
        self.put_take_attempts.append(&mut other.put_take_attempts);
        self.throw_attempts.append(&mut other.throw_attempts);
        self.throw_preludes.append(&mut other.throw_preludes);
        self.entrance_attempts.append(&mut other.entrance_attempts);
        self.control_transfers.append(&mut other.control_transfers);
        self.call_attempts.append(&mut other.call_attempts);
        self.exit_attempts.append(&mut other.exit_attempts);
        self.failure_feedback.append(&mut other.failure_feedback);
        self.move_to_stops.append(&mut other.move_to_stops);
        self.build_stops.append(&mut other.build_stops);
        self.build_actions.append(&mut other.build_actions);
        self.dig_attempts.append(&mut other.dig_attempts);
        self.physical_reads.append(&mut other.physical_reads);
    }

    fn had_live_attempt(&self) -> bool {
        !self.buy_attempts.is_empty()
            || !self.sell_attempts.is_empty()
            || !self.grab_attempts.is_empty()
            || !self.put_attempts.is_empty()
            || !self.drop_attempts.is_empty()
            || !self.ungrab_attempts.is_empty()
            || !self.put_take_attempts.is_empty()
            || !self.throw_attempts.is_empty()
            || !self.throw_preludes.is_empty()
            || !self.entrance_attempts.is_empty()
            || !self.control_transfers.is_empty()
            || !self.call_attempts.is_empty()
            || !self.exit_attempts.is_empty()
            || !self.failure_feedback.is_empty()
            || !self.move_to_stops.is_empty()
            || !self.build_stops.is_empty()
            || !self.build_actions.is_empty()
            || !self.dig_attempts.is_empty()
    }
}

/// Structural command snapshots never read target physicals. Resolve at most
/// the executing actor. A missing FairCrew projection is deliberately left
/// unresolved and marked on that actor snapshot; the exact command branch
/// which reaches native GetPhysical will suspend and request the live fill.
fn prepare_command_runtime_data(
    physical_actor: Option<ObjectId>,
) -> Option<PreparedCommandRuntimeData> {
    let resolution = HOST_CONTEXT.with(|cell| {
        let borrow = cell.borrow();
        let context = borrow.as_ref()?;
        Some(physical_actor.and_then(|id| {
            context
                .prepare_object_physical(id, false)
                .map(|plan| (id, plan))
        }))
    })?;
    let (physicals, deferred_actor) = match resolution {
        Some((id, plan)) if plan.needs_fair_crew_fill() => (HashMap::new(), Some(id)),
        Some((id, plan)) => (HashMap::from([(id, plan.resolve())]), None),
        None => (HashMap::new(), None),
    };
    HOST_CONTEXT.with(|cell| {
        cell.borrow()
            .as_ref()
            .map(|context| context.command_runtime_data(&physicals, deferred_actor))
    })
}

/// Rebuild command snapshots after a physical hook using the exact value
/// returned by its final GetPhysical call. The actor is no longer deferred:
/// this continuation must retain that pointer value even if the hook changed
/// other live state while the read was in flight.
fn prepare_command_runtime_data_with_physical(
    actor: ObjectId,
    physical: PhysicalInfo,
) -> Option<PreparedCommandRuntimeData> {
    let physicals = HashMap::from([(actor, physical)]);
    HOST_CONTEXT.with(|cell| {
        cell.borrow()
            .as_ref()
            .map(|context| context.command_runtime_data(&physicals, None))
    })
}

/// Run one pending live-command continuation for `actor` against a fresh
/// runtime snapshot, then fold the outcome back the way every command seam
/// does: the stack counts as replaced, a staged update goes through the host
/// context, and the scope's command count is resynced.
///
/// `snapshot_actor` is the argument each seam already passed to
/// `prepare_command_runtime_data` — `Some(actor)` when the continuation needs
/// the actor's own post-callback state, `None` when it must not.
fn preview_pending_command<F>(
    actor: ObjectId,
    snapshot_actor: Option<ObjectId>,
    step: F,
) -> Vec<CommandEvent>
where
    F: FnOnce(
        &mut ObjectScopeContext,
        &CommandRuntimeContext<'_>,
    ) -> Option<crate::CommandStepResult>,
{
    let Some((objects, players, definitions, transfers)) =
        prepare_command_runtime_data(snapshot_actor)
    else {
        return Vec::new();
    };
    with_host_context_mut(Vec::new(), |context| {
        let Some(object_snapshot) = objects.get(&actor) else {
            return Vec::new();
        };
        let landscape = context.world.landscape_shared();
        let runtime = CommandRuntimeContext {
            rng: None,
            frame: context.world.frame,
            position: object_snapshot.position,
            landscape: landscape.as_deref(),
            object: object_snapshot,
            objects: &objects,
            players: &players,
            definitions: &definitions,
            structures_need_energy: context.world.structures_need_energy,
            base_buy_enabled: context.world.base_buy_enabled,
            base_sell_enabled: context.world.base_sell_enabled,
            transfer_zones: &transfers,
        };
        let Some(mut result) = context
            .object_scope_mut(actor)
            .and_then(|scope| step(scope, &runtime))
        else {
            return Vec::new();
        };
        if let Some(scope) = context.object_scope_mut(actor) {
            scope.command_stack_replaced = true;
        }
        if let Some(update) = result.update.take() {
            context.stage_object_command_update(actor, update);
        }
        if let Some(scope) = context.object_scope_mut(actor) {
            scope.command_count = scope.live_commands.len();
        }
        result.events
    })
}

/// The throw/put prelude gravity: `C4Physical` gravity scaled to the
/// command machine's fifth-of-a-pixel step.
fn preview_command_gravity() -> C4Fixed {
    PHYSICS_CONTEXT.with(|cell| {
        cell.borrow()
            .as_ref()
            .map(|context| fixed100(context.gravity()) / 5)
            .unwrap_or_else(|| PhysicsSettings::default().gravity_as_c4fixed())
    })
}

fn preview_command_event_instance_id(
    actor: ObjectId,
    kind: CommandEventInstanceKind,
    supplied: u64,
) -> u64 {
    HOST_CONTEXT.with(|cell| {
        cell.borrow()
            .as_ref()
            .and_then(|context| context.object_scope(actor))
            .map(|scope| {
                scope
                    .live_commands
                    .resolve_event_instance_id(kind, supplied)
            })
            .unwrap_or(supplied)
    })
}

/// Execute callbackful GetPhysical reads without holding HOST_CONTEXT, then
/// resume the retained native command against a fresh structural snapshot.
/// The command identity is bound before the first hook because that hook may
/// replace the visible stack which originally emitted the event.
fn preview_resume_command_after_physical(
    actor: ObjectId,
    reads: u8,
    supplied_instance_id: u64,
    rng: Option<&RefCell<LcgRng>>,
) -> Option<(Vec<CommandEvent>, Option<CommandView>)> {
    let command_instance_id = preview_command_event_instance_id(
        actor,
        CommandEventInstanceKind::Physical,
        supplied_instance_id,
    );

    let mut physical = None;
    for _ in 0..reads {
        let Some(resolved) = resolve_object_physical(actor, false) else {
            break;
        };
        physical = Some(resolved);
    }
    let physical = physical?;
    let command_data = prepare_command_runtime_data_with_physical(actor, physical)?;
    HOST_CONTEXT.with(|cell| {
        cell.borrow_mut().as_mut().and_then(|context| {
            context.execute_pending_command_physical_preview(
                actor,
                command_instance_id,
                physical,
                rng,
                &command_data,
            )
        })
    })
}

/// Script-level ExecuteCommand twin of CommandEvent::ObjectComStopMoveTo.
/// ObjectComStop callbacks and the retained command continuation must both
/// complete before ExecuteCommand returns to its caller.
fn preview_move_to_stop(actor: ObjectId) -> Result<Vec<CommandEvent>, RuntimeError> {
    preview_object_com_stop(actor)?;

    Ok(preview_pending_command(
        actor,
        Some(actor),
        |scope, runtime| scope.live_commands.execute_pending_move_to_stop(runtime),
    ))
}

/// Continue the exact MoveTo after FlightControl's live Fly transition and
/// callbacks. A WALK origin runs JumpControl against fresh post-callback
/// state; DFA_FLIGHT only consumes the boundary, without another interval.
fn preview_resume_move_to_after_flight(
    actor: ObjectId,
    command_instance_id: u64,
) -> Vec<CommandEvent> {
    preview_pending_command(actor, None, |scope, runtime| {
        scope
            .live_commands
            .execute_pending_move_to_flight(runtime, command_instance_id)
    })
}

/// Host-preview twin of `ObjectComBuild` (C4ObjectCom.cpp:690-697).
fn preview_object_com_build(
    actor: ObjectId,
    target: ObjectId,
    stop_first: bool,
) -> Result<(), RuntimeError> {
    if stop_first {
        preview_object_com_stop(actor)?;
    }
    if !preview_object_is_present(target) {
        return Ok(());
    }
    let can_build = HOST_CONTEXT.with(|cell| {
        cell.borrow()
            .as_ref()
            .and_then(|context| context.object_scope(actor))
            .is_some_and(|scope| {
                scope.action_library.is_idle_entry(
                    scope.effective_action_name(),
                    scope.effective_action_index(),
                ) || scope.effective_action_procedure() == ActionProcedure::Walk
            })
    });
    if can_build {
        let _ = native_set_action_by_name_with_target(actor, "Build", Some(target))?;
    }
    Ok(())
}

/// Resolve ObjectComPut entirely inside the live ExecuteCommand scope. The
/// callback outcome's command-stack Restore is applied after deferred events;
/// deferring Put would therefore let a recursively executed replacement's
/// result resolve the suspended outer Put (and vice versa).
fn preview_resolve_put_attempt(
    actor_id: ObjectId,
    target_id: ObjectId,
    object_id: ObjectId,
    ungrab_on_success: bool,
    command_instance_id: u64,
) -> Result<(), RuntimeError> {
    let command_instance_id = HOST_CONTEXT.with(|cell| {
        cell.borrow()
            .as_ref()
            .and_then(|context| context.object_scope(actor_id))
            .map_or(command_instance_id, |scope| {
                scope
                    .live_commands
                    .resolve_event_instance_id(CommandEventInstanceKind::Put, command_instance_id)
            })
    });
    let succeeded = preview_object_com_put(actor_id, target_id, object_id)?;
    let feedback = HOST_CONTEXT.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let context = borrow.as_mut()?;
        let scope = context.object_scope_mut(actor_id)?;
        if succeeded && ungrab_on_success {
            let _ = scope
                .live_commands
                .push_front(CommandRequest::new(CommandId::UnGrab));
        }
        let feedback = scope
            .live_commands
            .resolve_put_attempt(command_instance_id, succeeded);
        scope.command_stack_replaced = true;
        scope.command_count = scope.live_commands.len();
        Some(feedback)
    });
    if let Some(feedback) = feedback.flatten() {
        preview_command_failure_feedback(actor_id, feedback)?;
    }
    Ok(())
}

/// Script-level same-Execute continuation for Build's Dig stop.
fn preview_build_stop(
    actor: ObjectId,
    command_instance_id: u64,
) -> Result<Vec<CommandEvent>, RuntimeError> {
    let command_instance_id = preview_command_event_instance_id(
        actor,
        CommandEventInstanceKind::Prelude(CommandId::Build),
        command_instance_id,
    );
    preview_object_com_stop(actor)?;

    let Some((objects, players, definitions, transfers)) =
        prepare_command_runtime_data(Some(actor))
    else {
        return Ok(Vec::new());
    };
    let events = with_host_context_mut(Vec::new(), |context| {
        let Some(object_snapshot) = objects.get(&actor) else {
            return Vec::new();
        };
        let landscape = context.world.landscape_shared();
        let runtime = CommandRuntimeContext {
            rng: None,
            frame: context.world.frame,
            position: object_snapshot.position,
            landscape: landscape.as_deref(),
            object: object_snapshot,
            objects: &objects,
            players: &players,
            definitions: &definitions,
            structures_need_energy: context.world.structures_need_energy,
            base_buy_enabled: context.world.base_buy_enabled,
            base_sell_enabled: context.world.base_sell_enabled,
            transfer_zones: &transfers,
        };
        let Some(scope) = context.object_scope_mut(actor) else {
            return Vec::new();
        };
        let Some(mut result) = scope
            .live_commands
            .execute_pending_build_stop(&runtime, command_instance_id)
        else {
            return Vec::new();
        };
        scope.command_stack_replaced = true;
        if let Some(update) = result.update.take() {
            scope.stage_command_update(update);
        }
        scope.command_count = scope.live_commands.len();
        result.events
    });

    Ok(events)
}

fn preview_resolve_put_take_attempt(
    actor_id: ObjectId,
    target_id: ObjectId,
    requested_item: Option<ObjectId>,
    command: CommandId,
    command_instance_id: u64,
) -> Result<(), RuntimeError> {
    let outcome = preview_object_com_put_take(actor_id, target_id, requested_item)?;
    with_host_context_mut((), |context| {
        let Some(scope) = context.object_scope_mut(actor_id) else {
            return;
        };
        match outcome {
            PreviewObjectComPutTakeOutcome::Finished => match command {
                CommandId::Throw => {
                    scope
                        .live_commands
                        .finish_pending_throw(command_instance_id);
                }
                CommandId::Drop => {
                    scope.live_commands.finish_pending_drop(command_instance_id);
                }
                _ => debug_assert!(false, "ObjectComPutTake must come from Throw/Drop"),
            },
            PreviewObjectComPutTakeOutcome::NeedsGet(item) => {
                if scope
                    .live_commands
                    .clear_pending_put_take(command, command_instance_id)
                {
                    let _ = scope.live_commands.push_front(
                        CommandRequest::new(CommandId::Get)
                            .with_target(Some(item))
                            .with_update_interval(40)
                            .with_mode(CommandMode::SilentSub),
                    );
                }
            }
        }
        scope.command_stack_replaced = true;
        scope.command_count = scope.live_commands.len();
    });
    Ok(())
}

fn preview_resolve_throw_attempt(
    actor_id: ObjectId,
    object_id: ObjectId,
    complete_on_success: bool,
    command_instance_id: u64,
) -> Result<(), RuntimeError> {
    let success = preview_object_action_throw(actor_id, object_id)?;
    if success || !complete_on_success {
        HOST_CONTEXT.with(|cell| {
            let mut borrow = cell.borrow_mut();
            let Some(scope) = borrow
                .as_mut()
                .and_then(|context| context.object_scope_mut(actor_id))
            else {
                return;
            };
            scope
                .live_commands
                .finish_command_instance(CommandId::Throw, command_instance_id);
            scope.command_stack_replaced = true;
            scope.command_count = scope.live_commands.len();
        });
    }
    Ok(())
}

fn preview_resume_throw_after_prelude(
    actor: ObjectId,
    command_instance_id: u64,
) -> Vec<CommandEvent> {
    preview_pending_command(actor, Some(actor), |scope, runtime| {
        scope.live_commands.execute_pending_throw_prelude(
            runtime,
            preview_command_gravity(),
            command_instance_id,
        )
    })
}

fn preview_resume_put_after_stop(actor: ObjectId, command_instance_id: u64) -> Vec<CommandEvent> {
    preview_pending_command(actor, Some(actor), |scope, runtime| {
        scope.live_commands.execute_pending_put_stop(
            runtime,
            preview_command_gravity(),
            command_instance_id,
        )
    })
}

fn preview_resume_construct(
    actor: ObjectId,
    command_instance_id: u64,
    script_result: Option<AcquireScriptResult>,
) -> Vec<CommandEvent> {
    preview_pending_command(actor, None, |scope, runtime| match script_result {
        Some(script_result) => scope.live_commands.execute_pending_construct_script(
            runtime,
            command_instance_id,
            script_result,
        ),
        None => scope
            .live_commands
            .execute_pending_construct_stop(runtime, command_instance_id),
    })
}

/// Resume Construct after the native Con=1 creation attempt and the retained
/// conkit's AssignRemoval. Native performs Finish(true) and pushes Build
/// before returning even when NewObject returned null.
fn preview_resume_construct_spawn(
    actor: ObjectId,
    command_instance_id: u64,
    construction_id: Option<ObjectId>,
) -> Vec<CommandEvent> {
    preview_pending_command(actor, None, |scope, runtime| {
        scope.live_commands.execute_pending_construct_spawn(
            runtime,
            command_instance_id,
            construction_id,
        )
    })
}

/// Live host twin of C4Game::CreateObjectConstruction from Construct. The
/// command identity is bound before terrain, Construction, or kit callbacks
/// can clear the visible stack; the exact detached command is then resumed.
fn preview_spawn_construction(
    actor: ObjectId,
    definition_id: String,
    owner: i32,
    position: Vector2,
    kit_id: ObjectId,
    supplied_instance_id: u64,
) -> Result<Vec<CommandEvent>, RuntimeError> {
    let command_instance_id = preview_command_event_instance_id(
        actor,
        CommandEventInstanceKind::ConstructSpawn,
        supplied_instance_id,
    );

    with_host_context_mut((), |context| {
        let Some(metadata) = context.definition_metadata(&definition_id).cloned() else {
            return;
        };
        let (width, height) = metadata
            .shape
            .map(|shape| (shape.width, shape.height))
            .unwrap_or_default();
        context.prepare_construction_terrain(
            position.x,
            position.y,
            width,
            height,
            metadata.basement,
        );
    });

    let construction_id = create_native_object(NativeObjectCreation {
        definition: definition_id,
        creator: None,
        owner,
        controller: owner,
        construction: 1,
        position,
        rotation: 0,
        velocity: FixedVec2::ZERO,
        rotation_velocity: C4Fixed::ZERO,
    })?;

    // Construct consumes the selected kit after NewObject returns, using
    // the complete callbackful AssignRemoval(false) path.
    let _ = assign_removal_live(kit_id, false)?;
    Ok(preview_resume_construct_spawn(
        actor,
        command_instance_id,
        construction_id,
    ))
}

fn preview_control_command_construction(
    caller: ObjectId,
    target: Option<ObjectId>,
    site: Vector2,
    target2: Option<ObjectId>,
    definition_id: &str,
) -> AcquireScriptResult {
    let args = [
        target.map(object_reference_value).unwrap_or(Value::Nil),
        Value::Int(site.x),
        Value::Int(site.y),
        target2.map(object_reference_value).unwrap_or(Value::Nil),
        definition_id_to_c4id(definition_id)
            .map(Value::Int)
            .unwrap_or_else(|| Value::String(definition_id.to_string().into())),
    ];
    let value = match call_world_object_function(caller, "~ControlCommandConstruction", &args) {
        Some(Ok(value)) => value,
        Some(Err(error)) => {
            tracing::error!(
                %error,
                "ControlCommandConstruction error; continuing like the C++ fail-safe exec"
            );
            log_runtime_call_frames("", error.call_frames());
            Value::Nil
        }
        None => Value::Nil,
    };
    let code = match value {
        Value::Int(code) => Some(code),
        Value::Bool(flag) => Some(if flag { 1 } else { 0 }),
        _ => None,
    };
    code.and_then(AcquireScriptResult::from_code)
        .unwrap_or(AcquireScriptResult::Continue)
}

fn preview_resume_drop_after_prelude(
    actor: ObjectId,
    command_instance_id: u64,
) -> Vec<CommandEvent> {
    preview_pending_command(actor, None, |scope, runtime| {
        scope
            .live_commands
            .execute_pending_drop_prelude(runtime, command_instance_id)
    })
}

fn preview_resume_exit_after_stop(actor: ObjectId, command_instance_id: u64) -> Vec<CommandEvent> {
    preview_pending_command(actor, None, |scope, runtime| {
        scope
            .live_commands
            .execute_pending_exit_stop(runtime, command_instance_id)
    })
}

fn preview_defer_command_event(actor: ObjectId, event: CommandEvent) {
    with_host_context_mut((), |context| {
        context.pending_command_events.push(event.clone());
        if let Some(scope) = context.object_scope_mut(actor) {
            scope
                .queued_commands
                .push(QueuedCommand::immediate(ObjectUpdate::default()).with_events(vec![event]));
        }
    });
}

/// Drain every callback continuation produced inside script ExecuteCommand.
/// In particular, ResolveCommandPhysical is not an engine-queue event here:
/// native is still inside the same C4Command::Execute invocation, so the
/// FairCrew hook and retained body must finish before this VM call returns.
fn preview_dispatch_command_continuation_events(
    actor: ObjectId,
    initial: impl IntoIterator<Item = CommandEvent>,
) -> Result<(), RuntimeError> {
    let mut events = initial.into_iter().collect::<VecDeque<_>>();
    while let Some(event) = events.pop_front() {
        let resumed = match event {
            CommandEvent::ResolveCommandPhysical {
                object_id,
                reads,
                command_instance_id,
            } => {
                let random = RANDOM_CONTEXT.with(|cell| cell.borrow().clone());
                preview_resume_command_after_physical(
                    object_id,
                    reads,
                    command_instance_id,
                    random.as_ref().map(|rng| &rng.rng),
                )
                .map(|(events, _finished)| events)
                .unwrap_or_default()
            }
            CommandEvent::MoveToFlightControlTakeoff {
                object_id,
                command_instance_id,
            } => {
                let command_instance_id = preview_command_event_instance_id(
                    object_id,
                    CommandEventInstanceKind::MoveToFlightControl,
                    command_instance_id,
                );
                // FlightControl ignores SetActionByName's return value and
                // returns to the retained procedure-specific tail.
                let _ = native_set_action_by_name(object_id, "Fly")?;
                preview_resume_move_to_after_flight(object_id, command_instance_id)
            }
            CommandEvent::ObjectComStopMoveTo { object_id } => preview_move_to_stop(object_id)?,
            CommandEvent::ObjectComStopBuild {
                object_id,
                command_instance_id,
            } => preview_build_stop(object_id, command_instance_id)?,
            CommandEvent::ObjectComStopChop { object_id } => {
                preview_object_com_stop(object_id)?;
                Vec::new()
            }
            CommandEvent::ObjectComStopConstruct {
                object_id,
                command_instance_id,
            } => {
                let command_instance_id = preview_command_event_instance_id(
                    object_id,
                    CommandEventInstanceKind::Prelude(CommandId::Construct),
                    command_instance_id,
                );
                preview_object_com_stop(object_id)?;
                preview_resume_construct(object_id, command_instance_id, None)
            }
            CommandEvent::ObjectComStopPut {
                object_id,
                command_instance_id,
            } => {
                let command_instance_id = preview_command_event_instance_id(
                    object_id,
                    CommandEventInstanceKind::Prelude(CommandId::Put),
                    command_instance_id,
                );
                preview_object_com_stop(object_id)?;
                preview_resume_put_after_stop(object_id, command_instance_id)
            }
            CommandEvent::ControlCommandConstruction {
                caller,
                target,
                site,
                target2,
                definition_id,
                command_instance_id,
            } => {
                let command_instance_id = preview_command_event_instance_id(
                    caller,
                    CommandEventInstanceKind::Script(CommandId::Construct),
                    command_instance_id,
                );
                let script_result = preview_control_command_construction(
                    caller,
                    target,
                    site,
                    target2,
                    &definition_id,
                );
                if preview_object_is_present(caller) {
                    preview_resume_construct(caller, command_instance_id, Some(script_result))
                } else {
                    Vec::new()
                }
            }
            CommandEvent::SpawnConstruction {
                actor_id,
                definition_id,
                owner,
                position,
                kit_id,
                command_instance_id,
            } => preview_spawn_construction(
                actor_id,
                definition_id,
                owner,
                position,
                kit_id,
                command_instance_id,
            )?,
            CommandEvent::ObjectComBuild {
                object_id,
                target_id,
                stop_first,
            } => {
                preview_object_com_build(object_id, target_id, stop_first)?;
                Vec::new()
            }
            CommandEvent::ObjectComStopThrow {
                object_id,
                command_instance_id,
            } => {
                let command_instance_id = preview_command_event_instance_id(
                    object_id,
                    CommandEventInstanceKind::Prelude(CommandId::Throw),
                    command_instance_id,
                );
                preview_object_com_stop(object_id)?;
                preview_resume_throw_after_prelude(object_id, command_instance_id)
            }
            CommandEvent::ObjectComSetDirThrow {
                object_id,
                direction,
                command_instance_id,
            } => {
                let command_instance_id = preview_command_event_instance_id(
                    object_id,
                    CommandEventInstanceKind::Prelude(CommandId::Throw),
                    command_instance_id,
                );
                native_set_dir(object_id, direction)?;
                preview_resume_throw_after_prelude(object_id, command_instance_id)
            }
            CommandEvent::ObjectComStopDrop {
                object_id,
                command_instance_id,
            } => {
                let command_instance_id = preview_command_event_instance_id(
                    object_id,
                    CommandEventInstanceKind::Prelude(CommandId::Drop),
                    command_instance_id,
                );
                preview_object_com_stop(object_id)?;
                preview_resume_drop_after_prelude(object_id, command_instance_id)
            }
            CommandEvent::ObjectComStopExit {
                object_id,
                command_instance_id,
            } => {
                let command_instance_id = preview_command_event_instance_id(
                    object_id,
                    CommandEventInstanceKind::Prelude(CommandId::Exit),
                    command_instance_id,
                );
                preview_object_com_stop(object_id)?;
                preview_resume_exit_after_stop(object_id, command_instance_id)
            }
            CommandEvent::ObjectComPutTake {
                actor_id,
                target_id,
                requested_item,
                command,
                command_instance_id,
            } => {
                preview_resolve_put_take_attempt(
                    actor_id,
                    target_id,
                    requested_item,
                    command,
                    command_instance_id,
                )?;
                Vec::new()
            }
            CommandEvent::ObjectComDrop {
                actor_id,
                object_id,
                command_instance_id,
            } => {
                let _ = preview_object_com_drop(actor_id, object_id)?;
                HOST_CONTEXT.with(|cell| {
                    let mut borrow = cell.borrow_mut();
                    let Some(scope) = borrow
                        .as_mut()
                        .and_then(|context| context.object_scope_mut(actor_id))
                    else {
                        return;
                    };
                    scope.live_commands.finish_pending_drop(command_instance_id);
                    scope.command_stack_replaced = true;
                    scope.command_count = scope.live_commands.len();
                });
                Vec::new()
            }
            CommandEvent::ThrowObject {
                actor_id,
                object_id,
                complete_command_on_success,
                command_instance_id,
            } => {
                preview_resolve_throw_attempt(
                    actor_id,
                    object_id,
                    complete_command_on_success,
                    command_instance_id,
                )?;
                Vec::new()
            }
            CommandEvent::FailureFeedback { actor_id, feedback } => {
                preview_command_failure_feedback(actor_id, feedback)?;
                Vec::new()
            }
            event @ (CommandEvent::CommandExitObject { .. }
            | CommandEvent::CommandExitIntoParent { .. }) => {
                preview_command_exit(event)?;
                Vec::new()
            }
            CommandEvent::ActivateEntrance {
                object_id,
                caller,
                on_result,
                command_instance_id,
            } => {
                preview_resolve_activate_entrance(
                    object_id,
                    caller,
                    on_result,
                    command_instance_id,
                )?;
                Vec::new()
            }
            CommandEvent::SetPathFinderSettings {
                level,
                transfer_zones_enabled,
            } => {
                HOST_CONTEXT.with(|cell| {
                    if let Some(context) = cell.borrow_mut().as_mut() {
                        context
                            .world
                            .set_pathfinder_settings(level, transfer_zones_enabled);
                    }
                });
                preview_defer_command_event(
                    actor,
                    CommandEvent::SetPathFinderSettings {
                        level,
                        transfer_zones_enabled,
                    },
                );
                Vec::new()
            }
            CommandEvent::SetPathFinderDebug { snapshot } => {
                HOST_CONTEXT.with(|cell| {
                    if let Some(context) = cell.borrow_mut().as_mut() {
                        *context.world.pathfinder_debug.borrow_mut() = snapshot;
                    }
                });
                Vec::new()
            }
            CommandEvent::NativeCommandSuccess { object_id, command } => {
                HOST_CONTEXT.with(|cell| {
                    if let Some(context) = cell.borrow_mut().as_mut() {
                        apply_preview_native_command_success(context, object_id, command);
                    }
                });
                Vec::new()
            }
            CommandEvent::OpenMenu(request) => {
                HOST_CONTEXT.with(|cell| {
                    if let Some(context) = cell.borrow_mut().as_mut() {
                        context.pending_menu_requests.push(request);
                    }
                });
                Vec::new()
            }
            other => {
                preview_defer_command_event(actor, other);
                Vec::new()
            }
        };
        for event in resumed.into_iter().rev() {
            events.push_front(event);
        }
    }
    Ok(())
}

fn preview_command_prelude(initial: CommandEvent) -> Result<(), RuntimeError> {
    let actor = match &initial {
        CommandEvent::ObjectComStopThrow { object_id, .. }
        | CommandEvent::ObjectComSetDirThrow { object_id, .. }
        | CommandEvent::ObjectComStopDrop { object_id, .. }
        | CommandEvent::ObjectComStopPut { object_id, .. }
        | CommandEvent::ObjectComStopChop { object_id }
        | CommandEvent::ObjectComStopConstruct { object_id, .. }
        | CommandEvent::ObjectComStopExit { object_id, .. } => *object_id,
        CommandEvent::MoveToFlightControlTakeoff { object_id, .. } => *object_id,
        CommandEvent::ControlCommandConstruction { caller, .. } => *caller,
        CommandEvent::SpawnConstruction { actor_id, .. } => *actor_id,
        _ => return Ok(()),
    };
    preview_dispatch_command_continuation_events(actor, [initial])
}

fn preview_object_com_grab(actor: ObjectId, target: ObjectId) -> Result<bool, RuntimeError> {
    let walking = HOST_CONTEXT.with(|cell| {
        cell.borrow()
            .as_ref()
            .and_then(|context| context.object_scope(actor))
            .is_some_and(|scope| scope.effective_action_procedure() == ActionProcedure::Walk)
    });
    if !walking || !native_set_action_by_name_with_target(actor, "Push", Some(target))? {
        return Ok(false);
    }
    if !preview_object_is_present(actor) {
        return Ok(true);
    }
    let _ = call_object_own_fail_safe(
        actor,
        "Grab",
        &[object_reference_value(target), Value::Bool(true)],
    );
    if !preview_object_is_present(actor) || !preview_object_is_present(target) {
        return Ok(true);
    }
    let controller = HOST_CONTEXT.with(|cell| {
        cell.borrow()
            .as_ref()
            .and_then(|context| context.object_scope(actor))
            .map(ObjectScopeContext::controller)
            .unwrap_or(OWNER_NONE)
    });
    HOST_CONTEXT.with(|cell| {
        let mut borrow = cell.borrow_mut();
        if let Some(context) = borrow.as_mut() {
            if context.ensure_object_scope(target) {
                if let Some(scope) = context.object_scope_mut(target) {
                    scope.set_controller(controller);
                }
            }
        }
    });
    let _ = call_object_own_fail_safe(
        target,
        "Grabbed",
        &[object_reference_value(actor), Value::Bool(true)],
    );
    Ok(true)
}

/// Synchronous host preview for CommandEvent::AttemptGrab. The regular
/// engine applies the same event before its command-finished tail; this
/// form preserves ObjectComStop, At, MoveTo and callback ordering inside
/// script-level ExecuteCommand too.
fn preview_grab_attempt(actor: ObjectId, target: ObjectId) -> Result<(), RuntimeError> {
    let (initial_procedure, offsets) = HOST_CONTEXT
        .with(|cell| {
            let mut borrow = cell.borrow_mut();
            let context = borrow.as_mut()?;
            if !context.ensure_object_scope(actor) {
                return None;
            }
            let object = context.object_scope(actor)?;
            Some((
                object.effective_action_procedure(),
                object
                    .live_commands
                    .pending_grab_offsets(target)
                    .unwrap_or((0, 0)),
            ))
        })
        .unwrap_or((ActionProcedure::Undefined, (0, 0)));

    let mut stopped_for_grab = false;
    if matches!(
        initial_procedure,
        ActionProcedure::Build | ActionProcedure::Chop
    ) {
        stopped_for_grab = true;
        preview_object_com_stop(actor)?;
    }
    let procedure_after_build_or_chop = HOST_CONTEXT.with(|cell| {
        cell.borrow()
            .as_ref()
            .and_then(|context| context.object_scope(actor))
            .map(ObjectScopeContext::effective_action_procedure)
            .unwrap_or(ActionProcedure::Undefined)
    });
    if procedure_after_build_or_chop == ActionProcedure::Dig {
        stopped_for_grab = true;
        preview_object_com_stop(actor)?;
    }

    let snapshots = prepare_command_runtime_data(None).and_then(|(objects, _, _, _)| {
        Some((objects.get(&actor)?.clone(), objects.get(&target).cloned()))
    });
    let Some((actor_snapshot, target_snapshot)) = snapshots else {
        return Ok(());
    };

    if actor_snapshot.action_procedure == ActionProcedure::Push {
        with_host_context_mut((), |context| {
            let Some(scope) = context.object_scope_mut(actor) else {
                return;
            };
            let _ = scope.live_commands.resolve_grab_attempt(target, false);
            let _ = scope.live_commands.push_front(
                CommandRequest::new(CommandId::UnGrab)
                    .with_update_interval(50)
                    .with_mode(CommandMode::SilentSub),
            );
            scope.command_stack_replaced = true;
            scope.command_count = scope.live_commands.len();
        });
        return Ok(());
    }

    if stopped_for_grab {
        let failed = with_host_context_mut(false, |context| {
            let Some(scope) = context.object_scope_mut(actor) else {
                return false;
            };
            let failed = scope
                .live_commands
                .fail_pending_grab_if_target_cleared(target);
            if failed {
                scope.command_stack_replaced = true;
                scope.command_count = scope.live_commands.len();
            }
            failed
        });
        if failed {
            return Ok(());
        }
    }

    let Some(target_snapshot) = target_snapshot else {
        return Ok(());
    };

    let target_at_actor = actor_snapshot.container.is_none()
        && !target_snapshot.destroyed
        && target_snapshot.status != ObjectStatus::Deleted
        && target_snapshot.container.is_none()
        && target_snapshot.ocf & ocf::ALL != 0
        && target_snapshot.at_point(actor_snapshot.position.x, actor_snapshot.position.y);
    if !target_at_actor {
        with_host_context_mut((), |context| {
            let Some(scope) = context.object_scope_mut(actor) else {
                return;
            };
            let retained = scope
                .live_commands
                .resolve_grab_attempt(target, false)
                .unwrap_or(true);
            if retained {
                let _ = scope.live_commands.push_front(
                    CommandRequest::new(CommandId::MoveTo)
                        .with_tx(Some(target_snapshot.position.x.wrapping_add(offsets.0)))
                        .with_ty(Some(target_snapshot.position.y.wrapping_add(offsets.1)))
                        .with_update_interval(50)
                        .with_mode(CommandMode::SilentSub),
                );
            }
            scope.command_stack_replaced = true;
            scope.command_count = scope.live_commands.len();
        });
        return Ok(());
    }

    let let_go_xdir = matches!(
        actor_snapshot.action_procedure,
        ActionProcedure::Scale | ActionProcedure::Hang
    )
    .then_some(if actor_snapshot.direction == Direction::Left {
        1
    } else {
        -1
    });
    if let Some(xdir) = let_go_xdir {
        let _ = preview_object_action_jump(actor, FixedVec2::new(itofix(xdir), C4Fixed::ZERO))?;
    }

    let rejected = preview_object_is_present(target)
        && call_object_own_fail_safe(target, "RejectGrabbed", &[object_reference_value(actor)])
            .as_bool();
    if !preview_object_is_present(actor) {
        return Ok(());
    }
    let target_retained = with_host_context_mut(true, |context| {
        if let Some(object) = context.object_scope_mut(actor) {
            let resolution = object.live_commands.resolve_grab_attempt(target, rejected);
            if resolution.is_some() {
                object.command_stack_replaced = true;
                object.command_count = object.live_commands.len();
            }
            if rejected {
                return resolution.unwrap_or(true);
            }
            object.set_command_direction(CommandDirection::Stop);
            return resolution.unwrap_or(true);
        }
        true
    });
    if rejected {
        return Ok(());
    }
    if target_retained {
        let _ = preview_object_com_grab(actor, target)?;
    }
    Ok(())
}

/// Synchronous C4Object::ActivateEntrance twin for script ExecuteCommand.
/// The ordinary engine event cannot be deferred past the enclosing VM call:
/// C++ exposes the gate, callback, and Exit result to the very next script
/// instruction (C4Script.cpp:884-888; C4Object.cpp:1654-1670).
fn preview_activate_entrance(target: ObjectId, caller: ObjectId) -> bool {
    let Some((objects, _, _, _)) = prepare_command_runtime_data(None) else {
        return false;
    };
    let should_call = with_host_context_mut(false, |context| {
        let (Some(target_snapshot), Some(caller_snapshot)) =
            (objects.get(&target), objects.get(&caller))
        else {
            return false;
        };
        if target_snapshot.destroyed
            || target_snapshot.status == ObjectStatus::Deleted
            || caller_snapshot.destroyed
            || caller_snapshot.status == ObjectStatus::Deleted
        {
            return false;
        }
        let target_ocf = context
            .object_scope(target)
            .map(ObjectScopeContext::ocf)
            .or_else(|| context.get_world_object(target).map(|object| object.ocf()))
            .unwrap_or(target_snapshot.ocf);

        let by_player = caller_snapshot.controller;
        let base = target_snapshot.base;
        let hostile = if by_player == base {
            false
        } else {
            match (context.player_state(by_player), context.player_state(base)) {
                (Some(player), Some(base_player)) => {
                    player.is_hostile_towards(base) || base_player.is_hostile_towards(by_player)
                }
                _ => false,
            }
        };
        if context.world.base_reject_entrance_enabled && hostile {
            if let Some(owner_name) = context
                .player_state(target_snapshot.owner)
                .map(|player| player.name.clone())
            {
                context.register_message(MessageCommand::Add(MessageSpec::target(
                    format!("{owner_name} hostile.|No entrance!"),
                    target,
                )));
            }
            return false;
        }
        target_ocf & ocf::ENTRANCE != 0
    });
    if !should_call {
        return false;
    }
    call_object_own_fail_safe(
        target,
        "ActivateEntrance",
        &[object_reference_value(caller)],
    )
    .as_bool()
}

fn resolve_preview_buy(
    actor: ObjectId,
    base: ObjectId,
    definition_id: &str,
    succeeded: bool,
) -> Result<(), RuntimeError> {
    let feedback = HOST_CONTEXT.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let context = borrow.as_mut()?;
        if !context.ensure_object_scope(actor) {
            return None;
        }
        let scope = context.object_scope_mut(actor)?;
        let resolution = scope
            .live_commands
            .resolve_pending_buy(base, definition_id, succeeded)?;
        scope.command_stack_replaced = true;
        scope.command_count = scope.live_commands.len();
        Some(resolution.feedback)
    });
    if let Some(feedback) = feedback.flatten() {
        preview_command_failure_feedback(actor, feedback)?;
    }
    Ok(())
}

fn resolve_preview_sell(
    actor: ObjectId,
    base: ObjectId,
    definition_id: &str,
    succeeded: bool,
) -> Result<(), RuntimeError> {
    let feedback = HOST_CONTEXT.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let context = borrow.as_mut()?;
        if !context.ensure_object_scope(actor) {
            return None;
        }
        let scope = context.object_scope_mut(actor)?;
        let resolution =
            scope
                .live_commands
                .resolve_pending_sell(base, definition_id, succeeded)?;
        scope.command_stack_replaced = true;
        scope.command_count = scope.live_commands.len();
        Some(resolution.feedback)
    });
    if let Some(feedback) = feedback.flatten() {
        preview_command_failure_feedback(actor, feedback)?;
    }
    Ok(())
}

/// Synchronous C4Command::Buy continuation for script ExecuteCommand.
/// The definition/base pricing callbacks, Enter child insertion, repeated
/// C4Player::Buy calls and command-finished state must all be visible to the
/// very next VM instruction; deferring this as an engine event would replay
/// it after the enclosing script's later mutations.
fn preview_evaluate_buy(
    actor: ObjectId,
    base: ObjectId,
    definition_id: &str,
    buyer: i32,
    payer: i32,
    count: i32,
) -> Result<(), RuntimeError> {
    let price = calculated_definition_value(definition_id, Some(base), buyer)?.unwrap_or(0);
    let enough_wealth = HOST_CONTEXT.with(|cell| {
        cell.borrow()
            .as_ref()
            .and_then(|context| context.player_state(payer))
            .is_some_and(|player| price <= player.wealth)
    });
    if !enough_wealth {
        return resolve_preview_buy(actor, base, definition_id, false);
    }

    let contained = with_host_context(false, |context| {
        context
            .object_scope(actor)
            .map(ObjectScopeContext::container)
            .or_else(|| {
                context
                    .get_world_object(actor)
                    .map(|object| object.container())
            })
            == Some(Some(base))
    });
    if !contained {
        with_host_context_mut((), |context| {
            if !context.ensure_object_scope(actor) {
                return;
            }
            let Some(scope) = context.object_scope_mut(actor) else {
                return;
            };
            scope
                .live_commands
                .defer_pending_buy_for_enter(base, definition_id);
            let _ = scope.live_commands.push_front(
                CommandRequest::new(CommandId::Enter)
                    .with_target(Some(base))
                    .with_update_interval(50)
                    .with_mode(CommandMode::SilentSub),
            );
            scope.command_stack_replaced = true;
            scope.command_count = scope.live_commands.len();
        });
        return Ok(());
    }

    let purchase_count = with_host_context_mut(count.max(1), |context| {
        if !context.ensure_object_scope(actor) {
            return count.max(1);
        }
        let Some(scope) = context.object_scope_mut(actor) else {
            return count.max(1);
        };
        let count = scope
            .live_commands
            .normalize_pending_buy_count(base, definition_id)
            .unwrap_or_else(|| count.max(1));
        scope.command_stack_replaced = true;
        scope.command_count = scope.live_commands.len();
        count
    });

    for _ in 0..purchase_count {
        let parties = HOST_CONTEXT.with(|cell| {
            let borrow = cell.borrow();
            let context = borrow.as_ref()?;
            if !context.world.base_buy_enabled {
                return None;
            }
            let live_buyer = context
                .object_scope(actor)
                .map(ObjectScopeContext::owner)
                .or_else(|| context.get_world_object(actor).map(|object| object.owner()))?;
            let live_payer = context
                .get_world_object(base)?
                .full_state()
                .map(|state| state.base)?;
            let (buyer_player, payer_player) = (
                context.player_state(live_buyer)?,
                context.player_state(live_payer)?,
            );
            let hostile = live_buyer != live_payer
                && (buyer_player.is_hostile_towards(live_payer)
                    || payer_player.is_hostile_towards(live_buyer));
            (!hostile).then_some((live_buyer, live_payer))
        });
        let Some((live_buyer, live_payer)) = parties else {
            return resolve_preview_buy(actor, base, definition_id, false);
        };
        let bought = buy(&[
            Value::C4Id(definition_id.to_string()),
            Value::Int(live_buyer),
            Value::Int(live_payer),
            object_reference_value(base),
            Value::Bool(false),
        ])?;
        if !matches!(bought, Value::Object(_)) {
            return resolve_preview_buy(actor, base, definition_id, false);
        }
        with_host_context_mut((), |context| {
            let Some(scope) = context.object_scope_mut(actor) else {
                return;
            };
            scope
                .live_commands
                .record_pending_buy_success(base, definition_id);
            scope.command_stack_replaced = true;
            scope.command_count = scope.live_commands.len();
        });
    }
    resolve_preview_buy(actor, base, definition_id, true)
}

/// Synchronous C4Command::Sell continuation for script ExecuteCommand.
/// Recursive CalcValue/CalcSellValue/SellTo/Sale effects and the complete
/// count loop must be visible to the next instruction in the same VM call.
fn preview_evaluate_sell(
    actor: ObjectId,
    base: ObjectId,
    definition_id: &str,
    preferred: Option<ObjectId>,
    count: i32,
) -> Result<(), RuntimeError> {
    let sale_count = with_host_context_mut(count.max(1), |context| {
        if !context.ensure_object_scope(actor) {
            return count.max(1);
        }
        let Some(scope) = context.object_scope_mut(actor) else {
            return count.max(1);
        };
        let count = scope
            .live_commands
            .normalize_pending_sell_count(base, definition_id)
            .unwrap_or_else(|| count.max(1));
        scope.command_stack_replaced = true;
        scope.command_count = scope.live_commands.len();
        count
    });
    let mut preferred = preferred;

    for _ in 0..sale_count {
        let attempt = HOST_CONTEXT.with(|cell| {
            let borrow = cell.borrow();
            let context = borrow.as_ref()?;
            if !context.world.base_sell_enabled {
                return None;
            }
            let seller = context
                .object_scope(actor)
                .map(ObjectScopeContext::owner)
                .or_else(|| context.get_world_object(actor).map(|object| object.owner()))?;
            let base_object = context.get_world_object(base)?;
            let base_owner = base_object.full_state()?.base;
            let (seller_player, base_player) = (
                context.player_state(seller)?,
                context.player_state(base_owner)?,
            );
            if matches!(
                base_player.status,
                crate::PlayerStatus::Eliminated | crate::PlayerStatus::Surrendered
            ) || base_player.surrendered
            {
                return None;
            }
            let hostile = seller != base_owner
                && (seller_player.is_hostile_towards(base_owner)
                    || base_player.is_hostile_towards(seller));
            if hostile {
                return None;
            }

            let preferred_candidate = preferred.filter(|candidate| {
                context
                    .get_world_object(*candidate)
                    .is_some_and(|object| object.is_present() && object.container() == Some(base))
            });
            let candidate = preferred_candidate.or_else(|| {
                base_object.contents().iter().copied().find(|candidate| {
                    context.get_world_object(*candidate).is_some_and(|object| {
                        object.is_present()
                            && object.container() == Some(base)
                            && context
                                .object_effective_definition_id(*candidate)
                                .is_some_and(|id| id == definition_id)
                    })
                })
            })?;
            let candidate_definition = context.object_effective_definition_id(candidate)?;
            if context
                .world
                .definition_no_sell(candidate_definition.as_str())
            {
                return None;
            }
            Some((base_owner, candidate))
        });
        let Some((base_owner, candidate)) = attempt else {
            return resolve_preview_sell(actor, base, definition_id, false);
        };
        if !sell_object_to_home_live(candidate, base_owner)? {
            return resolve_preview_sell(actor, base, definition_id, false);
        }
        preferred = None;
        with_host_context_mut((), |context| {
            if !context.ensure_object_scope(actor) {
                return;
            }
            let Some(scope) = context.object_scope_mut(actor) else {
                return;
            };
            scope
                .live_commands
                .record_pending_sell_success(base, definition_id);
            scope.command_stack_replaced = true;
            scope.command_count = scope.live_commands.len();
        });
    }
    resolve_preview_sell(actor, base, definition_id, true)
}

pub(crate) fn apply_preview_native_command_success(
    context: &mut EffectHostContext,
    target: ObjectId,
    command: CommandId,
) {
    let gain = command.experience_gain();
    if gain == 0 {
        return;
    }
    let Some(link) = context.object_scope(target).and_then(|scope| {
        scope.info_core()?;
        scope.info_link()
    }) else {
        return;
    };
    let experience_awards = {
        let mut state = context.world.crew_info_state.borrow_mut();
        let control_count = state.control_counts.entry(link).or_default();
        let mut awards = 0;
        for _ in 0..gain {
            *control_count = control_count.wrapping_add(1);
            if *control_count % 5 == 0 {
                awards += 1;
            }
        }
        awards
    };
    context.record_player_command(PlayerCommand::AdjustCrewControlCount { link, gain });
    for _ in 0..experience_awards {
        apply_host_crew_experience(context, target, 1);
    }
}

fn apply_preview_command_experience(target: ObjectId) -> Result<(), RuntimeError> {
    with_host_context_mut(Ok(()), |context| {
        let successful_finishes = context
            .object_scope_mut(target)
            .map(|scope| scope.live_commands.take_successful_finishes())
            .unwrap_or_default();
        for command in successful_finishes {
            apply_preview_native_command_success(context, target, command);
        }
        Ok(())
    })
}

/// C4Command::Call invokes Target->Call(...) before ExecuteCommand returns
/// (C4Command.cpp:2355-2368). Deferring that event past later script
/// mutations can erase the queued arguments.
///
/// Freshly emitted Call commands carry no result action: `C4Command::Call`
/// runs `Finish(true)` before the call and deliberately touches nothing
/// afterwards. A result action only reaches here from a save predating the
/// dedicated ControlTransfer event, so this mirrors the non-preview handler
/// in `Engine::apply_command_event` for that shape rather than inventing a
/// second set of rules for it.
fn preview_call_object_function(event: CommandEvent) -> Result<(), RuntimeError> {
    let CommandEvent::CallObjectFunction {
        object_id,
        function,
        caller,
        tx,
        tx_value,
        tx_definition,
        ty,
        target2,
        on_result,
    } = event
    else {
        return Ok(());
    };
    let tx_value = tx_value
        .or_else(|| tx_definition.map(Value::C4Id))
        .or_else(|| tx.map(Value::Int))
        .unwrap_or(Value::Nil);
    // Bind the completion target before the callback can replace the command
    // stack, exactly like the non-preview path does.
    let command_instance_id = on_result
        .as_ref()
        .map_or(0, |action| preview_call_result_instance_id(caller, action));

    // Saves from before the dedicated ControlTransfer event retain the old
    // generic-call shape. Execute the cached definition function with
    // native's Status bypass and getBool return conversion just like a newly
    // emitted ControlTransfer event.
    let legacy_transfer = function == "ControlTransfer"
        && matches!(
            &on_result,
            Some(CallResultAction::CompleteCommandOnFalse {
                command: CommandId::Transfer
            })
        );
    if legacy_transfer {
        preview_control_transfer(
            object_id,
            caller,
            tx_value,
            ty.unwrap_or(0),
            command_instance_id,
        );
        return Ok(());
    }

    if !preview_object_is_present(object_id) {
        return Ok(());
    }
    let args = [
        object_reference_value(caller),
        tx_value,
        Value::Int(ty.unwrap_or(0)),
        target2.map(object_reference_value).unwrap_or(Value::Nil),
    ];
    let result = match call_world_object_function(object_id, &function, &args) {
        Some(Ok(value)) => value.as_bool(),
        Some(Err(error)) => {
            tracing::error!(
                %error,
                "Call command error; continuing like the C++ fail-safe exec"
            );
            log_runtime_call_frames("", error.call_frames());
            false
        }
        None => false,
    };
    let Some(action) = on_result else {
        return Ok(());
    };
    preview_apply_call_result(action, caller, result, command_instance_id);
    Ok(())
}

/// Host-preview twin of `Engine::resolve_call_result_instance_id`.
fn preview_call_result_instance_id(caller: ObjectId, action: &CallResultAction) -> u64 {
    let kind = match action {
        CallResultAction::CompleteCommandOnFalse { command }
        | CallResultAction::CompleteCommandOnTrue { command }
        | CallResultAction::FailCommandOnFalse { command } => {
            CommandEventInstanceKind::Exact(*command)
        }
        CallResultAction::ResolveExitActivation => CommandEventInstanceKind::ExitActivation,
    };
    HOST_CONTEXT.with(|cell| {
        cell.borrow()
            .as_ref()
            .and_then(|context| context.object_scope(caller))
            .map_or(0, |scope| {
                scope.live_commands.resolve_event_instance_id(kind, 0)
            })
    })
}

/// Host-preview twin of `Engine::apply_call_result`. The exact instance is
/// resolved before the callback runs, so a callback that pushes another
/// command of the same kind cannot inherit this result.
fn preview_apply_call_result(
    action: CallResultAction,
    caller: ObjectId,
    result: bool,
    command_instance_id: u64,
) {
    HOST_CONTEXT.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let Some(scope) = borrow
            .as_mut()
            .and_then(|context| context.object_scope_mut(caller))
        else {
            return;
        };
        match action {
            CallResultAction::CompleteCommandOnFalse { command } if !result => {
                scope
                    .live_commands
                    .finish_command_instance(command, command_instance_id);
            }
            CallResultAction::CompleteCommandOnTrue { command } if result => {
                scope
                    .live_commands
                    .finish_command_instance(command, command_instance_id);
            }
            CallResultAction::FailCommandOnFalse { command } if !result => {
                scope
                    .live_commands
                    .fail_command_instance(command, command_instance_id);
            }
            CallResultAction::ResolveExitActivation => {
                // Only ActivateEntrance carries this (command/model.rs), and
                // it has its own preview twin with the feedback plumbing.
                debug_assert!(
                    false,
                    "CallObjectFunction never carries ResolveExitActivation"
                );
            }
            CallResultAction::CompleteCommandOnFalse { .. }
            | CallResultAction::CompleteCommandOnTrue { .. }
            | CallResultAction::FailCommandOnFalse { .. } => {}
        }
        scope.command_stack_replaced = true;
        scope.command_count = scope.live_commands.len();
    });
}

/// C4Command::Transfer invokes the definition host's AfterLink-cached
/// function pointer directly. Script ExecuteCommand must complete that call
/// before returning to the next VM instruction, without C4Object::Call's
/// receiver-Status gate.
fn preview_control_transfer(
    object_id: ObjectId,
    caller: ObjectId,
    tx_value: Value,
    ty: i32,
    command_instance_id: u64,
) {
    let callback = HOST_CONTEXT.with(|cell| {
        let borrow = cell.borrow();
        let context = borrow.as_ref()?;
        let definition_id = context.object_effective_definition_id(object_id)?;
        context
            .definition_metadata(&definition_id)
            .and_then(|metadata| metadata.control_transfer_callback.clone())
    });
    let handled = callback
        .and_then(|callback| {
            call_world_object_script_callback(
                object_id,
                &callback,
                &[object_reference_value(caller), tx_value, Value::Int(ty)],
            )
        })
        .is_some_and(|result| match result {
            Ok(value) => value
                .c4_bool_raw()
                .map_or_else(|| value.as_bool(), |raw| raw != 0),
            Err(error) => {
                tracing::error!(
                    %error,
                    "ControlTransfer error; continuing like the C++ fail-safe exec"
                );
                log_runtime_call_frames("", error.call_frames());
                false
            }
        });
    if handled {
        return;
    }
    HOST_CONTEXT.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let Some(scope) = borrow
            .as_mut()
            .and_then(|context| context.object_scope_mut(caller))
        else {
            return;
        };
        scope
            .live_commands
            .finish_command_instance(CommandId::Transfer, command_instance_id);
        scope.command_stack_replaced = true;
        scope.command_count = scope.live_commands.len();
    });
}

/// Host-preview twin of the live Exit command events. Both C4Object::Exit
/// and nested C4Object::Enter are callbackful and Finish(true) occurs only
/// after they return, so deferring these events past FnExecuteCommand would
/// expose stale containment and command state to the script caller.
fn preview_command_exit(event: CommandEvent) -> Result<(), RuntimeError> {
    let (object_id, command_instance_id) = match event {
        CommandEvent::CommandExitObject {
            object_id,
            previous_container,
            position,
            jump_after,
            command_instance_id,
        } => {
            let still_in_expected_container = HOST_CONTEXT.with(|cell| {
                cell.borrow()
                    .as_ref()
                    .and_then(|context| context.get_world_object(object_id))
                    .is_some_and(|object| object.container() == Some(previous_container))
            });
            if still_in_expected_container {
                let _ = exit_object_at_position_with_calls(object_id, position, true)?;
            }
            if jump_after {
                let _ = jump(&[object_reference_value(object_id)])?;
            }
            (object_id, command_instance_id)
        }
        CommandEvent::CommandExitIntoParent {
            object_id,
            container_id,
            command_instance_id,
        } => {
            let _ = enter_object_live(object_id, container_id)?;
            (object_id, command_instance_id)
        }
        _ => unreachable!("preview_command_exit only accepts Exit events"),
    };
    HOST_CONTEXT.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let Some(scope) = borrow
            .as_mut()
            .and_then(|context| context.object_scope_mut(object_id))
        else {
            return;
        };
        scope
            .live_commands
            .finish_command_instance(CommandId::Exit, command_instance_id);
        scope.command_stack_replaced = true;
        scope.command_count = scope.live_commands.len();
    });
    Ok(())
}

fn preview_resolve_activate_entrance(
    object_id: ObjectId,
    caller: ObjectId,
    on_result: Option<CallResultAction>,
    command_instance_id: u64,
) -> Result<(), RuntimeError> {
    let detached_feedback = matches!(&on_result, Some(CallResultAction::ResolveExitActivation))
        .then(|| {
            HOST_CONTEXT.with(|cell| {
                cell.borrow()
                    .as_ref()
                    .and_then(|context| context.object_scope(caller))
                    .and_then(|scope| {
                        scope
                            .live_commands
                            .pending_exit_activation_failure_feedback(command_instance_id)
                    })
            })
        })
        .flatten();
    let activated = preview_activate_entrance(object_id, caller);
    let feedback = HOST_CONTEXT.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let context = borrow.as_mut()?;
        let scope = context.object_scope_mut(caller)?;
        let feedback = match on_result {
            Some(CallResultAction::ResolveExitActivation) => match scope
                .live_commands
                .resolve_exit_activation(activated, command_instance_id)
            {
                Some(resolution) => resolution.feedback,
                None if !activated => detached_feedback,
                None => None,
            },
            Some(CallResultAction::CompleteCommandOnFalse { command }) if !activated => {
                scope.live_commands.complete_front_if(command);
                None
            }
            Some(CallResultAction::CompleteCommandOnTrue { command }) if activated => {
                scope.live_commands.complete_front_if(command);
                None
            }
            Some(CallResultAction::FailCommandOnFalse { command }) if !activated => {
                scope.live_commands.fail_front_if(command);
                None
            }
            _ => None,
        };
        scope.command_stack_replaced = true;
        scope.command_count = scope.live_commands.len();
        Some(feedback)
    });
    if let Some(feedback) = feedback.flatten() {
        preview_command_failure_feedback(caller, feedback)?;
    }
    Ok(())
}

pub(crate) fn execute_command(args: &[Value]) -> Result<Value, RuntimeError> {
    let active = active_object_id();
    let target = match args.first() {
        Some(value) => {
            parse_object_reference_argument(value, "ExecuteCommand", "target")?.or(active)
        }
        None => active,
    };
    let Some(target) = target else {
        return Ok(Value::Bool(false));
    };
    if active != Some(target) {
        return match call_world_object_function(target, "ExecuteCommand", &[]) {
            Some(result) => result,
            None => Ok(Value::Bool(false)),
        };
    }

    let random = RANDOM_CONTEXT.with(|cell| cell.borrow().clone());
    let command_data = prepare_command_runtime_data(Some(target));
    let preview = command_data.as_ref().and_then(|command_data| {
        HOST_CONTEXT.with(|cell| {
            cell.borrow_mut().as_mut().and_then(|context| {
                context.execute_command_preview(
                    target,
                    random.as_ref().map(|rng| &rng.rng),
                    command_data,
                )
            })
        })
    });
    let Some(mut preview) = preview else {
        return Ok(Value::Bool(false));
    };

    // GetFairCrewPhysical is ordinary script and must run at the exact
    // command branch which requested it, never while the initial structural
    // snapshot is built. Resolve every native read with no HOST_CONTEXT
    // borrow held, then resume that same command body synchronously before
    // FnExecuteCommand returns to its caller.
    while !preview.physical_reads.is_empty() {
        let pending = std::mem::take(&mut preview.physical_reads);
        for (actor_id, reads, supplied_instance_id) in pending {
            let Some((events, finished)) = preview_resume_command_after_physical(
                actor_id,
                reads,
                supplied_instance_id,
                random.as_ref().map(|rng| &rng.rng),
            ) else {
                continue;
            };
            let resumed = HOST_CONTEXT.with(|cell| {
                cell.borrow_mut().as_mut().and_then(|context| {
                    context.collect_command_preview_events(actor_id, finished, events)
                })
            });
            if let Some(resumed) = resumed {
                preview.append(resumed);
            }
        }
    }

    let had_live_attempt = preview.had_live_attempt();
    let CommandPreviewOutcome {
        mut finished,
        buy_attempts,
        sell_attempts,
        grab_attempts,
        put_attempts,
        drop_attempts,
        ungrab_attempts,
        put_take_attempts,
        throw_attempts,
        throw_preludes,
        entrance_attempts,
        control_transfers,
        call_attempts,
        exit_attempts,
        failure_feedback,
        move_to_stops,
        build_stops,
        build_actions,
        dig_attempts,
        physical_reads,
    } = preview;
    debug_assert!(physical_reads.is_empty());
    for (actor_id, base_id, definition_id, buyer, payer, count) in buy_attempts {
        preview_evaluate_buy(actor_id, base_id, &definition_id, buyer, payer, count)?;
    }
    for (actor_id, base_id, definition_id, preferred, count) in sell_attempts {
        preview_evaluate_sell(actor_id, base_id, &definition_id, preferred, count)?;
    }
    for actor_id in move_to_stops {
        let events = preview_move_to_stop(actor_id)?;
        preview_dispatch_command_continuation_events(actor_id, events)?;
    }
    for (actor_id, command_instance_id) in build_stops {
        let events = preview_build_stop(actor_id, command_instance_id)?;
        preview_dispatch_command_continuation_events(actor_id, events)?;
    }
    for (actor_id, target_id, stop_first) in build_actions {
        preview_object_com_build(actor_id, target_id, stop_first)?;
    }
    for (actor_id, dig_out_material, direction, command_instance_id) in dig_attempts {
        let succeeded = preview_object_com_dig(actor_id)?;
        let feedback = HOST_CONTEXT.with(|cell| {
            let mut borrow = cell.borrow_mut();
            let context = borrow.as_mut()?;
            let scope = context.object_scope_mut(actor_id)?;
            if succeeded {
                if dig_out_material {
                    scope.set_action_data(1);
                }
                if let Some(direction) = direction {
                    scope.set_command_direction(direction);
                }
            }
            let feedback = scope
                .live_commands
                .resolve_dig_attempt(command_instance_id, succeeded);
            scope.command_stack_replaced = true;
            scope.command_count = scope.live_commands.len();
            Some(feedback)
        });
        if let Some(feedback) = feedback.flatten() {
            preview_command_failure_feedback(actor_id, feedback)?;
        }
    }
    for (actor_id, target_id) in grab_attempts {
        preview_grab_attempt(actor_id, target_id)?;
        while let Some(feedback) = HOST_CONTEXT.with(|cell| {
            cell.borrow_mut()
                .as_mut()
                .and_then(|context| context.object_scope_mut(actor_id))
                .and_then(|scope| scope.live_commands.take_failure_feedback())
        }) {
            preview_command_failure_feedback(actor_id, feedback)?;
        }
    }
    for (actor_id, target_id, object_id, ungrab_on_success, command_instance_id) in put_attempts {
        preview_resolve_put_attempt(
            actor_id,
            target_id,
            object_id,
            ungrab_on_success,
            command_instance_id,
        )?;
    }
    for (actor_id, object_id, command_instance_id) in drop_attempts {
        let _ = preview_object_com_drop(actor_id, object_id)?;
        HOST_CONTEXT.with(|cell| {
            cell.borrow_mut()
                .as_mut()
                .and_then(|context| context.object_scope_mut(actor_id))
                .map(|scope| scope.live_commands.finish_pending_drop(command_instance_id))
        });
    }
    for (actor_id, command_instance_id) in ungrab_attempts {
        let _ = preview_object_com_ungrab(actor_id, false)?;
        HOST_CONTEXT.with(|cell| {
            let mut borrow = cell.borrow_mut();
            let Some(scope) = borrow
                .as_mut()
                .and_then(|context| context.object_scope_mut(actor_id))
            else {
                return;
            };
            scope.set_command_direction(CommandDirection::Stop);
            scope
                .live_commands
                .finish_command_instance(CommandId::UnGrab, command_instance_id);
            scope.command_stack_replaced = true;
            scope.command_count = scope.live_commands.len();
        });
    }
    for (actor_id, target_id, requested_item, command, command_instance_id) in put_take_attempts {
        preview_resolve_put_take_attempt(
            actor_id,
            target_id,
            requested_item,
            command,
            command_instance_id,
        )?;
    }
    for (actor_id, object_id, complete_on_success, command_instance_id) in throw_attempts {
        preview_resolve_throw_attempt(
            actor_id,
            object_id,
            complete_on_success,
            command_instance_id,
        )?;
    }
    for event in throw_preludes {
        preview_command_prelude(event)?;
    }
    for (object_id, caller, on_result, command_instance_id) in entrance_attempts {
        preview_resolve_activate_entrance(object_id, caller, on_result, command_instance_id)?;
    }
    for (object_id, caller, tx_value, ty, command_instance_id) in control_transfers {
        preview_control_transfer(object_id, caller, tx_value, ty, command_instance_id);
    }
    for event in call_attempts {
        preview_call_object_function(event)?;
    }
    for event in exit_attempts {
        preview_command_exit(event)?;
    }
    for (actor_id, feedback) in failure_feedback {
        preview_command_failure_feedback(actor_id, feedback)?;
    }
    apply_preview_command_experience(target)?;
    if had_live_attempt {
        finished = HOST_CONTEXT.with(|cell| {
            cell.borrow()
                .as_ref()
                .and_then(|context| context.object_scope(target))
                .and_then(|scope| scope.live_commands.finished_front_view())
        });
    }

    if let Some(command) = finished {
        let callback_args = [
            Value::String(command.name.clone().into()),
            command
                .target
                .map(object_reference_value)
                .unwrap_or(Value::Nil),
            command_view_tx_value(&command),
            Value::Int(command.ty.unwrap_or(0)),
            command
                .target2
                .map(object_reference_value)
                .unwrap_or(Value::Nil),
            command_view_data_value(&command),
        ];
        if object_has_status(target) {
            if let Some(Err(error)) =
                call_world_object_function(target, "ControlCommandFinished", &callback_args)
            {
                tracing::error!(
                    %error,
                    "script error in ControlCommandFinished; continuing like the C++ fail-safe exec"
                );
                log_runtime_call_frames("", error.call_frames());
            }
        }
        HOST_CONTEXT.with(|cell| {
            if let Some(context) = cell.borrow_mut().as_mut() {
                context.clear_finished_command_fronts(target);
            }
        });
    }

    Ok(Value::Bool(true))
}

/// C4Value::getInt as used for FnPlayerObjectCommand's untyped Tx slot.
/// Bool converts directly; conversions from C4ID/string/object fail and
/// therefore yield zero (C4Value.h:159, C4Script.cpp:961-985).
fn player_object_command_tx(value: Option<&Value>) -> i32 {
    match value.unwrap_or(&Value::Nil) {
        Value::Int(value) => *value,
        Value::Bool(value) => i32::from(*value),
        _ => 0,
    }
}

/// C4Value::getIntOrID for FnPlayerObjectCommand's data slot. Unsupported
/// types are deliberately zero rather than script errors.
fn player_object_command_data(value: Option<&Value>) -> i32 {
    match value.unwrap_or(&Value::Nil) {
        Value::Int(value) => *value,
        Value::Bool(value) => i32::from(*value),
        Value::C4Id(id) => cast_c4id_payload(id) as i32,
        _ => 0,
    }
}

fn native_set_command_tx(request: &CommandRequest) -> Value {
    request
        .tx_value
        .clone()
        .or_else(|| {
            request
                .tx_definition
                .as_ref()
                .map(|id| Value::C4Id(id.as_str().to_string()))
        })
        .or_else(|| request.tx.map(Value::Int))
        .unwrap_or(Value::Int(0))
}

fn set_command_callback_args(request: &CommandRequest, tx: Value) -> [Value; 6] {
    let data = match &request.data {
        CommandData::Integer(value) => *value,
        CommandData::Text(_) | CommandData::None => 0,
    };
    [
        Value::String(request.id.to_name().to_string().into()),
        request
            .target
            .map(object_reference_value)
            .unwrap_or(Value::Nil),
        tx,
        Value::Int(request.ty.unwrap_or(0)),
        request
            .target2
            .map(object_reference_value)
            .unwrap_or(Value::Nil),
        Value::Int(data),
    ]
}

/// `C4Object::Call` is fail-safe and returns nil without executing when the
/// receiver has status zero. Controller transfer happens before this helper.
fn call_control_command_fail_safe(target: ObjectId, args: &[Value]) -> bool {
    let present = HOST_CONTEXT.with(|cell| {
        cell.borrow()
            .as_ref()
            .is_some_and(|context| context.object_status_present(target))
    });
    if !present {
        return false;
    }
    match call_world_object_own_function(target, "ControlCommand", args) {
        Some(Ok(value)) => value_raw_truthy(&value),
        Some(Err(error)) => {
            tracing::error!(
                object = %target,
                %error,
                "script error in ControlCommand; continuing like the C++ fail-safe exec"
            );
            log_runtime_call_frames("", error.call_frames());
            false
        }
        None => false,
    }
}

fn clear_command_stack_live(target: ObjectId) -> bool {
    with_host_context_mut(false, |context| {
        if !context.ensure_object_scope(target) {
            return false;
        }
        let Some(object) = context.object_scope_mut(target) else {
            return false;
        };
        object.clear_command_stack();
        true
    })
}

/// Native C4Object::SetCommand over a live host scope. Only menu closing and
/// the receiver's own ControlCommand are `fControl`-gated; inside/outside
/// vehicle overloads run for script and engine SetCommand calls too
/// (C4Object.cpp:3939-3983).
fn set_command_live(
    target: ObjectId,
    request: CommandRequest,
    f_control: bool,
    callback_tx: Value,
) -> bool {
    let staged = with_host_context_mut(false, |context| {
        // FnSetCommand only checks that pObj is non-null. A status-zero object
        // can still be the in-flight script receiver and SetCommand continues
        // operating on its removal-delay tombstone.
        if !context.ensure_object_scope(target) {
            return false;
        }
        let Some(object) = context.object_scope_mut(target) else {
            return false;
        };
        // SetCommand decrements this delay before clearing the old stack.
        object.decrement_no_collect_delay();
        object.clear_command_stack();
        true
    });
    if !staged {
        return false;
    }

    if f_control {
        // The soft menu close happens after ClearCommands. A denial therefore
        // leaves the stack cleared (plus any command the query callback created).
        if !close_object_menu(target, false) {
            return true;
        }
    }

    let callback_args = set_command_callback_args(&request, callback_tx);
    if f_control && call_control_command_fail_safe(target, &callback_args) {
        return true;
    }

    // The contained vehicle receives a seventh argument naming the command
    // object, and inherits that object's live Controller before its call.
    let inside = HOST_CONTEXT.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let context = borrow.as_mut()?;
        let (container, controller) = {
            let actor = context.object_scope(target)?;
            (actor.container()?, actor.controller())
        };
        let definition = context.object_effective_definition_id(container)?;
        let enabled = context
            .definition_metadata(definition.as_str())
            .is_some_and(|metadata| metadata.vehicle_control & crate::VEHICLE_CONTROL_INSIDE != 0);
        if !enabled || !context.ensure_object_scope(container) {
            return None;
        }
        context
            .object_scope_mut(container)?
            .set_controller(controller);
        Some(container)
    });
    if let Some(container) = inside {
        let mut args = callback_args.to_vec();
        args.push(object_reference_value(target));
        if call_control_command_fail_safe(container, &args) {
            return true;
        }
    }

    // Re-read action/procedure/target/controller after the inside callback:
    // it may have redirected the pushed vehicle or changed the actor.
    let outside = HOST_CONTEXT.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let context = borrow.as_mut()?;
        let (pushed, controller) = {
            let actor = context.object_scope(target)?;
            if actor.effective_action_procedure() != ActionProcedure::Push {
                return None;
            }
            (actor.effective_action_target(0)?, actor.controller())
        };
        let definition = context.object_effective_definition_id(pushed)?;
        let enabled = context
            .definition_metadata(definition.as_str())
            .is_some_and(|metadata| metadata.vehicle_control & crate::VEHICLE_CONTROL_OUTSIDE != 0);
        if !enabled || !context.ensure_object_scope(pushed) {
            return None;
        }
        context.object_scope_mut(pushed)?.set_controller(controller);
        Some(pushed)
    });
    if let Some(pushed) = outside {
        if call_control_command_fail_safe(pushed, &callback_args) {
            return true;
        }
    }

    with_host_context_mut((), |context| {
        if let Some(object) = context.object_scope_mut(target) {
            let _ = object.push_command_front(request);
        }
    });
    // FnSetCommand reports success for every recognized command regardless of
    // overload consumption or the final AddCommand stack-limit result.
    true
}

/// C4Object::SetCommand(..., fControl=true) for one recipient of
/// C4Player::ObjectCommand. Command operations live on the target's copied
/// object scope, so GetCommand observes them before this host call returns and
/// the ordinary callback outcome fold persists the exact same stack.
fn set_player_control_command(target: ObjectId, request: CommandRequest) {
    let callback_tx = native_set_command_tx(&request);
    let _ = set_command_live(target, request, true, callback_tx);
}

fn player_object_command_request(
    command: CommandId,
    target: Option<ObjectId>,
    tx: i32,
    ty: i32,
    target2: Option<ObjectId>,
    data: i32,
) -> CommandRequest {
    CommandRequest::new(command)
        .with_target(target)
        .with_target2(target2)
        .with_tx(Some(tx))
        .with_ty(Some(ty))
        .with_data(CommandData::Integer(data))
        .with_mode(CommandMode::Base)
}

/// FnPlayerObjectCommand (C4Script.cpp:961-985) ->
/// C4Player::ObjectCommand(..., C4P_Command_Set) (C4Player.cpp:1397-1451).
pub(crate) fn player_object_command_host(args: &[Value]) -> Result<Value, RuntimeError> {
    let player_id = value_to_i32(
        args.first().unwrap_or(&Value::Nil),
        "PlayerObjectCommand",
        "player",
    )?;
    let command_name = parse_optional_string(args.get(1), "PlayerObjectCommand", "command")?;
    let target = parse_object_reference_argument(
        args.get(2).unwrap_or(&Value::Nil),
        "PlayerObjectCommand",
        "target",
    )?;
    let tx = player_object_command_tx(args.get(3));
    let ty = value_to_i32(
        args.get(4).unwrap_or(&Value::Nil),
        "PlayerObjectCommand",
        "y",
    )?;
    let target2 = parse_object_reference_argument(
        args.get(5).unwrap_or(&Value::Nil),
        "PlayerObjectCommand",
        "target2",
    )?;
    let data = player_object_command_data(args.get(6));

    // Native parameter conversion precedes the C++ function body, so only
    // now may player/name/command validation short-circuit. Extra arguments
    // have already been discarded by C4Aul's fixed parameter frame.
    let player_exists = HOST_CONTEXT.with(|cell| {
        cell.borrow()
            .as_ref()
            .is_some_and(|context| context.player_state(player_id).is_some())
    });
    let Some(command_name) = command_name else {
        return Ok(Value::Bool(false));
    };
    if !player_exists {
        return Ok(Value::Bool(false));
    }
    let Some(command) = CommandId::from_name(&command_name) else {
        return Ok(Value::Bool(false));
    };
    let data = if command == CommandId::Call {
        const MESSAGE: &str = "PlayerObjectCommand: Command \"Call\" not supported";
        // StrictError reads cthr->Caller->Func->Owner->Strict. Direct native,
        // non-strict, and strict-1/2 callers warn and continue; strict-3 and
        // above abort before C4Player::ObjectCommand performs any mutation.
        if matches!(
            clonk_script::caller_strictness(),
            clonk_script::HostCallerStrictness::Strict(level) if level >= 3
        ) {
            return Err(RuntimeError::new(MESSAGE));
        }
        tracing::warn!(target: SCRIPT_LOG_TARGET, "{MESSAGE}");
        // FnPlayerObjectCommand deliberately skips data.getIntOrID() for Call.
        0
    } else {
        data
    };

    // FnPlayerObjectCommand ignores ObjectCommand's false result and reports
    // true for an existing, but eliminated, player.
    let eliminated = HOST_CONTEXT.with(|cell| {
        cell.borrow()
            .as_ref()
            .and_then(|context| context.player_state(player_id))
            .is_some_and(|player| {
                matches!(
                    player.status,
                    crate::PlayerStatus::Eliminated | crate::PlayerStatus::Surrendered
                )
            })
    });
    if eliminated {
        return Ok(Value::Bool(true));
    }

    update_player_selection_toggle_status_host(player_id);
    let crew = HOST_CONTEXT.with(|cell| {
        cell.borrow()
            .as_ref()
            .and_then(|context| context.player_state(player_id))
            .map(|player| player.crew.clone())
            .unwrap_or_default()
    });

    let mut routed_target = target;
    let mut cursor_processed = false;
    for crew_id in crew {
        let route = with_host_context(None, |context| {
            let cursor = context
                .player_state(player_id)
                .and_then(|player| player.cursor);
            let object = context.get_world_object(crew_id)?;
            let selected = context
                .object_scope(crew_id)
                .map(ObjectScopeContext::selected)
                .unwrap_or(object.selected);
            Some((
                cursor == Some(crew_id),
                context.object_status_present(crew_id)
                    && selected
                    && Some(crew_id) != routed_target,
            ))
        });
        let Some((is_cursor, should_route)) = route else {
            continue;
        };
        cursor_processed |= is_cursor;
        if !should_route {
            continue;
        }

        let mut object_tx = tx;
        if command == CommandId::Put && target2.is_none() {
            let contents_count = with_host_context(0, |context| {
                let Some(object) = context.get_world_object(crew_id) else {
                    return 0;
                };
                let count = object
                    .contents()
                    .iter()
                    .filter(|content_id| {
                        context
                            .get_world_object(**content_id)
                            .is_some_and(|content| {
                                content.is_present()
                                    && (data == 0
                                        || context
                                            .object_effective_definition_id(**content_id)
                                            .and_then(|definition| {
                                                definition_id_to_c4id(&definition)
                                            })
                                            == Some(data))
                            })
                    })
                    .count();
                i32::try_from(count).unwrap_or(i32::MAX)
            });
            if contents_count == 0 {
                continue;
            }
            object_tx = object_tx.min(contents_count);
        }

        set_player_control_command(
            crew_id,
            player_object_command_request(command, routed_target, object_tx, ty, target2, data),
        );
        if command == CommandId::Construct {
            routed_target = Some(crew_id);
        }
    }

    // Always command a cursor outside Crew; unlike the crew loop this final
    // path deliberately does not apply Put's contents-count workaround.
    let cursor = HOST_CONTEXT.with(|cell| {
        cell.borrow()
            .as_ref()
            .and_then(|context| context.player_state(player_id))
            .and_then(|player| player.cursor)
    });
    if let Some(cursor) = cursor.filter(|cursor| {
        !cursor_processed
            && Some(*cursor) != routed_target
            && HOST_CONTEXT.with(|cell| {
                cell.borrow()
                    .as_ref()
                    .is_some_and(|context| context.object_status_present(*cursor))
            })
    }) {
        set_player_control_command(
            cursor,
            player_object_command_request(command, routed_target, tx, ty, target2, data),
        );
    }

    Ok(Value::Bool(true))
}

pub(crate) fn set_command(args: &[Value]) -> Result<Value, RuntimeError> {
    // C++ FnSetCommand leads with the object slot (pObj, szCommand, ...;
    // C4Script.cpp:840-844); 0/nil means the calling object. The
    // name-first form stays for the command-DSL fixtures.
    let mut args = args;
    let mut leading_target: Option<ObjectId> = None;
    let leads_with_object_slot = matches!(
        (args.first(), args.get(1)),
        (Some(Value::Object(_) | Value::Proplist(_)), _)
            | (Some(Value::Nil | Value::Int(0)), Some(Value::String(_)))
    );
    if leads_with_object_slot {
        leading_target = parse_object_reference_argument(&args[0], "SetCommand", "target")?;
        args = &args[1..];
    }
    if args.is_empty() {
        // C++ FnSetCommand: !szCommand -> false (C4Script.cpp:843-899).
        return Ok(Value::Bool(false));
    }
    let Some(target) = leading_target.or_else(active_object_id) else {
        return Ok(Value::Bool(false));
    };

    let command_name = match &args[0] {
        Value::String(name) if !name.is_empty() => name.clone(),
        Value::String(_) => {
            clear_command_stack_live(target);
            return Ok(Value::Bool(false));
        }
        // FnSetCommand returns before ClearCommands when szCommand is null.
        Value::Nil => return Ok(Value::Bool(false)),
        other => {
            return Err(RuntimeError::new(format!(
                "SetCommand: expected string for command name, got {}",
                other.type_name()
            )));
        }
    };

    let Some(command_id) = CommandId::from_name(&command_name) else {
        clear_command_stack_live(target);
        return Ok(Value::Bool(false));
    };

    // FnSetCommand's ignored ConvertTo(Int) preserves a nil/default Tx value
    // as nil. Retain the callback value separately from the normalized
    // CommandRequest used by the hardcoded command stack.
    let callback_tx = args.get(2).cloned().unwrap_or(Value::Nil);
    let request = parse_command_request(command_id, args, CommandArgLayout::Set, "SetCommand")?;
    Ok(Value::Bool(set_command_live(
        target,
        request,
        false,
        callback_tx,
    )))
}

pub(crate) fn add_command(args: &[Value]) -> Result<Value, RuntimeError> {
    // C++ FnAddCommand leads with the object slot (pObj, szCommand, ...;
    // C4Script.cpp:870-874); 0/nil means the calling object. The
    // name-first form stays for the command-DSL fixtures. A FOREIGN target
    // re-dispatches through the reentrancy seam like SetCommand and
    // AppendCommand so the write folds into that object's command stack.
    let mut args = args;
    let mut leading_target: Option<ObjectId> = None;
    let leads_with_object_slot = matches!(
        (args.first(), args.get(1)),
        (Some(Value::Object(_) | Value::Proplist(_)), _)
            | (Some(Value::Nil | Value::Int(0)), Some(Value::String(_)))
    );
    if leads_with_object_slot {
        leading_target = parse_object_reference_argument(&args[0], "AddCommand", "target")?;
        args = &args[1..];
    }
    if args.is_empty() {
        // C++ FnAddCommand: !szCommand -> false (C4Script.cpp:843-899).
        return Ok(Value::Bool(false));
    }
    if let Some(target) = leading_target {
        if active_object_id() != Some(target) {
            // Re-enter the target VM with the real FnAddCommand frame. The
            // object slot is part of the C++ native signature even though we
            // consumed it above to select the Rust world scope.
            let mut forwarded = Vec::with_capacity(args.len() + 1);
            forwarded.push(object_reference_value(target));
            forwarded.extend_from_slice(args);
            return match call_world_object_function(target, "AddCommand", &forwarded) {
                Some(result) => result,
                None => Ok(Value::Bool(false)),
            };
        }
    }

    let command_name = match &args[0] {
        Value::String(name) if !name.is_empty() => name.clone(),
        Value::String(_) | Value::Nil => return Ok(Value::Bool(false)),
        other => {
            return Err(RuntimeError::new(format!(
                "AddCommand: expected string for command name, got {}",
                other.type_name()
            )));
        }
    };

    let command_id = match CommandId::from_name(&command_name) {
        Some(id) => id,
        None => return Ok(Value::Bool(false)),
    };

    let request = parse_command_request(command_id, args, CommandArgLayout::Add, "AddCommand")?;

    try_with_host_context_mut("AddCommand requires an active engine context", |context| {
        let object = match context.object_context_mut() {
            Some(object) => object,
            None => return Ok(Value::Bool(false)),
        };
        let success = object.push_command_front(request);
        Ok(Value::Bool(success))
    })
}

pub(crate) fn append_command(args: &[Value]) -> Result<Value, RuntimeError> {
    // C++ FnAppendCommand leads with the object slot (pObj, szCommand, ...;
    // C4Script.cpp:894-916); 0/nil means the calling object. The name-first
    // form stays for the command-DSL fixtures. A FOREIGN target re-dispatches
    // through the reentrancy seam like SetCommand so the queued command lands
    // on the target's own stack.
    let mut args = args;
    let mut leading_target: Option<ObjectId> = None;
    let leads_with_object_slot = matches!(
        (args.first(), args.get(1)),
        (Some(Value::Object(_) | Value::Proplist(_)), _)
            | (Some(Value::Nil | Value::Int(0)), Some(Value::String(_)))
    );
    if leads_with_object_slot {
        leading_target = parse_object_reference_argument(&args[0], "AppendCommand", "target")?;
        args = &args[1..];
    }
    if args.is_empty() {
        // C++ FnAppendCommand: !szCommand -> false (C4Script.cpp:843-899).
        return Ok(Value::Bool(false));
    }
    if let Some(target) = leading_target {
        if active_object_id() != Some(target) {
            // Keep the explicit pObj slot when the compatibility layer must
            // re-enter another VM. Native parameter conversion sees the same
            // frame as C++ FnAppendCommand rather than the post-slot parser
            // view used below.
            let mut forwarded = Vec::with_capacity(args.len() + 1);
            forwarded.push(object_reference_value(target));
            forwarded.extend_from_slice(args);
            return match call_world_object_function(target, "AppendCommand", &forwarded) {
                Some(result) => result,
                None => Ok(Value::Bool(false)),
            };
        }
    }

    let command_name = match &args[0] {
        Value::String(name) if !name.is_empty() => name.clone(),
        Value::String(_) | Value::Nil => return Ok(Value::Bool(false)),
        other => {
            return Err(RuntimeError::new(format!(
                "AppendCommand: expected string for command name, got {}",
                other.type_name()
            )));
        }
    };

    let command_id = match CommandId::from_name(&command_name) {
        Some(id) => id,
        None => return Ok(Value::Bool(false)),
    };

    let request = parse_command_request(command_id, args, CommandArgLayout::Add, "AppendCommand")?;

    try_with_host_context_mut(
        "AppendCommand requires an active engine context",
        |context| {
            let object = match context.object_context_mut() {
                Some(object) => object,
                None => return Ok(Value::Bool(false)),
            };

            let success = object.push_command_back(request);
            Ok(Value::Bool(success))
        },
    )
}

pub(crate) fn parse_command_target(value: &Value) -> Result<Option<i32>, RuntimeError> {
    match value {
        Value::Object(_) => Ok(object_id_from_value(value).map(|id| truncate_to_i32(id.as_u64()))),
        Value::Proplist(map) => match map.get("id") {
            Some(Value::Int(id)) => Ok(Some(*id)),
            _ => Err(RuntimeError::new(
                "AddEffect: command target proplist must contain int `id`",
            )),
        },
        Value::Nil => Ok(None),
        Value::Int(value) if *value == 0 => Ok(None),
        other => Err(RuntimeError::new(format!(
            "AddEffect: expected object, proplist, nil, or 0 for command target, got {}",
            other.type_name()
        ))),
    }
}
