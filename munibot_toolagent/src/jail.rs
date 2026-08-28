//! Resolves a tool-supplied, repository-relative path against the sandbox's
//! own repository root, refusing anything that would land outside it.
//!
//! **The single most security-critical function in this crate.** Every tool
//! that touches the filesystem (`read`, `write`, `edit`, `grep`, `glob`) or a
//! working directory (`bash`) must resolve through here before touching
//! anything on disk - a model-authored path is adversarial input, and the
//! whole point of the sandbox's own filesystem isolation would be
//! undermined by one tool that trusted a path without checking it.

use std::path::{Component, Path, PathBuf};

/// What went wrong resolving a path against the jail.
#[derive(thiserror::Error, Debug)]
pub enum JailError {
    /// The requested path was absolute. Every tool's own paths are
    /// repository-relative by contract; an absolute path is refused
    /// outright rather than reinterpreted as relative or joined in a way
    /// that could let it override the root entirely (`Path::join` replaces
    /// its base when given an absolute second argument).
    #[error(
        "{requested:?} is an absolute path -- every path must be relative to the repository root"
    )]
    AbsolutePath { requested: String },
    /// The path, once every existing symlink in it was resolved, lands
    /// outside the repository root -- via literal `..` traversal, a symlink
    /// inside the root pointing out of it, or a `..` component in a segment
    /// that does not exist yet.
    #[error("{requested:?} resolves outside the repository root :<")]
    EscapesJail { requested: String },
    /// The repository root itself does not exist or could not be
    /// canonicalised - a jail misconfiguration, not something a tool's own
    /// input caused.
    #[error("couldn't resolve the repository root {root:?}: {source}")]
    InvalidRoot {
        root: PathBuf,
        #[source]
        source: std::io::Error,
    },
    /// Every other I/O failure resolving an existing ancestor directory.
    #[error("i/o error resolving a path: {0}")]
    Io(#[from] std::io::Error),
}

/// Resolves `requested` - a path a tool asked for, relative to the
/// repository root - to a real, absolute path guaranteed to sit under
/// `root`, or refuses it.
///
/// `root` itself is canonicalised fresh on every call rather than once and
/// cached, so a change to the root's own symlink chain between calls is
/// never trusted from a stale resolution.
///
/// Because a target file may not exist yet (`write` creating a new file is
/// the motivating case), this cannot simply canonicalise the whole joined
/// path - canonicalisation itself requires every component to exist, and
/// `Path::parent()`/`file_name()` do not resolve `..` either, so naively
/// walking up a path that still contains one breaks as soon as it reaches
/// that component. Instead:
///
/// 1. `requested` is rejected outright if absolute - see
///    [`JailError::AbsolutePath`] - never joined in a way that could let it
///    override the root entirely (`Path::join` replaces its base when given an
///    absolute second argument).
/// 2. `requested`'s components are replayed against a stack seeded with
///    `root`'s own components, exactly like a shell's `cd`: a normal component
///    pushes, `..` pops - **unless the stack is already back down to the root's
///    own depth**, in which case it is refused immediately. This alone makes
///    every lexical escape attempt impossible by construction, regardless of
///    whether any component along the way exists on disk yet.
/// 3. The longest **existing** ancestor of that lexically-normalised path is
///    found and canonicalised, resolving every symlink actually on disk within
///    it, and checked against the canonical root - catching a symlink anywhere
///    in the existing portion of the path that points outside the jail.
/// 4. The final path is that canonical, symlink-resolved ancestor with whatever
///    comes after it (already `..`-free, and unable to be a symlink since it
///    does not exist yet) appended.
///
/// **Known limitation:** this checks the filesystem as it stood at the
/// moment of the call. It is not a defence against a symlink swapped in
/// between this resolution and whatever the caller does with the path next
/// (a TOCTOU race) - callers must resolve immediately before the operation
/// that uses the result, never cache a resolution and reuse it later.
pub fn resolve_in_jail(root: &Path, requested: &str) -> Result<PathBuf, JailError> {
    let canonical_root = std::fs::canonicalize(root).map_err(|source| JailError::InvalidRoot {
        root: root.to_path_buf(),
        source,
    })?;

    let requested_path = Path::new(requested);
    if requested_path.is_absolute() {
        return Err(JailError::AbsolutePath {
            requested: requested.to_string(),
        });
    }

    let normalized = lexically_normalize(&canonical_root, requested_path).ok_or_else(|| {
        JailError::EscapesJail {
            requested: requested.to_string(),
        }
    })?;

    let (existing_ancestor, remainder) = longest_existing_ancestor(&normalized);
    let canonical_ancestor = std::fs::canonicalize(&existing_ancestor)?;

    if !canonical_ancestor.starts_with(&canonical_root) {
        return Err(JailError::EscapesJail {
            requested: requested.to_string(),
        });
    }

    Ok(canonical_ancestor.join(remainder))
}

/// Replays `relative`'s components against a stack seeded with `base`'s own
/// components, the same way a shell's `cd` chain would: a normal component
/// pushes, `..` pops. Returns `None` the moment a `..` would pop past
/// `base`'s own depth - a lexical escape attempt, refused unconditionally
/// and without ever touching the filesystem.
///
/// This never needs to look at disk, and its result may still contain
/// components that do not exist yet - only [`longest_existing_ancestor`]
/// and the canonicalisation after it decide what is actually there.
fn lexically_normalize(base: &Path, relative: &Path) -> Option<PathBuf> {
    let base_depth = base.components().count();
    let mut stack: Vec<Component> = base.components().collect();

    for component in relative.components() {
        match component {
            Component::Normal(_) => stack.push(component),
            Component::CurDir => {}
            Component::ParentDir => {
                if stack.len() <= base_depth {
                    return None;
                }
                stack.pop();
            }
            // requested_path.is_absolute() was already checked by the
            // caller, so a RootDir/Prefix component here would mean
            // `relative` was not actually relative - treat it the same as
            // an escape attempt rather than silently accepting it
            Component::RootDir | Component::Prefix(_) => return None,
        }
    }

    Some(stack.into_iter().collect())
}

/// Walks up from `path` until it finds a component chain that actually
/// exists, returning that existing ancestor and everything after it as a
/// still-relative remainder.
///
/// `path` itself is returned as the ancestor (with an empty remainder) when
/// it already exists - the common case for `read`, `edit`, `grep`, and
/// `glob`, which only ever touch files already in the checked-out
/// repository. Safe to call with a path that has no `..` components at all
/// (guaranteed by [`lexically_normalize`] already having run) - `parent()`
/// and `file_name()` behave exactly as expected on a path built only from
/// `Normal` components.
fn longest_existing_ancestor(path: &Path) -> (PathBuf, PathBuf) {
    let mut ancestor = path.to_path_buf();
    let mut remainder_parts = Vec::new();

    while !ancestor.exists() {
        // lexically_normalize only ever produces Normal components beyond
        // the root, so file_name()/parent() are well-defined here all the
        // way up to the root itself, which this loop never needs to go
        // past - the root always exists, or resolve_in_jail's own
        // canonicalize(root) call would already have failed
        let file_name = ancestor
            .file_name()
            .expect("normalized path components are never RootDir/ParentDir/CurDir");
        remainder_parts.push(PathBuf::from(file_name));
        ancestor = ancestor
            .parent()
            .expect("the root itself always exists, so this loop stops before running out")
            .to_path_buf();
    }

    let remainder = remainder_parts.into_iter().rev().collect::<PathBuf>();
    (ancestor, remainder)
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::symlink;

    use super::*;

    /// Builds a fresh scratch directory to act as a repository root, with a
    /// `real` subdirectory and file already in it, cleaned up when the
    /// returned guard drops.
    struct Jail {
        root: PathBuf,
    }

    impl Jail {
        fn new(name: &str) -> Self {
            let root = std::env::temp_dir().join(format!(
                "munibot_toolagent_jail_test_{name}_{}",
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            std::fs::create_dir_all(root.join("src")).unwrap();
            std::fs::write(root.join("src/main.rs"), "fn main() {}").unwrap();
            Self { root }
        }
    }

    impl Drop for Jail {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.root).ok();
        }
    }

    #[test]
    fn test_resolves_an_existing_file_inside_the_root() {
        let jail = Jail::new("existing_file");
        let resolved = resolve_in_jail(&jail.root, "src/main.rs").expect("should resolve");
        assert_eq!(
            resolved,
            std::fs::canonicalize(&jail.root)
                .unwrap()
                .join("src/main.rs")
        );
    }

    #[test]
    fn test_resolves_a_new_file_that_does_not_exist_yet() {
        let jail = Jail::new("new_file");
        let resolved = resolve_in_jail(&jail.root, "src/new_module.rs").expect("should resolve");
        assert_eq!(
            resolved,
            std::fs::canonicalize(&jail.root)
                .unwrap()
                .join("src/new_module.rs")
        );
    }

    #[test]
    fn test_resolves_a_new_file_in_a_directory_that_does_not_exist_yet() {
        let jail = Jail::new("new_nested_dirs");
        let resolved =
            resolve_in_jail(&jail.root, "src/deeply/nested/new_file.rs").expect("should resolve");
        assert_eq!(
            resolved,
            std::fs::canonicalize(&jail.root)
                .unwrap()
                .join("src/deeply/nested/new_file.rs")
        );
    }

    #[test]
    fn test_resolves_the_root_itself_for_an_empty_relative_path() {
        let jail = Jail::new("root_itself");
        let resolved = resolve_in_jail(&jail.root, "").expect("should resolve");
        assert_eq!(resolved, std::fs::canonicalize(&jail.root).unwrap());
    }

    #[test]
    fn test_rejects_an_absolute_path() {
        let jail = Jail::new("absolute");
        let error = resolve_in_jail(&jail.root, "/etc/passwd").expect_err("should reject");
        assert!(
            matches!(error, JailError::AbsolutePath { .. }),
            "got {error:?}"
        );
    }

    #[test]
    fn test_rejects_literal_parent_dir_traversal_out_of_the_root() {
        let jail = Jail::new("dotdot_traversal");
        let error =
            resolve_in_jail(&jail.root, "../../../../etc/passwd").expect_err("should reject");
        assert!(
            matches!(error, JailError::EscapesJail { .. }),
            "got {error:?}"
        );
    }

    #[test]
    fn test_rejects_parent_dir_traversal_that_stays_lexically_prefixed_but_escapes() {
        // "src/../../.." never contains a *leading* ".." and, read as raw text,
        // still starts with "src" -- proving the jail resolves the real path
        // rather than doing a naive string-prefix check
        let jail = Jail::new("mixed_traversal");
        let error =
            resolve_in_jail(&jail.root, "src/../../../etc/passwd").expect_err("should reject");
        assert!(
            matches!(error, JailError::EscapesJail { .. }),
            "got {error:?}"
        );
    }

    #[test]
    fn test_rejects_parent_dir_traversal_in_a_segment_that_does_not_exist_yet() {
        // "brand_new" does not exist, so canonicalize can never resolve this
        // ".." away -- the lexical check on the unresolved remainder must
        // catch it instead
        let jail = Jail::new("traversal_in_new_segment");
        let error =
            resolve_in_jail(&jail.root, "brand_new/../../etc/passwd").expect_err("should reject");
        assert!(
            matches!(error, JailError::EscapesJail { .. }),
            "got {error:?}"
        );
    }

    #[test]
    fn test_allows_parent_dir_traversal_that_stays_inside_the_root() {
        let jail = Jail::new("internal_traversal");
        let resolved = resolve_in_jail(&jail.root, "src/../src/main.rs").expect("should resolve");
        assert_eq!(
            resolved,
            std::fs::canonicalize(&jail.root)
                .unwrap()
                .join("src/main.rs")
        );
    }

    #[test]
    fn test_rejects_a_symlink_inside_the_root_pointing_outside_it() {
        let jail = Jail::new("symlink_escape");
        let outside = std::env::temp_dir().join(format!(
            "munibot_toolagent_jail_outside_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::write(&outside, "secret").unwrap();
        symlink(&outside, jail.root.join("escape_link")).unwrap();

        let error = resolve_in_jail(&jail.root, "escape_link").expect_err("should reject");
        assert!(
            matches!(error, JailError::EscapesJail { .. }),
            "got {error:?}"
        );

        std::fs::remove_file(&outside).ok();
    }

    #[test]
    fn test_rejects_a_symlinked_directory_inside_the_root_pointing_outside_it() {
        let jail = Jail::new("symlink_dir_escape");
        let outside = std::env::temp_dir().join(format!(
            "munibot_toolagent_jail_outside_dir_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&outside).unwrap();
        symlink(&outside, jail.root.join("escape_dir")).unwrap();

        // the file itself does not need to exist for this to be caught: the
        // symlinked *directory* component is what escapes, and it is on the
        // existing-ancestor side of the walk regardless of the leaf
        let error =
            resolve_in_jail(&jail.root, "escape_dir/whatever.rs").expect_err("should reject");
        assert!(
            matches!(error, JailError::EscapesJail { .. }),
            "got {error:?}"
        );

        std::fs::remove_dir_all(&outside).ok();
    }

    #[test]
    fn test_allows_a_symlink_inside_the_root_pointing_at_another_spot_inside_it() {
        let jail = Jail::new("symlink_internal");
        symlink(jail.root.join("src"), jail.root.join("src_link")).unwrap();

        let resolved = resolve_in_jail(&jail.root, "src_link/main.rs").expect("should resolve");
        assert_eq!(
            resolved,
            std::fs::canonicalize(&jail.root)
                .unwrap()
                .join("src/main.rs")
        );
    }

    #[test]
    fn test_a_symlink_created_after_an_earlier_successful_resolution_is_still_caught() {
        // models the TOCTOU-adjacent case this function's own doc comment
        // calls out: resolving twice, with a symlink swapped in between,
        // must never trust the first resolution for the second call
        let jail = Jail::new("symlink_created_later");
        std::fs::create_dir_all(jail.root.join("later")).unwrap();
        std::fs::write(jail.root.join("later/file.rs"), "fn f() {}").unwrap();

        resolve_in_jail(&jail.root, "later/file.rs")
            .expect("should resolve while still a plain file");

        // now swap "later" for a symlink pointing outside the root, exactly
        // as if a sandboxed shell command had just done this
        let outside = std::env::temp_dir().join(format!(
            "munibot_toolagent_jail_outside_later_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::remove_dir_all(jail.root.join("later")).unwrap();
        symlink(&outside, jail.root.join("later")).unwrap();

        let error =
            resolve_in_jail(&jail.root, "later/file.rs").expect_err("should reject after the swap");
        assert!(
            matches!(error, JailError::EscapesJail { .. }),
            "got {error:?}"
        );

        std::fs::remove_dir_all(&outside).ok();
    }

    #[test]
    fn test_error_names_the_original_requested_path() {
        let jail = Jail::new("error_message");
        let error = resolve_in_jail(&jail.root, "../escape").expect_err("should reject");
        assert!(error.to_string().contains("../escape"));
    }

    #[test]
    fn test_invalid_root_is_a_distinct_error() {
        let missing_root =
            std::env::temp_dir().join("munibot_toolagent_jail_does_not_exist_at_all");
        let error = resolve_in_jail(&missing_root, "src/main.rs").expect_err("should fail");
        assert!(
            matches!(error, JailError::InvalidRoot { .. }),
            "got {error:?}"
        );
    }
}
