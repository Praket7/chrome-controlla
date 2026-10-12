use sha2::{Digest, Sha256};
use std::io::Read;

const MAX_LOCAL_ARTIFACT_BYTES: u64 = 10 * 1024 * 1024;

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
    let path = path
        .canonicalize()
        .map_err(|_| "artifact file_path could not be opened".to_owned())?;
    let metadata = path
        .metadata()
        .map_err(|_| "artifact file_path could not be opened".to_owned())?;
    if !metadata.is_file() || metadata.len() == 0 || metadata.len() > MAX_LOCAL_ARTIFACT_BYTES {
        return Err("artifact file must be a regular file between 1 and 10485760 bytes".into());
    }
    read_bounded(
        std::fs::File::open(path)
            .map_err(|_| "artifact file_path could not be opened".to_owned())?,
    )
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
    use super::{read_bounded, read_local_file};
    use std::io::Cursor;

    #[test]
    fn local_artifact_file_requires_absolute_regular_bounded_file() {
        assert!(read_local_file("relative-file").is_err());
        let path = std::env::temp_dir().join(format!("controlla-artifact-{}", std::process::id()));
        std::fs::write(&path, b"image-bytes").unwrap();
        assert_eq!(
            read_local_file(path.to_str().unwrap()).unwrap(),
            b"image-bytes"
        );
        std::fs::remove_file(path).unwrap();
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
}
