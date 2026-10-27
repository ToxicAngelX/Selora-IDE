//! Preserve `world.run` argument boundaries at the PowerShell boundary.
//!
//! PowerShell parses everything after `-Command` as code, even when the OS
//! received separate argv entries. Only the confirmed command is code;
//! runtime arguments must be data, not shell syntax. Numeric UTF-16 expressions
//! avoid both ASCII and PowerShell's typographic quote delimiters.

pub(crate) fn invocation(command: &str, arguments: &[String]) -> Result<String, &'static str> {
    // Windows PowerShell's legacy native argument binder drops empty strings.
    // Fail visibly rather than executing a different argv than requested.
    if arguments.iter().any(String::is_empty) {
        return Err("world.run cannot preserve empty arguments through Windows PowerShell");
    }
    if arguments.iter().any(|argument| argument.contains('"')) {
        return Err("world.run cannot preserve embedded double quotes through Windows PowerShell");
    }
    if arguments.is_empty() {
        return Ok(command.to_owned());
    }
    let literals = arguments
        .iter()
        .map(|argument| {
            let units = argument
                .encode_utf16()
                .map(|unit| unit.to_string())
                .collect::<Vec<_>>()
                .join(",");
            format!("([string]::new([char[]]@({units})))")
        })
        .collect::<Vec<_>>()
        .join(", ");
    Ok(format!("{command} @({literals})"))
}

#[cfg(test)]
mod tests {
    use super::invocation;

    #[test]
    fn leaves_the_confirmed_command_unchanged_without_arguments() {
        assert_eq!(invocation("git status", &[]).unwrap(), "git status");
    }

    #[test]
    fn encodes_arguments_without_any_shell_delimiters() {
        assert_eq!(
            invocation("git", &["a’".into(), "b'".into()]).unwrap(),
            "git @(([string]::new([char[]]@(97,8217))), ([string]::new([char[]]@(98,39))))"
        );
        assert!(invocation("Write-Output", &[String::new()]).is_err());
        assert!(invocation("node", &["a\"b".into()]).is_err());
    }

    #[cfg(windows)]
    #[test]
    fn powershell_does_not_execute_script_text_inside_an_argument() {
        let argument = "safe’); Write-Output INJECTED; # $env:SystemRoot `x";
        // Character codes avoid dependence on Windows' console code page.
        let script = format!(
            "{} | ForEach-Object {{ -join ([int[]][char[]]$_ | ForEach-Object {{ \"$_,\" }}) }}",
            invocation("Write-Output", &[argument.to_owned()]).unwrap()
        );
        let output = std::process::Command::new("powershell")
            .args(["-NoProfile", "-NonInteractive", "-Command", &script])
            .output()
            .expect("Windows PowerShell is available");
        assert!(output.status.success());
        let expected = argument
            .encode_utf16()
            .map(|unit| format!("{unit},"))
            .collect::<String>();
        assert_eq!(String::from_utf8_lossy(&output.stdout).trim_end(), expected);
    }

    #[cfg(windows)]
    #[test]
    fn powershell_preserves_supported_native_argument_boundaries() {
        if std::process::Command::new("node")
            .arg("--version")
            .output()
            .is_err()
        {
            eprintln!("Node is unavailable; native argument probe skipped");
            return;
        }
        let values = [
            "two words",
            "safe’); Write-Output INJECTED; #",
            "a'b",
            "trailing\\",
        ];
        let mut arguments = vec![
            "-e".to_owned(),
            "process.stdout.write(process.argv.slice(1).join('|'))".to_owned(),
        ];
        arguments.extend(values.iter().map(|value| (*value).to_owned()));
        let script = invocation("node", &arguments).unwrap();
        let output = std::process::Command::new("powershell")
            .args(["-NoProfile", "-NonInteractive", "-Command", &script])
            .output()
            .unwrap();
        assert!(output.status.success());
        assert_eq!(String::from_utf8_lossy(&output.stdout), values.join("|"));
    }
}
