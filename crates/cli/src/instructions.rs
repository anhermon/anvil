use std::collections::HashSet;
use std::ffi::OsStr;
use std::path::{Component, Path, PathBuf};

const USER_INSTRUCTIONS_DIR: &str = ".claude";
const INSTRUCTIONS_FILE: &str = "CLAUDE.md";
const LOCAL_INSTRUCTIONS_FILE: &str = "CLAUDE.local.md";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InstructionScope {
    User,
    Project,
    Local,
}

impl InstructionScope {
    fn as_str(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Project => "project",
            Self::Local => "local",
        }
    }
}

#[derive(Debug, Clone)]
struct InstructionEntry {
    path: PathBuf,
    scope: InstructionScope,
    content: String,
}

/// Resolved instruction files for the current session.
///
/// Files are loaded in precedence order: earlier entries are lower priority.
/// More specific files are appended later and therefore take precedence.
#[derive(Debug, Clone)]
pub struct ScopedInstructionSet {
    cwd: PathBuf,
    project_root: PathBuf,
    loaded_dirs: HashSet<PathBuf>,
    loaded_files: HashSet<PathBuf>,
    entries: Vec<InstructionEntry>,
}

impl ScopedInstructionSet {
    pub fn for_current_dir() -> Self {
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        Self::new(cwd)
    }

    pub fn new(cwd: PathBuf) -> Self {
        let project_root = find_project_root(&cwd).unwrap_or_else(|| cwd.clone());
        Self {
            cwd,
            project_root,
            loaded_dirs: HashSet::new(),
            loaded_files: HashSet::new(),
            entries: Vec::new(),
        }
    }

    /// Load user and initial project/cwd instruction files.
    pub fn load_startup(&mut self) -> std::io::Result<()> {
        if let Some(home) = home_dir() {
            self.try_load_file(
                home.join(USER_INSTRUCTIONS_DIR).join(INSTRUCTIONS_FILE),
                InstructionScope::User,
            )?;
        }

        for dir in directories_between(&self.project_root, &self.cwd) {
            self.load_dir_files(&dir)?;
        }

        Ok(())
    }

    /// Lazily load scoped instruction files for a touched path under cwd.
    ///
    /// Returns true when at least one new instruction file was loaded.
    pub fn load_for_touched_path(&mut self, touched_path: &str) -> std::io::Result<bool> {
        let Some(target_dir) = self.resolve_target_dir(touched_path) else {
            return Ok(false);
        };

        if !is_within(&target_dir, &self.cwd) {
            return Ok(false);
        }

        let mut loaded_new = false;
        for dir in directories_between(&self.cwd, &target_dir) {
            if self.loaded_dirs.contains(&dir) {
                continue;
            }
            let before = self.entries.len();
            self.load_dir_files(&dir)?;
            if self.entries.len() > before {
                loaded_new = true;
            }
        }

        Ok(loaded_new)
    }

    pub fn apply_to_base_system(&self, base_system: &str) -> String {
        match self.render_overlay() {
            Some(overlay) => format!("{base_system}\n\n{overlay}"),
            None => base_system.to_string(),
        }
    }

    fn render_overlay(&self) -> Option<String> {
        if self.entries.is_empty() {
            return None;
        }

        let mut out = String::new();
        out.push_str("[Scoped instructions]\n");
        out.push_str(
            "Loaded in precedence order (later entries override earlier entries on conflicts):\n",
        );

        for entry in &self.entries {
            out.push('\n');
            out.push_str(&format!(
                "Source ({}) {}:\n",
                entry.scope.as_str(),
                entry.path.display()
            ));
            out.push_str(entry.content.trim());
            out.push('\n');
        }

        Some(out)
    }

    fn load_dir_files(&mut self, dir: &Path) -> std::io::Result<()> {
        if self.loaded_dirs.contains(dir) {
            return Ok(());
        }

        self.try_load_file(dir.join(INSTRUCTIONS_FILE), InstructionScope::Project)?;
        self.try_load_file(dir.join(LOCAL_INSTRUCTIONS_FILE), InstructionScope::Local)?;

        self.loaded_dirs.insert(dir.to_path_buf());
        Ok(())
    }

    fn try_load_file(&mut self, path: PathBuf, scope: InstructionScope) -> std::io::Result<()> {
        if self.loaded_files.contains(&path) || !path.is_file() {
            return Ok(());
        }

        let content = std::fs::read_to_string(&path)?;
        if content.trim().is_empty() {
            self.loaded_files.insert(path);
            return Ok(());
        }

        self.entries.push(InstructionEntry {
            path: path.clone(),
            scope,
            content,
        });
        self.loaded_files.insert(path);
        Ok(())
    }

    fn resolve_target_dir(&self, touched_path: &str) -> Option<PathBuf> {
        if touched_path.trim().is_empty() {
            return None;
        }

        let candidate = Path::new(touched_path);
        let absolute = if candidate.is_absolute() {
            candidate.to_path_buf()
        } else {
            self.cwd.join(candidate)
        };

        if absolute.is_dir() {
            return Some(canonical_or_original(&absolute));
        }

        if absolute.is_file() {
            return absolute.parent().map(canonical_or_original);
        }

        let looks_like_file = absolute
            .file_name()
            .and_then(OsStr::to_str)
            .is_some_and(|name| name.contains('.'));

        if looks_like_file {
            absolute.parent().map(canonical_or_original)
        } else {
            Some(canonical_or_original(&absolute))
        }
    }

    #[cfg(test)]
    fn loaded_source_paths(&self) -> Vec<PathBuf> {
        self.entries
            .iter()
            .map(|entry| entry.path.clone())
            .collect()
    }
}

fn canonical_or_original(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| normalize_path(path))
}

fn normalize_path(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            _ => {
                normalized.push(component.as_os_str());
            }
        }
    }
    normalized
}

fn find_project_root(cwd: &Path) -> Option<PathBuf> {
    for dir in cwd.ancestors() {
        if dir.join(".git").exists() {
            return Some(dir.to_path_buf());
        }
    }
    None
}

fn directories_between(start: &Path, end: &Path) -> Vec<PathBuf> {
    if !is_within(end, start) {
        return Vec::new();
    }

    let mut dirs = vec![start.to_path_buf()];
    let Ok(relative) = end.strip_prefix(start) else {
        return dirs;
    };

    let mut current = start.to_path_buf();
    for component in relative.components() {
        current.push(component.as_os_str());
        dirs.push(current.clone());
    }

    dirs
}

fn is_within(path: &Path, base: &Path) -> bool {
    let path_abs = canonical_or_original(path);
    let base_abs = canonical_or_original(base);
    path_abs.starts_with(base_abs)
}

fn home_dir() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        std::env::var("USERPROFILE").ok().map(PathBuf::from)
    }
    #[cfg(not(windows))]
    {
        std::env::var("HOME").ok().map(PathBuf::from)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn startup_loads_user_project_and_local_with_deterministic_precedence() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let root = temp_test_dir();
        let home = root.join("home");
        let repo = root.join("repo");

        std::fs::create_dir_all(home.join(".claude")).expect("create home dir");
        std::fs::create_dir_all(&repo).expect("create repo dir");
        std::fs::create_dir_all(repo.join(".git")).expect("create fake git dir");

        write_file(
            &home.join(".claude").join(INSTRUCTIONS_FILE),
            "user instruction",
        );
        write_file(&repo.join(INSTRUCTIONS_FILE), "project instruction");
        write_file(&repo.join(LOCAL_INSTRUCTIONS_FILE), "local instruction");

        let prev_home = std::env::var("HOME").ok();
        std::env::set_var("HOME", &home);

        let mut set = ScopedInstructionSet::new(repo.clone());
        set.load_startup().expect("load startup instructions");

        let paths = set.loaded_source_paths();
        assert_eq!(
            paths,
            vec![
                home.join(".claude").join(INSTRUCTIONS_FILE),
                repo.join(INSTRUCTIONS_FILE),
                repo.join(LOCAL_INSTRUCTIONS_FILE),
            ]
        );

        let overlay = set.render_overlay().expect("expected instruction overlay");
        let user_idx = overlay.find("user instruction").expect("missing user");
        let project_idx = overlay
            .find("project instruction")
            .expect("missing project");
        let local_idx = overlay.find("local instruction").expect("missing local");
        assert!(user_idx < project_idx);
        assert!(project_idx < local_idx);

        restore_home(prev_home);
        std::fs::remove_dir_all(root).expect("cleanup temp dir");
    }

    #[test]
    fn subdirectory_files_load_lazily_when_touched() {
        let root = temp_test_dir();
        let repo = root.join("repo");
        let src = repo.join("src");
        let nested = src.join("bin");

        std::fs::create_dir_all(repo.join(".git")).expect("create fake git dir");
        std::fs::create_dir_all(&nested).expect("create nested dir");

        write_file(&repo.join(INSTRUCTIONS_FILE), "repo instruction");
        write_file(&src.join(INSTRUCTIONS_FILE), "src instruction");
        write_file(
            &nested.join(LOCAL_INSTRUCTIONS_FILE),
            "bin local instruction",
        );

        let mut set = ScopedInstructionSet::new(repo.clone());
        set.load_startup().expect("load startup instructions");

        let startup_overlay = set.render_overlay().expect("expected startup overlay");
        assert!(startup_overlay.contains("repo instruction"));
        assert!(!startup_overlay.contains("src instruction"));
        assert!(!startup_overlay.contains("bin local instruction"));

        let changed = set
            .load_for_touched_path("src/main.rs")
            .expect("load touched path");
        assert!(changed);
        let after_src_overlay = set.render_overlay().expect("overlay after src");
        assert!(after_src_overlay.contains("src instruction"));
        assert!(!after_src_overlay.contains("bin local instruction"));

        let changed_nested = set
            .load_for_touched_path("src/bin/app.rs")
            .expect("load nested touched path");
        assert!(changed_nested);
        let after_nested_overlay = set.render_overlay().expect("overlay after nested");
        assert!(after_nested_overlay.contains("bin local instruction"));

        let changed_again = set
            .load_for_touched_path("src/bin/app.rs")
            .expect("reload touched path");
        assert!(!changed_again);

        std::fs::remove_dir_all(root).expect("cleanup temp dir");
    }

    fn temp_test_dir() -> PathBuf {
        let mut path = std::env::temp_dir();
        path.push(format!("anvil-scoped-instructions-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&path).expect("create temp test dir");
        path
    }

    fn write_file(path: &Path, content: &str) {
        let parent = path.parent().expect("path should have parent");
        std::fs::create_dir_all(parent).expect("create file parent dir");
        std::fs::write(path, content).expect("write file");
    }

    fn restore_home(previous: Option<String>) {
        if let Some(value) = previous {
            std::env::set_var("HOME", value);
        } else {
            std::env::remove_var("HOME");
        }
    }
}
