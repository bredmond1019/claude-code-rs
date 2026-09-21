//! Doc-parity guard: every public field of `Config` in `src/config.rs` must be
//! documented (in backticks) in `docs/api.md`. Catches a new field landing
//! without its row in the Config table.

use std::path::Path;

/// Collect `pub <name>:` field names inside `pub struct Config { ... }`.
fn config_fields(src: &str) -> Vec<String> {
    let start = src
        .find("pub struct Config {")
        .expect("`pub struct Config {` not found in src/config.rs");
    let mut fields = Vec::new();
    for line in src[start..].lines().skip(1) {
        let trimmed = line.trim();
        if line.starts_with('}') {
            break;
        }
        if let Some(rest) = trimmed.strip_prefix("pub ") {
            if let Some((name, _)) = rest.split_once(':') {
                let name = name.trim();
                if !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
                    fields.push(name.to_string());
                }
            }
        }
    }
    fields
}

#[test]
fn every_config_field_is_documented_in_api_md() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let src = std::fs::read_to_string(root.join("src/config.rs")).expect("read src/config.rs");
    let doc = std::fs::read_to_string(root.join("docs/api.md")).expect("read docs/api.md");

    let fields = config_fields(&src);
    // Positive control: the parser must actually find fields, or an empty
    // list would make this test vacuously pass.
    assert!(
        fields.iter().any(|f| f == "system_prompt") && fields.len() >= 10,
        "field parser found too few Config fields: {fields:?}"
    );

    let missing: Vec<&String> = fields
        .iter()
        .filter(|name| !(doc.contains(&format!("`{name}`")) || doc.contains(&format!("`{name}:"))))
        .collect();
    assert!(
        missing.is_empty(),
        "Config fields missing from docs/api.md (expected in backticks): {missing:?}"
    );
}
