//! Distribution decisions are separate from proof of licence permission.
use anyhow::{bail, Context, Result};
use std::collections::{BTreeMap, BTreeSet};

pub struct Distribution {
    scopes: BTreeMap<String, Scope>,
    pub notice: String,
    dependencies: BTreeMap<String, Vec<String>>,
}

struct Scope {
    rights: String,
    evidence: Vec<String>,
}

impl Distribution {
    pub fn parse(manifest: &str) -> Result<Self> {
        let table: toml::Table = manifest.parse()?;
        let settings = table
            .get("distribution")
            .and_then(toml::Value::as_table)
            .context("missing distribution policy")?;
        if settings.get("version").and_then(toml::Value::as_integer) != Some(1) {
            bail!("unsupported distribution policy version");
        }
        for key in settings.keys() {
            if !matches!(key.as_str(), "version" | "notice" | "dependencies") {
                bail!("unknown distribution policy field {key}");
            }
        }
        let notice = required_text(settings, "notice")?;
        check_path(&notice)?;
        let mut scopes = BTreeMap::new();
        for (path, fields) in table
            .get("packs")
            .and_then(toml::Value::as_table)
            .context("missing packs")?
        {
            check_path(path)?;
            let fields = fields.as_table().context("pack must be a table")?;
            let rights = required_text(fields, "rights")?;
            let evidence = string_list(fields.get("evidence"))?;
            for path in &evidence {
                check_path(path)?;
            }
            match rights.as_str() {
                "licensed" => {
                    required_text(fields, "license")?;
                }
                "assumed" | "excluded" => {
                    required_text(fields, "reason")?;
                }
                _ => bail!("{path}: unknown or missing rights decision"),
            }
            if rights != "excluded" && evidence.is_empty() {
                bail!("{path}: rights decision needs evidence or an assumption record");
            }
            if scopes
                .insert(path.to_ascii_lowercase(), Scope { rights, evidence })
                .is_some()
            {
                bail!("{path}: duplicate case-insensitive scope");
            }
        }
        if scopes.is_empty() {
            bail!("distribution policy lists no packs");
        }
        let mut dependencies = BTreeMap::new();
        if let Some(value) = settings.get("dependencies") {
            for (consumer, value) in value.as_table().context("dependencies must be a table")? {
                check_path(consumer)?;
                let required = string_list(Some(value))?;
                for path in &required {
                    check_path(path)?;
                }
                dependencies.insert(consumer.to_ascii_lowercase(), required);
            }
        }
        Ok(Self {
            scopes,
            notice,
            dependencies,
        })
    }

    pub fn includes(&self, path: &str) -> bool {
        let path = path.to_ascii_lowercase();
        let scope = self
            .scopes
            .iter()
            .filter(|(scope, _)| within(&path, scope))
            .max_by_key(|(scope, _)| scope.len());
        scope.map_or_else(
            || {
                path == self.notice.to_ascii_lowercase()
                    || self.scopes.values().any(|scope| {
                        scope.rights != "excluded"
                            && scope
                                .evidence
                                .iter()
                                .any(|evidence| evidence.eq_ignore_ascii_case(&path))
                    })
            },
            |(_, scope)| scope.rights != "excluded",
        )
    }

    pub fn select(&self, files: &[String]) -> Result<Vec<String>> {
        let mut names = BTreeSet::new();
        for file in files {
            check_path(file)?;
            if self.includes(file) && !names.insert(file.to_ascii_lowercase()) {
                bail!("duplicate case-insensitive package path {file}");
            }
        }
        if !files.contains(&self.notice) || !self.includes(&self.notice) {
            bail!("missing distribution notice {}", self.notice);
        }
        for (path, scope) in &self.scopes {
            if scope.rights != "excluded"
                && !files.iter().any(|file| {
                    let file = file.to_ascii_lowercase();
                    self.includes(&file) && (within(&file, path) || within(path, &file))
                })
            {
                bail!("{path}: included scope contains no package files");
            }
            for file in files {
                let lower = file.to_ascii_lowercase();
                if path != &lower
                    && within(path, &lower)
                    && (scope.rights != "excluded") != self.includes(file)
                {
                    bail!("{path}: decision crosses packed file {file}; change the packing boundary first");
                }
            }
            for evidence in &scope.evidence {
                if scope.rights != "excluded"
                    && (!files.contains(evidence) || !self.includes(evidence))
                {
                    bail!("{path}: missing rights evidence {evidence}");
                }
            }
        }
        let selected: Vec<String> = files
            .iter()
            .filter(|path| self.includes(path))
            .cloned()
            .collect();
        let lower: Vec<String> = selected
            .iter()
            .map(|path| path.to_ascii_lowercase())
            .collect();
        for (consumer, required) in &self.dependencies {
            if lower.iter().any(|path| within(path, consumer)) {
                for dependency in required {
                    if !lower
                        .iter()
                        .any(|path| within(path, &dependency.to_ascii_lowercase()))
                    {
                        bail!("{consumer}: required dependency {dependency} would not ship");
                    }
                }
            }
        }
        Ok(selected)
    }
}

fn within(path: &str, scope: &str) -> bool {
    path == scope
        || path
            .strip_prefix(scope)
            .is_some_and(|rest| rest.starts_with('/'))
}

fn required_text(table: &toml::Table, key: &str) -> Result<String> {
    table
        .get(key)
        .and_then(toml::Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(str::to_owned)
        .with_context(|| format!("missing or empty {key}"))
}

fn string_list(value: Option<&toml::Value>) -> Result<Vec<String>> {
    value
        .map(|value| {
            value
                .as_array()
                .context("expected a list of paths")?
                .iter()
                .map(|item| {
                    item.as_str()
                        .map(str::to_owned)
                        .context("expected a path string")
                })
                .collect()
        })
        .unwrap_or_else(|| Ok(Vec::new()))
}

fn check_path(path: &str) -> Result<()> {
    if path.is_empty()
        || path.contains(['\\', ':', '\0'])
        || path.split('/').any(|part| matches!(part, "" | "." | ".."))
    {
        bail!("unsafe distribution path {path:?}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const LICENSED: &str = r#"
[distribution]
version = 1
notice = "CONTENT-NOTICES.md"
[packs."Objects.c4d"]
rights = "licensed"
license = "CC-BY-NC-4.0"
evidence = ["Objects.c4d/COPYING"]
"#;

    #[test]
    fn selection_refuses_empty_scopes_unsafe_names_and_excluded_evidence() {
        let external = LICENSED.replace("Objects.c4d/COPYING", "LICENSE");
        let policy = Distribution::parse(&external).unwrap();
        for files in [
            vec!["CONTENT-NOTICES.md", "LICENSE"],
            vec!["CONTENT-NOTICES.md", "LICENSE", "Objects.c4d/../secret"],
            vec![
                "CONTENT-NOTICES.md",
                "LICENSE",
                "Objects.c4d/A",
                "Objects.c4d/a",
            ],
        ] {
            assert!(policy
                .select(&files.into_iter().map(str::to_owned).collect::<Vec<_>>())
                .is_err());
        }
        let excluded = format!("{LICENSED}\n[packs.\"Objects.c4d/COPYING\"]\nrights = \"excluded\"\nreason = \"omitted\"\n");
        assert!(Distribution::parse(&excluded)
            .unwrap()
            .select(&[
                "CONTENT-NOTICES.md".into(),
                "Objects.c4d/COPYING".into(),
                "Objects.c4d/Script.c".into()
            ])
            .is_err());
    }

    #[test]
    fn assumed_content_ships_with_its_explicit_assumption_record() {
        let policy = Distribution::parse(
            &LICENSED
                .replace(
                    "rights = \"licensed\"",
                    "rights = \"assumed\"\nreason = \"maintainer decision\"",
                )
                .replace("Objects.c4d/COPYING", "third_party/RIGHTS.md"),
        )
        .unwrap();
        let files = [
            "Objects.c4d/Script.c",
            "third_party/RIGHTS.md",
            "CONTENT-NOTICES.md",
            "Unlisted.c4d/Script.c",
        ]
        .map(str::to_owned);
        assert_eq!(policy.select(&files).unwrap(), files[..3]);
    }

    #[test]
    fn packaging_refuses_to_drop_a_selected_scenarios_dependency() {
        let policy = Distribution::parse(&format!(
            "{LICENSED}\n[distribution.dependencies]\n\"Objects.c4d\" = [\"Missing.c4d\"]\n"
        ))
        .unwrap();
        let files = vec!["Objects.c4d/COPYING".into(), "CONTENT-NOTICES.md".into()];
        assert!(policy.select(&files).is_err());
    }

    #[test]
    fn selection_requires_the_notice_and_recorded_evidence() {
        let policy = Distribution::parse(LICENSED).unwrap();
        for files in [
            vec!["Objects.c4d/COPYING".into()],
            vec!["CONTENT-NOTICES.md".into(), "Objects.c4d/Script.c".into()],
        ] {
            assert!(policy.select(&files).is_err());
        }
    }

    #[test]
    fn a_nested_exclusion_cannot_silently_leave_a_packed_parent_in_the_archive() {
        let policy = Distribution::parse(&format!("{LICENSED}\n[packs.\"Objects.c4d/Private.c4d\"]\nrights = \"excluded\"\nreason = \"not selected\"\n")).unwrap();
        let files = vec![
            "Objects.c4d".into(),
            "Objects.c4d/COPYING".into(),
            "CONTENT-NOTICES.md".into(),
        ];
        assert!(policy.select(&files).is_err());
    }

    #[test]
    fn incomplete_or_unsafe_policies_are_refused() {
        for bad in [
            LICENSED.replace("version = 1", "version = 2"),
            LICENSED.replace("version = 1", "version = 1\ndependenciez = {}"),
            LICENSED.replace("CONTENT-NOTICES.md", "../outside.md"),
            LICENSED.replace("[packs.\"Objects.c4d\"]", "[packs.\"../Objects.c4d\"]"),
            LICENSED.replace("CC-BY-NC-4.0", ""),
            LICENSED.replace("[\"Objects.c4d/COPYING\"]", "[]"),
            LICENSED.replace("\"licensed\"", "\"assumed\""),
        ] {
            assert!(Distribution::parse(&bad).is_err(), "accepted {bad}");
        }
    }

    #[test]
    fn an_unrecognized_rights_decision_is_refused() {
        assert!(Distribution::parse(
            r#"
[distribution]
version = 1
notice = "CONTENT-NOTICES.md"
[packs."Objects.c4d"]
rights = "probably-fine"
"#
        )
        .is_err());
    }

    #[test]
    fn nested_exclusion_overrides_an_included_parent() {
        let policy = Distribution::parse(
            r#"
[distribution]
version = 1
notice = "CONTENT-NOTICES.md"
[packs."Melees.c4f"]
rights = "licensed"
license = "CC-BY-NC-4.0"
evidence = ["Melees.c4f/COPYING"]
[packs."Melees.c4f/Queron3.c4s"]
rights = "excluded"
reason = "Unresolved dependencies"
"#,
        )
        .unwrap();
        assert!(policy.includes("Melees.c4f/Round.c4s/Scenario.txt"));
        assert!(!policy.includes("Melees.c4f/Queron3.c4s/Scenario.txt"));
    }
}
