//! The Rust side of the GUI bridge differential (clonk-org/clonk-rs#1266).
//!
//! `parity/bridge/gui-harness.cpp` drives the pinned oracle's `RustGuiBridge`
//! wrapper through exactly this script over the C ABI and prints the same
//! lines; `parity/bridge/run-gui-differential.sh` diffs the two dumps. This
//! side never touches the ABI: it builds the tree through the safe API with
//! the font the bridge measures with, so agreement means the ABI carried every
//! id, rectangle, colour, payload, capture flag and action across intact.
//!
//! `--perturb` changes one label so the dumps must differ; the differential
//! uses it on each side in turn to prove the comparison is live.

use clonk_gui::ffi::default_font;
use clonk_gui::{
    DrawCommand, Gui, GuiAction, GuiEvent, GuiEventResult, KeyCode, LayoutConstraints, Point, Rect,
    Size,
};

const LABEL: &str = "Hello, bridge";
const PERTURBED_LABEL: &str = "Hello, bridge!";
const BUTTON: &str = "Press me";
const NESTED: &str = "Nested";
const BOUNDED: Size = Size::new(320.0, 240.0);

fn rect(rect: &Rect) -> String {
    format!(
        "{:.3},{:.3},{:.3},{:.3}",
        rect.origin.x, rect.origin.y, rect.size.width, rect.size.height
    )
}

fn dump_render(stage: &str, commands: &[DrawCommand]) {
    println!("render {stage} count={}", commands.len());
    for (index, command) in commands.iter().enumerate() {
        match command {
            DrawCommand::Quad { rect: r, color } => println!(
                "draw {index} quad rect={} color={},{},{},{}",
                rect(r),
                color.r,
                color.g,
                color.b,
                color.a
            ),
            DrawCommand::Text {
                rect: r,
                text,
                color,
                font_size,
                padding,
            } => println!(
                "draw {index} text rect={} color={},{},{},{} text=\"{text}\" font={font_size:.3} padding={padding:.3}",
                rect(r),
                color.r,
                color.g,
                color.b,
                color.a
            ),
            DrawCommand::Image { rect: r, image } => println!(
                "draw {index} image rect={} width={} height={} bytes={}",
                rect(r),
                image.width(),
                image.height(),
                image.pixels().len()
            ),
        }
    }
}

fn dump_result(event: &str, result: &GuiEventResult) {
    let actions: Vec<String> = result
        .actions
        .iter()
        .map(|(widget, action)| {
            let name = match action {
                GuiAction::Activate => "activate",
            };
            format!("{}:{name}", widget.to_raw())
        })
        .collect();
    println!(
        "event {event} captured={} actions=[{}]",
        result.captured,
        actions.join(",")
    );
}

/// The pointer target both sides derive the same way: the centre of the text
/// command that carries the button's caption. Neither side can ask the ABI
/// which widget a draw command belongs to, so the caption is the handle.
fn button_centre(commands: &[DrawCommand]) -> Point {
    commands
        .iter()
        .find_map(|command| match command {
            DrawCommand::Text { rect, text, .. } if text == BUTTON => Some(Point::new(
                rect.origin.x + rect.size.width / 2.0,
                rect.origin.y + rect.size.height / 2.0,
            )),
            _ => None,
        })
        .expect("the button caption is rendered")
}

fn main() {
    let perturb = std::env::args().any(|argument| argument == "--perturb");
    let mut gui = Gui::new(default_font());
    let root = gui.root();
    let column = gui.add_column(root, true);
    let label = gui.add_label(column, if perturb { PERTURBED_LABEL } else { LABEL });
    let button = gui.add_button(column, BUTTON);
    let inner = gui.add_column(column, false);
    let nested = gui.add_label(inner, NESTED);
    println!(
        "ids root={} column={} label={} button={} inner={} nested={}",
        root.to_raw(),
        column.to_raw(),
        label.to_raw(),
        button.to_raw(),
        inner.to_raw(),
        nested.to_raw()
    );

    gui.layout_with_constraints(LayoutConstraints::tight(BOUNDED));
    let bounded = gui.render();
    dump_render("bounded", &bounded);

    let centre = button_centre(&bounded);
    println!("pointer target={:.3},{:.3}", centre.x, centre.y);
    let outside = Point::new(-1.0, -1.0);
    let script: [(&str, GuiEvent); 9] = [
        ("move", GuiEvent::PointerMove { position: centre }),
        ("down", GuiEvent::PointerDown { position: centre }),
        ("up", GuiEvent::PointerUp { position: centre }),
        ("down-outside", GuiEvent::PointerDown { position: outside }),
        ("up-outside", GuiEvent::PointerUp { position: outside }),
        ("key-down-tab", GuiEvent::KeyDown { key: KeyCode::Tab }),
        (
            "key-down-enter",
            GuiEvent::KeyDown {
                key: KeyCode::Enter,
            },
        ),
        (
            "key-up-enter",
            GuiEvent::KeyUp {
                key: KeyCode::Enter,
            },
        ),
        (
            "key-down-escape",
            GuiEvent::KeyDown {
                key: KeyCode::Escape,
            },
        ),
    ];
    for (name, event) in script {
        dump_result(name, &gui.handle_event(event));
    }

    gui.layout_with_constraints(LayoutConstraints::unbounded());
    dump_render("unbounded", &gui.render());

    // The bridge's reset replaces the tree with a fresh one under the same
    // handle; the safe API's equivalent is a fresh Gui.
    gui = Gui::new(default_font());
    gui.layout_with_constraints(LayoutConstraints::tight(BOUNDED));
    dump_render("after-reset", &gui.render());
    println!("root-after-reset={}", gui.root().to_raw());
}
