//! Explicitly chosen local plaintext persistence. Separate from the usage/account DB
//! so ordinary statistics backups cannot accidentally include credentials.
use super::{validate_ref, SecretString, Vault, VaultError, VaultResult};
use rusqlite::{params, Connection, OpenFlags, OptionalExtension};
use std::fs::{self, OpenOptions};
use std::path::PathBuf;
use std::sync::Mutex;

pub struct LocalPlaintextVault {
    path: PathBuf,
    lock: Mutex<()>,
}

impl LocalPlaintextVault {
    /// Does not read/create anything at startup. A broken credential file must not
    /// prevent the app from displaying accounts and historical usage.
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            lock: Mutex::new(()),
        }
    }

    fn open(&self, create: bool) -> VaultResult<Option<Connection>> {
        let parent = self.path.parent().ok_or(VaultError::Unavailable)?;
        match fs::symlink_metadata(parent) {
            Ok(meta) if meta.file_type().is_dir() => {}
            Ok(_) => return Err(VaultError::Unavailable),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound && !create => return Ok(None),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                let mut builder = fs::DirBuilder::new();
                // Only the dedicated credentials directory may be created here.
                builder.recursive(false);
                #[cfg(unix)]
                {
                    use std::os::unix::fs::DirBuilderExt;
                    builder.mode(0o700);
                }
                builder
                    .create(parent)
                    .map_err(|_| VaultError::Unavailable)?;
            }
            Err(_) => return Err(VaultError::Unavailable),
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(parent, fs::Permissions::from_mode(0o700))
                .map_err(|_| VaultError::Unavailable)?;
        }

        let exists = match fs::symlink_metadata(&self.path) {
            Ok(meta) if meta.file_type().is_file() => true,
            Ok(_) => return Err(VaultError::Unavailable),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => false,
            Err(_) => return Err(VaultError::Unavailable),
        };
        if !exists && !create {
            return Ok(None);
        }
        let mut options = OpenOptions::new();
        options.read(true).write(true).create_new(!exists);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let file = options
            .open(&self.path)
            .map_err(|_| VaultError::Unavailable)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            file.set_permissions(fs::Permissions::from_mode(0o600))
                .map_err(|_| VaultError::Unavailable)?;
        }
        drop(file);
        let conn = Connection::open_with_flags(&self.path, OpenFlags::SQLITE_OPEN_READ_WRITE)
            .map_err(|_| VaultError::Backend)?;
        conn.busy_timeout(std::time::Duration::from_secs(2))
            .map_err(|_| VaultError::Backend)?;
        // DELETE avoids persistent WAL copies. It does not promise forensic erasure
        // of OS backups or SSD data. Never attach the main application database.
        conn.execute_batch(
            "PRAGMA journal_mode=DELETE; PRAGMA secure_delete=ON;
            CREATE TABLE IF NOT EXISTS secrets (vault_ref TEXT PRIMARY KEY, secret TEXT NOT NULL);",
        )
        .map_err(|_| VaultError::Backend)?;
        Ok(Some(conn))
    }
}

impl Vault for LocalPlaintextVault {
    fn store(&self, vault_ref: &str, secret: &SecretString) -> VaultResult<()> {
        validate_ref(vault_ref)?;
        let _guard = self.lock.lock().map_err(|_| VaultError::Backend)?;
        let conn = self.open(true)?.ok_or(VaultError::Unavailable)?;
        conn.execute(
            "INSERT INTO secrets(vault_ref,secret) VALUES (?1,?2)
            ON CONFLICT(vault_ref) DO UPDATE SET secret=excluded.secret",
            params![vault_ref, secret.expose()],
        )
        .map_err(|_| VaultError::Backend)?;
        Ok(())
    }

    fn get(&self, vault_ref: &str) -> VaultResult<SecretString> {
        validate_ref(vault_ref)?;
        let _guard = self.lock.lock().map_err(|_| VaultError::Backend)?;
        let conn = self.open(false)?.ok_or(VaultError::NotFound)?;
        conn.query_row(
            "SELECT secret FROM secrets WHERE vault_ref=?1",
            [vault_ref],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|_| VaultError::Backend)?
        .map(SecretString::new)
        .ok_or(VaultError::NotFound)
    }

    fn delete(&self, vault_ref: &str) -> VaultResult<()> {
        validate_ref(vault_ref)?;
        let _guard = self.lock.lock().map_err(|_| VaultError::Backend)?;
        if let Some(conn) = self.open(false)? {
            conn.execute("DELETE FROM secrets WHERE vault_ref=?1", [vault_ref])
                .map_err(|_| VaultError::Backend)?;
        }
        Ok(())
    }

    fn exists(&self, vault_ref: &str) -> VaultResult<bool> {
        validate_ref(vault_ref)?;
        let _guard = self.lock.lock().map_err(|_| VaultError::Backend)?;
        let Some(conn) = self.open(false)? else {
            return Ok(false);
        };
        conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM secrets WHERE vault_ref=?1)",
            [vault_ref],
            |row| row.get(0),
        )
        .map_err(|_| VaultError::Backend)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    struct TestDir(PathBuf);
    impl TestDir {
        fn new() -> Self {
            let p = std::env::temp_dir()
                .join(format!("providock-local-vault-{}", rand::random::<u64>()));
            fs::create_dir(&p).unwrap();
            Self(p)
        }
        fn path(&self) -> PathBuf {
            self.0.join("credentials").join("keys.sqlite3")
        }
    }
    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    #[test]
    fn missing_store_is_lazy_and_invalid_references_do_not_create_files() {
        let dir = TestDir::new();
        let v = LocalPlaintextVault::new(dir.path());
        assert!(!v.exists("synthetic-key").unwrap());
        assert!(matches!(v.get("synthetic-key"), Err(VaultError::NotFound)));
        v.delete("synthetic-key").unwrap();
        assert!(matches!(
            v.store("../invalid", &SecretString::new("synthetic")),
            Err(VaultError::InvalidRef)
        ));
        assert!(!dir.path().parent().unwrap().exists());
    }
    #[test]
    fn durable_replace_delete_preserves_other_credentials_after_restart() {
        let dir = TestDir::new();
        {
            let v = LocalPlaintextVault::new(dir.path());
            v.store("ref-a", &SecretString::new("synthetic-first"))
                .unwrap();
            v.store("ref-b", &SecretString::new("synthetic-other"))
                .unwrap();
            v.store("ref-a", &SecretString::new("synthetic-replaced"))
                .unwrap();
        }
        let v = LocalPlaintextVault::new(dir.path());
        assert!(v.exists("ref-a").unwrap());
        assert_eq!(v.get("ref-a").unwrap().expose(), "synthetic-replaced");
        v.delete("ref-a").unwrap();
        v.delete("ref-a").unwrap();
        assert!(matches!(v.get("ref-a"), Err(VaultError::NotFound)));
        assert_eq!(v.get("ref-b").unwrap().expose(), "synthetic-other");
        // Verify the deliberately plaintext on-disk contract independently.
        let conn = Connection::open(dir.path()).unwrap();
        let stored: String = conn
            .query_row(
                "SELECT secret FROM secrets WHERE vault_ref='ref-b'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(stored, "synthetic-other");
    }
    #[test]
    fn corrupt_file_is_preserved_and_error_does_not_echo_content_or_path() {
        let dir = TestDir::new();
        fs::create_dir(dir.path().parent().unwrap()).unwrap();
        let bytes = b"synthetic-secret-corrupt-file";
        fs::write(dir.path(), bytes).unwrap();
        let v = LocalPlaintextVault::new(dir.path());
        let err = v
            .store("ref-a", &SecretString::new("synthetic-new-secret"))
            .unwrap_err();
        let message = format!("{err:?} {err}");
        assert!(!message.contains("synthetic"));
        assert!(!message.contains(dir.path().to_str().unwrap()));
        assert_eq!(fs::read(dir.path()).unwrap(), bytes);
    }
    #[cfg(unix)]
    #[test]
    fn unix_directory_and_file_permissions_are_private() {
        use std::os::unix::fs::PermissionsExt;
        let dir = TestDir::new();
        let v = LocalPlaintextVault::new(dir.path());
        v.store("ref-a", &SecretString::new("synthetic")).unwrap();
        assert_eq!(
            fs::metadata(dir.path().parent().unwrap())
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
        assert_eq!(
            fs::metadata(dir.path()).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
    #[cfg(unix)]
    #[test]
    fn symlink_file_or_directory_is_rejected_without_modifying_target() {
        use std::os::unix::fs::symlink;
        let dir = TestDir::new();
        let target = dir.0.join("untouched");
        fs::write(&target, b"synthetic-untouched").unwrap();
        fs::create_dir(dir.path().parent().unwrap()).unwrap();
        symlink(&target, dir.path()).unwrap();
        let v = LocalPlaintextVault::new(dir.path());
        assert!(v.store("ref-a", &SecretString::new("synthetic")).is_err());
        assert_eq!(fs::read(&target).unwrap(), b"synthetic-untouched");
        fs::remove_file(dir.path()).unwrap();
        fs::remove_dir(dir.path().parent().unwrap()).unwrap();
        symlink(&dir.0, dir.path().parent().unwrap()).unwrap();
        assert!(v.store("ref-a", &SecretString::new("synthetic")).is_err());
        assert!(!dir.0.join("keys.sqlite3").exists());
    }
}
