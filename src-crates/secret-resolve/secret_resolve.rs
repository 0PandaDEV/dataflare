//! # Secret Resolver Library
//!
//! This library provides a mechanism for resolving secrets from multiple sources.
//! - **`env:`** - Resolves environment variables or keys from `.env` files
//! - **`file:`** - Reads secrets from local files
//! - **`exec:`** - Executes shell commands to retrieve secrets
//! - **Plain text** - Returns plain text values as-is
//!
//! TODO:
//! Read from the keychain? 'keychain: service/account'
//! Read from popups or other secure input methods? 'ask: prompt message'
extern crate self as secret_resolve;

use std::env;
use std::io;
use std::time::Duration;
use tokio::fs;
use tokio::process::Command;
use tokio::time::timeout;

pub use secret_resolve_derive::ResolveSecrets;

#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;

// TODO: Output from .profile can pollute the resolved secret.

#[cfg(target_os = "linux")]
const SHELL: (&str, &[&str]) = ("/bin/sh", &["-l", "-c"]);
#[cfg(target_os = "macos")]
const SHELL: (&str, &[&str]) = ("/bin/sh", &["-l", "-c"]);
#[cfg(target_os = "windows")]
const SHELL: (&str, &[&str]) = ("cmd.exe", &["/C"]);

const MAX_FILE_SIZE: u64 = 1024 * 1024;

const EXEC_TIMEOUT: Duration = Duration::from_secs(30);

// List of sensitive commands restricted to mitigate accidental misuse or basic security risks
// While not a bulletproof security barrier, this serves as a safeguard against common mistakes
const FORBIDDEN_COMMANDS: &'static [&'static str] = &[
    "rm", "rmdir", "dd", "mkfs", "reboot", "shutdown", "poweroff", "halt", "sudo", "su", "mount",
    "umount", "chown", "chmod", "chroot", "killall", "kill", "fork", "forkbomb",
    // For Windows
    "del", "erase", "rd", "format", "diskpart", "runas", "taskkill", "tskill", "deltree", "fsutil",
    "vssadmin", "wbadmin", "bcdedit", "takeown", "icacls",
];

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("IO error for `{path}`: {source}")]
    Io { path: String, source: io::Error },
    #[error("File too large: `{path}`")]
    FileTooLarge { path: String },

    #[error("Environment variable `{name}` not found")]
    EnvVarNotFound { name: String },
    #[error("Environment variable `{key}` not found in file `{path}`")]
    EnvFileKeyNotFound { path: String, key: String },

    #[error("Failed to run command: `{command}`")]
    CommandFailed { command: String, source: io::Error },
    #[error("Command timed out: `{command}`")]
    CommandTimedOut { command: String },
    #[error("Command produced no output: `{command}`")]
    CommandNoOutput { command: String },
    #[error(
        "Command exited with non-zero status: code: `{code:?}`, stderr: `{stderr}`, command: `{command}`"
    )]
    CommandNonZeroExit {
        command: String,
        code: Option<i32>,
        stderr: String,
    },
    #[error("Dangerous command not allowed: `{command}`")]
    DangerousCommand { command: String },
}

pub async fn resolve_secret(value: &str) -> Result<String, Error> {
    Secret::resolve(value).await
}

#[allow(async_fn_in_trait)]
pub trait ResolveSecrets {
    async fn resolve_secrets(&mut self) -> Result<(), Error>;
}

struct Secret;

impl Secret {
    async fn resolve(secret: impl AsRef<str>) -> Result<String, Error> {
        let secret = secret.as_ref();
        // NOTE: Keep these prefixes in sync with src-web/ui/password-input.tsx.
        let trimmed = secret.trim_start();
        if let Some(rest) = trimmed.strip_prefix("env:") {
            return Self::resolve_env(rest.trim()).await;
        }
        if let Some(rest) = trimmed.strip_prefix("file:") {
            return Self::resolve_file(rest.trim()).await;
        }
        if let Some(rest) = trimmed.strip_prefix("exec:") {
            return Self::resolve_exec(rest.trim()).await;
        }
        Ok(secret.to_string())
    }

    async fn resolve_env(rest: &str) -> Result<String, Error> {
        let Some((path, key)) = rest.split_once('#') else {
            let v = env::var(rest).map_err(|_| Error::EnvVarNotFound {
                name: rest.to_string(),
            })?;
            return Ok(v);
        };

        let path = path.trim_end();
        let key = key.trim_start();

        let content = fs::read(path).await.map_err(|err| Error::Io {
            path: path.into(),
            source: err,
        })?;
        let mut iter = dotenvy::from_read_iter(&content[..]);
        while let Some(rst) = iter.next() {
            if let Ok((k, v)) = rst {
                if k == key {
                    return Ok(v);
                }
            }
        }

        Err(Error::EnvFileKeyNotFound {
            path: path.to_string(),
            key: key.to_string(),
        })
    }

    async fn resolve_file(path: &str) -> Result<String, Error> {
        let meta = fs::metadata(path).await.map_err(|err| Error::Io {
            path: path.into(),
            source: err,
        })?;
        if meta.len() > MAX_FILE_SIZE {
            return Err(Error::FileTooLarge { path: path.into() });
        }
        let content = fs::read_to_string(path).await.map_err(|err| Error::Io {
            path: path.into(),
            source: err,
        })?;
        let rst = content.trim_end().to_string();
        Ok(rst)
    }

    async fn resolve_exec(command: &str) -> Result<String, Error> {
        for forbidden in FORBIDDEN_COMMANDS {
            if command.starts_with(forbidden) {
                let after = &command[forbidden.len()..];
                if after.is_empty() || after.chars().next().map_or(false, char::is_whitespace) {
                    return Err(Error::DangerousCommand {
                        command: command.to_string(),
                    });
                }
            }
        }

        let (shell, args) = SHELL;
        let mut process = Command::new(shell);
        process.args(args).arg(command);
        #[cfg(target_os = "windows")]
        {
            const CREATE_NO_WINDOW: u32 = 0x08000000;
            process.as_std_mut().creation_flags(CREATE_NO_WINDOW);
        }

        let output = timeout(EXEC_TIMEOUT, process.output())
            .await
            .map_err(|_| Error::CommandTimedOut {
                command: command.into(),
            })?
            .map_err(|e| Error::CommandFailed {
                command: command.into(),
                source: e,
            })?;

        if !output.status.success() {
            return Err(Error::CommandNonZeroExit {
                command: command.into(),
                code: output.status.code(),
                stderr: String::from_utf8_lossy(&output.stderr).into(),
            });
        }
        let stdout = String::from_utf8_lossy(&output.stdout);
        let rst = stdout.trim_end().to_string();
        if rst.is_empty() {
            return Err(Error::CommandNoOutput {
                command: command.into(),
            });
        }
        Ok(rst)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::fs;

    fn temp_path() -> String {
        let pid = std::process::id();
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let count = COUNTER.fetch_add(1, Ordering::SeqCst);
        let path = env::temp_dir().join(format!("secret_resolve_test_{}_{}", pid, count));
        path.display().to_string()
    }

    #[tokio::test]
    async fn test_resolve_plain() {
        assert_eq!(Secret::resolve("plain").await.unwrap(), "plain");
        assert_eq!(Secret::resolve("  plain").await.unwrap(), "  plain");
        assert_eq!(
            Secret::resolve("  123456\r\n").await.unwrap(),
            "  123456\r\n"
        );
        assert_eq!(Secret::resolve("asd:asd").await.unwrap(), "asd:asd");
        assert_eq!(Secret::resolve("file :asd").await.unwrap(), "file :asd");
    }

    #[tokio::test]
    async fn test_resolve_empty() {
        // TODO
        // let result = Secret::resolve("env:").await.unwrap();
        // let result = Secret::resolve("env:#TOKEN").await.unwrap();
        // let result = Secret::resolve("file:").await.unwrap();
        // let result = Secret::resolve("exec:").await.unwrap();
    }

    #[tokio::test]
    async fn test_resolve_env_var() {
        unsafe { env::set_var("TEST_SECRET_VAR", "secret_value") };

        let result = Secret::resolve("env:TEST_SECRET_VAR").await.unwrap();
        assert_eq!(result, "secret_value");

        let result = Secret::resolve("  env:  TEST_SECRET_VAR ").await.unwrap();
        assert_eq!(result, "secret_value");

        let result = Secret::resolve("  env:  NONEXISTENT ").await;
        assert!(matches!(result, Err(Error::EnvVarNotFound { .. })));
    }

    #[tokio::test]
    async fn test_resolve_env_file() {
        let path = temp_path();
        fs::write(
            &path,
            r#"
            DB_PASSWORD=file_secret
            OTHER=value
        "#,
        )
        .await
        .unwrap();

        let result = Secret::resolve(&format!("env:{}#DB_PASSWORD", path))
            .await
            .unwrap();
        assert_eq!(result, "file_secret");

        let result = Secret::resolve(&format!("env:  {}  #  OTHER ", path))
            .await
            .unwrap();
        assert_eq!(result, "value");

        let result = Secret::resolve(&format!("env:{}#NONEXISTENT", path)).await;
        assert!(matches!(result, Err(Error::EnvFileKeyNotFound { .. })));

        fs::remove_file(&path).await.unwrap();
    }

    #[tokio::test]
    async fn test_resolve_file() {
        let path = temp_path();
        fs::write(&path, "my_file_secret\n").await.unwrap();

        let result = Secret::resolve(&format!("file:{}", path)).await.unwrap();
        assert_eq!(result, "my_file_secret");

        let result = Secret::resolve(&format!("file:   {}\n", path))
            .await
            .unwrap();
        assert_eq!(result, "my_file_secret");

        let result = Secret::resolve("file:/nonexistent/path/file.txt").await;
        assert!(matches!(result, Err(Error::Io { .. })));

        fs::remove_file(&path).await.unwrap();
    }

    #[tokio::test]
    async fn test_resolve_exec() {
        let result = Secret::resolve("exec:echo exec_secret").await.unwrap();
        assert_eq!(result, "exec_secret");

        let result = Secret::resolve("exec:echo $HOME").await.unwrap();
        assert!(!result.is_empty());

        let result = Secret::resolve("exec:printf 'hello\\n'").await.unwrap();
        assert_eq!(result, "hello");

        let result = Secret::resolve("exec:echo 'hello world' | awk '{print $2}'")
            .await
            .unwrap();
        assert_eq!(result, "world");
    }

    #[tokio::test]
    async fn test_resolve_exec_non_zero_exit() {
        let result = Secret::resolve("exec:exit 1").await;
        assert!(matches!(result, Err(Error::CommandNonZeroExit { .. })));

        let result = Secret::resolve("exec:nonexistent_command").await;
        assert!(matches!(result, Err(Error::CommandNonZeroExit { .. })));
    }

    #[cfg(not(target_os = "windows"))]
    #[tokio::test]
    async fn test_resolve_exec_no_output() {
        let result = Secret::resolve("exec: sleep 1").await;
        assert!(matches!(result, Err(Error::CommandNoOutput { .. })));

        let result = Secret::resolve("exec: echo ''").await;
        assert!(matches!(result, Err(Error::CommandNoOutput { .. })));
    }

    // #[tokio::test]
    // async fn test_resolve_exec_timeout() {
    //     let result = Secret::resolve("exec:sleep 10").await;
    //     assert!(matches!(result, Err(Error::CommandTimedOut { .. })));
    // }

    #[tokio::test]
    async fn test_empty_derive() {
        #[derive(ResolveSecrets, PartialEq, Eq, Debug)]
        struct S {}
        let mut config = S {};
        config.resolve_secrets().await.unwrap();
        assert_eq!(config, S {});
    }

    #[tokio::test]
    async fn test_simple_derive() {
        #[derive(ResolveSecrets, PartialEq, Eq, Debug)]
        struct S {
            #[secret]
            value: String,
            #[secret]
            optional: Option<String>,
        }
        let mut config = S {
            value: "exec: echo value".to_string(),
            optional: Some("exec: echo optional".to_string()),
        };
        config.resolve_secrets().await.unwrap();
        assert_eq!(
            config,
            S {
                value: "value".to_string(),
                optional: Some("optional".to_string()),
            }
        );
    }

    #[tokio::test]
    async fn test_resolve_secrets_derive() {
        #[derive(ResolveSecrets)]
        struct Nested {
            #[secret]
            value: String,
        }

        #[derive(ResolveSecrets)]
        struct Config {
            #[secret]
            password: String,
            #[secret]
            optional: Option<String>,
            #[secret]
            nested: Nested,
        }

        #[derive(ResolveSecrets)]
        enum Auth {
            Password {
                user: String,
                #[secret]
                password: String,
            },
        }

        #[derive(ResolveSecrets)]
        enum Source {
            Nested(Nested),
        }

        let mut config = Config {
            password: "plain".to_string(),
            optional: Some("optional".to_string()),
            nested: Nested {
                value: "nested".to_string(),
            },
        };
        config.resolve_secrets().await.unwrap();
        assert_eq!(config.password, "plain");
        assert_eq!(config.optional.as_deref(), Some("optional"));
        assert_eq!(config.nested.value, "nested");

        let mut auth = Auth::Password {
            user: "user".to_string(),
            password: "password".to_string(),
        };
        auth.resolve_secrets().await.unwrap();
        let Auth::Password { user, password } = auth;
        assert_eq!(user, "user");
        assert_eq!(password, "password");

        let mut source = Source::Nested(Nested {
            value: "source".to_string(),
        });
        source.resolve_secrets().await.unwrap();
        let Source::Nested(nested) = source;
        assert_eq!(nested.value, "source");
    }
}
