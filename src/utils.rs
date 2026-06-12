use std::path::PathBuf;

pub(crate) fn natural_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    let mut a_chars = a.chars().peekable();
    let mut b_chars = b.chars().peekable();

    loop {
        match (a_chars.peek(), b_chars.peek()) {
            (None, None) => return std::cmp::Ordering::Equal,
            (None, Some(_)) => return std::cmp::Ordering::Less,
            (Some(_), None) => return std::cmp::Ordering::Greater,
            (Some(&ac), Some(&bc)) => {
                if ac.is_ascii_digit() && bc.is_ascii_digit() {
                    let read_num = |chars: &mut std::iter::Peekable<std::str::Chars>| {
                        let mut num = 0u64;
                        while let Some(&c) = chars.peek() {
                            if c.is_ascii_digit() {
                                num = num * 10 + c.to_digit(10).unwrap() as u64;
                                chars.next();
                            } else {
                                break;
                            }
                        }
                        num
                    };
                    match read_num(&mut a_chars).cmp(&read_num(&mut b_chars)) {
                        std::cmp::Ordering::Equal => continue,
                        ord => return ord,
                    }
                } else {
                    match ac.to_ascii_lowercase().cmp(&bc.to_ascii_lowercase()) {
                        std::cmp::Ordering::Equal => {
                            a_chars.next();
                            b_chars.next();
                        }
                        ord => return ord,
                    }
                }
            }
        }
    }
}

fn walk_dir(dir: &std::path::Path, files: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return; };
    for entry in entries.flatten() {
        let p = entry.path();
        if p.is_dir() {
            walk_dir(&p, files);
        } else if p.is_file() {
            if let Some(ext) = p.extension().and_then(|s| s.to_str()) {
                if matches!(ext.to_ascii_lowercase().as_str(), "wav" | "flac" | "mp3" | "ogg" | "opus" | "m4a" | "aac" | "aiff" | "aif" | "wma" | "mp4" | "ape") {
                    files.push(p);
                }
            }
        }
    }
}

pub fn collect_audio_files(path: &PathBuf) -> Vec<PathBuf> {
    let mut files = Vec::new();
    if path.is_file() {
        files.push(path.clone());
    } else if path.is_dir() {
        walk_dir(path, &mut files);
    }
    
    // sort files naturally
    files.sort_by(|a, b| {
        let a_str = a.file_name().and_then(|s| s.to_str()).unwrap_or("");
        let b_str = b.file_name().and_then(|s| s.to_str()).unwrap_or("");
        natural_cmp(a_str, b_str)
    });
    
    files
}

pub fn format_time(seconds: f64) -> String {
    let total_secs = seconds.floor() as u64;
    let hours = total_secs / 3600;
    let minutes = (total_secs % 3600) / 60;
    let secs = total_secs % 60;
    let frac = seconds.fract();

    let time_str = if hours > 0 {
        format!("{hours}:{minutes:02}:{secs:02}")
    } else {
        format!("{minutes}:{secs:02}")
    };

    if frac > 0.001 {
        let decimals = format!("{frac:.3}");
        let trimmed = decimals.strip_prefix('0').unwrap_or("").trim_end_matches('0');
        if trimmed == "." { time_str } else { format!("{time_str}{trimmed}") }
    } else {
        time_str
    }
}
