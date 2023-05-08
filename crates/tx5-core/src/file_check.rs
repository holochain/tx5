//! A couple crates that depend on tx5-core need to be able to write/verify
//! files on system. Enable this `file_check` feature to provide that ability.

use crate::{Error, Result};

/// A handle to a verified system file. Keep this instance in memory as
/// long as you intend to keep using the validated file.
pub struct FileCheck {
    path: std::path::PathBuf,
    _file: Option<std::fs::File>,
    _tmp: Option<tempfile::NamedTempFile>,
}

impl FileCheck {
    /// Close / Cleanup this FileCheck instance.
    pub fn close(mut self) {
        if let Some(tmp) = self._tmp.take() {
            let _ = tmp.close();
        }
    }

    /// Get the path of the validated FileCheck file.
    pub fn path(&self) -> &std::path::Path {
        &self.path
    }
}

/// Write a file if needed, verify the file, and return a handle to that file.
pub fn file_check(
    file_data: &[u8],
    file_hash: &str,
    file_name_prefix: &str,
    file_name_ext: &str,
) -> Result<FileCheck> {
    let file_name = format!("{file_name_prefix}-{file_hash}{file_name_ext}");

    let mut pref_path =
        dirs::data_local_dir().expect("failed to get data_local_dir");
    pref_path.push(&file_name);

    if let Ok(file) = validate(&pref_path, file_hash) {
        return Ok(FileCheck {
            path: pref_path,
            _file: Some(file),
            _tmp: None,
        });
    }

    let tmp = write(file_data)?;

    let tmp = if std::fs::metadata(&pref_path).is_err() {
        // NOTE: This is NOT atomic, the file may suddenly exist between system
        //       calls. However, rust does not provide a rename that will not
        //       overwrite the target, and since we're overwriting it with
        //       theoretically valid data, let's just go with it for now.
        if std::fs::rename(tmp.path(), &pref_path).is_ok() {
            match validate(&pref_path, file_hash) {
                Ok(file) => {
                    return Ok(FileCheck {
                        path: pref_path,
                        _file: Some(file),
                        _tmp: None,
                    });
                }
                Err(_) => {
                    // in the worst fallback, write another tmp file and use it
                    write(file_data)?
                }
            }
        } else {
            tmp
        }
    } else {
        // We could check again here to see if the perf_path is correct.
        // Some other parallel process my have written it. Then delete
        // our temp file, which would be a slightly better custodian
        // of disk space.
        tmp
    };

    if let Ok(file) = validate(tmp.path(), file_hash) {
        return Ok(FileCheck {
            path: tmp.path().into(),
            _file: Some(file),
            _tmp: Some(tmp),
        });
    }

    Err(Error::id("CouldNotWriteValidFile"))
}

/// Validate a file.
fn validate(path: &std::path::Path, hash: &str) -> Result<std::fs::File> {
    use std::io::Read;

    let mut file = std::fs::OpenOptions::new().read(true).open(path)?;

    let mut data = Vec::new();
    file.read_to_end(&mut data).expect("failed to read lib");

    use sha2::Digest;
    let mut hasher = sha2::Sha256::new();
    hasher.update(data);
    let on_disk_hash =
        base64::encode_config(hasher.finalize(), base64::URL_SAFE_NO_PAD);

    if on_disk_hash != hash {
        return Err(Error::err(format!("FileCheckHashMiss({path:?})")));
    }

    let perms = file
        .metadata()
        .expect("failed to get lib metadata")
        .permissions();

    if !perms.readonly() {
        return Err(Error::err(format!("FileCheckNotReadonly({path:?})")));
    }

    tracing::trace!("success correct file_check: {path:?}");

    Ok(file)
}

/// Write a temp file.
fn write(file_data: &[u8]) -> Result<tempfile::NamedTempFile> {
    use std::io::Write;

    let mut tmp = tempfile::NamedTempFile::new()?;

    tmp.as_file_mut().write_all(file_data)?;
    tmp.as_file_mut().flush()?;

    let mut perms = tmp.as_file().metadata()?.permissions();

    perms.set_readonly(true);
    #[cfg(unix)]
    std::os::unix::fs::PermissionsExt::set_mode(&mut perms, 0o500);

    tmp.as_file_mut().set_permissions(perms)?;

    Ok(tmp)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[tokio::test(flavor = "multi_thread")]
    async fn file_check_stress() {
        use rand::Rng;
        let mut data = vec![0; 1024 * 1024 * 10]; // 10 MiB
        rand::thread_rng().fill(&mut data[..]);
        let data = Arc::new(data);

        use sha2::Digest;
        let mut hasher = sha2::Sha256::new();
        hasher.update(&data[..]);
        let hash =
            base64::encode_config(hasher.finalize(), base64::URL_SAFE_NO_PAD);

        let mut task_list = Vec::new();

        const COUNT: usize = 3;

        let barrier = Arc::new(std::sync::Barrier::new(COUNT));

        for _ in 0..3 {
            let data = data.clone();
            let hash = hash.clone();
            let barrier = barrier.clone();
            task_list.push(tokio::task::spawn_blocking(move || {
                barrier.wait();

                file_check(
                    data.as_slice(),
                    &hash,
                    "tx5-core-file-check-test",
                    ".data",
                )
            }));
        }

        // make sure they're not dropped until the test is over
        let mut tmp = Vec::new();
        for task in task_list {
            tmp.push(task.await.unwrap().unwrap());
        }

        // cleanup
        for tmp in tmp {
            let path = tmp.path().to_owned();
            tmp.close();
            let _ = std::fs::remove_file(&path);
        }
    }
}
