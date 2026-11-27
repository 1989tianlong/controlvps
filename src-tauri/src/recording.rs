use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::time::{Instant, SystemTime};

use chrono::{DateTime, SecondsFormat, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::{AppError, AppResult};

const MAX_RECORDING_BYTES: usize = 64 * 1024 * 1024;
const MAX_PLAYBACK_BYTES: u64 = MAX_RECORDING_BYTES as u64;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordingSummary {
    pub id: String,
    pub server_id: String,
    pub server_name: String,
    pub started_at: String,
    pub duration_seconds: f64,
    pub size_bytes: u64,
    pub frames: usize,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordingFrame {
    pub time: f64,
    pub kind: String,
    pub data: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordingDetail {
    pub summary: RecordingSummary,
    pub frames: Vec<RecordingFrame>,
}

pub struct Recorder {
    id: String,
    dir: PathBuf,
    file: BufWriter<File>,
    started: Instant,
    started_at: String,
    server_id: String,
    server_name: String,
    frames: usize,
    bytes: usize,
    limit_hit: bool,
    width: u32,
    height: u32,
}

impl Recorder {
    pub fn create(
        data_dir: &Path,
        server_id: &str,
        server_name: &str,
        width: u32,
        height: u32,
    ) -> AppResult<(Self, String, String)> {
        let dir = directory(data_dir);
        fs::create_dir_all(&dir)?;
        let id = Uuid::new_v4().to_string();
        let path = dir.join(format!("{id}.cast"));
        let mut file = BufWriter::new(create_private_file(&path)?);
        let started_at = Utc::now();
        let header = serde_json::json!({
            "version": 2,
            "width": width.max(20),
            "height": height.max(5),
            "timestamp": started_at.timestamp(),
            "title": server_name,
            "env": { "TERM": "xterm-256color", "SHELL": "/bin/sh" },
        });
        writeln!(file, "{header}")?;
        file.flush()?;
        Ok((
            Self {
                id: id.clone(),
                dir,
                file,
                started: Instant::now(),
                started_at: started_at.to_rfc3339_opts(SecondsFormat::Secs, true),
                server_id: server_id.to_string(),
                server_name: server_name.to_string(),
                frames: 0,
                bytes: 0,
                limit_hit: false,
                width: width.max(20),
                height: height.max(5),
            },
            id,
            started_at.to_rfc3339_opts(SecondsFormat::Secs, true),
        ))
    }

    pub fn write_text(&mut self, kind: &str, text: &str) -> bool {
        if text.is_empty() {
            return false;
        }
        if self.limit_hit {
            return true;
        }
        let time = self.started.elapsed().as_secs_f64();
        let line = serde_json::json!([time, kind, text]);
        let rendered = line.to_string();
        if self.bytes + rendered.len() + 1 > MAX_RECORDING_BYTES {
            self.limit_hit = true;
            return true;
        }
        if writeln!(self.file, "{rendered}").is_ok() {
            self.frames += 1;
            self.bytes += rendered.len() + 1;
        }
        self.limit_hit
    }

    pub fn resize(&mut self, columns: u32, rows: u32) {
        self.width = columns.max(20);
        self.height = rows.max(5);
        let _ = self.write_text("r", &format!("{}x{}", self.width, self.height));
    }

    pub fn finish(mut self) -> AppResult<()> {
        self.file.flush()?;
        let path = self.dir.join(format!("{}.cast", self.id));
        let size = fs::metadata(&path).map(|item| item.len()).unwrap_or(0);
        let summary = RecordingSummary {
            id: self.id.clone(),
            server_id: self.server_id.clone(),
            server_name: self.server_name.clone(),
            started_at: self.started_at.clone(),
            duration_seconds: self.started.elapsed().as_secs_f64(),
            size_bytes: size,
            frames: self.frames,
            width: self.width,
            height: self.height,
        };
        let meta = serde_json::to_vec_pretty(&summary)
            .map_err(|error| AppError::Message(error.to_string()))?;
        let mut meta_file = create_private_file(&self.dir.join(format!("{}.meta.json", self.id)))?;
        meta_file.write_all(&meta)?;
        Ok(())
    }
}

fn create_private_file(path: &Path) -> std::io::Result<File> {
    let mut options = OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path)
}

pub fn directory(data_dir: &Path) -> PathBuf {
    data_dir.join("recordings")
}

pub fn list(data_dir: &Path) -> AppResult<Vec<RecordingSummary>> {
    let dir = directory(data_dir);
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let mut summaries = Vec::new();
    for entry in fs::read_dir(&dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|value| value.to_str()) != Some("cast") {
            continue;
        }
        let Some(id) = path
            .file_stem()
            .and_then(|value| value.to_str())
            .map(str::to_string)
        else {
            continue;
        };
        let size = entry.metadata().map(|item| item.len()).unwrap_or(0);
        let modified = entry
            .metadata()
            .and_then(|item| item.modified())
            .ok();
        summaries.push(load_summary(&dir, &id, &path, size, modified));
    }
    summaries.sort_by(|a, b| b.started_at.cmp(&a.started_at));
    Ok(summaries)
}

fn load_summary(
    dir: &Path,
    id: &str,
    cast_path: &Path,
    size: u64,
    modified: Option<SystemTime>,
) -> RecordingSummary {
    let meta_path = dir.join(format!("{id}.meta.json"));
    let mut summary = match fs::read(&meta_path)
        .ok()
        .and_then(|raw| serde_json::from_slice::<RecordingSummary>(&raw).ok())
    {
        Some(summary) => summary,
        None => {
            let started = modified
                .and_then(|value| value.duration_since(SystemTime::UNIX_EPOCH).ok())
                .map(|value| value.as_secs())
                .unwrap_or(0);
            RecordingSummary {
                id: id.to_string(),
                server_id: String::new(),
                server_name: "未完成的录制".into(),
                started_at: DateTime::<Utc>::from_timestamp(started as i64, 0)
                    .unwrap_or_else(Utc::now)
                    .to_rfc3339_opts(SecondsFormat::Secs, true),
                duration_seconds: 0.0,
                size_bytes: size,
                frames: 0,
                width: 100,
                height: 32,
            }
        }
    };
    if summary.frames == 0 && size <= MAX_PLAYBACK_BYTES {
        if let Ok((frames, duration)) = scan(cast_path) {
            summary.frames = frames;
            summary.duration_seconds = duration;
        }
    }
    summary.size_bytes = size;
    summary
}

pub fn read(data_dir: &Path, id: &str) -> AppResult<RecordingDetail> {
    let path = recording_path(data_dir, id)?;
    let file = File::open(&path)?;
    let metadata = file.metadata()?;
    let size = metadata.len();
    if size > MAX_PLAYBACK_BYTES {
        return Err(AppError::Message("录制文件过大，无法在应用内回放".into()));
    }
    let reader = BufReader::new(file);
    let mut frames = Vec::new();
    let mut header = serde_json::Value::Null;
    for (index, line) in reader.lines().enumerate() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        if index == 0 {
            header = serde_json::from_str(&line).unwrap_or(serde_json::Value::Null);
            continue;
        }
        let Ok(value) = serde_json::from_str::<serde_json::Value>(&line) else {
            continue;
        };
        let Some(items) = value.as_array() else {
            continue;
        };
        if items.len() < 3 {
            continue;
        }
        frames.push(RecordingFrame {
            time: items[0].as_f64().unwrap_or(0.0),
            kind: items[1].as_str().unwrap_or("o").to_string(),
            data: items[2].as_str().unwrap_or_default().to_string(),
        });
    }
    let dir = directory(data_dir);
    let mut summary = load_summary(&dir, id, &path, size, metadata.modified().ok());
    summary.frames = frames.len();
    if let Some(last) = frames.last() {
        if summary.duration_seconds <= 0.0 {
            summary.duration_seconds = last.time;
        }
    }
    summary.width = header
        .get("width")
        .and_then(|value| value.as_u64())
        .map(|value| value as u32)
        .unwrap_or(summary.width);
    summary.height = header
        .get("height")
        .and_then(|value| value.as_u64())
        .map(|value| value as u32)
        .unwrap_or(summary.height);
    if let Some(title) = header.get("title").and_then(|value| value.as_str()) {
        if summary.server_name.is_empty() || summary.server_name == "未完成的录制" {
            summary.server_name = title.to_string();
        }
    }
    Ok(RecordingDetail { summary, frames })
}

pub fn delete(data_dir: &Path, id: &str) -> AppResult<()> {
    let path = recording_path(data_dir, id)?;
    fs::remove_file(path)?;
    let meta = directory(data_dir).join(format!("{id}.meta.json"));
    let _ = fs::remove_file(meta);
    Ok(())
}

fn recording_path(data_dir: &Path, id: &str) -> AppResult<PathBuf> {
    if id.is_empty()
        || id.len() > 64
        || !id
            .chars()
            .all(|value| value.is_ascii_alphanumeric() || value == '-')
    {
        return Err(AppError::Message("录制编号不合法".into()));
    }
    let path = directory(data_dir).join(format!("{id}.cast"));
    if !path.exists() {
        return Err(AppError::Message("未找到这段录制".into()));
    }
    Ok(path)
}

fn scan(path: &Path) -> AppResult<(usize, f64)> {
    let file = File::open(path)?;
    let reader = BufReader::new(file);
    let mut frames = 0;
    let mut last = 0.0;
    for line in reader.lines().skip(1) {
        let line = line?;
        let Some(rest) = line.trim().strip_prefix('[') else {
            continue;
        };
        let Some((time, _)) = rest.split_once(',') else {
            continue;
        };
        if let Ok(value) = time.trim().parse::<f64>() {
            last = value;
            frames += 1;
        }
    }
    Ok((frames, last))
}
