use super::*;

fn dialog_item(caption: String) -> ObjectMenuItem {
    ObjectMenuItem {
        caption,
        info_caption: String::new(),
        command: String::new(),
        command2: String::new(),
        count: 0,
        item_id: "NONE".into(),
        symbol: ObjectMenuSymbol::Definition,
        image: ObjectMenuImage::Definition,
        presentation_definition_id: None,
        picture_snapshot: None,
        picture_object: None,
        components: Vec::new(),
        selectable: false,
        value: None,
        text_display_progress: 0,
    }
}

#[test]
fn menu_text_progress_and_info_caption_use_native_c4_bytes() {
    let mut item = dialog_item(clonk_script::c4_string_from_bytes(&[0xff, b'Z']));
    let mut amount = 1;
    item.do_text_progress(&mut amount);
    assert_eq!(item.text_display_progress, 1);
    amount = 1;
    item.do_text_progress(&mut amount);
    assert_eq!(item.text_display_progress, -1);

    let raw = clonk_script::c4_string_from_bytes(&[0xff, b'\n', 0, b'X']);
    assert_eq!(
        clonk_script::c4_string_bytes(&normalize_menu_info_caption(raw)),
        [0xff, b' ']
    );
    let overlong = clonk_script::c4_string_from_bytes(&vec![0xff; C4_MAX_TITLE + 1]);
    assert_eq!(
        clonk_script::c4_string_byte_len(&normalize_menu_info_caption(overlong)),
        C4_MAX_TITLE
    );
}
