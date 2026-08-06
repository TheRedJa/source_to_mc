#[test]
fn example_config_parses() {
    let text = std::fs::read_to_string("example-config.toml").unwrap();
    let cfg: src2mc::config::Config = toml::from_str(&text).expect("example-config.toml must parse");
    assert_eq!(cfg.scale.units_per_block, 16.0);
    assert!(cfg.shapes.enabled);
    assert_eq!(cfg.materials.texture_size, 16);
}
