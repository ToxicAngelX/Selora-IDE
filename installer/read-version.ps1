# Prints the workspace version (the root Cargo.toml's bare
# `version = "…"` line — [workspace.package]; dependency versions are
# inline tables). Used by prebuild.bat; safe to run standalone.
(Select-String -Path (Join-Path $PSScriptRoot '..\Cargo.toml') -Pattern '^version\s*=\s*"([^"]+)"').Matches.Groups[1].Value
