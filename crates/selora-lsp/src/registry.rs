//! Server registry (ARCHITECTURE.md §5.3): which command runs which language.
//!
//! Static table for the two Lite servers, fed by the settings overrides
//! (`lsp.rust_analyzer_path`, `lsp.typescript_server_path` — read by the
//! app, passed here as plain strings: this crate does not depend on
//! `selora-settings`). An unset path falls back to a PATH lookup, the
//! `which`-style equivalent of the defaults below. The registry is
//! extensible — [`LspRegistry::register`] adds a language → command mapping
//! without touching the table — but only Rust and TypeScript/JavaScript are
//! wired in Lite (PLAN.md non-goals).
//!
//! `LspRegistry::with_search_dirs` pins the PATH lookup to explicit
//! directories so tests (and only tests) don't depend on the machine's PATH.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// The languages with an LSP server in Lite.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum LanguageId {
    Rust,
    TypeScript,
    JavaScript,
}

impl LanguageId {
    /// The LSP `languageId` wire string.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Rust => "rust",
            Self::TypeScript => "typescript",
            Self::JavaScript => "javascript",
        }
    }

    /// The human/server name the status bar shows.
    pub fn server_name(self) -> &'static str {
        match self {
            Self::Rust => "rust-analyzer",
            Self::TypeScript | Self::JavaScript => "typescript-language-server",
        }
    }

    /// The language for a file path, by extension. `None` = no server for
    /// that file in Lite (the file still opens, just without language
    /// intelligence).
    pub fn from_path(path: &Path) -> Option<Self> {
        let extension = path.extension()?.to_str()?.to_ascii_lowercase();
        Some(match extension.as_str() {
            "rs" => Self::Rust,
            "ts" | "mts" | "cts" | "tsx" => Self::TypeScript,
            "js" | "mjs" | "cjs" | "jsx" => Self::JavaScript,
            _ => return None,
        })
    }
}

/// One runnable server: program + args + display name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerSpec {
    /// Display name (status bar, events).
    pub name: String,
    /// The executable (path or PATH-resolvable name).
    pub program: String,
    /// Command-line arguments.
    pub args: Vec<String>,
}

/// The settings-fed overrides (`lsp.*`): `None` means "resolve the default
/// binary name from PATH". A non-empty `*_args` list replaces the built-in
/// arguments; `enabled == false` disables every language server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LspServerPaths {
    pub rust_analyzer: Option<String>,
    pub typescript_language_server: Option<String>,
    pub rust_analyzer_args: Vec<String>,
    pub typescript_language_server_args: Vec<String>,
    pub enabled: bool,
}

impl Default for LspServerPaths {
    fn default() -> Self {
        Self {
            rust_analyzer: None,
            typescript_language_server: None,
            rust_analyzer_args: Vec::new(),
            typescript_language_server_args: Vec::new(),
            enabled: true,
        }
    }
}

/// Language → server resolution.
#[derive(Debug, Clone)]
pub struct LspRegistry {
    paths: LspServerPaths,
    /// `Some(dirs)` pins the PATH lookup to these directories (tests);
    /// `None` uses the process PATH.
    search_dirs: Option<Vec<PathBuf>>,
    /// Extra registrations; checked before the built-in table.
    custom: BTreeMap<LanguageId, ServerSpec>,
}

impl LspRegistry {
    pub fn new(paths: LspServerPaths) -> Self {
        Self {
            paths,
            search_dirs: None,
            custom: BTreeMap::new(),
        }
    }

    /// Pin the PATH lookup to explicit directories (deterministic tests).
    pub fn with_search_dirs(mut self, dirs: Vec<PathBuf>) -> Self {
        self.search_dirs = Some(dirs);
        self
    }

    /// Register (or replace) a server for a language. The extension point
    /// for future editions — nothing in Lite calls it.
    pub fn register(&mut self, language: LanguageId, spec: ServerSpec) {
        self.custom.insert(language, spec);
    }

    /// Whether the settings master switch allows language servers at all.
    /// A disabled registry resolves nothing, and the manager must not emit a
    /// misleading "server missing" toast for that case.
    pub fn enabled(&self) -> bool {
        self.paths.enabled
    }

    /// The server command for a language, or `None` when no executable was
    /// found (the app shows "install it or set the path in settings",
    /// ARCHITECTURE.md §5.3).
    pub fn resolve(&self, language: LanguageId) -> Option<ServerSpec> {
        if !self.paths.enabled {
            return None;
        }
        if let Some(spec) = self.custom.get(&language) {
            return Some(spec.clone());
        }
        match language {
            LanguageId::Rust => resolve_server(
                "rust-analyzer",
                &[],
                self.paths.rust_analyzer.as_deref(),
                &self.paths.rust_analyzer_args,
                self,
            ),
            LanguageId::TypeScript | LanguageId::JavaScript => resolve_server(
                "typescript-language-server",
                &["--stdio"],
                self.paths.typescript_language_server.as_deref(),
                &self.paths.typescript_language_server_args,
                self,
            ),
        }
    }

    fn find(&self, binary: &str) -> Option<String> {
        match &self.search_dirs {
            Some(dirs) => find_in_dirs(binary, dirs.iter().cloned()),
            None => find_on_path(binary),
        }
    }
}

fn resolve_server(
    default_name: &str,
    default_args: &[&str],
    override_path: Option<&str>,
    override_args: &[String],
    registry: &LspRegistry,
) -> Option<ServerSpec> {
    let program = match override_path {
        Some(path) => path.to_owned(),
        // Default: PATH lookup (ARCHITECTURE.md §5.3).
        None => registry.find(default_name)?,
    };
    let args = if override_args.is_empty() {
        default_args.iter().map(|arg| (*arg).to_owned()).collect()
    } else {
        override_args.to_vec()
    };
    Some(ServerSpec {
        name: default_name.to_owned(),
        program,
        args,
    })
}

/// `which`-style PATH lookup (pure `std`, no new dependency).
pub fn find_on_path(binary: &str) -> Option<String> {
    let path_var = std::env::var("PATH").ok()?;
    find_in_dirs(binary, std::env::split_paths(&path_var))
}

/// The pure core of [`find_on_path`] over explicit directories, so tests
/// pass temp dirs instead of mutating the process-global PATH.
pub fn find_in_dirs(binary: &str, dirs: impl IntoIterator<Item = PathBuf>) -> Option<String> {
    let candidates = candidate_names(binary);
    for dir in dirs {
        for name in &candidates {
            let candidate = dir.join(name);
            // `std` can't portably check the execute bit; a regular file
            // with the right name is the practical signal (the spawn
            // surfaces a real error if it isn't runnable).
            if candidate.is_file() {
                return Some(candidate.to_string_lossy().into_owned());
            }
        }
    }
    None
}

/// The file names a lookup probes, in preference order.
///
/// Windows: `.exe` first (a real executable), then the shell shims npm
/// writes for CLI tools (`.cmd`, `.bat`, then anything else in `PATHEXT`,
/// e.g. `.com`), and the bare name last. The extensionless file npm also
/// installs is a POSIX `sh` script: it is a file, so probing it before
/// `.cmd` used to select a program Windows cannot execute.
///
/// Non-Windows: the bare name first (the historical behavior), then the
/// `.exe` spelling.
fn candidate_names(binary: &str) -> Vec<String> {
    if !cfg!(windows) {
        return vec![binary.to_owned(), format!("{binary}.exe")];
    }

    // An input that already carries an executable extension is honored
    // as-is — no `rust-analyzer.exe.exe` probing.
    if let Some(extension) = Path::new(binary).extension().and_then(|ext| ext.to_str()) {
        let known = windows_extensions()
            .iter()
            .any(|candidate| extension.eq_ignore_ascii_case(candidate));
        if known {
            return vec![binary.to_owned()];
        }
    }

    let mut candidates: Vec<String> = windows_extensions()
        .into_iter()
        .map(|extension| format!("{binary}.{extension}"))
        .collect();
    candidates.push(binary.to_owned());
    candidates
}

/// Executable extensions to probe on Windows: `.exe` first, then the npm
/// shell shims, then any extra `PATHEXT` entries (deduplicated, lowercased).
fn windows_extensions() -> Vec<String> {
    let mut extensions = vec!["exe".to_owned(), "cmd".to_owned(), "bat".to_owned()];
    if let Ok(pathext) = std::env::var("PATHEXT") {
        for extension in pathext.split(';') {
            let extension = extension
                .trim()
                .trim_start_matches('.')
                .to_ascii_lowercase();
            if !extension.is_empty() && !extensions.contains(&extension) {
                extensions.push(extension);
            }
        }
    }
    extensions
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "selora-lsp-registry-{label}-{}",
            std::process::id()
        ))
    }

    /// A registry whose PATH lookup is pinned to an empty dir list: `None`
    /// everywhere unless the settings override fills it in.
    fn isolated_registry() -> LspRegistry {
        LspRegistry::new(LspServerPaths::default()).with_search_dirs(vec![])
    }

    #[test]
    fn language_ids_from_paths() {
        assert_eq!(
            LanguageId::from_path(Path::new("crates/x/src/buffer.rs")),
            Some(LanguageId::Rust)
        );
        assert_eq!(
            LanguageId::from_path(Path::new("app/main.ts")),
            Some(LanguageId::TypeScript)
        );
        assert_eq!(
            LanguageId::from_path(Path::new("app/comp.tsx")),
            Some(LanguageId::TypeScript)
        );
        assert_eq!(
            LanguageId::from_path(Path::new("app/index.js")),
            Some(LanguageId::JavaScript)
        );
        // Unknown / no extension: no server in Lite.
        assert_eq!(LanguageId::from_path(Path::new("notes.md")), None);
        assert_eq!(LanguageId::from_path(Path::new("Cargo.toml")), None);
        assert_eq!(LanguageId::from_path(Path::new("noext")), None);
        // Case-insensitive extension.
        assert_eq!(
            LanguageId::from_path(Path::new("a.RS")),
            Some(LanguageId::Rust)
        );
    }

    #[test]
    fn language_wire_strings() {
        assert_eq!(LanguageId::Rust.as_str(), "rust");
        assert_eq!(LanguageId::TypeScript.as_str(), "typescript");
        assert_eq!(LanguageId::JavaScript.as_str(), "javascript");
        assert_eq!(LanguageId::Rust.server_name(), "rust-analyzer");
        assert_eq!(
            LanguageId::JavaScript.server_name(),
            "typescript-language-server"
        );
    }

    #[test]
    fn resolve_uses_the_settings_override_verbatim() {
        let registry = LspRegistry::new(LspServerPaths {
            rust_analyzer: Some("C:/tools/rust-analyzer.exe".into()),
            typescript_language_server: None,
            ..LspServerPaths::default()
        })
        .with_search_dirs(vec![]);
        let spec = registry.resolve(LanguageId::Rust).expect("resolved");
        assert_eq!(spec.program, "C:/tools/rust-analyzer.exe");
        assert_eq!(spec.name, "rust-analyzer");
        assert!(spec.args.is_empty());
        // TS with no override and no PATH dirs: not found.
        assert_eq!(registry.resolve(LanguageId::TypeScript), None);
    }

    #[test]
    fn settings_args_replace_defaults_and_the_master_switch_disables() {
        let registry = LspRegistry::new(LspServerPaths {
            rust_analyzer: Some("C:/tools/rust-analyzer.exe".into()),
            typescript_language_server: Some("C:/tools/tsserver.cmd".into()),
            rust_analyzer_args: vec!["--log-file".into(), "ra.log".into()],
            typescript_language_server_args: vec![
                "--stdio".into(),
                "--log-level".into(),
                "4".into(),
            ],
            enabled: true,
        })
        .with_search_dirs(vec![]);
        let rust = registry.resolve(LanguageId::Rust).expect("rust");
        assert_eq!(
            rust.args,
            vec!["--log-file".to_owned(), "ra.log".to_owned()]
        );
        let ts = registry.resolve(LanguageId::TypeScript).expect("ts");
        assert_eq!(
            ts.args,
            vec![
                "--stdio".to_owned(),
                "--log-level".to_owned(),
                "4".to_owned()
            ]
        );

        let disabled = LspRegistry::new(LspServerPaths {
            rust_analyzer: Some("C:/tools/rust-analyzer.exe".into()),
            enabled: false,
            ..LspServerPaths::default()
        })
        .with_search_dirs(vec![]);
        assert!(!disabled.enabled());
        assert_eq!(disabled.resolve(LanguageId::Rust), None);
        assert_eq!(disabled.resolve(LanguageId::TypeScript), None);
    }

    #[test]
    fn resolve_falls_back_to_path_lookup_with_stdio_arg() {
        let dir = temp_dir("fallback");
        std::fs::create_dir_all(&dir).expect("mkdir");
        let ts_server = dir.join(if cfg!(windows) {
            "typescript-language-server.exe"
        } else {
            "typescript-language-server"
        });
        std::fs::write(&ts_server, b"stub").expect("write stub");

        let registry =
            LspRegistry::new(LspServerPaths::default()).with_search_dirs(vec![dir.clone()]);
        let spec = registry
            .resolve(LanguageId::TypeScript)
            .expect("resolved from dir");
        assert_eq!(spec.program, ts_server.to_string_lossy());
        assert_eq!(spec.args, vec!["--stdio".to_owned()]);
        // JavaScript shares the TypeScript server.
        assert_eq!(registry.resolve(LanguageId::JavaScript), Some(spec));
        // rust-analyzer is not in that dir.
        assert_eq!(registry.resolve(LanguageId::Rust), None);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn find_in_dirs_matches_windows_exe_spelling() {
        let dir = temp_dir("exe");
        std::fs::create_dir_all(&dir).expect("mkdir");
        std::fs::write(dir.join("rust-analyzer.exe"), b"stub").expect("write stub");

        assert_eq!(
            find_in_dirs("rust-analyzer", std::iter::once(dir.clone())),
            Some(dir.join("rust-analyzer.exe").to_string_lossy().into_owned())
        );
        // Directory ordering: first matching dir wins.
        let empty = temp_dir("exe-empty");
        std::fs::create_dir_all(&empty).expect("mkdir");
        let found = find_in_dirs("rust-analyzer", vec![empty.clone(), dir.clone()]);
        assert!(found.is_some());
        assert_eq!(find_in_dirs("missing-binary", vec![dir.clone()]), None);

        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&empty);
    }

    /// npm installs `typescript-language-server.cmd` alongside an
    /// extensionless `sh` script on Windows. The sh script is a regular file
    /// but cannot be spawned, so the `.cmd` shim must win.
    #[cfg(windows)]
    #[test]
    fn resolve_prefers_the_npm_cmd_shim_over_the_bare_sh_script() {
        let dir = temp_dir("ts-cmd");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("mkdir");
        let sh_script = dir.join("typescript-language-server");
        let cmd_shim = dir.join("typescript-language-server.cmd");
        // The bare file is the POSIX sh wrapper npm writes; it is present
        // but not runnable on Windows.
        std::fs::write(&sh_script, b"#!/bin/sh\n").expect("write sh stub");
        std::fs::write(&cmd_shim, b"@echo off\r\n").expect("write cmd stub");

        let registry =
            LspRegistry::new(LspServerPaths::default()).with_search_dirs(vec![dir.clone()]);
        let spec = registry
            .resolve(LanguageId::TypeScript)
            .expect("resolved the cmd shim");
        assert_eq!(spec.program, cmd_shim.to_string_lossy());
        assert!(spec.program.ends_with(".cmd"), "program: {}", spec.program);
        assert_eq!(spec.args, vec!["--stdio".to_owned()]);

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A real executable still outranks the shell shims when both exist.
    #[cfg(windows)]
    #[test]
    fn resolve_prefers_exe_over_cmd_when_both_exist() {
        let dir = temp_dir("ts-exe-cmd");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("mkdir");
        let exe = dir.join("typescript-language-server.exe");
        let cmd = dir.join("typescript-language-server.cmd");
        std::fs::write(&exe, b"stub").expect("write exe stub");
        std::fs::write(&cmd, b"@echo off\r\n").expect("write cmd stub");

        assert_eq!(
            find_in_dirs("typescript-language-server", std::iter::once(dir.clone())),
            Some(exe.to_string_lossy().into_owned())
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn custom_registration_wins_over_the_builtin_table() {
        let mut registry = isolated_registry();
        registry.register(
            LanguageId::Rust,
            ServerSpec {
                name: "my-analyzer".into(),
                program: "/opt/my-analyzer".into(),
                args: vec!["--stdio".into()],
            },
        );
        let spec = registry.resolve(LanguageId::Rust).expect("resolved");
        assert_eq!(spec.name, "my-analyzer");
        assert_eq!(spec.program, "/opt/my-analyzer");
        // Other languages are untouched.
        assert_eq!(registry.resolve(LanguageId::TypeScript), None);
    }
}
