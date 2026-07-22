use super::Config;

#[test]
fn parse_fonts_config() {
    let data = include_str!("../../../../planet/System.c4g/Fonts.txt");
    let mut cursor = std::io::Cursor::new(data.replace('\r', ""));
    let cfg = Config::from_reader(&mut cursor).unwrap();
    assert!(cfg.get("Name").is_none());
}
