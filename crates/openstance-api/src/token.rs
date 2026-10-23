//! The API bearer token (interfaces.md section 3, threat model T3/T5): 32
//! random bytes from the OS CSPRNG, base64url without padding, stored in
//! `serve.token_path` (default `~/.openstance/api.token`) with mode 0600.
//! Created by `openstance init` or the first `serve`; `serve --rotate-token`
//! replaces it. Compared in constant time and never logged.

use std::path::{Path, PathBuf};

use base64::Engine as _;
use subtle::ConstantTimeEq;

/// A loaded token. `Debug` never prints the secret.
#[derive(Clone, PartialEq, Eq)]
pub struct Token(String);

impl std::fmt::Debug for Token {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Token(<redacted>)")
    }
}

impl Token {
    /// A fresh random token.
    pub fn generate() -> std::io::Result<Token> {
        let mut bytes = [0u8; 32];
        getrandom::fill(&mut bytes).map_err(std::io::Error::other)?;
        Ok(Token(
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes),
        ))
    }

    /// Wrap a known value (tests, a token read elsewhere).
    pub fn from_secret(s: impl Into<String>) -> Token {
        Token(s.into())
    }

    /// The secret, for the `#token=` link and `--print-token` only.
    pub fn expose(&self) -> &str {
        &self.0
    }

    /// Constant-time comparison with a presented token.
    pub fn verify(&self, presented: &str) -> bool {
        let (a, b) = (self.0.as_bytes(), presented.as_bytes());
        // `ct_eq` on slices of different lengths is false without comparing
        // contents; only the length can leak, and every token is 43 bytes.
        !a.is_empty() && bool::from(a.ct_eq(b))
    }
}

/// What [`ensure`] did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Loaded {
    /// The token.
    pub token: Token,
    /// The file.
    pub path: PathBuf,
    /// The file was created (or replaced by a rotation) just now.
    pub created: bool,
}

/// Load the token at `path`, creating it (0600, parent 0700) when missing
/// or empty.
pub fn ensure(path: &Path) -> std::io::Result<Loaded> {
    match std::fs::read_to_string(path) {
        Ok(s) if !s.trim().is_empty() => Ok(Loaded {
            token: Token(s.trim().to_string()),
            path: path.to_path_buf(),
            created: false,
        }),
        Ok(_) => rotate(path),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => rotate(path),
        Err(e) => Err(e),
    }
}

/// Replace the token at `path` with a new one.
pub fn rotate(path: &Path) -> std::io::Result<Loaded> {
    let token = Token::generate()?;
    let dir = path
        .parent()
        .filter(|d| !d.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    if !dir.exists() {
        std::fs::create_dir_all(dir)?;
        set_mode(dir, 0o700)?;
    }
    // Write a private temp file next to the target, then rename it over the
    // old token, so the file is never readable by others or half-written.
    let mut tmp = tempfile::Builder::new()
        .prefix(".api.token.")
        .tempfile_in(dir)?;
    {
        use std::io::Write;
        tmp.write_all(token.0.as_bytes())?;
        tmp.write_all(b"\n")?;
        tmp.as_file().sync_all()?;
    }
    set_mode(tmp.path(), 0o600)?;
    tmp.persist(path).map_err(|e| e.error)?;
    Ok(Loaded {
        token,
        path: path.to_path_buf(),
        created: true,
    })
}

#[cfg(unix)]
fn set_mode(path: &Path, mode: u32) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))
}

#[cfg(not(unix))]
fn set_mode(_path: &Path, _mode: u32) -> std::io::Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_tokens_are_43_char_base64url_and_distinct() {
        let a = Token::generate().unwrap();
        let b = Token::generate().unwrap();
        assert_eq!(a.expose().len(), 43, "32 bytes, no padding");
        assert!(a
            .expose()
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_'));
        assert_ne!(a, b);
    }

    #[test]
    fn verify_accepts_only_the_exact_token() {
        let t = Token::from_secret("abc123");
        assert!(t.verify("abc123"));
        assert!(!t.verify("abc124"));
        assert!(!t.verify("abc12"));
        assert!(!t.verify(""));
        assert!(!t.verify("abc1234"));
    }

    #[test]
    fn debug_redacts_the_secret() {
        let t = Token::from_secret("supersecret");
        assert!(!format!("{t:?}").contains("supersecret"));
    }

    #[test]
    fn ensure_creates_a_private_file_once() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("home").join("api.token");
        let first = ensure(&path).unwrap();
        assert!(first.created);
        assert_eq!(first.path, path);
        let second = ensure(&path).unwrap();
        assert!(!second.created);
        assert_eq!(first.token, second.token);
        assert_eq!(
            std::fs::read_to_string(&path).unwrap().trim(),
            first.token.expose()
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
            let dmode = std::fs::metadata(path.parent().unwrap())
                .unwrap()
                .permissions()
                .mode()
                & 0o777;
            assert_eq!(dmode, 0o700);
        }
    }

    #[test]
    fn ensure_replaces_an_empty_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("api.token");
        std::fs::write(&path, "\n").unwrap();
        let l = ensure(&path).unwrap();
        assert!(l.created);
        assert_eq!(l.token.expose().len(), 43);
    }

    #[test]
    fn rotate_invalidates_the_old_token() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("api.token");
        let old = ensure(&path).unwrap().token;
        let new = rotate(&path).unwrap();
        assert!(new.created);
        assert_ne!(old, new.token);
        assert_eq!(ensure(&path).unwrap().token, new.token);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
        }
    }
}
