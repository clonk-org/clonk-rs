//! `C4Object::DrawCommands` (src/C4Object.cpp:2940-3098) resolved into
//! presentation [`CommandIcon`]s for the viewport command rows
//! (C4Viewport::DrawCursorInfo, src/C4Viewport.cpp:947-962).
//!
//! The icon LIST order is the C++ draw-call order: bottom icons fill the
//! bottom bar right-to-left, side icons stack the side strip bottom-to-top.
//!
//! Known residual vs C++: function `Method=` descriptors default to
//! C4AUL_ControlMethod_All
//! (C4AulParse.cpp:366), so the Classic/JumpAndRun split reduces to
//! defined-or-not (no base-content def sets Method=).

use clonk_engine::{
    ocf, CommandDirection, DefinitionRect, ObjectId, ObjectSnapshot, SimulationSnapshot, FULL_CON,
};
use clonk_frontend::{CommandIcon, CommandImage, CommandOverlayIcon, ImageData};
use clonk_graphics::Color;

/// COM_* codes (src/C4Constants.h:173-235).
pub const COM_LEFT: u8 = 1;
pub const COM_RIGHT: u8 = 2;
pub const COM_UP: u8 = 3;
pub const COM_DOWN: u8 = 4;
pub const COM_THROW: u8 = 5;
pub const COM_DIG: u8 = 6;
pub const COM_SPECIAL: u8 = 7;
pub const COM_SPECIAL2: u8 = 8;
pub const COM_SINGLE: u8 = 64;
pub const COM_DOUBLE: u8 = 128;

/// `C4D_Grab_Put` / `C4D_Grab_Get` (src/C4Def.h:80-81).
pub const C4D_GRAB_PUT: i32 = 1;
pub const C4D_GRAB_GET: i32 = 2;

/// `C4D_Get` = StaticBack|Structure|Vehicle|Object|TradeLiving
/// (src/C4Def.h:43-47,65,115-116).
const C4D_GET: i32 = 1 | 2 | 4 | 16 | (1 << 16);

/// `ComOrder` (src/C4ObjectCom.cpp:786-798).
const COM_ORDER: [u8; 24] = [
    COM_LEFT,
    COM_RIGHT,
    COM_UP,
    COM_DOWN,
    COM_THROW,
    COM_DIG,
    COM_SPECIAL,
    COM_SPECIAL2,
    COM_LEFT | COM_SINGLE,
    COM_RIGHT | COM_SINGLE,
    COM_UP | COM_SINGLE,
    COM_DOWN | COM_SINGLE,
    COM_THROW | COM_SINGLE,
    COM_DIG | COM_SINGLE,
    COM_SPECIAL | COM_SINGLE,
    COM_SPECIAL2 | COM_SINGLE,
    COM_LEFT | COM_DOUBLE,
    COM_RIGHT | COM_DOUBLE,
    COM_UP | COM_DOUBLE,
    COM_DOWN | COM_DOUBLE,
    COM_THROW | COM_DOUBLE,
    COM_DIG | COM_DOUBLE,
    COM_SPECIAL | COM_DOUBLE,
    COM_SPECIAL2 | COM_DOUBLE,
];

/// `ComName` (src/C4ObjectCom.cpp:800-855) for the coms ComOrder yields.
fn com_name(com: u8) -> &'static str {
    match (
        com & !(COM_SINGLE | COM_DOUBLE),
        com & COM_SINGLE != 0,
        com & COM_DOUBLE != 0,
    ) {
        (COM_LEFT, false, false) => "Left",
        (COM_LEFT, true, false) => "LeftSingle",
        (COM_LEFT, false, true) => "LeftDouble",
        (COM_RIGHT, false, false) => "Right",
        (COM_RIGHT, true, false) => "RightSingle",
        (COM_RIGHT, false, true) => "RightDouble",
        (COM_UP, false, false) => "Up",
        (COM_UP, true, false) => "UpSingle",
        (COM_UP, false, true) => "UpDouble",
        (COM_DOWN, false, false) => "Down",
        (COM_DOWN, true, false) => "DownSingle",
        (COM_DOWN, false, true) => "DownDouble",
        (COM_THROW, false, false) => "Throw",
        (COM_THROW, true, false) => "ThrowSingle",
        (COM_THROW, false, true) => "ThrowDouble",
        (COM_DIG, false, false) => "Dig",
        (COM_DIG, true, false) => "DigSingle",
        (COM_DIG, false, true) => "DigDouble",
        (COM_SPECIAL, false, false) => "Special",
        (COM_SPECIAL, true, false) => "SpecialSingle",
        (COM_SPECIAL, false, true) => "SpecialDouble",
        (COM_SPECIAL2, false, false) => "Special2",
        (COM_SPECIAL2, true, false) => "Special2Single",
        (COM_SPECIAL2, false, true) => "Special2Double",
        _ => "Undefined",
    }
}

/// `Com2Control` (src/C4ObjectCom.cpp:857-877); CON_* indexes
/// (src/C4Constants.h:158-169).
pub fn com2control(com: u8) -> i32 {
    match com & !(COM_SINGLE | COM_DOUBLE) {
        COM_THROW => 3,
        COM_UP => 4,
        COM_DIG => 5,
        COM_LEFT => 6,
        COM_DOWN => 7,
        COM_RIGHT => 8,
        COM_SPECIAL => 10,
        COM_SPECIAL2 => 11,
        _ => 9, // CON_Menu
    }
}

/// A function's `Image=` descriptor (C4AulScriptFunc::idImage/iImagePhase,
/// C4AulParse.cpp:330-347) as GetControlDesc reports it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ImageAnnotation {
    /// `Image=ID[:phase]` — draw that def's picture phase
    /// (src/C4Object.cpp:4039-4055).
    Def { id: String, phase: i32 },
    /// `Image=Contents` (C4ID_Contents) — draw the first contents object's
    /// picture, else the own picture (src/C4Object.cpp:4056-4065).
    Contents,
}

/// The head of `func <name>` in raw script text: `<name>` preceded by the
/// `func` keyword and followed by `(` — the textual stand-in for
/// C4AulScript::GetSFunc while clonk-script drops descriptor metadata.
fn function_head(source: &str, function: &str) -> Option<usize> {
    let mut search = 0;
    while let Some(found) = source[search..].find(function) {
        let pos = search + found;
        search = pos + 1;
        let before = source[..pos].trim_end();
        if !before.ends_with("func") {
            continue;
        }
        let keyword_start = before.len() - "func".len();
        if keyword_start > 0 && !source.as_bytes()[keyword_start - 1].is_ascii_whitespace() {
            continue;
        }
        let after = source[pos + function.len()..].trim_start();
        if !after.starts_with('(') {
            continue;
        }
        return Some(pos + function.len());
    }
    None
}

/// Whether the script text defines `function` itself (child definitions
/// shadow `#include` parents like the C4Aul include merge).
pub fn source_defines_function(source: &str, function: &str) -> bool {
    function_head(source, function).is_some()
}

/// `GetControlDesc`'s image data (src/C4ScriptHost.cpp:151-172): the
/// `[Desc|Image=ID[:phase]|...]` descriptor block that heads the function
/// body (C4AulParse.cpp:301-375). None = function absent or no Image=.
pub fn control_image_annotation(source: &str, function: &str) -> Option<ImageAnnotation> {
    let head = function_head(source, function)?;
    let brace = head + source[head..].find('{')? + 1;
    // Skip whitespace and comments to the first statement.
    let mut rest = &source[brace..];
    loop {
        rest = rest.trim_start();
        if let Some(comment) = rest.strip_prefix("//") {
            rest = comment.split_once('\n').map(|(_, tail)| tail).unwrap_or("");
        } else if let Some(comment) = rest.strip_prefix("/*") {
            rest = comment.split_once("*/").map(|(_, tail)| tail).unwrap_or("");
        } else {
            break;
        }
    }
    let block = rest.strip_prefix('[')?;
    let block = &block[..block.find(']')?];
    block.split('|').find_map(|segment| {
        let value = segment.trim().strip_prefix("Image=")?;
        if value == "Contents" {
            // C4AUL_Contents (C4AulParse.cpp:333-334).
            Some(ImageAnnotation::Contents)
        } else {
            let (id, phase) = value
                .split_once(':')
                .map(|(id, phase)| (id, phase.trim().parse().unwrap_or(0)))
                .unwrap_or((value, 0));
            Some(ImageAnnotation::Def {
                id: id.trim().to_string(),
                phase,
            })
        }
    })
}

/// The engine/asset queries DrawCommands needs — implemented over the
/// Engine by the app, over stubs in tests.
pub trait CommandContext {
    /// `GetSFunc(name)` on the def script (C4ScriptHost::GetControlMethodMask,
    /// src/C4ScriptHost.cpp:100-120; '~' failsafe prefixes are stripped by
    /// GetSFunc, src/C4Aul.cpp:311-315).
    fn def_has_function(&self, definition_id: &str, function: &str) -> bool;
    /// The def picture the image cells draw (C4Def::Draw / DrawPicture).
    fn def_picture(&self, definition_id: &str) -> Option<ImageData>;
    /// A picture PHASE — C4Def::Draw's iPhaseX offsets the picture rect
    /// by phase widths (src/C4Object.cpp:4055).
    fn def_picture_phase(&self, definition_id: &str, phase: i32) -> Option<ImageData> {
        let _ = phase;
        self.def_picture(definition_id)
    }
    /// `GetControlDesc`'s Image= descriptor for the def's `function`,
    /// resolved across the #include chain like GetSFunc
    /// (src/C4ScriptHost.cpp:151-172).
    fn control_image(&self, definition_id: &str, function: &str) -> Option<ImageAnnotation> {
        let _ = (definition_id, function);
        None
    }
    /// `GetControlDesc`'s first descriptor segment. `Some("")` is distinct
    /// from no descriptor and suppresses the receiver-name fallback.
    fn control_description(&self, definition_id: &str, function: &str) -> Option<String> {
        let _ = (definition_id, function);
        None
    }
    /// Effective `C4Object::GetName()` presentation text.
    fn object_name(&self, object: &ObjectSnapshot) -> String {
        object
            .custom_name
            .clone()
            .unwrap_or_else(|| object.definition_id.clone())
    }
    /// `LoadResStr` plus its sequential `%s` substitutions.
    fn localized_caption(&self, key: &str, fallback: &str, arguments: &[&str]) -> String {
        let _ = key;
        let mut caption = fallback.to_owned();
        for argument in arguments {
            let Some(index) = caption.find("%s") else {
                break;
            };
            caption.replace_range(index..index + 2, argument);
        }
        caption
    }
    /// `Def->GrabPutGet` (src/C4Def.cpp:364-373).
    fn def_grab_put_get(&self, definition_id: &str) -> i32;
    /// The def Shape rect for the AtObject enclose test.
    fn def_shape(&self, definition_id: &str) -> Option<DefinitionRect>;
    /// `PlrControlKeyName(iPlayer, iControl, true)` (src/C4Viewport.cpp:1363).
    fn key_label(&self, owner: i32, control: i32) -> String;
    /// `C4Object::Base` of the container — None until the engine models
    /// per-object bases (gates the contained Buy/Sell commands,
    /// src/C4Object.cpp:3020-3034).
    fn base_owner(&self, container: &ObjectSnapshot) -> Option<i32>;
    /// `BASEFUNC_Sell` / `BASEFUNC_Buy` (Game.C4S.Game.Realism.BaseFunctionality).
    fn base_sell_enabled(&self) -> bool;
    fn base_buy_enabled(&self) -> bool;
    /// The player color for DrawMenuSymbol's flag (src/C4Menu.cpp:47-48).
    fn owner_color(&self, owner: i32) -> Color;
}

/// `DrawCommandQuery` (src/C4Object.cpp:2924-2938): the def script defines
/// the control function and the controller resolves to a player. Without
/// `Method=` descriptors every defined function is
/// C4AUL_ControlMethod_All, which passes for both control styles.
fn draw_command_query(
    snapshot: &SimulationSnapshot,
    ctx: &impl CommandContext,
    controller: i32,
    definition_id: &str,
    function: &str,
) -> bool {
    snapshot
        .players
        .iter()
        .any(|player| player.id == controller)
        && ctx.def_has_function(definition_id, function)
}

/// The def Shape scaled by Con like C4Shape::Jolt (height axis only) for
/// under-construction objects.
fn scaled_shape(object: &ObjectSnapshot, ctx: &impl CommandContext) -> Option<DefinitionRect> {
    let mut shape = ctx.def_shape(&object.definition_id)?;
    let con = object.construction.clamp(0, FULL_CON);
    if con < FULL_CON {
        shape.y = shape.y * con / FULL_CON;
        shape.height = shape.height * con / FULL_CON;
    }
    Some(shape)
}

/// `C4GameObjects::AtObject(x, y, OCF_Construct, cursor)`
/// (src/C4GameObjects.cpp:234-252) with the `C4Object::At` enclose test
/// (Shape rect + the 18px build-top padding, src/C4Object.h:340): the first
/// construction site under the cursor; an OCF_Exclusive object at the point
/// blocks the search.
fn construction_site_at<'a>(
    snapshot: &'a SimulationSnapshot,
    cursor: &ObjectSnapshot,
    ctx: &impl CommandContext,
) -> Option<&'a ObjectSnapshot> {
    let (x, y) = (cursor.position.x, cursor.position.y);
    for object in &snapshot.objects {
        if object.id == cursor.id || !object.status.is_active() || object.container.is_some() {
            continue;
        }
        if object.ocf & (ocf::CONSTRUCT | ocf::EXCLUSIVE) == 0 {
            continue;
        }
        let Some(shape) = scaled_shape(object, ctx) else {
            continue;
        };
        let addtop = (18 - shape.height).max(0);
        let left = object.position.x + shape.x;
        let top = object.position.y + shape.y - addtop;
        if !(left..left + shape.width).contains(&x)
            || !(top..top + shape.height + addtop).contains(&y)
        {
            continue;
        }
        if object.ocf & ocf::CONSTRUCT != 0 {
            return Some(object);
        }
        // EXCLUSIVE block (src/C4GameObjects.cpp:245-248).
        return None;
    }
    None
}

/// `C4ObjectList::ListIDCount(C4D_Get)` (src/C4ObjectList.cpp:83-107):
/// distinct def ids among the contents whose def Category intersects C4D_Get.
fn list_id_count_get(snapshot: &SimulationSnapshot, contents: &[ObjectId]) -> usize {
    contents
        .iter()
        .filter_map(|id| snapshot.object(*id))
        .filter(|object| object.status.is_active() && object.category & C4D_GET != 0)
        .map(|object| object.definition_id.as_str())
        .collect::<std::collections::HashSet<_>>()
        .len()
}

/// `Contents.GetObject()` — the first live contents object.
fn first_contents<'a>(
    snapshot: &'a SimulationSnapshot,
    contents: &[ObjectId],
) -> Option<&'a ObjectSnapshot> {
    contents
        .iter()
        .filter_map(|id| snapshot.object(*id))
        .find(|object| object.status.is_active())
}

fn procedure_is(cursor: &ObjectSnapshot, name: &str) -> bool {
    cursor
        .action_procedure
        .as_deref()
        .is_some_and(|procedure| procedure.eq_ignore_ascii_case(name))
}

/// `C4Object::DrawCommands` (src/C4Object.cpp:2940-3098). The caller skips
/// this entirely while the cursor's menu is active (src/C4Object.cpp:2952).
pub fn build_cursor_commands(
    snapshot: &SimulationSnapshot,
    cursor_id: ObjectId,
    ctx: &impl CommandContext,
) -> Vec<CommandIcon> {
    let Some(cursor) = snapshot.object(cursor_id) else {
        return Vec::new();
    };
    let mut icons = Vec::new();
    let controller = cursor.controller;
    let owner = cursor.owner;
    let mut contained_down_override = false;
    let mut contained_left_override = false;
    let mut contained_right_override = false;
    let mut contents_activation_override = false;

    let bottom = |com: u8, caption: String, image: CommandImage| CommandIcon {
        com,
        key_label: ctx.key_label(owner, com2control(com)),
        side: false,
        caption,
        image,
    };
    let side = |com: u8, caption: String, image: CommandImage| CommandIcon {
        com,
        key_label: ctx.key_label(owner, com2control(com)),
        side: true,
        caption,
        image,
    };

    let scripted_caption = |receiver: &ObjectSnapshot, function: &str| {
        ctx.control_description(&receiver.definition_id, function)
            .unwrap_or_else(|| ctx.object_name(receiver))
    };

    // DrawCommand's default image chain (src/C4Object.cpp:4022-4068): the
    // function's Image= descriptor def (with phase), Image=Contents ->
    // first contents picture, else the receiver's own picture.
    let function_image =
        |receiver_def: &str, receiver_contents: &[ObjectId], function: &str| match ctx
            .control_image(receiver_def, function)
        {
            Some(ImageAnnotation::Def { id, phase }) => {
                CommandImage::Picture(ctx.def_picture_phase(&id, phase))
            }
            Some(ImageAnnotation::Contents) => CommandImage::Picture(
                first_contents(snapshot, receiver_contents)
                    .and_then(|thing| ctx.def_picture(&thing.definition_id))
                    .or_else(|| ctx.def_picture(receiver_def)),
            ),
            None => CommandImage::Picture(ctx.def_picture(receiver_def)),
        };

    // Build at a construction site (src/C4Object.cpp:2954-2963): standing
    // (ComDir Stop, DFA_WALK) on an OCF_Construct object; Jump'n'Run
    // players get the plain COM_Down.
    if cursor.command_direction == CommandDirection::Stop && procedure_is(cursor, "WALK") {
        if let Some(site) = construction_site_at(snapshot, cursor, ctx) {
            let jump_and_run = snapshot
                .players
                .iter()
                .find(|player| player.id == controller)
                .map(|player| player.control.control_style)
                .unwrap_or(false);
            let com = if jump_and_run {
                COM_DOWN
            } else {
                COM_DOWN | COM_DOUBLE
            };
            let site_name = ctx.object_name(site);
            icons.push(bottom(
                com,
                ctx.localized_caption("IDS_CON_BUILD", "Build %s.", &[site_name.as_str()]),
                CommandImage::Composite {
                    picture: ctx.def_picture(&site.definition_id),
                    icon: CommandOverlayIcon::Build,
                },
            ));
        }
    }

    // Grab target control (src/C4Object.cpp:2966-2997).
    if procedure_is(cursor, "PUSH") {
        if let Some(target) = cursor.action.target.and_then(|id| snapshot.object(id)) {
            for cnt in (0..COM_ORDER.len()).rev() {
                let com = COM_ORDER[cnt];
                let function = format!("Control{}", com_name(com));
                if draw_command_query(snapshot, ctx, controller, &target.definition_id, &function) {
                    let image = function_image(&target.definition_id, &target.contents, &function);
                    icons.push(bottom(com, scripted_caption(target, &function), image));
                } else if com == COM_DOWN | COM_DOUBLE {
                    // Let go (src/C4Object.cpp:2976-2979).
                    let target_name = ctx.object_name(target);
                    icons.push(bottom(
                        com,
                        ctx.localized_caption(
                            "IDS_CON_UNGRAB",
                            "Let go of %s.",
                            &[target_name.as_str()],
                        ),
                        CommandImage::Composite {
                            picture: ctx.def_picture(&target.definition_id),
                            icon: CommandOverlayIcon::Hand(6),
                        },
                    ));
                } else if com == COM_THROW {
                    let grab_put_get = ctx.def_grab_put_get(&target.definition_id);
                    if let Some(thing) = first_contents(snapshot, &cursor.contents)
                        .filter(|_| grab_put_get & C4D_GRAB_PUT != 0)
                    {
                        // Put (src/C4Object.cpp:2983-2988).
                        let thing_name = ctx.object_name(thing);
                        let target_name = ctx.object_name(target);
                        icons.push(bottom(
                            com,
                            ctx.localized_caption(
                                "IDS_CON_PUT",
                                "Drop %s in %s",
                                &[thing_name.as_str(), target_name.as_str()],
                            ),
                            CommandImage::Composite {
                                picture: ctx.def_picture(&thing.definition_id),
                                icon: CommandOverlayIcon::Hand(0),
                            },
                        ));
                    } else if list_id_count_get(snapshot, &target.contents) > 0
                        && grab_put_get & C4D_GRAB_GET != 0
                    {
                        // Get (src/C4Object.cpp:2990-2995).
                        let target_name = ctx.object_name(target);
                        icons.push(bottom(
                            com,
                            ctx.localized_caption(
                                "IDS_CON_GET",
                                "Take object from %s.",
                                &[target_name.as_str()],
                            ),
                            CommandImage::Composite {
                                picture: ctx.def_picture(&target.definition_id),
                                icon: CommandOverlayIcon::Hand(1),
                            },
                        ));
                    }
                }
            }
        }
    }

    // Contained control (src/C4Object.cpp:3000-3068).
    if let Some(container) = cursor.container.and_then(|id| snapshot.object(id)) {
        for cnt in (0..COM_ORDER.len()).rev() {
            let com = COM_ORDER[cnt];
            let function = format!("Contained{}", com_name(com));
            if draw_command_query(
                snapshot,
                ctx,
                controller,
                &container.definition_id,
                &function,
            ) {
                let image =
                    function_image(&container.definition_id, &container.contents, &function);
                icons.push(bottom(com, scripted_caption(container, &function), image));
                match com2control(com) {
                    7 => contained_down_override = true,  // CON_Down
                    6 => contained_left_override = true,  // CON_Left
                    8 => contained_right_override = true, // CON_Right
                    _ => {}
                }
            }
        }
        // Contained exit (src/C4Object.cpp:3013-3018).
        if !contained_down_override {
            icons.push(bottom(
                COM_DOWN,
                ctx.localized_caption("IDS_CON_EXIT", "Exit building", &[]),
                CommandImage::Exit,
            ));
        }
        // Contained base commands (src/C4Object.cpp:3020-3034).
        if let Some(base) = ctx.base_owner(container) {
            if ctx.base_sell_enabled() {
                icons.push(bottom(
                    COM_DIG,
                    ctx.localized_caption("IDS_CON_SELL", "Sell", &[]),
                    CommandImage::SellMenu {
                        owner_color: ctx.owner_color(base),
                    },
                ));
            }
            if ctx.base_buy_enabled() {
                icons.push(bottom(
                    COM_UP,
                    ctx.localized_caption("IDS_CON_BUY", "Buy", &[]),
                    CommandImage::BuyMenu {
                        owner_color: ctx.owner_color(base),
                    },
                ));
            }
        }
        // Contained take (src/C4Object.cpp:3037-3054).
        let n_contents = list_id_count_get(snapshot, &container.contents);
        if n_contents > 0 {
            if !contained_right_override {
                // Direct get ("Take2").
                let container_name = ctx.object_name(container);
                icons.push(bottom(
                    COM_RIGHT,
                    ctx.localized_caption(
                        "IDS_CON_GET",
                        "Take object from %s.",
                        &[container_name.as_str()],
                    ),
                    CommandImage::Composite {
                        picture: ctx.def_picture(&container.definition_id),
                        icon: CommandOverlayIcon::Hand(1),
                    },
                ));
            }
            if !contained_left_override {
                // Get ("Take").
                let container_name = ctx.object_name(container);
                icons.push(bottom(
                    COM_LEFT,
                    ctx.localized_caption(
                        "IDS_CON_ACTIVATEFROM",
                        "Activate object in %s",
                        &[container_name.as_str()],
                    ),
                    CommandImage::Composite {
                        picture: ctx.def_picture(&container.definition_id),
                        icon: CommandOverlayIcon::Hand(0),
                    },
                ));
            }
        }
        // Contained put & activate (src/C4Object.cpp:3055-3068).
        if let Some(thing) = first_contents(snapshot, &cursor.contents) {
            let thing_name = ctx.object_name(thing);
            let container_name = ctx.object_name(container);
            icons.push(bottom(
                COM_THROW,
                ctx.localized_caption(
                    "IDS_CON_PUT",
                    "Drop %s in %s",
                    &[thing_name.as_str(), container_name.as_str()],
                ),
                CommandImage::Composite {
                    picture: ctx.def_picture(&thing.definition_id),
                    icon: CommandOverlayIcon::Hand(0),
                },
            ));
        } else if n_contents > 0 {
            let container_name = ctx.object_name(container);
            icons.push(bottom(
                COM_THROW,
                ctx.localized_caption(
                    "IDS_CON_ACTIVATEFROM",
                    "Activate object in %s",
                    &[container_name.as_str()],
                ),
                CommandImage::Composite {
                    picture: ctx.def_picture(&container.definition_id),
                    icon: CommandOverlayIcon::Hand(0),
                },
            ));
        }
    } else {
        // Contents activation (src/C4Object.cpp:3072-3081).
        if procedure_is(cursor, "WALK")
            || procedure_is(cursor, "SWIM")
            || procedure_is(cursor, "DIG")
        {
            if let Some(thing) = first_contents(snapshot, &cursor.contents) {
                if draw_command_query(snapshot, ctx, controller, &thing.definition_id, "Activate") {
                    let image = function_image(&thing.definition_id, &thing.contents, "Activate");
                    icons.push(bottom(
                        COM_DIG | COM_DOUBLE,
                        scripted_caption(thing, "Activate"),
                        image,
                    ));
                    contents_activation_override = true;
                }
            }
        }
        // Self activation (src/C4Object.cpp:3083-3087).
        if !contents_activation_override
            && (procedure_is(cursor, "WALK")
                || procedure_is(cursor, "SWIM")
                || procedure_is(cursor, "FLOAT"))
            && draw_command_query(snapshot, ctx, controller, &cursor.definition_id, "Activate")
        {
            let image = function_image(&cursor.definition_id, &cursor.contents, "Activate");
            icons.push(side(
                COM_DIG | COM_DOUBLE,
                scripted_caption(cursor, "Activate"),
                image,
            ));
        }
    }

    // Self special control (src/C4Object.cpp:3090-3098): hardcoded ComOrder
    // indexes 6,7 then 14,15 then 22,23 — Special/Special2 base, Single,
    // Double variants.
    for cnt in [6usize, 7, 14, 15, 22, 23] {
        let com = COM_ORDER[cnt];
        let function = format!("Control{}", com_name(com));
        if draw_command_query(snapshot, ctx, controller, &cursor.definition_id, &function) {
            let image = function_image(&cursor.definition_id, &cursor.contents, &function);
            icons.push(side(com, scripted_caption(cursor, &function), image));
        }
    }

    icons
}

#[cfg(test)]
mod tests {
    use super::*;
    use clonk_engine::{
        ActionState, CommandStackSnapshot, ObjectId, PlayerState, SimulationSnapshot, Vector2,
    };
    use std::collections::{HashMap, HashSet};

    struct StubContext {
        functions: HashSet<(String, String)>,
        grab_put_get: HashMap<String, i32>,
        shapes: HashMap<String, DefinitionRect>,
        base_owner: Option<i32>,
    }

    impl StubContext {
        fn new() -> Self {
            Self {
                functions: HashSet::new(),
                grab_put_get: HashMap::new(),
                shapes: HashMap::new(),
                base_owner: None,
            }
        }

        fn with_function(mut self, definition_id: &str, function: &str) -> Self {
            self.functions
                .insert((definition_id.to_string(), function.to_string()));
            self
        }
    }

    impl CommandContext for StubContext {
        fn def_has_function(&self, definition_id: &str, function: &str) -> bool {
            self.functions
                .contains(&(definition_id.to_string(), function.to_string()))
        }

        fn def_picture(&self, _definition_id: &str) -> Option<ImageData> {
            None
        }

        fn def_grab_put_get(&self, definition_id: &str) -> i32 {
            self.grab_put_get.get(definition_id).copied().unwrap_or(0)
        }

        fn def_shape(&self, definition_id: &str) -> Option<DefinitionRect> {
            self.shapes.get(definition_id).copied()
        }

        fn key_label(&self, _owner: i32, control: i32) -> String {
            format!("K{control}")
        }

        fn base_owner(&self, _container: &ObjectSnapshot) -> Option<i32> {
            self.base_owner
        }

        fn base_sell_enabled(&self) -> bool {
            true
        }

        fn base_buy_enabled(&self) -> bool {
            true
        }

        fn owner_color(&self, _owner: i32) -> Color {
            Color::opaque(0, 100, 200)
        }
    }

    fn object(id: u64, definition_id: &str) -> ObjectSnapshot {
        ObjectSnapshot {
            id: ObjectId::new(id),
            definition_id: definition_id.to_string(),
            custom_name: None,
            position: Vector2::new(100, 100),
            velocity: Vector2::ZERO,
            rotation: 0,
            energy: 100,
            need_energy: false,
            construction: FULL_CON,
            damage: 0,
            magic_energy: 0,
            magic_capacity: 0,
            action: ActionState::default(),
            direction: Default::default(),
            command_direction: Default::default(),
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
            components: HashMap::new(),
            component_order: Vec::new(),
            status: Default::default(),
            owner: 0,
            controller: 0,
            category: clonk_engine::DEFAULT_CATEGORY,
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

    fn snapshot_with(objects: Vec<ObjectSnapshot>) -> SimulationSnapshot {
        let mut snapshot = SimulationSnapshot {
            frame: 0,
            game_time: 0,
            game_over: false,
            round_results: Default::default(),
            league_name: Vec::new(),
            player_info_league_progress_data: Default::default(),
            player_info_league_scores: Default::default(),
            physics: None,
            objects,
            render_order: Vec::new(),
            environment: Default::default(),
            sky: None,
            weather_events: Vec::new(),
            global_effects: Vec::new(),
            script_globals: Default::default(),
            particles: Vec::new(),
            players: Vec::new(),
            fow_players: Default::default(),
            crew_selection: Default::default(),
            crew_roles: Default::default(),
            known_crew_owners: Vec::new(),
            eliminated_crew_owners: Vec::new(),
            landscape: None,
            rng: Default::default(),
            surfaces: Vec::new(),
            hud: Default::default(),
            controls: Vec::new(),
            network_packets: Vec::new(),
            definition_categories: Default::default(),
            definition_closed_containers: Default::default(),
            definition_lines: Default::default(),
            transfer_zones: Vec::new(),
            pathfinder_debug: Default::default(),
            menu_requests: Vec::new(),
            audio: Vec::new(),
        };
        snapshot.players.push(PlayerState {
            id: 0,
            ..PlayerState::default()
        });
        snapshot
    }

    #[test]
    fn control_image_annotation_reads_the_descriptor_block() {
        // Function descriptors (C4AulParse.cpp:301-375): the
        // `[Desc|Image=ID[:phase]|...]` block heading the function body sets
        // pFn->idImage/iImagePhase, which GetControlDesc hands to
        // DrawCommand (src/C4ScriptHost.cpp:151-172).
        let source = "/* Cowboy */\r\n#strict\r\n#include CLNK\r\n\r\n\
            public func ControlSpecial()            // Inventarwechsel\r\n  {\r\n  \
            [$CtrlInventoryDesc$|Image=CXIV]\r\n  return(1);\r\n  }\r\n\r\n\
            protected func ControlSpecial2()\r\n{\r\n  [$CtrlMenuDesc$|Image=SG01:3]\r\n}\r\n\
            func ControlUp() { return(1); }\r\n\
            func Activate() { [$A$|Image=Contents] }\r\n";
        assert_eq!(
            control_image_annotation(source, "ControlSpecial"),
            Some(ImageAnnotation::Def {
                id: "CXIV".to_string(),
                phase: 0
            })
        );
        assert_eq!(
            control_image_annotation(source, "ControlSpecial2"),
            Some(ImageAnnotation::Def {
                id: "SG01".to_string(),
                phase: 3
            })
        );
        assert_eq!(control_image_annotation(source, "ControlUp"), None);
        assert_eq!(
            control_image_annotation(source, "Activate"),
            Some(ImageAnnotation::Contents)
        );
        assert_eq!(control_image_annotation(source, "ControlDown"), None);

        assert!(source_defines_function(source, "ControlSpecial"));
        assert!(source_defines_function(source, "ControlSpecial2"));
        assert!(!source_defines_function(source, "ControlDown"));
        // No substring hit: ControlSpecial must not match ControlSpecial2's head.
        assert!(!source_defines_function(
            "func ControlSpecial2() {}",
            "ControlSpecial"
        ));
    }

    #[test]
    fn special_control_functions_fill_the_side_rows() {
        // Self special control (src/C4Object.cpp:3090-3098): ComOrder
        // indexes 6,7,14,15,22,23 in ascending order against the own def's
        // Control<Com> functions; icons go to the side strip.
        let mut clonk = object(1, "CLNK");
        clonk.action_procedure = Some("WALK".to_string());
        let snapshot = snapshot_with(vec![clonk]);
        let ctx = StubContext::new()
            .with_function("CLNK", "ControlSpecial")
            .with_function("CLNK", "ControlSpecial2Double");

        let icons = build_cursor_commands(&snapshot, ObjectId::new(1), &ctx);
        let coms: Vec<(u8, bool)> = icons.iter().map(|icon| (icon.com, icon.side)).collect();
        assert_eq!(
            coms,
            vec![(COM_SPECIAL, true), (COM_SPECIAL2 | COM_DOUBLE, true)]
        );
        assert_eq!(icons[0].key_label, "K10", "CON_Special key label");
    }

    #[test]
    fn contained_crew_gets_exit_take_and_activate_commands() {
        // Contained branch (src/C4Object.cpp:3000-3068): Exit (no
        // ContainedDown), Take2/Take for C4D_Get contents, and the
        // activate-from COM_Throw when the crew carries nothing.
        let mut clonk = object(1, "CLNK");
        clonk.container = Some(ObjectId::new(2));
        let mut hut = object(2, "HUT1");
        hut.crew_member = false;
        hut.contents = vec![ObjectId::new(3)];
        let mut loot = object(3, "LOOT");
        loot.crew_member = false;
        loot.category = 16; // C4D_Object
        loot.container = Some(ObjectId::new(2));
        let snapshot = snapshot_with(vec![clonk, hut, loot]);
        let ctx = StubContext::new();

        let icons = build_cursor_commands(&snapshot, ObjectId::new(1), &ctx);
        let coms: Vec<u8> = icons.iter().map(|icon| icon.com).collect();
        assert_eq!(coms, vec![COM_DOWN, COM_RIGHT, COM_LEFT, COM_THROW]);
        assert_eq!(icons[0].image, CommandImage::Exit);
        assert!(matches!(
            icons[1].image,
            CommandImage::Composite {
                icon: CommandOverlayIcon::Hand(1),
                ..
            }
        ));
        assert!(icons.iter().all(|icon| !icon.side));
    }

    #[test]
    fn contained_down_function_overrides_the_exit_command() {
        // `Com2Control(ComOrder(cnt)) == CON_Down` sets the override that
        // suppresses the exit icon (src/C4Object.cpp:3005-3018).
        let mut clonk = object(1, "CLNK");
        clonk.container = Some(ObjectId::new(2));
        let mut boat = object(2, "BOAT");
        boat.crew_member = false;
        let snapshot = snapshot_with(vec![clonk, boat]);
        let ctx = StubContext::new().with_function("BOAT", "ContainedDown");

        let icons = build_cursor_commands(&snapshot, ObjectId::new(1), &ctx);
        assert_eq!(icons.len(), 1);
        assert_eq!(icons[0].com, COM_DOWN);
        assert!(
            !matches!(icons[0].image, CommandImage::Exit),
            "ContainedDown replaces the exit icon"
        );
    }

    #[test]
    fn pushing_a_vehicle_offers_control_ungrab_and_get() {
        // Grab target control (src/C4Object.cpp:2966-2997), descending
        // ComOrder: defined Control<Com> icons, the COM_Down_D let-go with
        // fctHand phase 6, and the COM_Throw get (hand phase 1) for
        // C4D_Grab_Get targets with C4D_Get contents.
        let mut clonk = object(1, "CLNK");
        clonk.action_procedure = Some("PUSH".to_string());
        clonk.action.target = Some(ObjectId::new(2));
        let mut lorry = object(2, "LORY");
        lorry.crew_member = false;
        lorry.contents = vec![ObjectId::new(3)];
        let mut rock = object(3, "ROCK");
        rock.crew_member = false;
        rock.category = 16;
        rock.container = Some(ObjectId::new(2));
        let snapshot = snapshot_with(vec![clonk, lorry, rock]);
        let mut ctx = StubContext::new().with_function("LORY", "ControlDig");
        ctx.grab_put_get.insert("LORY".to_string(), C4D_GRAB_GET);

        let icons = build_cursor_commands(&snapshot, ObjectId::new(1), &ctx);
        let coms: Vec<u8> = icons.iter().map(|icon| icon.com).collect();
        // Descending ComOrder: index 19 (Down_D let-go), 5 (ControlDig), 4 (Throw get).
        assert_eq!(coms, vec![COM_DOWN | COM_DOUBLE, COM_DIG, COM_THROW]);
        assert!(matches!(
            icons[0].image,
            CommandImage::Composite {
                icon: CommandOverlayIcon::Hand(6),
                ..
            }
        ));
        assert!(matches!(
            icons[2].image,
            CommandImage::Composite {
                icon: CommandOverlayIcon::Hand(1),
                ..
            }
        ));
    }

    #[test]
    fn standing_on_a_construction_site_offers_build() {
        // Build (src/C4Object.cpp:2954-2963): DFA_WALK + COMD_Stop over an
        // OCF_Construct object -> COM_Down_D with the site picture and
        // fctBuild composite.
        let mut clonk = object(1, "CLNK");
        clonk.action_procedure = Some("WALK".to_string());
        let mut site = object(2, "SITE");
        site.crew_member = false;
        site.ocf = ocf::CONSTRUCT;
        site.position = Vector2::new(100, 110);
        let snapshot = snapshot_with(vec![clonk, site]);
        let mut ctx = StubContext::new();
        ctx.shapes
            .insert("SITE".to_string(), DefinitionRect::new(-10, -10, 20, 20));

        let icons = build_cursor_commands(&snapshot, ObjectId::new(1), &ctx);
        assert_eq!(icons.len(), 1);
        assert_eq!(icons[0].com, COM_DOWN | COM_DOUBLE);
        assert!(matches!(
            icons[0].image,
            CommandImage::Composite {
                icon: CommandOverlayIcon::Build,
                ..
            }
        ));
    }

    #[test]
    fn contents_activation_overrides_self_activation() {
        // Contents activation flags fContentsActivationOverride
        // (src/C4Object.cpp:3072-3087): the carried item's Activate wins
        // over the crew's own side-row Activate.
        let mut clonk = object(1, "CLNK");
        clonk.action_procedure = Some("WALK".to_string());
        clonk.contents = vec![ObjectId::new(2)];
        let mut flint = object(2, "FLNT");
        flint.crew_member = false;
        flint.container = Some(ObjectId::new(1));
        let snapshot = snapshot_with(vec![clonk, flint]);
        let ctx = StubContext::new()
            .with_function("FLNT", "Activate")
            .with_function("CLNK", "Activate");

        let icons = build_cursor_commands(&snapshot, ObjectId::new(1), &ctx);
        assert_eq!(icons.len(), 1);
        assert_eq!(icons[0].com, COM_DIG | COM_DOUBLE);
        assert!(!icons[0].side, "contents activation goes to the bottom bar");
    }
}
