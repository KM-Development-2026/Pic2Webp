//! 設定ファイル (exe と同じフォルダの `pic2webp.conf`)。`key=value` 形式、`#` 以降はコメント。
use std::path::PathBuf;

const FILE_NAME: &str = "pic2webp.conf";

#[derive(Debug, Clone, PartialEq)]
pub struct Config {
    /// 画像変換の並列数。0 は自動（論理コア数）。
    pub threads: usize,
}

impl Default for Config {
    fn default() -> Self {
        Self { threads: 0 }
    }
}

impl Config {
    fn path() -> Option<PathBuf> {
        Some(std::env::current_exe().ok()?.parent()?.join(FILE_NAME))
    }

    pub fn parse(text: &str) -> Self {
        let mut c = Self::default();
        for line in text.lines() {
            let line = line.split('#').next().unwrap_or("").trim();
            if let Some((k, v)) = line.split_once('=') {
                if k.trim() == "threads" {
                    if let Ok(n) = v.trim().parse() {
                        c.threads = n;
                    }
                }
            }
        }
        c
    }

    pub fn to_text(&self) -> String {
        format!("# 画像変換の並列数 (0 = 自動: 論理コア数)\nthreads={}\n", self.threads)
    }

    /// 読み込み。ファイルが無い・読めない場合は既定値。
    pub fn load() -> Self {
        Self::path()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .map(|t| Self::parse(&t))
            .unwrap_or_default()
    }

    pub fn save(&self) {
        if let Some(p) = Self::path() {
            let _ = std::fs::write(p, self.to_text());
        }
    }
}

/// 設定値 (0=自動) を実際のスレッド数にする
pub fn resolve_threads(configured: usize) -> usize {
    if configured == 0 {
        std::thread::available_parallelism().map_or(1, |n| n.get())
    } else {
        configured
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 設定のパースと往復() {
        assert_eq!(Config::parse("threads = 4 # コメント\n").threads, 4);
        assert_eq!(Config::parse("").threads, 0);
        assert_eq!(Config::parse("threads=abc").threads, 0);
        let c = Config { threads: 6 };
        assert_eq!(Config::parse(&c.to_text()), c);
    }

    #[test]
    fn 自動は1以上() {
        assert!(resolve_threads(0) >= 1);
        assert_eq!(resolve_threads(3), 3);
    }
}
