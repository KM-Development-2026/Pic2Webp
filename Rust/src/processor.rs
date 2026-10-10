//! archive_processor.py の Rust 移植。
use anyhow::{anyhow, bail, Context, Result};
use std::os::windows::process::CommandExt;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::{fs, io};
use walkdir::WalkDir;

const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const IMAGE_EXTS: &[&str] = &[
    "jpg", "jpeg", "jfif", "png", "webp", "bmp", "dib", "tiff", "tif", "gif", "tga", "ico",
];

fn ext_lower(p: &Path) -> String {
    p.extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default()
}

fn which(name: &str) -> Option<PathBuf> {
    let paths = std::env::var_os("PATH")?;
    std::env::split_paths(&paths)
        .map(|d| d.join(format!("{name}.exe")))
        .find(|c| c.is_file())
}

/// RAR展開用の外部ツール (7z / unrar / WinRAR) を探す
pub fn find_rar_tool() -> Option<PathBuf> {
    for tool in ["7z", "7zg", "unrar"] {
        if let Some(p) = which(tool) {
            return Some(p);
        }
    }
    [
        r"C:\Program Files\7-Zip\7z.exe",
        r"C:\Program Files (x86)\7-Zip\7z.exe",
        r"C:\Program Files\WinRAR\UnRAR.exe",
        r"C:\Program Files\WinRAR\WinRAR.exe",
    ]
    .iter()
    .map(PathBuf::from)
    .find(|p| p.exists())
}

fn clean_empty_directories(dir: &Path) {
    // 深い階層から順に、空なら削除（空でなければ remove_dir は失敗するだけ）
    for e in WalkDir::new(dir).contents_first(true).into_iter().flatten() {
        if e.file_type().is_dir() && e.path() != dir {
            let _ = fs::remove_dir(e.path());
        }
    }
}

fn extract_archive(archive: &Path, dest: &Path, rar_tool: Option<&Path>) -> Result<()> {
    match ext_lower(archive).as_str() {
        "zip" => {
            let mut z = zip::ZipArchive::new(fs::File::open(archive)?)?;
            z.extract(dest)?;
        }
        "7z" => {
            sevenz_rust::decompress_file(archive, dest).map_err(|e| anyhow!("7z展開エラー: {e}"))?;
        }
        "rar" => {
            let tool = rar_tool.ok_or_else(|| {
                anyhow!("RARファイルの展開に必要な外部ツール（7-Zip または WinRAR）が見つかりません。")
            })?;
            fs::create_dir_all(dest)?;
            let is_7z = tool
                .file_stem()
                .map(|n| n.to_string_lossy().to_lowercase().starts_with("7z"))
                .unwrap_or(false);
            let mut cmd = Command::new(tool);
            if is_7z {
                cmd.arg("x").arg("-y").arg(format!("-o{}", dest.display())).arg(archive);
            } else {
                // unrar / WinRAR: 展開先は末尾に区切り文字が必要
                cmd.arg("x").arg("-y").arg(archive).arg(format!("{}\\", dest.display()));
            }
            let out = cmd.creation_flags(CREATE_NO_WINDOW).output()?;
            if !out.status.success() {
                bail!("RAR展開に失敗しました: {}", String::from_utf8_lossy(&out.stderr));
            }
        }
        other => bail!("未対応の圧縮形式です: .{other}"),
    }
    Ok(())
}

/// 1枚の画像を `out` へ WebP として保存（元ファイルは削除しない）
fn convert_one(path: &Path, out: &Path, quality: f32) -> Result<()> {
    let img = image::ImageReader::open(path)?.with_guessed_format()?.decode()?;
    let (w, h) = (img.width(), img.height());
    let mem = if img.color().has_alpha() {
        let buf = img.into_rgba8();
        webp::Encoder::from_rgba(&buf, w, h).encode(quality)
    } else {
        let buf = img.into_rgb8();
        webp::Encoder::from_rgb(&buf, w, h).encode(quality)
    };
    fs::write(out, &*mem)?;
    Ok(())
}

/// 変換後の出力パスを決める。既存ファイルや他画像の出力と衝突する場合は
/// `元名.拡張子.webp`（さらに衝突すれば連番）にして上書きを避ける。
fn unique_webp_path(path: &Path, taken: &mut HashSet<PathBuf>) -> PathBuf {
    let mut cand = path.with_extension("webp");
    if taken.contains(&cand) || cand.exists() {
        let base = path.file_name().unwrap_or_default().to_string_lossy().to_string();
        cand = path.with_file_name(format!("{base}.webp"));
        let mut n = 2;
        while taken.contains(&cand) || cand.exists() {
            cand = path.with_file_name(format!("{base}.{n}.webp"));
            n += 1;
        }
    }
    taken.insert(cand.clone());
    cand
}

/// 画像はWebPに変換、画像以外は削除。変換に失敗した画像は元ファイルを残す。
fn convert_images_in_dir(
    dir: &Path,
    quality: f32,
    threads: usize,
    log: &(dyn Fn(String) + Sync),
    progress: &(dyn Fn(usize, usize) + Sync),
) -> usize {
    let files: Vec<PathBuf> = WalkDir::new(dir)
        .into_iter()
        .flatten()
        .filter(|e| !e.file_type().is_dir())
        .map(|e| e.into_path())
        .collect();

    // 画像以外は先に削除し、変換対象 (入力, 出力) を決定する（出力名の衝突はここで解決）
    let mut taken: HashSet<PathBuf> = HashSet::new();
    let mut jobs: Vec<(PathBuf, PathBuf)> = Vec::new();
    let mut kept = 0; // 変換不要（元から .webp）の枚数
    for path in files {
        let ext = ext_lower(&path);
        if !IMAGE_EXTS.contains(&ext.as_str()) {
            let _ = fs::remove_file(&path);
        } else if ext == "webp" {
            taken.insert(path);
            kept += 1;
        } else {
            jobs.push((path, PathBuf::new()));
        }
    }
    for job in jobs.iter_mut() {
        job.1 = unique_webp_path(&job.0, &mut taken);
    }

    let total = jobs.len();
    let next = AtomicUsize::new(0);
    let done = AtomicUsize::new(0);
    let converted = AtomicUsize::new(0);
    let workers = crate::config::resolve_threads(threads).min(total.max(1));

    std::thread::scope(|s| {
        for _ in 0..workers {
            s.spawn(|| loop {
                let i = next.fetch_add(1, Ordering::Relaxed);
                let Some((src, out)) = jobs.get(i) else { break };
                match convert_one(src, out, quality) {
                    Ok(()) => {
                        let _ = fs::remove_file(src);
                        converted.fetch_add(1, Ordering::Relaxed);
                    }
                    Err(e) => {
                        let _ = fs::remove_file(out);
                        log(format!(
                            "[警告] 変換に失敗したため元画像を残します: {} ({e})",
                            src.file_name().unwrap_or_default().to_string_lossy()
                        ));
                    }
                }
                progress(done.fetch_add(1, Ordering::Relaxed) + 1, total);
            });
        }
    });
    clean_empty_directories(dir);
    converted.into_inner() + kept
}

fn create_zip_archive(src: &Path, out_zip: &Path) -> Result<()> {
    if let Some(parent) = out_zip.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut zw = zip::ZipWriter::new(fs::File::create(out_zip)?);
    let opts = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Stored)
        .large_file(true);
    for e in WalkDir::new(src).into_iter().flatten().filter(|e| e.file_type().is_file()) {
        let rel = e.path().strip_prefix(src)?;
        let name = rel.to_string_lossy().replace('\\', "/");
        zw.start_file(name, opts)?;
        io::copy(&mut fs::File::open(e.path())?, &mut zw)?;
    }
    zw.finish()?;
    Ok(())
}

/// shutil.move 相当（ドライブ跨ぎは copy + remove）
fn move_file(from: &Path, to: &Path) -> Result<()> {
    if fs::rename(from, to).is_err() {
        fs::copy(from, to)?;
        fs::remove_file(from)?;
    }
    Ok(())
}

/// 単一アーカイブ処理のメインフロー。成功時は new_temp 内の zip パスを返す。
pub fn process_single_archive(
    archive: &Path,
    quality: f32,
    threads: usize,
    log: &(dyn Fn(String) + Sync),
    progress: &(dyn Fn(usize, usize) + Sync),
) -> Result<PathBuf> {
    let name = archive.file_name().unwrap().to_string_lossy().to_string();
    log(format!("【処理開始】 {name}"));

    let ext = ext_lower(archive);
    if !["zip", "rar", "7z"].contains(&ext.as_str()) {
        bail!("対応していない圧縮形式です: .{ext}");
    }

    let mut rar_tool = None;
    if ext == "rar" {
        log("RAR解凍ツールを探索中...".into());
        let t = find_rar_tool().ok_or_else(|| {
            anyhow!("RARファイルの展開に必要な外部ツール（7-Zip または WinRAR）が見つかりません。インストールしてください。")
        })?;
        log(format!(
            "解凍ツールが見つかりました: {}",
            t.file_name().unwrap().to_string_lossy()
        ));
        rar_tool = Some(t);
    }

    let parent = std::path::absolute(archive)?
        .parent()
        .context("親フォルダが取得できません")?
        .to_path_buf();
    let old_temp = parent.join("old_temp");
    let new_temp = parent.join("new_temp");

    let work = tempfile::tempdir()?;
    log("アーカイブを一時フォルダに解凍中...".into());
    extract_archive(archive, work.path(), rar_tool.as_deref())?;

    log("画像をWebPに変換し、不要ファイルを削除中...".into());
    let converted = convert_images_in_dir(work.path(), quality, threads, log, progress);
    log(format!("画像変換完了: {converted} 枚の画像を処理しました。"));

    let stem = archive.file_stem().unwrap().to_string_lossy().to_string();
    let new_zip_name = format!("{stem}.zip");

    let zip_dir = tempfile::tempdir()?;
    let temp_zip = zip_dir.path().join("_output_.zip");
    log("WebP画像を再圧縮中...".into());
    create_zip_archive(work.path(), &temp_zip)?;

    fs::create_dir_all(&new_temp)?;
    let final_zip = new_temp.join(&new_zip_name);
    if final_zip.exists() {
        fs::remove_file(&final_zip)?;
    }
    move_file(&temp_zip, &final_zip)?;
    log(format!("新ファイルを配置しました: new_temp\\{new_zip_name}"));

    fs::create_dir_all(&old_temp)?;
    let final_old = old_temp.join(&name);
    if final_old.exists() {
        fs::remove_file(&final_old)?;
    }
    move_file(archive, &final_old)?;
    log(format!("旧ファイルを移動しました: old_temp\\{name}"));

    log(format!("【処理成功】 {name}\n"));
    Ok(final_zip)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 出力名の衝突を避ける() {
        let dir = tempfile::tempdir().unwrap();
        let mut taken = HashSet::new();
        let a = unique_webp_path(&dir.path().join("a.png"), &mut taken);
        let b = unique_webp_path(&dir.path().join("a.jpg"), &mut taken);
        assert_eq!(a, dir.path().join("a.webp"));
        assert_eq!(b, dir.path().join("a.jpg.webp"));
    }

    #[test]
    fn 変換と失敗時の挙動() {
        let dir = tempfile::tempdir().unwrap();
        image::RgbImage::new(4, 4).save(dir.path().join("ok.png")).unwrap();
        fs::write(dir.path().join("broken.jpg"), b"not an image").unwrap();
        fs::write(dir.path().join("note.txt"), b"x").unwrap();
        let n = convert_images_in_dir(dir.path(), 80.0, 2, &|_| {}, &|_, _| {});
        assert_eq!(n, 1);
        assert!(dir.path().join("ok.webp").exists());
        assert!(!dir.path().join("ok.png").exists());
        assert!(dir.path().join("broken.jpg").exists()); // 失敗した元画像は残る
        assert!(!dir.path().join("note.txt").exists());
    }
}
