//! A couple crates that depend on tx5-core need to be able to write/verify
//! files on system. Enable this `file_check` feature to provide that ability.

use crate::{Error, Result};

/// A handle to a verified system file. Keep this instance in memory as
/// long as you intend to keep using the validated file.
pub struct FileCheck {
    path: std::path::PathBuf,
    _file: Option<std::fs::File>,
}

impl FileCheck {
    /// Get the path of the validated FileCheck file.
    pub fn path(&self) -> &std::path::Path {
        &self.path
    }
}

fn get_user_cache_dir() -> Option<std::path::PathBuf> {
    match app_dirs2::app_root(
        app_dirs2::AppDataType::UserCache,
        &app_dirs2::AppInfo {
            name: "host.holo.tx5",
            author: "host.holo.tx5",
        },
    ) {
        Ok(dir) => Some(dir),
        _ => None,
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

    let pref_path = get_user_cache_dir().map(|mut d| {
        d.push(&file_name);
        d
    });

    if let Some(pref_path) = pref_path.as_ref() {
        if let Ok(file) = validate(pref_path, file_hash) {
            return Ok(FileCheck {
                path: pref_path.clone(),
                _file: Some(file),
            });
        }
    }

    let mut tmp = write(file_data)?;

    if let Some(pref_path) = pref_path.as_ref() {
        // NOTE: This is NOT atomic, nor secure, but being able to validate the
        //       file hash post-op mitigates this a bit. And we can let the os
        //       clean up a dangling tmp file if it failed to unlink.
        match tmp.persist_noclobber(pref_path) {
            Ok(mut file) => {
                set_perms(&mut file)?;

                drop(file);

                let file = validate(pref_path, file_hash)?;

                return Ok(FileCheck {
                    path: pref_path.clone(),
                    _file: Some(file),
                });
            }
            Err(err) => {
                let tempfile::PersistError { file, .. } = err;
                tmp = file;
            }
        }

        // before we go on to just using the tmp file,
        // check to see if a different process wrote correctly
        if let Ok(file) = validate(pref_path, file_hash) {
            // we no longer need the tmp file, clean it up
            let _ = tmp.close();

            return Ok(FileCheck {
                path: pref_path.clone(),
                _file: Some(file),
            });
        }
    }

    // we're just going to use the tmp file, do what we need to
    // do to make sure it isn't deleted when the handle drops.

    let path = tmp.path().to_owned();
    let tmp = tmp.into_temp_path();

    // This seems wrong, but it is how tempfile internally goes
    // about doing persist/keep, so we're using it already,
    // and it's only once-ish per process...
    std::mem::forget(tmp);

    let file = validate(&path, file_hash)?;

    Ok(FileCheck {
        path,
        _file: Some(file),
    })
}

/// Validate a file.
fn validate(path: &std::path::PathBuf, hash: &str) -> Result<std::fs::File> {
    use std::io::Read;

    let mut file = std::fs::OpenOptions::new().read(true).open(path)?;

    let mut data = Vec::new();
    file.read_to_end(&mut data).expect("failed to read lib");

    use sha2::Digest;
    let mut hasher = sha2::Sha256::new();
    hasher.update(data);

    use base64::Engine;
    let on_disk_hash = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .encode(hasher.finalize());

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

    set_perms(tmp.as_file_mut())?;

    Ok(tmp)
}

/// Set file permissions.
fn set_perms(file: &mut std::fs::File) -> Result<()> {
    let mut perms = file.metadata()?.permissions();

    perms.set_readonly(true);
    #[cfg(unix)]
    std::os::unix::fs::PermissionsExt::set_mode(&mut perms, 0o500);

    file.set_permissions(perms)
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

        use base64::Engine;
        let hash = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(hasher.finalize());

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
            drop(tmp);
            let _ = std::fs::remove_file(&path);
        }
    }
}
