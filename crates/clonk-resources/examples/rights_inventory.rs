//! Read-only inventory of nested content, including packed C4Groups.
use clonk_resources::Group;
use serde_json::{json, Value};
use std::error::Error;
use std::path::Path;

fn inventory(group: &Group, path: &str, records: &mut Vec<Value>) -> Result<(), Box<dyn Error>> {
    let entries = group.entries()?;
    let mut texts = serde_json::Map::new();
    for entry in &entries {
        let name = entry.relative_path.to_string_lossy();
        let lower = name.to_ascii_lowercase();
        if entry.is_directory
            || [".c4d", ".c4f", ".c4g", ".c4s"]
                .iter()
                .any(|suffix| lower.ends_with(suffix))
        {
            inventory(
                &group.open_child_entry_exact(entry)?,
                &format!("{path}/{name}"),
                records,
            )?;
        } else if [
            "scenario.txt",
            "folder.txt",
            "defcore.txt",
            "author.txt",
            "authors.txt",
            "credits.txt",
            "copyright.txt",
            "readme.txt",
            "copying",
            "license",
            "license.txt",
            "licence.txt",
            "clonkrslocalization.txt",
        ]
        .contains(&lower.as_str())
        {
            let bytes = group.read_entry_bytes_exact(entry)?;
            texts.insert(
                name.into_owned(),
                Value::String(encoding_rs::WINDOWS_1252.decode(&bytes).0.into_owned()),
            );
        }
    }
    records.push(json!({"path": path, "entries": entries.iter().map(|entry| entry.relative_path.to_string_lossy()).collect::<Vec<_>>(), "texts": texts}));
    Ok(())
}

fn main() -> Result<(), Box<dyn Error>> {
    let root = std::env::args()
        .nth(1)
        .ok_or("usage: rights_inventory <content-root>")?;
    let root = Path::new(&root);
    let mut records = Vec::new();
    for entry in std::fs::read_dir(root)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if [".c4d", ".c4f", ".c4g", ".c4s"]
            .iter()
            .any(|suffix| name.to_ascii_lowercase().ends_with(suffix))
        {
            inventory(&Group::open(entry.path())?, &name, &mut records)?;
        }
    }
    records.sort_by(|a, b| a["path"].as_str().cmp(&b["path"].as_str()));
    println!("{}", serde_json::to_string_pretty(&records)?);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inventory_records_nested_scenarios_and_their_source_notices() {
        let root = tempfile::tempdir().unwrap();
        let pack = root.path().join("Pack.c4f");
        let scenario = pack.join("Round.c4s");
        std::fs::create_dir_all(&scenario).unwrap();
        std::fs::write(
            scenario.join("Scenario.txt"),
            b"[Definitions]\nDefinition1=Objects.c4d\n",
        )
        .unwrap();
        std::fs::write(scenario.join("Author.txt"), b"Original author").unwrap();
        let mut records = Vec::new();
        inventory(&Group::open(&pack).unwrap(), "Pack.c4f", &mut records).unwrap();
        let round = records
            .iter()
            .find(|record| record["path"] == "Pack.c4f/Round.c4s")
            .unwrap();
        assert_eq!(round["texts"]["Author.txt"], "Original author");
        assert_eq!(
            round["texts"]["Scenario.txt"],
            "[Definitions]\nDefinition1=Objects.c4d\n"
        );
    }
}
