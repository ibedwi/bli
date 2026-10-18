//! bli's own skills, as other repos install them: the real files, no fakes. The `skills` CLI skips a skill whose
//! frontmatter isn't valid YAML.

use std::path::Path;

const SKILLS: [&str; 3] = ["fan-out", "fan-in", "remove-lane"];

#[test]
fn every_skill_has_frontmatter_that_parses_as_yaml_with_its_name_and_a_description() {
    for skill in SKILLS {
        let path =
            Path::new(env!("CARGO_MANIFEST_DIR")).join(format!(".agents/skills/{skill}/SKILL.md"));
        let text = std::fs::read_to_string(&path).unwrap();
        let frontmatter = text
            .strip_prefix("---\n")
            .and_then(|rest| rest.split_once("\n---\n"))
            .map(|(frontmatter, _)| frontmatter)
            .unwrap_or_else(|| panic!("{skill}: no frontmatter"));

        let fields: serde_yaml::Mapping = serde_yaml::from_str(frontmatter)
            .unwrap_or_else(|e| panic!("{skill}: frontmatter is not valid YAML: {e}"));

        assert_eq!(
            fields.get("name").and_then(|v| v.as_str()),
            Some(skill),
            "{skill}"
        );
        let description = fields
            .get("description")
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        assert!(!description.is_empty(), "{skill}: no description");
    }
}
