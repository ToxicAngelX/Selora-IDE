//! Refuse `world.run` commands that ask a Windows broker to create the
//! process instead of creating it as a descendant.
//!
//! `workflows_process` contains a `world.run` child in a Job Object with
//! `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`. Windows puts every `CreateProcess`
//! descendant of that child into the same job. An out-of-process broker does
//! not: WMI (`Win32_Process.Create`), Task Scheduler and Explorer create the
//! process as *their* child, outside the job, so a Stop, timeout or normal
//! exit leaves it running.
//!
//! This module is the policy half of the defence. It scans the confirmed
//! command text before PowerShell is spawned and refuses the known broker
//! launch forms with a typed error, so the command never runs at all. The
//! spawn path's post-exit job audit (`WorldProcess::audit`) is the other half,
//! for processes that were in the job.
//!
//! # Exact limit
//!
//! The scan is a policy, not a security boundary. It sees the confirmed
//! command text only; `world.run` arguments are data and are not scanned, so a
//! confirmed general-purpose interpreter (`powershell`, `pwsh`, `cmd`) can
//! still carry broker code in an argument. Obfuscated PowerShell (string
//! concatenation, `[scriptblock]::Create`, aliases) can spell a token
//! differently. Those forms are not refused, and a process they hand to a
//! broker still escapes the Job Object. Keep such commands out of the
//! confirmed set; `workflows_process`'s module documentation states the same
//! residual.

/// One refused broker launch form: the lowercase token and what it means.
const BROKER_LAUNCHES: &[(&str, &str)] = &[
    (
        "invoke-cimmethod",
        "WMI process creation (`Invoke-CimMethod`)",
    ),
    (
        "invoke-wmimethod",
        "WMI process creation (`Invoke-WmiMethod`)",
    ),
    ("get-wmiobject", "WMI access (`Get-WmiObject`)"),
    ("win32_process", "WMI `Win32_Process`"),
    ("wmic", "the WMI command line (`wmic`)"),
    ("schtasks", "Task Scheduler (`schtasks`)"),
    (
        "register-scheduledtask",
        "Task Scheduler (`Register-ScheduledTask`)",
    ),
    (
        "start-scheduledtask",
        "Task Scheduler (`Start-ScheduledTask`)",
    ),
    (
        "shell.application",
        "the shell broker (`Shell.Application`)",
    ),
    ("shellexecute", "a ShellExecute launch"),
    ("explorer.exe", "the Explorer broker (`explorer.exe`)"),
];

/// Why `world.run` must refuse this command, or `None` when it may run.
///
/// The match is ASCII case-insensitive and whole-token, so `wmic` does not
/// fire inside `wmicrometer`. `ShellExecute` is matched as a prefix because
/// its API name carries an `Ex`/`ExW` suffix.
#[must_use]
pub(crate) fn refusal(command: &str) -> Option<String> {
    let lowered = command.to_ascii_lowercase();
    let (token, what) = BROKER_LAUNCHES.iter().find(|(token, _)| {
        if *token == "shellexecute" {
            lowered.contains(token)
        } else {
            contains_token(&lowered, token)
        }
    })?;
    Some(format!(
        "`world.run` refused `{token}`: {what} lets a Windows broker create the process outside \
         the Job Object, so it would not be killed with the run. Start the process as a direct \
         descendant instead (for example `Start-Process -NoNewWindow`). Nothing was run."
    ))
}

/// Whether `haystack` contains `needle` bounded by non-identifier characters.
fn contains_token(haystack: &str, needle: &str) -> bool {
    let mut from = 0;
    while let Some(offset) = haystack[from..].find(needle) {
        let start = from + offset;
        let end = start + needle.len();
        let before = haystack[..start].chars().next_back();
        let after = haystack[end..].chars().next();
        if before.is_none_or(is_separator) && after.is_none_or(is_separator) {
            return true;
        }
        from = start + needle.len();
    }
    false
}

fn is_separator(ch: char) -> bool {
    !(ch.is_ascii_alphanumeric() || ch == '_')
}

#[cfg(test)]
mod tests {
    use super::refusal;

    #[test]
    fn refuses_known_broker_launch_forms() {
        for command in [
            "Invoke-CimMethod -ClassName Win32_Process -MethodName Create",
            "Get-WmiObject Win32_Process",
            "wmic process call create notepad.exe",
            "schtasks /create /tn x /tr notepad.exe",
            "Register-ScheduledTask -TaskName x -Action $a",
            "explorer.exe C:\\Windows\\System32\\notepad.exe",
            "New-Object -ComObject Shell.Application",
            "$s.ShellExecute('x.exe', '', '', 'open', 0)",
        ] {
            let refusal = refusal(command).unwrap_or_else(|| panic!("not refused: {command}"));
            assert!(
                refusal.contains("broker") || refusal.contains("Job Object"),
                "refusal names the escape: {refusal}"
            );
            assert!(
                refusal.contains("Nothing was run"),
                "refusal says the command did not run: {refusal}"
            );
        }
    }

    #[test]
    fn leaves_confirmed_commands_alone() {
        for command in [
            "cargo test",
            "git status",
            "Start-Process -FilePath pwsh -ArgumentList '-NoProfile' -NoNewWindow",
            "Get-Process powershell",
            "Write-Output wmicrometer",
            "dotnet test --filter FullyQualifiedName~Wmi",
        ] {
            assert!(refusal(command).is_none(), "wrongly refused: {command}");
        }
    }

    #[test]
    fn token_match_is_case_insensitive_and_bounded() {
        assert!(refusal("INVOKE-CIMMETHOD -ClassName Win32_Process").is_some());
        assert!(refusal("wmics").is_none());
        assert!(refusal("notwmic").is_none());
        assert!(refusal("Get-Content shellexecute.txt").is_some());
    }
}
