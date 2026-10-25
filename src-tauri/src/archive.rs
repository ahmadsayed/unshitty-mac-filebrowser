//! Safe archive extraction for the context menu. Every entry is planned and
//! checked before anything is written: paths must stay inside the destination
//! and no existing file is ever overwritten.

use serde::Serialize;
use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Component, Path, PathBuf};
use zip::write::SimpleFileOptions;
use zip::ZipWriter;

#[derive(Debug, Serialize, PartialEq)]
pub struct ExtractResult {
    pub path: String,
    pub files: usize,
    pub folders: usize,
    pub skipped: usize,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Format {
    Zip,
    Tar,
    TarGz,
    TarBz2,
    TarXz,
    TarZst,
    Gz,
    Bz2,
    Xz,
    Zst,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum CreateFormat {
    Zip,
    TarGz,
    TarXz,
}

fn create_format(value: &str) -> Result<CreateFormat, String> {
    match value {
        "zip" => Ok(CreateFormat::Zip),
        "tar.gz" => Ok(CreateFormat::TarGz),
        "tar.xz" => Ok(CreateFormat::TarXz),
        _ => Err("That archive format isn't supported yet.".into()),
    }
}

#[derive(Debug)]
struct Planned {
    rel: PathBuf,
    dir: bool,
    mode: Option<u32>,
}

struct Plan {
    entries: Vec<Planned>,
    skipped: usize,
}

fn format_for(name: &str) -> Option<Format> {
    let lower = name.to_lowercase();
    let format = if lower.ends_with(".tar.gz") || lower.ends_with(".tgz") {
        Format::TarGz
    } else if lower.ends_with(".tar.bz2") || lower.ends_with(".tbz2") || lower.ends_with(".tbz") {
        Format::TarBz2
    } else if lower.ends_with(".tar.xz") || lower.ends_with(".txz") {
        Format::TarXz
    } else if lower.ends_with(".tar.zst") || lower.ends_with(".tzst") {
        Format::TarZst
    } else if lower.ends_with(".tar") {
        Format::Tar
    } else if lower.ends_with(".zip") || lower.ends_with(".jar") {
        Format::Zip
    } else if lower.ends_with(".gz") {
        Format::Gz
    } else if lower.ends_with(".bz2") {
        Format::Bz2
    } else if lower.ends_with(".xz") {
        Format::Xz
    } else if lower.ends_with(".zst") {
        Format::Zst
    } else {
        return None;
    };
    Some(format)
}

fn single_name(name: &str, suffix: &str) -> Result<PathBuf, String> {
    let stem = &name[..name.len() - suffix.len()];
    if stem.is_empty() {
        return Err("The archive has no output name.".into());
    }
    Ok(PathBuf::from(stem))
}

/// A relative path with no `..`, root or prefix components.
fn safe_relative(path: &Path) -> Option<PathBuf> {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(part) => out.push(part),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => return None,
        }
    }
    (!out.as_os_str().is_empty()).then_some(out)
}

fn unsafe_archive() -> String {
    "The archive contains an unsafe path.".into()
}

fn plan_zip(src: &Path) -> Result<Plan, String> {
    let mut archive = zip::ZipArchive::new(File::open(src).map_err(|e| e.to_string())?)
        .map_err(|e| format!("Could not read the archive: {e}"))?;
    let mut plan = Plan { entries: Vec::new(), skipped: 0 };
    for i in 0..archive.len() {
        let file = archive
            .by_index(i)
            .map_err(|e| format!("Could not read the archive: {e}"))?;
        let Some(rel) = file.enclosed_name().and_then(|path| safe_relative(&path)) else {
            return Err(unsafe_archive());
        };
        let mode = file.unix_mode();
        if mode.is_some_and(|mode| mode & 0o170000 == 0o120000) {
            plan.skipped += 1;
            continue;
        }
        plan.entries.push(Planned { rel, dir: file.is_dir(), mode });
    }
    Ok(plan)
}

fn plan_tar<R: Read>(reader: R) -> Result<Plan, String> {
    let mut archive = tar::Archive::new(reader);
    let mut plan = Plan { entries: Vec::new(), skipped: 0 };
    for entry in archive
        .entries()
        .map_err(|e| format!("Could not read the archive: {e}"))?
    {
        let entry = entry.map_err(|e| format!("Could not read the archive: {e}"))?;
        let path = entry.path().map_err(|_| unsafe_archive())?.to_path_buf();
        if path.as_os_str().is_empty() {
            continue;
        }
        let Some(rel) = safe_relative(&path) else {
            return Err(unsafe_archive());
        };
        let kind = entry.header().entry_type();
        if !kind.is_dir() && !kind.is_file() {
            // Symlinks, hard links and device nodes could escape the folder.
            plan.skipped += 1;
            continue;
        }
        plan.entries.push(Planned {
            rel,
            dir: kind.is_dir(),
            mode: entry.header().mode().ok(),
        });
    }
    Ok(plan)
}

fn plan_single(name: &str, suffix: &str) -> Result<Plan, String> {
    Ok(Plan {
        entries: vec![Planned { rel: single_name(name, suffix)?, dir: false, mode: None }],
        skipped: 0,
    })
}

fn plan(src: &Path, format: Format) -> Result<Plan, String> {
    let name = src
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .ok_or_else(|| "The archive has no name.".to_string())?;
    if format == Format::Zip {
        return plan_zip(src);
    }
    if let Some(suffix) = single_suffix(format) {
        return plan_single(&name, suffix);
    }
    plan_tar(tar_reader(src, format)?)
}

fn single_suffix(format: Format) -> Option<&'static str> {
    match format {
        Format::Gz => Some(".gz"),
        Format::Bz2 => Some(".bz2"),
        Format::Xz => Some(".xz"),
        Format::Zst => Some(".zst"),
        _ => None,
    }
}

fn is_tar(format: Format) -> bool {
    matches!(
        format,
        Format::Tar | Format::TarGz | Format::TarBz2 | Format::TarXz | Format::TarZst
    )
}

fn tar_reader(src: &Path, format: Format) -> Result<Box<dyn Read>, String> {
    let file = File::open(src).map_err(|e| e.to_string())?;
    let reader: Box<dyn Read> = match format {
        Format::Tar => Box::new(file),
        Format::TarGz => Box::new(flate2::read::GzDecoder::new(file)),
        Format::TarBz2 => Box::new(bzip2::read::BzDecoder::new(file)),
        Format::TarXz => Box::new(xz2::read::XzDecoder::new(file)),
        Format::TarZst => Box::new(zstd::stream::read::Decoder::new(file).map_err(|e| e.to_string())?),
        _ => return Err("Not a tar archive.".into()),
    };
    Ok(reader)
}

fn single_reader(src: &Path, format: Format) -> Result<Box<dyn Read>, String> {
    let file = File::open(src).map_err(|e| e.to_string())?;
    let reader: Box<dyn Read> = match format {
        Format::Gz => Box::new(flate2::read::GzDecoder::new(file)),
        Format::Bz2 => Box::new(bzip2::read::BzDecoder::new(file)),
        Format::Xz => Box::new(xz2::read::XzDecoder::new(file)),
        Format::Zst => Box::new(zstd::stream::read::Decoder::new(file).map_err(|e| e.to_string())?),
        _ => return Err("Not a single-file archive.".into()),
    };
    Ok(reader)
}

/// Check every planned entry before writing anything.
fn check_destination(dest: &Path, plan: &Plan) -> Result<(), String> {
    let kinds: HashMap<&Path, bool> = plan
        .entries
        .iter()
        .map(|entry| (entry.rel.as_path(), entry.dir))
        .collect();
    if kinds.len() != plan.entries.len() {
        return Err("The archive contains the same path twice.".into());
    }

    for entry in &plan.entries {
        for ancestor in entry.rel.ancestors().skip(1) {
            if ancestor.as_os_str().is_empty() {
                break;
            }
            if kinds.get(ancestor) == Some(&false) {
                return Err("The archive contains a file and a folder with the same path.".into());
            }
        }
        let target = dest.join(&entry.rel);
        let Ok(meta) = fs::symlink_metadata(&target) else {
            continue;
        };
        if entry.dir {
            if !meta.is_dir() {
                return Err(format!("“{}” already exists.", entry.rel.display()));
            }
        } else {
            return Err(format!("“{}” already exists.", entry.rel.display()));
        }
    }
    Ok(())
}

fn write_file<R: Read>(
    reader: &mut R,
    target: &Path,
    mode: Option<u32>,
    result: &mut ExtractResult,
) -> Result<(), String> {
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("Could not create a folder: {e}"))?;
    }
    let mut out = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(target)
        .map_err(|e| format!("Could not create “{}”: {e}", target.display()))?;
    if let Err(e) = io::copy(reader, &mut out) {
        drop(out);
        let _ = fs::remove_file(target);
        return Err(format!("Could not write “{}”: {e}", target.display()));
    }
    drop(out);
    #[cfg(unix)]
    if let Some(mode) = mode {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(target, fs::Permissions::from_mode(mode & 0o777));
    }
    result.files += 1;
    Ok(())
}

fn extract_zip(src: &Path, dest: &Path, plan: &Plan, result: &mut ExtractResult) -> Result<(), String> {
    let files: HashMap<&Path, &Planned> = plan
        .entries
        .iter()
        .filter(|entry| !entry.dir)
        .map(|entry| (entry.rel.as_path(), entry))
        .collect();
    let mut archive = zip::ZipArchive::new(File::open(src).map_err(|e| e.to_string())?)
        .map_err(|e| format!("Could not read the archive: {e}"))?;
    for i in 0..archive.len() {
        let mut file = archive
            .by_index(i)
            .map_err(|e| format!("Could not read the archive: {e}"))?;
        let Some(rel) = file.enclosed_name().and_then(|path| safe_relative(&path)) else {
            continue;
        };
        if let Some(entry) = files.get(rel.as_path()) {
            write_file(&mut file, &dest.join(&entry.rel), entry.mode, result)?;
        }
    }
    Ok(())
}

fn extract_tar<R: Read>(
    reader: R,
    dest: &Path,
    plan: &Plan,
    result: &mut ExtractResult,
) -> Result<(), String> {
    let files: HashMap<&Path, &Planned> = plan
        .entries
        .iter()
        .filter(|entry| !entry.dir)
        .map(|entry| (entry.rel.as_path(), entry))
        .collect();
    let mut archive = tar::Archive::new(reader);
    for entry in archive
        .entries()
        .map_err(|e| format!("Could not read the archive: {e}"))?
    {
        let mut entry = entry.map_err(|e| format!("Could not read the archive: {e}"))?;
        let Ok(path) = entry.path() else {
            continue;
        };
        let Some(rel) = safe_relative(&path) else {
            continue;
        };
        if let Some(planned) = files.get(rel.as_path()) {
            write_file(&mut entry, &dest.join(&planned.rel), planned.mode, result)?;
        }
    }
    Ok(())
}

fn extract_single<R: Read>(reader: R, dest: &Path, plan: &Plan, result: &mut ExtractResult) -> Result<(), String> {
    let Some(entry) = plan.entries.first() else {
        return Ok(());
    };
    let mut reader = reader;
    write_file(&mut reader, &dest.join(&entry.rel), entry.mode, result)
}

fn create_folders(dest: &Path, plan: &Plan, result: &mut ExtractResult) -> Result<(), String> {
    for entry in plan.entries.iter().filter(|entry| entry.dir) {
        fs::create_dir_all(dest.join(&entry.rel))
            .map_err(|e| format!("Could not create “{}”: {e}", entry.rel.display()))?;
        result.folders += 1;
    }
    Ok(())
}

fn extract_entries(
    src: &Path,
    dest: &Path,
    format: Format,
    plan: &Plan,
    result: &mut ExtractResult,
) -> Result<(), String> {
    create_folders(dest, plan, result)?;
    if format == Format::Zip {
        return extract_zip(src, dest, plan, result);
    }
    if is_tar(format) {
        return extract_tar(tar_reader(src, format)?, dest, plan, result);
    }
    extract_single(single_reader(src, format)?, dest, plan, result)
}

#[cfg(unix)]
fn file_mode(meta: &fs::Metadata) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    meta.permissions().mode() & 0o777
}

#[cfg(not(unix))]
fn file_mode(_meta: &fs::Metadata) -> u32 {
    0o644
}

fn sorted_children(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut paths: Vec<PathBuf> = entries.flatten().map(|entry| entry.path()).collect();
    paths.sort();
    paths
}

fn add_zip_path(
    writer: &mut ZipWriter<File>,
    path: &Path,
    rel: &Path,
) -> Result<(), String> {
    let meta = fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    if meta.file_type().is_symlink() {
        return Ok(());
    }
    let name = rel.to_string_lossy().replace('\\', "/");
    if meta.is_dir() {
        writer
            .add_directory(
                format!("{name}/"),
                SimpleFileOptions::default().unix_permissions(0o755),
            )
            .map_err(|e| format!("Could not add “{name}”: {e}"))?;
        for child in sorted_children(path) {
            let Some(child_name) = child.file_name() else {
                continue;
            };
            if child_name == crate::versions::VERSIONS_DIR {
                continue;
            }
            add_zip_path(writer, &child, &rel.join(child_name))?;
        }
    } else if meta.is_file() {
        writer
            .start_file(
                name.clone(),
                SimpleFileOptions::default()
                    .unix_permissions(file_mode(&meta))
                    .large_file(meta.len() > u32::MAX as u64),
            )
            .map_err(|e| format!("Could not add “{name}”: {e}"))?;
        let mut file = File::open(path).map_err(|e| e.to_string())?;
        io::copy(&mut file, writer).map_err(|e| format!("Could not add “{name}”: {e}"))?;
    }
    Ok(())
}

fn add_tar_path<W: Write>(
    builder: &mut tar::Builder<W>,
    path: &Path,
    rel: &Path,
) -> Result<(), String> {
    let meta = fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    if meta.file_type().is_symlink() {
        return Ok(());
    }
    if meta.is_dir() {
        let mut header = tar::Header::new_gnu();
        header.set_entry_type(tar::EntryType::Directory);
        header.set_mode(0o755);
        header.set_size(0);
        header.set_cksum();
        builder
            .append_data(&mut header, rel, io::empty())
            .map_err(|e| format!("Could not add “{}”: {e}", rel.display()))?;
        for child in sorted_children(path) {
            let Some(child_name) = child.file_name() else {
                continue;
            };
            if child_name == crate::versions::VERSIONS_DIR {
                continue;
            }
            add_tar_path(builder, &child, &rel.join(child_name))?;
        }
    } else if meta.is_file() {
        let mut header = tar::Header::new_gnu();
        header.set_entry_type(tar::EntryType::Regular);
        header.set_mode(file_mode(&meta));
        header.set_size(meta.len());
        header.set_cksum();
        let file = File::open(path).map_err(|e| e.to_string())?;
        builder
            .append_data(&mut header, rel, file)
            .map_err(|e| format!("Could not add “{}”: {e}", rel.display()))?;
    }
    Ok(())
}

fn write_archive(
    temp: &Path,
    paths: &[String],
    format: CreateFormat,
) -> Result<(), String> {
    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(temp)
        .map_err(|e| format!("Could not create the archive: {e}"))?;
    match format {
        CreateFormat::Zip => {
            let mut writer = ZipWriter::new(file);
            for path in paths {
                let source = Path::new(path);
                let name = source
                    .file_name()
                    .ok_or_else(|| "An item has no name.".to_string())?;
                add_zip_path(&mut writer, source, Path::new(name))?;
            }
            writer.finish().map_err(|e| e.to_string())?;
        }
        CreateFormat::TarGz => {
            let encoder =
                flate2::write::GzEncoder::new(file, flate2::Compression::default());
            let mut builder = tar::Builder::new(encoder);
            add_tar_paths(&mut builder, paths)?;
            builder
                .into_inner()
                .map_err(|e| e.to_string())?
                .finish()
                .map_err(|e| e.to_string())?;
        }
        CreateFormat::TarXz => {
            let encoder = xz2::write::XzEncoder::new(file, 6);
            let mut builder = tar::Builder::new(encoder);
            add_tar_paths(&mut builder, paths)?;
            builder
                .into_inner()
                .map_err(|e| e.to_string())?
                .finish()
                .map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

fn add_tar_paths<W: Write>(builder: &mut tar::Builder<W>, paths: &[String]) -> Result<(), String> {
    for path in paths {
        let source = Path::new(path);
        let name = source
            .file_name()
            .ok_or_else(|| "An item has no name.".to_string())?;
        add_tar_path(builder, source, Path::new(name))?;
    }
    Ok(())
}

/// Create an archive of `paths` at `dest`. The archive is built beside the
/// destination and linked into place, so an existing file is never replaced.
pub fn compress_archive(paths: Vec<String>, dest: String, format: String) -> Result<(), String> {
    if paths.is_empty() {
        return Err("Choose at least one item.".into());
    }
    let format = create_format(&format)?;
    let dest = PathBuf::from(dest);
    let parent = dest
        .parent()
        .ok_or_else(|| "The archive has no destination folder.".to_string())?;
    if !parent.is_dir() {
        return Err("The destination folder no longer exists.".into());
    }
    let name = dest
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .ok_or_else(|| "The archive needs a name.".to_string())?;
    if fs::symlink_metadata(&dest).is_ok() {
        return Err(format!("A file named “{name}” already exists."));
    }

    let mut temp = None;
    for n in 0..1000 {
        let candidate = parent.join(format!(".{name}.{}.{n}.tmp", std::process::id()));
        if fs::symlink_metadata(&candidate).is_err() {
            temp = Some(candidate);
            break;
        }
    }
    let temp = temp.ok_or_else(|| "Could not find a temporary name.".to_string())?;

    if let Err(e) = write_archive(&temp, &paths, format) {
        let _ = fs::remove_file(&temp);
        return Err(e);
    }
    if let Err(e) = fs::hard_link(&temp, &dest) {
        let _ = fs::remove_file(&temp);
        if fs::symlink_metadata(&dest).is_ok() {
            return Err(format!("A file named “{name}” already exists."));
        }
        return Err(format!("Could not finish the archive: {e}"));
    }
    let _ = fs::remove_file(&temp);
    Ok(())
}

/// Extract `path` into `dest`. When `create` is true the destination may be
/// missing and is created first. Existing files are never replaced.
pub fn extract_archive(path: String, dest: String, create: bool) -> Result<ExtractResult, String> {
    let src = Path::new(&path);
    let dest = Path::new(&dest);
    if !src.is_file() {
        return Err("The archive no longer exists.".into());
    }
    let name = src
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .ok_or_else(|| "The archive has no name.".to_string())?;
    let format = format_for(&name)
        .ok_or_else(|| "This archive format isn't supported yet.".to_string())?;
    let plan = plan(src, format)?;

    if !dest.is_dir() && !create {
        return Err("The destination folder no longer exists.".into());
    }
    check_destination(dest, &plan)?;
    if create {
        fs::create_dir_all(dest).map_err(|e| format!("Could not create the folder: {e}"))?;
    }

    let mut result = ExtractResult {
        path: dest.to_string_lossy().to_string(),
        files: 0,
        folders: 0,
        skipped: plan.skipped,
    };
    extract_entries(src, dest, format, &plan, &mut result)?;
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::tempdir;
    use zip::write::SimpleFileOptions;

    #[test]
    fn zip_extracts_files_and_folders() {
        let dir = tempdir().unwrap();
        let archive = dir.path().join("a.zip");
        {
            let mut writer = zip::ZipWriter::new(File::create(&archive).unwrap());
            writer
                .start_file("one.txt", SimpleFileOptions::default())
                .unwrap();
            writer.write_all(b"one").unwrap();
            writer
                .add_directory("folder", SimpleFileOptions::default())
                .unwrap();
            writer
                .start_file("folder/two.txt", SimpleFileOptions::default())
                .unwrap();
            writer.write_all(b"two").unwrap();
            writer.finish().unwrap();
        }
        let out = dir.path().join("out");
        let result = extract_archive(
            archive.to_string_lossy().to_string(),
            out.to_string_lossy().to_string(),
            true,
        )
        .unwrap();
        assert_eq!(result.files, 2);
        assert_eq!(fs::read_to_string(out.join("one.txt")).unwrap(), "one");
        assert_eq!(fs::read_to_string(out.join("folder/two.txt")).unwrap(), "two");
    }

    #[test]
    fn traversal_names_are_refused_before_writing() {
        let dir = tempdir().unwrap();
        let archive = dir.path().join("evil.zip");
        {
            let mut writer = zip::ZipWriter::new(File::create(&archive).unwrap());
            writer
                .start_file("../escaped.txt", SimpleFileOptions::default())
                .unwrap();
            writer.write_all(b"no").unwrap();
            writer.finish().unwrap();
        }
        let out = dir.path().join("out");
        assert!(extract_archive(
            archive.to_string_lossy().to_string(),
            out.to_string_lossy().to_string(),
            true,
        )
        .is_err());
        assert!(!dir.path().join("escaped.txt").exists());
    }

    #[test]
    fn existing_files_are_never_overwritten() {
        let dir = tempdir().unwrap();
        let archive = dir.path().join("a.zip");
        {
            let mut writer = zip::ZipWriter::new(File::create(&archive).unwrap());
            writer
                .start_file("keep.txt", SimpleFileOptions::default())
                .unwrap();
            writer.write_all(b"new").unwrap();
            writer.finish().unwrap();
        }
        let out = dir.path().join("out");
        fs::create_dir(&out).unwrap();
        fs::write(out.join("keep.txt"), "original").unwrap();
        assert!(extract_archive(
            archive.to_string_lossy().to_string(),
            out.to_string_lossy().to_string(),
            false,
        )
        .is_err());
        assert_eq!(fs::read_to_string(out.join("keep.txt")).unwrap(), "original");
    }

    #[test]
    fn tar_gz_is_supported() {
        let dir = tempdir().unwrap();
        let archive = dir.path().join("a.tgz");
        {
            let encoder = flate2::write::GzEncoder::new(
                File::create(&archive).unwrap(),
                flate2::Compression::default(),
            );
            let mut builder = tar::Builder::new(encoder);
            let mut header = tar::Header::new_gnu();
            header.set_size(3);
            header.set_mode(0o640);
            header.set_cksum();
            builder
                .append_data(&mut header, "inside.txt", &b"tar"[..])
                .unwrap();
            builder.into_inner().unwrap().finish().unwrap();
        }
        let out = dir.path().join("out");
        let result = extract_archive(
            archive.to_string_lossy().to_string(),
            out.to_string_lossy().to_string(),
            true,
        )
        .unwrap();
        assert_eq!(result.files, 1);
        assert_eq!(fs::read_to_string(out.join("inside.txt")).unwrap(), "tar");
    }

    fn tar_into<W: Write>(writer: W) -> W {
        let mut builder = tar::Builder::new(writer);
        let mut header = tar::Header::new_gnu();
        header.set_size(3);
        header.set_mode(0o644);
        header.set_cksum();
        builder
            .append_data(&mut header, "inside.txt", &b"tar"[..])
            .unwrap();
        builder.into_inner().unwrap()
    }

    fn extract_one(dir: &Path, name: &str) -> String {
        let out = dir.join(format!("out-{}", name.replace('.', "_")));
        let result = extract_archive(
            dir.join(name).to_string_lossy().to_string(),
            out.to_string_lossy().to_string(),
            true,
        )
        .unwrap();
        assert_eq!(result.files, 1);
        fs::read_to_string(out.join("inside.txt")).unwrap()
    }

    #[test]
    fn plain_tar_is_supported() {
        let dir = tempdir().unwrap();
        tar_into(File::create(dir.path().join("a.tar")).unwrap());
        assert_eq!(extract_one(dir.path(), "a.tar"), "tar");
    }

    #[test]
    fn tar_bz2_and_xz_are_supported() {
        let dir = tempdir().unwrap();
        let bz = bzip2::write::BzEncoder::new(
            File::create(dir.path().join("a.tar.bz2")).unwrap(),
            bzip2::Compression::default(),
        );
        tar_into(bz).finish().unwrap();
        assert_eq!(extract_one(dir.path(), "a.tar.bz2"), "tar");

        let xz = xz2::write::XzEncoder::new(File::create(dir.path().join("a.tar.xz")).unwrap(), 6);
        tar_into(xz).finish().unwrap();
        assert_eq!(extract_one(dir.path(), "a.tar.xz"), "tar");
    }

    #[test]
    fn tar_zst_is_supported() {
        let dir = tempdir().unwrap();
        let zst = zstd::Encoder::new(File::create(dir.path().join("a.tar.zst")).unwrap(), 0).unwrap();
        tar_into(zst).finish().unwrap();
        assert_eq!(extract_one(dir.path(), "a.tar.zst"), "tar");
    }

    fn compressed(bytes: &[u8]) -> Vec<Vec<u8>> {
        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        gz.write_all(bytes).unwrap();
        let mut bz = bzip2::write::BzEncoder::new(Vec::new(), bzip2::Compression::default());
        bz.write_all(bytes).unwrap();
        let mut xz = xz2::write::XzEncoder::new(Vec::new(), 6);
        xz.write_all(bytes).unwrap();
        let mut zst = zstd::Encoder::new(Vec::new(), 0).unwrap();
        zst.write_all(bytes).unwrap();
        vec![
            gz.finish().unwrap(),
            bz.finish().unwrap(),
            xz.finish().unwrap(),
            zst.finish().unwrap(),
        ]
    }

    #[test]
    fn single_file_compression_formats_are_supported() {
        let dir = tempdir().unwrap();
        for (suffix, bytes) in [".gz", ".bz2", ".xz", ".zst"]
            .into_iter()
            .zip(compressed(b"hello"))
        {
            let archive = dir.path().join(format!("inside.txt{suffix}"));
            fs::write(&archive, bytes).unwrap();
            let out = dir.path().join(format!("out{suffix}"));
            let result = extract_archive(
                archive.to_string_lossy().to_string(),
                out.to_string_lossy().to_string(),
                true,
            )
            .unwrap();
            assert_eq!(result.files, 1);
            assert_eq!(fs::read_to_string(out.join("inside.txt")).unwrap(), "hello");
        }
    }

    #[test]
    fn compression_round_trips_for_each_format_and_never_overwrites() {
        let dir = tempdir().unwrap();
        let source = dir.path().join("source");
        fs::create_dir_all(source.join("folder")).unwrap();
        fs::write(source.join("a.txt"), "one").unwrap();
        fs::write(source.join("folder/b.txt"), "two").unwrap();
        let paths = vec![
            source.join("a.txt").to_string_lossy().to_string(),
            source.join("folder").to_string_lossy().to_string(),
        ];

        for (suffix, format) in [(".zip", "zip"), (".tar.gz", "tar.gz"), (".tar.xz", "tar.xz")] {
            let archive = dir.path().join(format!("archive{suffix}"));
            compress_archive(
                paths.clone(),
                archive.to_string_lossy().to_string(),
                format.into(),
            )
            .unwrap();
            let out = dir.path().join(format!("out{suffix}"));
            extract_archive(
                archive.to_string_lossy().to_string(),
                out.to_string_lossy().to_string(),
                true,
            )
            .unwrap();
            assert_eq!(fs::read_to_string(out.join("a.txt")).unwrap(), "one");
            assert_eq!(fs::read_to_string(out.join("folder/b.txt")).unwrap(), "two");

            assert!(compress_archive(
                paths.clone(),
                archive.to_string_lossy().to_string(),
                format.into(),
            )
            .is_err());
            assert_eq!(fs::read_to_string(out.join("a.txt")).unwrap(), "one");
        }

        assert!(compress_archive(paths, dir.path().join("x.7z").to_string_lossy().to_string(), "7z".into()).is_err());
    }

    #[test]
    fn unsupported_archives_are_refused() {        let dir = tempdir().unwrap();
        let archive = dir.path().join("a.7z");
        fs::write(&archive, b"not really").unwrap();
        assert!(extract_archive(
            archive.to_string_lossy().to_string(),
            dir.path().to_string_lossy().to_string(),
            false,
        )
        .is_err());
    }
}
