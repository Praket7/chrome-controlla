use sha2::{Digest, Sha256};
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
            return Err(
                "artifact file must be a regular file between 1 and 10485760 bytes".into(),
            );
        }
        read_bounded(file)
    }
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
    use super::{decode_base64, read_bounded, read_local_file};
    use std::io::Cursor;

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
