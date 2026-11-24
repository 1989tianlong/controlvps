use std::fs::File;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use chrono::DateTime;
use ssh2::{FileStat, RenameFlags, Session, Sftp};
use uuid::Uuid;

use crate::error::{AppError, AppResult};
use crate::models::{FileListing, FileTransfer, RemoteFileEntry, RemoteFileKind};

pub fn list(session: &Session, path: Option<&str>) -> AppResult<FileListing> {
    let sftp = session.sftp()?;
    let requested = path.filter(|value| !value.trim().is_empty()).unwrap_or(".");
    let path = sftp.realpath(Path::new(requested))?;
    let mut entries = sftp
        .readdir(&path)?
        .into_iter()
        .filter_map(|(entry_path, stat)| {
            let name = entry_path.file_name()?.to_string_lossy().into_owned();
            if name == "." || name == ".." {
                return None;
            }
            Some(remote_entry(entry_path, name, &stat))
        })
        .collect::<Vec<_>>();
    entries.sort_by(|left, right| {
        let left_directory = left.kind == RemoteFileKind::Directory;
        let right_directory = right.kind == RemoteFileKind::Directory;
        right_directory
            .cmp(&left_directory)
            .then_with(|| left.name.to_lowercase().cmp(&right.name.to_lowercase()))
            .then_with(|| left.name.cmp(&right.name))
    });
    let parent = path.parent().and_then(|parent| {
        if parent == path {
            None
        } else {
            Some(parent.to_string_lossy().into_owned())
        }
    });
    Ok(FileListing {
        path: path.to_string_lossy().into_owned(),
        parent,
        entries,
    })
}

pub fn create_directory(session: &Session, parent: &str, name: &str) -> AppResult<String> {
    let path = child_path(parent, name)?;
    session.sftp()?.mkdir(&path, 0o755)?;
    Ok(path.to_string_lossy().into_owned())
}

pub fn rename(session: &Session, path: &str, new_name: &str) -> AppResult<String> {
    let source = safe_remote_path(path)?;
    let parent = source
        .parent()
        .ok_or_else(|| AppError::Message("无法确定远程文件的父目录".into()))?;
    let destination = child_path(&parent.to_string_lossy(), new_name)?;
    session.sftp()?.rename(&source, &destination, None)?;
    Ok(destination.to_string_lossy().into_owned())
}

pub fn delete(session: &Session, path: &str, recursive: bool) -> AppResult<()> {
    let path = safe_remote_path(path)?;
    let sftp = session.sftp()?;
    let stat = sftp.lstat(&path)?;
    if stat.is_dir() {
        if !recursive {
            return Err(AppError::Message("删除目录需要明确确认递归删除".into()));
        }
        delete_directory(&sftp, &path)
    } else {
        sftp.unlink(&path).map_err(Into::into)
    }
}

pub fn upload(
    session: &Session,
    local_path: &str,
    remote_directory: &str,
    file_name: &str,
) -> AppResult<FileTransfer> {
    upload_with_progress(
        session,
        local_path,
        remote_directory,
        file_name,
        &AtomicBool::new(false),
        |_, _| {},
    )
}

pub fn upload_with_progress(
    session: &Session,
    local_path: &str,
    remote_directory: &str,
    file_name: &str,
    cancel: &AtomicBool,
    mut progress: impl FnMut(u64, u64),
) -> AppResult<FileTransfer> {
    let local = PathBuf::from(local_path);
    if !local.is_file() {
        return Err(AppError::Message("请选择要上传的本地文件".into()));
    }
    let remote = child_path(remote_directory, file_name)?;
    let temporary = child_path(
        remote_directory,
        &format!(".controlvps-{}.part", Uuid::new_v4()),
    )?;
    let total = local.metadata()?.len();
    let mut source = File::open(&local)?;
    let sftp = session.sftp()?;
    let result = (|| {
        let mut destination = sftp.create(&temporary)?;
        let bytes =
            copy_with_progress(&mut source, &mut destination, total, cancel, &mut progress)?;
        destination.flush()?;
        drop(destination);
        sftp.rename(
            &temporary,
            &remote,
            Some(RenameFlags::ATOMIC | RenameFlags::OVERWRITE | RenameFlags::NATIVE),
        )?;
        Ok(FileTransfer {
            bytes,
            local_path: local.to_string_lossy().into_owned(),
            remote_path: remote.to_string_lossy().into_owned(),
        })
    })();
    if result.is_err() {
        let _ = sftp.unlink(&temporary);
    }
    result
}

pub fn download(session: &Session, remote_path: &str, local_path: &str) -> AppResult<FileTransfer> {
    download_with_progress(
        session,
        remote_path,
        local_path,
        &AtomicBool::new(false),
        |_, _| {},
    )
}

pub fn download_with_progress(
    session: &Session,
    remote_path: &str,
    local_path: &str,
    cancel: &AtomicBool,
    mut progress: impl FnMut(u64, u64),
) -> AppResult<FileTransfer> {
    let remote = safe_remote_path(remote_path)?;
    let local = PathBuf::from(local_path);
    if local.file_name().is_none() {
        return Err(AppError::Message("请选择有效的本机下载文件路径".into()));
    }
    let temporary = local.with_file_name(format!(".controlvps-{}.part", Uuid::new_v4()));
    let sftp = session.sftp()?;
    let total = sftp.stat(&remote)?.size.unwrap_or(0);
    let mut source = sftp.open(&remote)?;
    let result = (|| {
        let mut destination = File::create(&temporary)?;
        let bytes =
            copy_with_progress(&mut source, &mut destination, total, cancel, &mut progress)?;
        destination.flush()?;
        drop(destination);
        std::fs::rename(&temporary, &local)?;
        Ok(FileTransfer {
            bytes,
            local_path: local.to_string_lossy().into_owned(),
            remote_path: remote.to_string_lossy().into_owned(),
        })
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result
}

fn copy_with_progress(
    source: &mut impl Read,
    destination: &mut impl Write,
    total: u64,
    cancel: &AtomicBool,
    progress: &mut impl FnMut(u64, u64),
) -> AppResult<u64> {
    let mut buffer = [0_u8; 64 * 1024];
    let mut transferred = 0_u64;
    progress(0, total);
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err(AppError::Message("传输已取消".into()));
        }
        let count = source.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        destination.write_all(&buffer[..count])?;
        transferred += count as u64;
        progress(transferred, total);
    }
    if total > 0 && transferred != total {
        return Err(AppError::Message(format!(
            "传输不完整：预期 {total} 字节，实际 {transferred} 字节"
        )));
    }
    Ok(transferred)
}

fn delete_directory(sftp: &Sftp, path: &Path) -> AppResult<()> {
    for (entry, stat) in sftp.readdir(path)? {
        let Some(name) = entry.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if name == "." || name == ".." {
            continue;
        }
        if stat.is_dir() {
            delete_directory(sftp, &entry)?;
        } else {
            sftp.unlink(&entry)?;
        }
    }
    sftp.rmdir(path).map_err(Into::into)
}

fn child_path(parent: &str, name: &str) -> AppResult<PathBuf> {
    let name = name.trim();
    if name.is_empty() || name == "." || name == ".." || name.contains('/') || name.contains('\0') {
        return Err(AppError::Message(
            "名称不能为空，也不能是 .、.. 或包含 /".into(),
        ));
    }
    let parent = PathBuf::from(parent.trim());
    if !parent.is_absolute() || parent.to_string_lossy().contains('\0') {
        return Err(AppError::Message("远程父目录必须是绝对路径".into()));
    }
    if parent.to_string_lossy().split('/').any(|part| part == "..") {
        return Err(AppError::Message("远程路径不能包含 ..".into()));
    }
    Ok(parent.join(name))
}

fn safe_remote_path(path: &str) -> AppResult<PathBuf> {
    let path = path.trim();
    let parsed = PathBuf::from(path);
    if path.is_empty() || path.contains('\0') || !parsed.is_absolute() || parsed.parent().is_none()
    {
        return Err(AppError::Message(
            "远程路径必须是根目录之外的绝对路径".into(),
        ));
    }
    if path.split('/').any(|part| part == "..") {
        return Err(AppError::Message("远程路径不能包含 ..".into()));
    }
    Ok(parsed)
}

fn remote_entry(path: PathBuf, name: String, stat: &FileStat) -> RemoteFileEntry {
    let file_type = stat.file_type();
    let kind = if file_type.is_dir() {
        RemoteFileKind::Directory
    } else if file_type.is_file() {
        RemoteFileKind::File
    } else if file_type.is_symlink() {
        RemoteFileKind::Symlink
    } else {
        RemoteFileKind::Other
    };
    let modified_at = stat.mtime.and_then(|timestamp| {
        DateTime::from_timestamp(timestamp as i64, 0).map(|value| value.to_rfc3339())
    });
    RemoteFileEntry {
        name,
        path: path.to_string_lossy().into_owned(),
        kind,
        size: stat.size.unwrap_or(0),
        modified_at,
        permissions: stat.perm.map(format_permissions),
    }
}

fn format_permissions(mode: u32) -> String {
    const BITS: [(u32, char); 9] = [
        (0o400, 'r'),
        (0o200, 'w'),
        (0o100, 'x'),
        (0o040, 'r'),
        (0o020, 'w'),
        (0o010, 'x'),
        (0o004, 'r'),
        (0o002, 'w'),
        (0o001, 'x'),
    ];
    BITS.map(|(bit, permission)| if mode & bit != 0 { permission } else { '-' })
        .into_iter()
        .collect()
}
