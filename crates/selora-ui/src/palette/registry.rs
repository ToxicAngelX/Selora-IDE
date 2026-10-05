//! The static command registry behind the command palette (UI_SPEC.md §5).
//!
//! A [`Command`] is pure data: a name, a category, an optional keybind hint
//! (from the keyboard model in UI_SPEC.md §15), and lowercase search tags.
//! Commands carry **no behavior** — the palette emits a [`CommandId`] and the
//! shell routes it (the `CommandId` indirection is the parallel-safe form of
//! the ROADMAP's "closure that fires an Effect": the orchestrator wires the
//! dispatch at merge, so sibling phases never race on closures).
//!
//! The registry is a `const` slice: the command set is fixed at compile time
//! (no dynamic registration, no MRU ordering, no chaining — those are
//! non-goals for Phase 2).

/// Identifies one command in the registry.
///
/// The palette emits this; the app shell maps it to an action (a UI-local
/// toggle, a dialog, or an `Effect` sent to the runtime).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CommandId {
    // File
    FileOpenFile,
    FileOpenFolder,
    FileSave,
    FileSaveAll,
    FileCloseEditor,
    FileCloseAllEditors,
    FileGoToFile,

    // Edit
    EditToggleComment,
    EditFindInFile,
    EditReplaceInFile,
    EditGoToDefinition,

    // View
    ViewToggleFileTree,
    ViewToggleChatSidebar,
    ViewTogglePanel,
    ViewSplitEditor,
    ViewGoToSymbol,
    ViewCommandPalette,

    // Selora
    SeloraSwitchModel,
    SeloraClearApiKey,

    // Settings
    SettingsOpen,
}

/// One palette entry. Pure data — see the module docs.
#[derive(Clone, Copy, Debug)]
pub struct Command {
    /// The command's identity. Emitted by the palette on Enter.
    pub id: CommandId,
    /// Display name, including the category prefix (`"File: Open File…"`).
    pub name: &'static str,
    /// The category prefix (`"File"`, `"Edit"`, `"View"`, …).
    pub category: &'static str,
    /// Default keybind from UI_SPEC.md §15, shown right-aligned in the row.
    pub keybind_hint: Option<&'static str>,
    /// Extra lowercase search terms matched by the fuzzy ranker (the name is
    /// always matched too).
    pub tags: &'static [&'static str],
}

/// The full command registry, in wireframe order (UI_SPEC.md §5: File, Edit,
/// View, Selora, …). Registry order is also the empty-query display order and
/// the tie-break order for equal fuzzy scores.
pub const COMMANDS: &[Command] = &[
    Command {
        id: CommandId::FileOpenFile,
        name: "File: Open File…",
        category: "File",
        keybind_hint: Some("Ctrl+O"),
        tags: &["open", "file", "load"],
    },
    Command {
        id: CommandId::FileOpenFolder,
        name: "File: Open Folder…",
        category: "File",
        keybind_hint: Some("Ctrl+K Ctrl+O"),
        tags: &["open", "folder", "workspace", "project"],
    },
    Command {
        id: CommandId::FileSave,
        name: "File: Save",
        category: "File",
        keybind_hint: Some("Ctrl+S"),
        tags: &["save", "write", "disk"],
    },
    Command {
        id: CommandId::FileSaveAll,
        name: "File: Save All",
        category: "File",
        keybind_hint: Some("Ctrl+Shift+S"),
        tags: &["save", "all", "write"],
    },
    Command {
        id: CommandId::FileCloseEditor,
        name: "File: Close Editor",
        category: "File",
        keybind_hint: Some("Ctrl+W"),
        tags: &["close", "tab", "editor"],
    },
    Command {
        id: CommandId::FileCloseAllEditors,
        name: "File: Close All Editors",
        category: "File",
        keybind_hint: Some("Ctrl+K Ctrl+W"),
        tags: &["close", "all", "tabs"],
    },
    Command {
        id: CommandId::FileGoToFile,
        name: "File: Go to File…",
        category: "File",
        keybind_hint: Some("Ctrl+P"),
        tags: &["fuzzy", "find", "file", "quick", "open"],
    },
    Command {
        id: CommandId::EditToggleComment,
        name: "Edit: Toggle Comment",
        category: "Edit",
        keybind_hint: Some("Ctrl+/"),
        tags: &["comment", "uncomment", "toggle"],
    },
    Command {
        id: CommandId::EditFindInFile,
        name: "Edit: Find in File",
        category: "Edit",
        keybind_hint: Some("Ctrl+F"),
        tags: &["find", "search"],
    },
    Command {
        id: CommandId::EditReplaceInFile,
        name: "Edit: Replace in File",
        category: "Edit",
        keybind_hint: Some("Ctrl+H"),
        tags: &["replace", "substitute"],
    },
    Command {
        id: CommandId::EditGoToDefinition,
        name: "Edit: Go to Definition",
        category: "Edit",
        keybind_hint: Some("F12"),
        tags: &["definition", "goto", "navigate", "lsp"],
    },
    Command {
        id: CommandId::ViewToggleFileTree,
        name: "View: Toggle File Tree",
        category: "View",
        keybind_hint: Some("Ctrl+B"),
        tags: &["sidebar", "explorer", "tree", "files", "toggle"],
    },
    Command {
        id: CommandId::ViewToggleChatSidebar,
        name: "View: Toggle Chat Sidebar",
        category: "View",
        keybind_hint: Some("Ctrl+L"),
        tags: &["chat", "sidebar", "ai", "assistant", "toggle"],
    },
    Command {
        id: CommandId::ViewTogglePanel,
        name: "View: Toggle Panel",
        category: "View",
        keybind_hint: Some("Ctrl+J"),
        tags: &["panel", "bottom", "toggle"],
    },
    Command {
        id: CommandId::ViewSplitEditor,
        name: "View: Split Editor",
        category: "View",
        keybind_hint: Some("Ctrl+\\"),
        tags: &["split", "pane", "column", "editor"],
    },
    Command {
        id: CommandId::ViewGoToSymbol,
        name: "View: Go to Symbol…",
        category: "View",
        keybind_hint: Some("Ctrl+Shift+O"),
        tags: &["symbol", "outline", "navigate"],
    },
    Command {
        id: CommandId::ViewCommandPalette,
        name: "View: Command Palette",
        category: "View",
        keybind_hint: Some("Ctrl+Shift+P"),
        tags: &["command", "palette", "commands", "menu"],
    },
    Command {
        id: CommandId::SeloraSwitchModel,
        name: "Selora: Switch Model…",
        category: "Selora",
        keybind_hint: None,
        tags: &["model", "switch", "ai", "chat", "gateway"],
    },
    Command {
        id: CommandId::SeloraClearApiKey,
        name: "Selora: Clear API Key",
        category: "Selora",
        keybind_hint: None,
        tags: &["key", "api", "clear", "remove", "credentials", "keyring"],
    },
    Command {
        id: CommandId::SettingsOpen,
        name: "Settings: Open",
        category: "Settings",
        keybind_hint: Some("Ctrl+,"),
        tags: &["settings", "preferences", "config"],
    },
];

/// Look a command up by id.
pub fn command(id: CommandId) -> Option<&'static Command> {
    COMMANDS.iter().find(|command| command.id == id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn registry_names_are_prefixed_with_their_category() {
        for command in COMMANDS {
            let prefix = format!("{}: ", command.category);
            assert!(
                command.name.starts_with(&prefix),
                "{:?} name {:?} lacks category prefix {:?}",
                command.id,
                command.name,
                prefix
            );
        }
    }

    #[test]
    fn registry_ids_are_unique() {
        let ids: HashSet<CommandId> = COMMANDS.iter().map(|c| c.id).collect();
        assert_eq!(ids.len(), COMMANDS.len());
    }

    #[test]
    fn every_command_is_findable_by_id() {
        for entry in COMMANDS {
            assert_eq!(command(entry.id).map(|c| c.id), Some(entry.id));
        }
    }
}
