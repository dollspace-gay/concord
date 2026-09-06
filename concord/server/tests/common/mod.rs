use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;

use sha2::{Digest, Sha256};

pub struct VerifiedBinary {
    path: PathBuf,
    sha256: [u8; 32],
}

fn sha256_file(path: &Path) -> [u8; 32] {
    let mut file = fs::File::open(path).unwrap();
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let count = file.read(&mut buffer).unwrap();
        if count == 0 {
            break;
        }
        digest.update(&buffer[..count]);
    }
    digest.finalize().into()
}

impl VerifiedBinary {
    pub fn copy_from(source: &Path, destination: PathBuf) -> Self {
        fs::create_dir_all(
            destination
                .parent()
                .expect("binary destination has a parent"),
        )
        .unwrap();
        fs::copy(source, &destination).unwrap();
        let source_hash = sha256_file(source);
        let destination_hash = sha256_file(&destination);
        assert_eq!(
            source_hash, destination_hash,
            "copied test binary hash drift"
        );
        let hash_hex = hex::encode(destination_hash);
        fs::write(
            destination.with_extension("sha256"),
            format!(
                "{hash_hex}  {}\n",
                destination.file_name().unwrap().to_string_lossy()
            ),
        )
        .unwrap();
        eprintln!(
            "immutable_test_binary path={} sha256={hash_hex}",
            destination.display()
        );
        Self {
            path: destination,
            sha256: destination_hash,
        }
    }

    pub fn command(&self) -> Command {
        let current = sha256_file(&self.path);
        assert_eq!(current, self.sha256, "immutable test binary changed");
        Command::new(&self.path)
    }
}
