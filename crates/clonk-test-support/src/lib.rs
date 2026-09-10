//! Fixture builders shared by test harnesses; only dev-dependencies use this crate.

use clonk_engine::{
    ActionState, CommandDirection, CommandStackSnapshot, ComponentList, Direction, ObjectId,
    ObjectSnapshot, ObjectStatus, Vector2, DEFAULT_CATEGORY,
};
use clonk_graphics::{clonk_font::ClonkFont, Color};
use std::collections::HashMap;

pub fn packed_test_group(entries: &[(&str, bool, &[u8])]) -> Vec<u8> {
    const HEADER_SIZE: usize = 204;
    const ENTRY_SIZE: usize = 316;
    const GROUP_FILE_ID: &[u8] = b"RedWolf Design GrpFolder";

    fn put_i32(buffer: &mut [u8], offset: usize, value: i32) {
        buffer[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }

    let mut header = [0_u8; HEADER_SIZE];
    header[..GROUP_FILE_ID.len()].copy_from_slice(GROUP_FILE_ID);
    put_i32(&mut header, 28, 1);
    put_i32(&mut header, 32, 2);
    put_i32(
        &mut header,
        36,
        i32::try_from(entries.len()).expect("fixture length fits i32"),
    );
    for byte in &mut header {
        *byte ^= 237;
    }
    for chunk in header.chunks_exact_mut(3) {
        chunk.swap(0, 2);
    }

    let mut image = header.to_vec();
    let mut data_offset = 0_usize;
    for (name, child, data) in entries {
        let mut entry = [0_u8; ENTRY_SIZE];
        entry[..name.len()].copy_from_slice(name.as_bytes());
        put_i32(&mut entry, 264, i32::from(*child));
        put_i32(
            &mut entry,
            268,
            i32::try_from(data.len()).expect("fixture length fits i32"),
        );
        put_i32(
            &mut entry,
            276,
            i32::try_from(data_offset).expect("fixture length fits i32"),
        );
        image.extend_from_slice(&entry);
        data_offset += data.len();
    }
    for (_, _, data) in entries {
        image.extend_from_slice(data);
    }
    image
}

pub fn unit_width_font(characters: &str) -> ClonkFont {
    let mut font = ClonkFont::new(3);
    font.h_space = 0;
    for character in characters.chars() {
        font.add_glyph(
            character,
            clonk_graphics::clonk_font::GlyphCell {
                width: 1,
                pixels: vec![Color::opaque(255, 255, 255); 4],
            },
        );
    }
    font
}

pub fn make_object(id: u64, definition: &str, position: Vector2) -> ObjectSnapshot {
    ObjectSnapshot {
        id: ObjectId::new(id),
        definition_id: definition.to_string(),
        custom_name: None,
        position,
        velocity: Vector2::new(0, 0),
        rotation: 0,
        energy: 100,
        need_energy: false,
        construction: clonk_engine::FULL_CON,
        damage: 0,
        magic_energy: 0,
        magic_capacity: 0,
        action: ActionState::default(),
        direction: Direction::default(),
        command_direction: CommandDirection::default(),
        action_procedure: None,
        effects: Vec::new(),
        vertices: Vec::new(),
        current_shape: None,
        current_fire_top: None,
        contact_density: 50,
        own_vertices: None,
        vertex_contacts: Vec::new(),
        solid_mask_override: None,
        container: None,
        layer: None,
        visibility: 0,
        blit_mode: 0,
        color: 0,
        color_modulation: 0,
        picture_rect: Default::default(),
        contents: Vec::new(),
        components: ComponentList::new(),
        component_order: Vec::new(),
        status: ObjectStatus::Normal,
        owner: 1,
        controller: 1,
        category: DEFAULT_CATEGORY,
        crew_member: true,
        plr_view_range: 0,
        selected: false,
        alive: true,
        base_graphics: None,
        graphics_overlays: Vec::new(),
        draw_transform: None,
        command_queue: Vec::new(),
        command_stack: CommandStackSnapshot::default(),
        local_vars: HashMap::new(),
        in_liquid: false,
        mobile: false,
        ocf: 0,
        timer: 0,
        own_mass: 0,
        on_fire: false,
        fire_phase: 0,
        fire_caused_by: -1,
        info_physical: None,
        temporary_physical: None,
        physical_changes: Vec::new(),
        breath: 0,
        last_energy_loss_cause: -1,
        base: -1,
        fixed_position: None,
        fixed_velocity: None,
        rotation_velocity: None,
        fixed_rotation: None,
    }
}
