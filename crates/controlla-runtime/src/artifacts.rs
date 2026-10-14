use sha2::{Digest, Sha256};
#[cfg(any(unix, test))]
use std::io::Read;

const MAX_LOCAL_ARTIFACT_BYTES: u64 = 10 * 1024 * 1024;

pub fn decode_base64(input: &str) -> Result<Vec<u8>, String> {
    use base64::Engine;
    if input.is_empty() || input.len() > 14 * 1024 * 1024 {
        return Err("base64 artifact must be between 1 and 10 MiB when decoded".into());
    }
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(input)
        .map_err(|_| "bytes_base64 must use standard padded Base64".to_owned())?;
    if bytes.is_empty() || bytes.len() > MAX_LOCAL_ARTIFACT_BYTES as usize {
        return Err("artifact bytes must be between 1 and 10485760 bytes".into());
    }
    Ok(bytes)
}

#[cfg(any(unix, test))]
fn read_bounded<R: Read>(reader: R) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    reader
        .take(MAX_LOCAL_ARTIFACT_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "artifact file_path could not be read".to_owned())?;
    if bytes.is_empty() || bytes.len() as u64 > MAX_LOCAL_ARTIFACT_BYTES {
        return Err("artifact file must be a regular file between 1 and 10485760 bytes".into());
    }
    Ok(bytes)
}

pub fn read_local_file(path: &str) -> Result<Vec<u8>, String> {
    let path = std::path::Path::new(path);
    if !path.is_absolute() {
        return Err("artifact file_path must be absolute".into());
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        Err(
            "local artifact paths are unavailable on this platform; use bounded Base64 input"
                .into(),
        )
    }
    #[cfg(unix)]
    {
        let file = open_local_file_nofollow(path)?;
        let metadata = file
            .metadata()
            .map_err(|_| "artifact file_path could not be opened".to_owned())?;
        if !metadata.is_file() || metadata.len() == 0 || metadata.len() > MAX_LOCAL_ARTIFACT_BYTES {
            return Err("artifact file must be a regular file between 1 and 10485760 bytes".into());
        }
        read_bounded(file)
    }
}

/// Atomically persists a bounded downloaded artifact without replacing an existing path.
///
/// The returned expectation attests only to the bytes written locally. It does not prove
/// where the bytes came from or that a web application accepted them.
#[cfg(unix)]
pub fn persist_local_file(path: &str, bytes: &[u8]) -> Result<ArtifactExpectation, String> {
    use std::{
        ffi::CString,
        io::{Read, Write},
        os::fd::{AsRawFd, FromRawFd},
    };

    if bytes.is_empty() || bytes.len() as u64 > MAX_LOCAL_ARTIFACT_BYTES {
        return Err("artifact bytes must be between 1 and 10485760 bytes".into());
    }
    let (directory, filename) = open_local_destination_parent(path)?;

    // A random sibling name plus O_EXCL prevents collisions and keeps the final link
    // operation on the same filesystem. linkat is an atomic no-replace commit.
    let mut random = [0u8; 16];
    getrandom::fill(&mut random)
        .map_err(|_| "secure artifact staging name could not be generated".to_owned())?;
    let temp_name = format!(
        ".controlla-download-{}-{}",
        std::process::id(),
        random
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    );
    let temp_c = CString::new(temp_name).expect("generated name has no NUL");
    let temp_fd = unsafe {
        libc::openat(
            directory.as_raw_fd(),
            temp_c.as_ptr(),
            libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | libc::O_CLOEXEC | libc::O_NOFOLLOW,
            0o600,
        )
    };
    if temp_fd < 0 {
        return Err("artifact staging file could not be created".into());
    }
    let mut temp_file = unsafe { std::fs::File::from_raw_fd(temp_fd) };
    if temp_file.write_all(bytes).is_err() || temp_file.sync_all().is_err() {
        drop(temp_file);
        unsafe { libc::unlinkat(directory.as_raw_fd(), temp_c.as_ptr(), 0) };
        return Err("artifact bytes could not be written".into());
    }
    drop(temp_file);

    // Verify the staged inode before publishing its final name. The subsequent hard
    // link is the commit point; no later verification failure can leave a reported
    // failure alongside a committed destination.
    let staged_read_fd = unsafe {
        libc::openat(
            directory.as_raw_fd(),
            temp_c.as_ptr(),
            libc::O_RDONLY | libc::O_CLOEXEC | libc::O_NOFOLLOW,
        )
    };
    if staged_read_fd < 0 {
        unsafe { libc::unlinkat(directory.as_raw_fd(), temp_c.as_ptr(), 0) };
        return Err("artifact staging file could not be verified".into());
    }
    let staged_file = unsafe { std::fs::File::from_raw_fd(staged_read_fd) };
    let mut staged_file = staged_file.take(bytes.len() as u64 + 1);
    let mut readback = Vec::with_capacity(bytes.len());
    if staged_file.read_to_end(&mut readback).is_err()
        || !ArtifactExpectation::from_bytes(bytes).matches(&readback)
    {
        drop(staged_file);
        unsafe { libc::unlinkat(directory.as_raw_fd(), temp_c.as_ptr(), 0) };
        return Err("artifact staging readback did not match downloaded bytes".into());
    }
    drop(staged_file);
    let expectation = ArtifactExpectation::from_bytes(&readback);

    let linked = unsafe {
        libc::linkat(
            directory.as_raw_fd(),
            temp_c.as_ptr(),
            directory.as_raw_fd(),
            filename.as_ptr(),
            0,
        )
    };
    if linked != 0 {
        unsafe { libc::unlinkat(directory.as_raw_fd(), temp_c.as_ptr(), 0) };
        return Err("artifact destination already exists or could not be created".into());
    }
    // linkat published the same verified inode atomically. Directory sync is best effort:
    // after this commit point, a durability-sync error cannot be returned as a failed
    // operation without misleading callers into retrying an already-created destination.
    unsafe { libc::fsync(directory.as_raw_fd()) };
    // Failure to remove the private staging name must not misreport the committed file.
    unsafe { libc::unlinkat(directory.as_raw_fd(), temp_c.as_ptr(), 0) };
    Ok(expectation)
}

/// Checks that a download destination can be used without following symlinks.
/// This is a preflight only; it does not reserve the name against later filesystem changes.
#[cfg(unix)]
pub fn validate_local_destination(path: &str) -> Result<(), String> {
    use std::os::fd::AsRawFd;

    let (directory, filename) = open_local_destination_parent(path)?;
    let mut metadata = unsafe { std::mem::zeroed::<libc::stat>() };
    let result = unsafe {
        libc::fstatat(
            directory.as_raw_fd(),
            filename.as_ptr(),
            &mut metadata,
            libc::AT_SYMLINK_NOFOLLOW,
        )
    };
    if result == 0 {
        return Err("artifact destination already exists".into());
    }
    if std::io::Error::last_os_error().raw_os_error() == Some(libc::ENOENT) {
        Ok(())
    } else {
        Err("artifact destination could not be checked".into())
    }
}

#[cfg(not(unix))]
pub fn validate_local_destination(_path: &str) -> Result<(), String> {
    Err("local artifact destinations are unavailable on this platform".into())
}

#[cfg(unix)]
fn open_local_destination_parent(path: &str) -> Result<(std::fs::File, std::ffi::CString), String> {
    use std::{
        ffi::CString,
        os::{
            fd::{AsRawFd, FromRawFd},
            unix::ffi::OsStrExt,
        },
        path::Component,
    };

    let path = std::path::Path::new(path);
    if !path.is_absolute() {
        return Err("artifact destination must be absolute".into());
    }
    let mut components = Vec::new();
    for component in path.components() {
        match component {
            Component::RootDir | Component::CurDir => {}
            Component::Normal(name) => components.push(name.to_owned()),
            Component::ParentDir => {
                return Err("artifact destination must not contain parent traversal".into());
            }
            Component::Prefix(_) => return Err("artifact destination must be absolute".into()),
        }
    }
    let (filename, parents) = components
        .split_last()
        .ok_or_else(|| "artifact destination must name a file".to_owned())?;
    let filename = CString::new(filename.as_bytes())
        .map_err(|_| "artifact destination is invalid".to_owned())?;

    let root = CString::new("/").expect("constant has no NUL");
    let root_fd = unsafe {
        libc::open(
            root.as_ptr(),
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC | libc::O_NOFOLLOW,
        )
    };
    if root_fd < 0 {
        return Err("artifact destination parent could not be opened".into());
    }
    let mut directory = unsafe { std::fs::File::from_raw_fd(root_fd) };
    for parent in parents {
        let name = CString::new(parent.as_bytes())
            .map_err(|_| "artifact destination is invalid".to_owned())?;
        let fd = unsafe {
            libc::openat(
                directory.as_raw_fd(),
                name.as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC | libc::O_NOFOLLOW,
            )
        };
        if fd < 0 {
            return Err("artifact destination parent must exist and contain no symlinks".into());
        }
        directory = unsafe { std::fs::File::from_raw_fd(fd) };
    }
    Ok((directory, filename))
}

#[cfg(not(unix))]
pub fn persist_local_file(_path: &str, _bytes: &[u8]) -> Result<ArtifactExpectation, String> {
    Err("local artifact destinations are unavailable on this platform".into())
}

#[cfg(unix)]
fn open_local_file_nofollow(path: &std::path::Path) -> Result<std::fs::File, String> {
    use std::{
        ffi::CString,
        os::{fd::FromRawFd, unix::ffi::OsStrExt},
        path::Component,
    };

    let mut components = Vec::new();
    for component in path.components() {
        match component {
            Component::RootDir | Component::CurDir => {}
            Component::Normal(name) => components.push(name.to_owned()),
            Component::ParentDir => {
                return Err("artifact file_path must not contain parent traversal".into());
            }
            Component::Prefix(_) => return Err("artifact file_path must be absolute".into()),
        }
    }
    let (filename, parents) = components
        .split_last()
        .ok_or_else(|| "artifact file_path could not be opened".to_owned())?;
    let root = CString::new("/").expect("constant has no NUL");
    // Keep each opened directory alive while opening its child so path components cannot be swapped.
    let root_fd = unsafe {
        libc::open(
            root.as_ptr(),
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC | libc::O_NOFOLLOW,
        )
    };
    if root_fd < 0 {
        return Err("artifact file_path could not be opened".into());
    }
    let mut directory = unsafe { std::fs::File::from_raw_fd(root_fd) };
    for parent in parents {
        let name = CString::new(parent.as_bytes())
            .map_err(|_| "artifact file_path could not be opened".to_owned())?;
        let fd = unsafe {
            libc::openat(
                std::os::fd::AsRawFd::as_raw_fd(&directory),
                name.as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC | libc::O_NOFOLLOW,
            )
        };
        if fd < 0 {
            return Err("artifact file_path must not contain symlinks and must be readable".into());
        }
        directory = unsafe { std::fs::File::from_raw_fd(fd) };
    }
    let filename = CString::new(filename.as_bytes())
        .map_err(|_| "artifact file_path could not be opened".to_owned())?;
    let fd = unsafe {
        libc::openat(
            std::os::fd::AsRawFd::as_raw_fd(&directory),
            filename.as_ptr(),
            libc::O_RDONLY | libc::O_CLOEXEC | libc::O_NOFOLLOW | libc::O_NONBLOCK,
        )
    };
    if fd < 0 {
        return Err("artifact file_path must not be a symlink and must be readable".into());
    }
    Ok(unsafe { std::fs::File::from_raw_fd(fd) })
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ArtifactExpectation {
    pub byte_length: u64,
    pub sha256: String,
}

impl ArtifactExpectation {
    pub fn from_bytes(bytes: &[u8]) -> Self {
        Self {
            byte_length: bytes.len() as u64,
            sha256: Self::sha256(bytes),
        }
    }

    pub fn sha256(bytes: &[u8]) -> String {
        Sha256::digest(bytes)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect()
    }

    pub fn matches(&self, bytes: &[u8]) -> bool {
        bytes.len() as u64 == self.byte_length && Self::sha256(bytes) == self.sha256
    }
}

#[cfg(test)]
mod tests {
    use super::{
        decode_base64, persist_local_file, read_bounded, read_local_file,
        validate_local_destination,
    };
    use std::io::Cursor;

    fn temporary_directory(label: &str) -> std::path::PathBuf {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "controlla-{label}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        std::fs::create_dir(&path).unwrap();
        path.canonicalize().unwrap()
    }

    #[cfg(unix)]
    #[test]
    fn downloaded_artifact_is_created_atomically_with_digest_receipt() {
        let directory = temporary_directory("download");
        let destination = directory.join("report.bin");
        let expected = super::ArtifactExpectation::from_bytes(b"downloaded bytes");
        let receipt =
            persist_local_file(destination.to_str().unwrap(), b"downloaded bytes").unwrap();
        assert_eq!(receipt, expected);
        assert!(receipt.matches(&std::fs::read(&destination).unwrap()));
        assert_eq!(std::fs::read_dir(&directory).unwrap().count(), 1);
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn downloaded_artifact_rejects_relative_parent_and_symlink_paths() {
        assert!(persist_local_file("relative.bin", b"data").is_err());
        assert!(persist_local_file("/tmp/../tmp/traversal.bin", b"data").is_err());

        use std::os::unix::fs::symlink;
        let directory = temporary_directory("download-link");
        let real_parent = directory.join("real");
        let linked_parent = directory.join("linked");
        std::fs::create_dir(&real_parent).unwrap();
        symlink(&real_parent, &linked_parent).unwrap();
        assert!(
            persist_local_file(linked_parent.join("report.bin").to_str().unwrap(), b"data")
                .is_err()
        );
        assert!(!real_parent.join("report.bin").exists());
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn downloaded_artifact_never_overwrites_existing_destination() {
        let directory = temporary_directory("download-existing");
        let destination = directory.join("report.bin");
        std::fs::write(&destination, b"original").unwrap();
        assert!(persist_local_file(destination.to_str().unwrap(), b"replacement").is_err());
        assert_eq!(std::fs::read(&destination).unwrap(), b"original");
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn downloaded_artifact_rejects_symlink_destination_and_redacts_paths() {
        use std::os::unix::fs::symlink;
        let directory = temporary_directory("download-destination-link");
        let target = directory.join("target.bin");
        let link = directory.join("link.bin");
        std::fs::write(&target, b"keep").unwrap();
        symlink(&target, &link).unwrap();
        let error = persist_local_file(link.to_str().unwrap(), b"replace").unwrap_err();
        assert!(!error.contains(directory.to_str().unwrap()));
        assert_eq!(std::fs::read(&target).unwrap(), b"keep");
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn concurrent_downloads_cannot_replace_each_other_or_leave_partial_bytes() {
        let directory = temporary_directory("download-race");
        let destination = directory.join("result.bin");
        let path = destination.to_string_lossy().into_owned();
        let writers: Vec<_> = [
            b"first complete payload".to_vec(),
            b"second complete payload".to_vec(),
        ]
        .into_iter()
        .map(|payload| {
            let path = path.clone();
            std::thread::spawn(move || {
                let result = persist_local_file(&path, &payload);
                (payload, result)
            })
        })
        .collect();
        let outcomes: Vec<_> = writers
            .into_iter()
            .map(|writer| writer.join().unwrap())
            .collect();
        assert_eq!(
            outcomes.iter().filter(|(_, result)| result.is_ok()).count(),
            1
        );
        let bytes = std::fs::read(&destination).unwrap();
        assert!(outcomes.iter().any(|(payload, _)| payload == &bytes));
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn downloaded_artifact_enforces_nonempty_ten_mib_bound() {
        let directory = temporary_directory("download-size");
        let destination = directory.join("too-large.bin");
        let too_large = vec![0; 10 * 1024 * 1024 + 1];
        assert!(persist_local_file(destination.to_str().unwrap(), &too_large).is_err());
        assert!(!destination.exists());
        assert!(persist_local_file(directory.join("empty.bin").to_str().unwrap(), b"").is_err());
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn destination_preflight_accepts_only_new_regular_destination_paths() {
        let directory = temporary_directory("destination-preflight");
        let destination = directory.join("new.bin");
        assert!(validate_local_destination(destination.to_str().unwrap()).is_ok());
        std::fs::write(&destination, b"existing").unwrap();
        assert!(validate_local_destination(destination.to_str().unwrap()).is_err());
        assert!(validate_local_destination(directory.to_str().unwrap()).is_err());
        assert!(validate_local_destination("relative.bin").is_err());
        assert!(validate_local_destination("/tmp/../tmp/traversal.bin").is_err());
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn destination_preflight_rejects_symlinked_parent_and_final_target() {
        use std::os::unix::fs::symlink;
        let directory = temporary_directory("destination-preflight-link");
        let real_parent = directory.join("real");
        let linked_parent = directory.join("linked");
        std::fs::create_dir(&real_parent).unwrap();
        symlink(&real_parent, &linked_parent).unwrap();
        assert!(
            validate_local_destination(linked_parent.join("new.bin").to_str().unwrap()).is_err()
        );

        let target = directory.join("target.bin");
        let link = directory.join("link.bin");
        std::fs::write(&target, b"target").unwrap();
        symlink(&target, &link).unwrap();
        assert!(validate_local_destination(link.to_str().unwrap()).is_err());
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn local_artifact_file_requires_absolute_regular_bounded_file() {
        assert!(read_local_file("relative-file").is_err());
        assert!(read_local_file("/tmp/../tmp/file").is_err());
        let path = std::env::temp_dir()
            .canonicalize()
            .unwrap()
            .join(format!("controlla-artifact-{}", std::process::id()));
        std::fs::write(&path, b"image-bytes").unwrap();
        assert_eq!(
            read_local_file(path.to_str().unwrap()).unwrap(),
            b"image-bytes"
        );
        std::fs::remove_file(path).unwrap();
    }

    #[cfg(not(unix))]
    #[test]
    fn local_artifact_paths_are_disabled_without_race_free_nofollow_open() {
        assert!(
            read_local_file("C:\\artifact.bin")
                .unwrap_err()
                .contains("use bounded Base64 input")
        );
        assert!(
            super::validate_local_destination("C:\\artifact.bin")
                .unwrap_err()
                .contains("unavailable")
        );
    }

    #[cfg(unix)]
    #[test]
    fn local_artifact_file_rejects_symlinks_in_the_path() {
        use std::os::unix::fs::symlink;
        let base = std::env::temp_dir()
            .canonicalize()
            .unwrap()
            .join(format!("controlla-artifact-link-{}", std::process::id()));
        let target = base.with_extension("target");
        let link = base.with_extension("link");
        std::fs::write(&target, b"data").unwrap();
        symlink(&target, &link).unwrap();
        assert!(
            read_local_file(link.to_str().unwrap())
                .unwrap_err()
                .contains("symlink")
        );
        std::fs::remove_file(link).unwrap();
        std::fs::remove_file(target).unwrap();

        let parent = base.with_extension("parent");
        let real_parent = base.with_extension("real-parent");
        std::fs::create_dir(&real_parent).unwrap();
        std::fs::write(real_parent.join("artifact"), b"data").unwrap();
        symlink(&real_parent, &parent).unwrap();
        assert!(
            read_local_file(parent.join("artifact").to_str().unwrap())
                .unwrap_err()
                .contains("symlink")
        );
        std::fs::remove_file(parent.join("artifact")).ok();
        std::fs::remove_file(parent).unwrap();
        std::fs::remove_dir_all(real_parent).unwrap();
    }

    #[test]
    fn bounded_reader_stops_at_one_byte_over_the_artifact_limit() {
        let input = Cursor::new(vec![0u8; 10 * 1024 * 1024 + 64]);
        assert!(
            read_bounded(input)
                .unwrap_err()
                .contains("between 1 and 10485760 bytes")
        );
    }

    #[test]
    fn base64_artifact_is_compact_bounded_and_strict() {
        use base64::Engine;
        let bytes = [0, 1, 127, 128, 255];
        let encoded = base64::engine::general_purpose::STANDARD.encode(bytes);
        assert!(
            encoded.len()
                < bytes
                    .iter()
                    .map(|byte| byte.to_string().len() + 1)
                    .sum::<usize>()
        );
        assert_eq!(decode_base64(&encoded).unwrap(), bytes);
        assert!(decode_base64("%%%!").is_err());
        assert!(decode_base64("====").is_err());
    }
}
