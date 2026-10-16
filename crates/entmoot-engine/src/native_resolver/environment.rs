//! Select installed hosts without downloading tools or changing the parent environment.
use entmoot_contracts::{ToolEnvironment, ToolSelection};
use std::{collections::BTreeMap, ffi::OsString, path::PathBuf, process::Command, time::Duration};

pub(crate) fn select_tool(
    name: &str,
    explicit: &str,
    home: Option<&str>,
) -> Result<ToolEnvironment, String> {
    select(
        name,
        explicit,
        home,
        &std::env::vars_os()
            .filter_map(|(k, v)| k.into_string().ok().map(|k| (k, v)))
            .collect(),
    )
}

fn select(
    name: &str,
    explicit: &str,
    home: Option<&str>,
    env: &BTreeMap<String, OsString>,
) -> Result<ToolEnvironment, String> {
    let file = if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.into()
    };
    let chosen = if let Some(value) = env.get(explicit) {
        Some((
            PathBuf::from(value),
            ToolSelection::Environment {
                variable: explicit.into(),
            },
        ))
    } else if let Some((key, value)) = home.and_then(|key| env.get(key).map(|v| (key, v))) {
        let directory = if key == "DOTNET_ROOT" {
            PathBuf::from(value)
        } else {
            PathBuf::from(value).join("bin")
        };
        Some((
            directory.join(&file),
            ToolSelection::Environment {
                variable: key.into(),
            },
        ))
    } else {
        env.get("PATH")
            .or_else(|| env.get("Path"))
            .into_iter()
            .flat_map(std::env::split_paths)
            .map(|dir| dir.join(&file))
            .find(|p| p.is_file())
            .map(|p| {
                (
                    p,
                    ToolSelection::Path {
                        command: name.into(),
                    },
                )
            })
    };
    let (path, selection) = chosen.ok_or_else(|| {
        format!("invalid_environment: {name} unavailable; configure {explicit} or PATH")
    })?;
    if !path.is_file() {
        return Err(format!(
            "invalid_environment: selected {name} does not exist: {}",
            path.display()
        ));
    }
    let path =
        super::producers::sidecar::compiler_root(&path.canonicalize().map_err(|e| e.to_string())?);
    Ok(ToolEnvironment {
        role: name.into(),
        executable: path.to_string_lossy().into_owned(),
        selection,
        version: None,
    })
}

/// A small bounded host probe; output never enters source or structural identities.
pub(crate) fn probe_version(tool: &mut ToolEnvironment, wall: Duration) {
    use std::io::{Read, Seek, SeekFrom};
    let probe = || -> std::io::Result<Option<String>> {
        let mut output = tempfile::tempfile()?;
        let mut command = Command::new(&tool.executable);
        command
            .arg(if tool.role == "java" {
                "-version"
            } else {
                "--version"
            })
            .stdout(output.try_clone()?)
            .stderr(output.try_clone()?);
        if !crate::process::run(command, wall)?
            .status
            .is_ok_and(|status| status.success())
        {
            return Ok(None);
        }
        output.seek(SeekFrom::Start(0))?;
        let mut bytes = Vec::new();
        output.take(4096).read_to_end(&mut bytes)?;
        Ok(String::from_utf8_lossy(&bytes)
            .lines()
            .find(|s| !s.trim().is_empty())
            .map(|s| s.trim().to_owned()))
    };
    tool.version = probe().ok().flatten();
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;
    #[test]
    fn java_home_wins_over_path_and_broken_overrides_never_fall_back() {
        let root = tempfile::tempdir().unwrap();
        let name = if cfg!(windows) { "java.exe" } else { "java" };
        let home = root.path().join("jdk");
        let path = root.path().join("old-java");
        std::fs::create_dir_all(home.join("bin")).unwrap();
        std::fs::create_dir_all(&path).unwrap();
        std::fs::write(home.join("bin").join(name), b"").unwrap();
        std::fs::write(path.join(name), b"").unwrap();
        let mut env = BTreeMap::from([
            ("JAVA_HOME".into(), home.as_os_str().to_owned()),
            ("PATH".into(), path.as_os_str().to_owned()),
        ]);
        let selected = select("java", "ENTMOOT_JAVA", Some("JAVA_HOME"), &env).unwrap();
        assert_eq!(
            Path::new(&selected.executable),
            super::super::producers::sidecar::compiler_root(
                &home.join("bin").join(name).canonicalize().unwrap()
            )
        );
        assert!(
            matches!(selected.selection, ToolSelection::Environment{variable} if variable=="JAVA_HOME")
        );
        env.insert(
            "ENTMOOT_JAVA".into(),
            root.path().join("missing").into_os_string(),
        );
        assert!(select("java", "ENTMOOT_JAVA", Some("JAVA_HOME"), &env).is_err());
        env.remove("ENTMOOT_JAVA");
        env.remove("JAVA_HOME");
        assert!(matches!(
            select("java", "ENTMOOT_JAVA", Some("JAVA_HOME"), &env)
                .unwrap()
                .selection,
            ToolSelection::Path { .. }
        ));
        let dotnet = if cfg!(windows) {
            "dotnet.exe"
        } else {
            "dotnet"
        };
        std::fs::write(home.join(dotnet), b"").unwrap();
        env.insert("DOTNET_ROOT".into(), home.into_os_string());
        assert!(select("dotnet", "ENTMOOT_DOTNET", Some("DOTNET_ROOT"), &env).is_ok());
    }
}
