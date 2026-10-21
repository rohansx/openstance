//! The age X25519 keypair used to encrypt and decrypt views (P3-CORE-09).
//!
//! The identity lives at `share.keypair` (default `~/.openstance/age.key`,
//! mode 0600) in the `age-keygen` file format: two comment lines (creation
//! time and public key) and the `AGE-SECRET-KEY-1...` line. It is generated
//! on first need and never overwritten. Spec:
//! `docs/tech-spec/sharing-and-views.md` section 5.

use std::io::Write;
use std::path::{Path, PathBuf};

use age::secrecy::ExposeSecret;
use age::x25519;

use crate::config::{expand_path, Config, Paths};
use crate::error::{Error, Result};

/// The key file path from `share.keypair`, expanded against the home dirs.
pub fn key_path(cfg: &Config, paths: &Paths, user_home: Option<&Path>) -> PathBuf {
    expand_path(&cfg.share.keypair, &paths.home, user_home)
}

/// A loaded identity and where it came from.
pub struct KeyPair {
    /// The secret identity.
    pub identity: x25519::Identity,
    /// The key file.
    pub path: PathBuf,
    /// The file was created by this call.
    pub created: bool,
}

impl std::fmt::Debug for KeyPair {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("KeyPair")
            .field("public", &self.public())
            .field("path", &self.path)
            .field("created", &self.created)
            .finish_non_exhaustive()
    }
}

impl KeyPair {
    /// The `age1...` public key to share.
    pub fn public(&self) -> String {
        self.identity.to_public().to_string()
    }
}

/// Load the identity at `path`.
pub fn load(path: &Path) -> Result<KeyPair> {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err(Error::NotFound(format!(
                "no age key at {}; run `openstance keys show` to create one",
                path.display()
            )))
        }
        Err(e) => return Err(Error::io("reading", path, e)),
    };
    let line = text
        .lines()
        .map(str::trim)
        .find(|l| l.starts_with("AGE-SECRET-KEY-"))
        .ok_or_else(|| Error::BadRequest(format!("{} holds no age identity", path.display())))?;
    let identity: x25519::Identity = line.parse().map_err(|_| {
        Error::BadRequest(format!("{} holds an invalid age identity", path.display()))
    })?;
    Ok(KeyPair {
        identity,
        path: path.to_path_buf(),
        created: false,
    })
}

/// Load the identity at `path`, generating it (mode 0600) when missing.
/// An existing file is never overwritten.
pub fn ensure(path: &Path, now: i64) -> Result<KeyPair> {
    if path.exists() {
        return load(path);
    }
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        if !dir.exists() {
            std::fs::create_dir_all(dir).map_err(|e| Error::io("creating", dir, e))?;
            set_mode(dir, 0o700)?;
        }
    }
    let identity = x25519::Identity::generate();
    let created = time::OffsetDateTime::from_unix_timestamp(now)
        .ok()
        .and_then(|t| {
            t.format(&time::format_description::well_known::Rfc3339)
                .ok()
        })
        .unwrap_or_default();
    let body = format!(
        "# created: {created}\n# public key: {}\n{}\n",
        identity.to_public(),
        identity.to_string().expose_secret()
    );
    let mut f = create_private(path)?;
    f.write_all(body.as_bytes())
        .and_then(|()| f.sync_all())
        .map_err(|e| Error::io("writing", path, e))?;
    Ok(KeyPair {
        identity,
        path: path.to_path_buf(),
        created: true,
    })
}

/// Create a new file with mode 0600, failing if it already exists.
pub(crate) fn create_private(path: &Path) -> Result<std::fs::File> {
    let mut o = std::fs::OpenOptions::new();
    o.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        o.mode(0o600);
    }
    o.open(path).map_err(|e| Error::io("creating", path, e))
}

#[cfg(unix)]
pub(crate) fn set_mode(path: &Path, mode: u32) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))
        .map_err(|e| Error::io("setting permissions on", path, e))
}

#[cfg(not(unix))]
pub(crate) fn set_mode(_path: &Path, _mode: u32) -> Result<()> {
    Ok(())
}

/// Parse one `age1...` recipient.
pub fn parse_recipient(s: &str) -> Result<x25519::Recipient> {
    s.trim()
        .parse()
        .map_err(|_| Error::BadRequest(format!("{s:?} is not an age X25519 recipient (age1...)")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ensure_creates_a_0600_key_and_never_overwrites_it() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("home").join("age.key");
        let first = ensure(&path, 1_792_454_400).unwrap();
        assert!(first.created);
        assert!(first.public().starts_with("age1"));
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("# public key: age1"), "{text}");
        assert!(text.contains("AGE-SECRET-KEY-1"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
        }
        let again = ensure(&path, 2).unwrap();
        assert!(!again.created);
        assert_eq!(again.public(), first.public());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), text);
    }

    #[test]
    fn load_reports_a_missing_key_as_not_found() {
        let dir = tempfile::tempdir().unwrap();
        let err = load(&dir.path().join("nope.key")).unwrap_err();
        assert!(matches!(err, Error::NotFound(_)), "{err}");
    }

    #[test]
    fn load_rejects_garbage() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("k");
        std::fs::write(&p, "AGE-SECRET-KEY-1NOPE\n").unwrap();
        assert!(matches!(load(&p).unwrap_err(), Error::BadRequest(_)));
    }

    #[test]
    fn recipients_parse_or_explain() {
        let dir = tempfile::tempdir().unwrap();
        let k = ensure(&dir.path().join("k"), 0).unwrap();
        assert_eq!(
            parse_recipient(&k.public()).unwrap().to_string(),
            k.public()
        );
        assert!(parse_recipient("alice").is_err());
    }
}
