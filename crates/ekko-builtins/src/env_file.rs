//! `.ekko-env`: extra environment for freshly spawned session shells, proving
//! the `BeforePtySpawn` interception path end to end. If the session's cwd
//! contains a `.ekko-env` file of `KEY=VALUE` lines (blank lines and `#`
//! comments ignored), those variables are added to the shell's environment.

use std::path::Path;
use std::sync::Arc;

use anyhow::Result;
use ekko_ext::{
    EventHandlerRegistration, EventKind, EventPayload, EventReturn, Extension, ExtensionHost,
    ExtensionManifest,
};

pub const ENV_FILE_NAME: &str = ".ekko-env";

pub struct EnvFileExtension;

pub fn parse_env_file(content: &str) -> Vec<(String, String)> {
    content
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .filter_map(|line| {
            let (key, value) = line.split_once('=')?;
            let key = key.trim();
            if key.is_empty() {
                return None;
            }
            Some((key.to_string(), value.trim().to_string()))
        })
        .collect()
}

fn env_for_cwd(cwd: &Path) -> Vec<(String, String)> {
    match std::fs::read_to_string(cwd.join(ENV_FILE_NAME)) {
        Ok(content) => parse_env_file(&content),
        Err(_) => Vec::new(),
    }
}

impl Extension for EnvFileExtension {
    fn manifest(&self) -> ExtensionManifest {
        ExtensionManifest {
            id: "ekko-builtins.env-file".into(),
            name: ".ekko-env".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            description: "per-project extra environment for session shells".into(),
        }
    }

    fn register(&self, host: &mut dyn ExtensionHost) -> Result<()> {
        host.subscribe(EventHandlerRegistration {
            event: EventKind::BeforePtySpawn,
            label: "ekko-builtins.env-file/inject".into(),
            handler: Arc::new(|event| {
                let EventPayload::PtySpawn { cwd, .. } = &event.payload else {
                    return Ok(None);
                };
                let env = env_for_cwd(cwd);
                if env.is_empty() {
                    return Ok(None);
                }
                Ok(Some(EventReturn::PtySpawnOverride {
                    shell: None,
                    cwd: None,
                    env,
                }))
            }),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_key_value_lines_skipping_comments_and_blanks() {
        let parsed = parse_env_file("# comment\nFOO=bar\n\n  BAZ = qux value \nBROKEN\n=nokey\n");
        assert_eq!(
            parsed,
            vec![
                ("FOO".to_string(), "bar".to_string()),
                ("BAZ".to_string(), "qux value".to_string()),
            ]
        );
    }
}
