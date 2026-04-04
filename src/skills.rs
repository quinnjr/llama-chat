use anyhow::Result;
use std::collections::HashMap;
use std::path::Path;

#[derive(Debug, Clone)]
pub struct Skill {
    pub name: String,
    pub description: String,
    pub content: String,
}

pub fn load_skills_from_dir(dir: &Path) -> Result<HashMap<String, Skill>> {
    let mut skills = HashMap::new();
    if !dir.exists() {
        return Ok(skills);
    }
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("md") {
            continue;
        }
        if let Ok(skill) = parse_skill_file(&path) {
            skills.insert(skill.name.clone(), skill);
        }
    }
    Ok(skills)
}

pub fn load_all_skills(global_dir: &Path, project_dir: &Path) -> Result<HashMap<String, Skill>> {
    let mut skills = load_skills_from_dir(global_dir)?;
    let project_skills = load_skills_from_dir(project_dir)?;
    skills.extend(project_skills);
    Ok(skills)
}

fn parse_skill_file(path: &Path) -> Result<Skill> {
    let raw = std::fs::read_to_string(path)?;
    let (frontmatter, content) = split_frontmatter(&raw)?;

    let name = extract_field(&frontmatter, "name")
        .unwrap_or_else(|| {
            path.file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("unknown")
                .to_string()
        });
    let description = extract_field(&frontmatter, "description")
        .unwrap_or_default();

    Ok(Skill { name, description, content: content.trim().to_string() })
}

fn split_frontmatter(text: &str) -> Result<(String, String)> {
    let text = text.trim_start();
    if !text.starts_with("---") {
        return Ok((String::new(), text.to_string()));
    }
    let after_first = &text[3..];
    if let Some(end) = after_first.find("---") {
        let fm = after_first[..end].to_string();
        let content = after_first[end + 3..].to_string();
        Ok((fm, content))
    } else {
        Ok((String::new(), text.to_string()))
    }
}

fn extract_field(frontmatter: &str, field: &str) -> Option<String> {
    for line in frontmatter.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix(&format!("{field}:")) {
            return Some(rest.trim().trim_matches('"').trim_matches('\'').to_string());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_frontmatter() {
        let text = "---\nname: review\ndescription: Review code\n---\n\nCheck the code for bugs.";
        let (fm, content) = split_frontmatter(text).unwrap();
        assert!(fm.contains("name: review"));
        assert_eq!(content.trim(), "Check the code for bugs.");
    }

    #[test]
    fn extract_fields() {
        let fm = "name: review\ndescription: Review code for bugs";
        assert_eq!(extract_field(fm, "name"), Some("review".into()));
        assert_eq!(extract_field(fm, "description"), Some("Review code for bugs".into()));
        assert_eq!(extract_field(fm, "missing"), None);
    }

    #[test]
    fn no_frontmatter() {
        let text = "Just some content without frontmatter.";
        let (fm, content) = split_frontmatter(text).unwrap();
        assert!(fm.is_empty());
        assert_eq!(content, text);
    }

    #[test]
    fn project_skills_override_global() {
        let dir = std::env::temp_dir().join("llama-chat-test-skills");
        let global = dir.join("global");
        let project = dir.join("project");
        std::fs::create_dir_all(&global).unwrap();
        std::fs::create_dir_all(&project).unwrap();

        std::fs::write(global.join("review.md"), "---\nname: review\ndescription: global\n---\nGlobal review.").unwrap();
        std::fs::write(project.join("review.md"), "---\nname: review\ndescription: project\n---\nProject review.").unwrap();

        let skills = load_all_skills(&global, &project).unwrap();
        assert_eq!(skills["review"].description, "project");
        assert_eq!(skills["review"].content, "Project review.");

        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn load_skills_from_empty_dir() {
        let dir = std::env::temp_dir().join("llama-chat-test-skills-empty");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let skills = load_skills_from_dir(&dir).unwrap();
        assert!(skills.is_empty());

        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn load_skills_from_nonexistent_dir() {
        let dir = std::path::Path::new("/tmp/llama-chat-test-skills-nonexistent-xyz");
        let skills = load_skills_from_dir(dir).unwrap();
        assert!(skills.is_empty());
    }

    #[test]
    fn skill_without_name_uses_filename() {
        let dir = std::env::temp_dir().join("llama-chat-test-skills-noname");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        std::fs::write(dir.join("myskill.md"), "---\ndescription: testing\n---\nSkill content here.").unwrap();

        let skills = load_skills_from_dir(&dir).unwrap();
        assert!(skills.contains_key("myskill"));
        assert_eq!(skills["myskill"].description, "testing");
        assert_eq!(skills["myskill"].content, "Skill content here.");

        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn skill_file_without_frontmatter() {
        let dir = std::env::temp_dir().join("llama-chat-test-skills-nofm");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        std::fs::write(dir.join("bare.md"), "Just plain markdown content.").unwrap();

        let skills = load_skills_from_dir(&dir).unwrap();
        assert!(skills.contains_key("bare"));
        assert_eq!(skills["bare"].content, "Just plain markdown content.");
        assert!(skills["bare"].description.is_empty());

        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn non_md_files_are_skipped() {
        let dir = std::env::temp_dir().join("llama-chat-test-skills-nonmd");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        std::fs::write(dir.join("skill.md"), "---\nname: good\n---\nGood skill.").unwrap();
        std::fs::write(dir.join("notes.txt"), "Some notes.").unwrap();
        std::fs::write(dir.join("data.json"), "{}").unwrap();

        let skills = load_skills_from_dir(&dir).unwrap();
        assert_eq!(skills.len(), 1);
        assert!(skills.contains_key("good"));

        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn extract_field_with_quotes() {
        let fm = "name: \"quoted-name\"\ndescription: 'single-quoted'";
        assert_eq!(extract_field(fm, "name"), Some("quoted-name".into()));
        assert_eq!(extract_field(fm, "description"), Some("single-quoted".into()));
    }

    #[test]
    fn split_frontmatter_unclosed_delimiters() {
        let text = "---\nname: test\nContent without closing delimiter.";
        let (fm, content) = split_frontmatter(text).unwrap();
        // Unclosed --- means no valid frontmatter
        assert!(fm.is_empty());
        assert!(content.starts_with("---"));
    }
}
