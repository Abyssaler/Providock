//! 路径规范化与根目录约束（T03）。
//!
//! 目标（docs/specs/SECURITY_PRIVACY.md 第 3 节）：只允许读取用户显式选择的根目录下的文件，
//! 拒绝任何逃逸该根的路径请求（`..`、根外绝对路径、符号链接 / Windows junction / 盘符切换 / UNC）。
//! IPC 层的 `session_ref` 必须由后端查索引解析，绝不接受前端任意系统路径。
//!
//! 处理顺序：
//! 1. 规范化根目录（`fs::canonicalize`，解析符号链接）；
//! 2. 把请求路径归约为相对根的组件序列，词法拒绝 `..` 逃逸；
//! 3. 解析最深已存在祖先的真实路径，要求其仍在根内，再拼回不存在的尾段。
//!
//! 平台差异（重要）：
//! - macOS/Linux：`fs::canonicalize` 解析全部符号链接；`Path::strip_prefix` 是**区分大小写**的
//!   按组件比较。macOS 默认文件系统大小写不敏感，但 canonicalize 返回磁盘上的真实大小写。
//! - Windows：`fs::canonicalize` 返回 `\\?\` 扩展长度（verbatim）路径；`\\?\` 前缀、盘符 `C:` 与
//!   UNC `\\server\share` 都按组件处理。跨盘符的绝对路径 `strip_prefix` 必然失败，判定为根外拒绝。
//!   junction / reparse point 由 canonicalize 解析，语义同符号链接。verbatim 前缀会绕过部分路径
//!   解析，因此这里只做“规范化后仍位于根内”的包含性校验，不额外信任任何前缀。
//!
//! 词法 `..` 先于符号链接解析：`link/../x` 会先被归约为根下 `x`，即使 `link` 指向根外也不会穿越
//! （保守、可解释；与 POSIX realpath 的语义不同，但绝不产生逃逸）。

use std::path::{Component, Path, PathBuf};

#[derive(Debug, thiserror::Error)]
pub enum SafePathError {
    #[error("selected root does not exist or cannot be resolved")]
    RootUnavailable,
    #[error("path escapes the selected root")]
    EscapesRoot,
    #[error("path is not inside the selected root")]
    OutsideRoot,
    #[error("path cannot be normalized")]
    InvalidPath,
    #[error("io error while resolving path: {0}")]
    Io(#[from] std::io::Error),
}

/// 把已相对根的路径做词法归一化：去掉 `.`，就地消解 `..`；任何越出根的 `..` 即拒绝。
pub fn normalize_relative(requested: &Path) -> Result<PathBuf, SafePathError> {
    let mut out = PathBuf::new();
    for component in requested.components() {
        match component {
            Component::CurDir => {}
            Component::Normal(segment) => out.push(segment),
            Component::ParentDir => {
                if !out.pop() {
                    return Err(SafePathError::EscapesRoot);
                }
            }
            // 相对路径里不应再出现根/盘符/UNC 前缀；调用方应先把根前缀剥离
            Component::RootDir | Component::Prefix(_) => return Err(SafePathError::InvalidPath),
        }
    }
    Ok(out)
}

/// 将 `requested` 解析为根内真实路径。
///
/// - `requested` 可为相对路径（相对根）或绝对路径（必须位于根内）；
/// - 返回的是规范化后的真实路径（符号链接已解析），保证位于规范化根目录之下；
/// - 不存在的叶子路径只要其父目录在根内即放行（用于按需读取尚未出现/新建的文件名）。
pub fn resolve_within_root(root: &Path, requested: &Path) -> Result<PathBuf, SafePathError> {
    let canonical_root = std::fs::canonicalize(root).map_err(|_| SafePathError::RootUnavailable)?;

    // 绝对路径：分别对规范化根与原始根尝试剥离前缀（后者兼容符号链接形式的根路径）
    let relative: PathBuf = if requested.is_absolute() {
        requested
            .strip_prefix(&canonical_root)
            .or_else(|_| requested.strip_prefix(root))
            .map(Path::to_path_buf)
            .map_err(|_| SafePathError::OutsideRoot)?
    } else {
        requested.to_path_buf()
    };

    let normalized = normalize_relative(&relative)?;
    let target = canonical_root.join(&normalized);

    // 解析最深已存在祖先，拒绝其真实路径逃逸根目录
    let (existing, tail) = split_existing_ancestor(&target)?;
    let real_existing = std::fs::canonicalize(&existing)?;
    if !real_existing.starts_with(&canonical_root) {
        return Err(SafePathError::EscapesRoot);
    }

    let mut resolved = real_existing;
    for segment in tail.iter().rev() {
        resolved.push(segment);
    }
    // 尾段均为 Normal（含 `..` 的路径已在词法归一化中消解），应仍在根内
    if !resolved.starts_with(&canonical_root) {
        return Err(SafePathError::EscapesRoot);
    }
    Ok(resolved)
}

/// 拆出“已存在的祖先路径”和“不存在的有序尾段”。
///
/// 若遇到损坏的符号链接（`symlink_metadata` 成功但 canonicalize 失败），由调用方的
/// `canonicalize` 返回错误而拒绝——不能信任指向根外的悬空链接。
fn split_existing_ancestor(
    target: &Path,
) -> Result<(PathBuf, Vec<std::ffi::OsString>), SafePathError> {
    let mut probe = target.to_path_buf();
    let mut tail: Vec<std::ffi::OsString> = Vec::new();
    loop {
        match std::fs::symlink_metadata(&probe) {
            Ok(_) => return Ok((probe, tail)),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                match probe.file_name() {
                    Some(name) => {
                        tail.push(name.to_os_string());
                        probe.pop();
                    }
                    // 已到根仍不存在（根已 canonicalize，理论上不可达）
                    None => return Ok((probe, tail)),
                }
            }
            Err(err) => return Err(SafePathError::Io(err)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    /// 自建临时目录（不引入 tempfile）：temp_dir + 随机名，Drop 时清理。
    struct TempDir {
        path: PathBuf,
    }

    impl TempDir {
        fn new(tag: &str) -> Self {
            static COUNTER: AtomicU64 = AtomicU64::new(0);
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let name = format!(
                "providock-safepath-{tag}-{}-{nanos}-{}",
                std::process::id(),
                COUNTER.fetch_add(1, Ordering::Relaxed)
            );
            let path = std::env::temp_dir().join(name);
            std::fs::create_dir_all(&path).unwrap();
            Self { path }
        }

        fn path(&self) -> &Path {
            &self.path
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }

    fn canon(path: &Path) -> PathBuf {
        std::fs::canonicalize(path).unwrap()
    }

    #[test]
    fn allows_relative_path_inside_root() {
        let root = TempDir::new("inside");
        std::fs::write(root.path().join("a.jsonl"), b"x").unwrap();
        let got = resolve_within_root(root.path(), Path::new("a.jsonl")).unwrap();
        assert_eq!(got, canon(root.path()).join("a.jsonl"));
    }

    #[test]
    fn allows_absolute_path_inside_root() {
        let root = TempDir::new("abs");
        std::fs::write(root.path().join("b.jsonl"), b"x").unwrap();
        let absolute = root.path().join("b.jsonl");
        let got = resolve_within_root(root.path(), &absolute).unwrap();
        assert_eq!(got, canon(&absolute));
    }

    #[test]
    fn rejects_parent_escape() {
        let root = TempDir::new("parent");
        assert!(matches!(
            resolve_within_root(root.path(), Path::new("../outside")),
            Err(SafePathError::EscapesRoot)
        ));
        assert!(matches!(
            resolve_within_root(root.path(), Path::new("a/../../outside")),
            Err(SafePathError::EscapesRoot)
        ));
    }

    #[test]
    fn allows_internal_parent_normalization() {
        let root = TempDir::new("internal");
        std::fs::create_dir_all(root.path().join("a")).unwrap();
        std::fs::write(root.path().join("c.jsonl"), b"x").unwrap();
        let got = resolve_within_root(root.path(), Path::new("a/../c.jsonl")).unwrap();
        assert_eq!(got, canon(root.path()).join("c.jsonl"));
    }

    #[test]
    fn rejects_absolute_path_outside_root() {
        let root = TempDir::new("rootonly");
        let outside = TempDir::new("elsewhere");
        let absolute = outside.path().join("f.txt");
        assert!(matches!(
            resolve_within_root(root.path(), &absolute),
            Err(SafePathError::OutsideRoot)
        ));
    }

    #[test]
    fn missing_root_is_rejected() {
        let root = TempDir::new("gone");
        let missing = root.path().join("does-not-exist");
        assert!(matches!(
            resolve_within_root(&missing, Path::new("x")),
            Err(SafePathError::RootUnavailable)
        ));
    }

    #[test]
    fn nonexistent_leaf_inside_root_is_allowed() {
        let root = TempDir::new("newleaf");
        std::fs::create_dir_all(root.path().join("sub")).unwrap();
        let got = resolve_within_root(root.path(), Path::new("sub/new_file.jsonl")).unwrap();
        assert_eq!(got, canon(root.path()).join("sub/new_file.jsonl"));
    }

    #[cfg(unix)]
    #[test]
    fn symlink_escape_is_rejected() {
        let root = TempDir::new("symroot");
        let outside = TempDir::new("symout");
        std::fs::write(outside.path().join("secret.txt"), b"x").unwrap();
        std::os::unix::fs::symlink(outside.path(), root.path().join("link")).unwrap();
        assert!(matches!(
            resolve_within_root(root.path(), Path::new("link/secret.txt")),
            Err(SafePathError::EscapesRoot)
        ));
    }

    #[cfg(unix)]
    #[test]
    fn symlink_inside_root_is_allowed() {
        let root = TempDir::new("symok");
        std::fs::create_dir_all(root.path().join("real")).unwrap();
        std::fs::write(root.path().join("real/f.jsonl"), b"x").unwrap();
        std::os::unix::fs::symlink(root.path().join("real"), root.path().join("alias")).unwrap();
        let got = resolve_within_root(root.path(), Path::new("alias/f.jsonl")).unwrap();
        assert_eq!(got, canon(root.path()).join("real/f.jsonl"));
    }
}
