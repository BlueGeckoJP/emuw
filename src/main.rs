use std::{
    io::SeekFrom,
    sync::{
        Arc,
        atomic::{AtomicU32, Ordering},
    },
};

use regex::Regex;
use tokio::io::{AsyncBufReadExt, AsyncSeekExt};

#[tokio::main]
async fn main() -> eyre::Result<()> {
    let build_watcher = BuildWatcher::new().await?;

    loop {
        let num = build_watcher.progress_num.load(Ordering::SeqCst);
        let denom = build_watcher.progress_denom.load(Ordering::SeqCst);
        if denom > 0 {
            println!(
                "Progress: {}/{} ({:.2}%)",
                num,
                denom,
                (num as f64 / denom as f64) * 100.0
            );
        } else {
            println!("Progress: N/A");
        }
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    }
}

struct BuildWatcher {
    progress_num: Arc<AtomicU32>,
    progress_denom: Arc<AtomicU32>,
}

impl BuildWatcher {
    async fn new() -> eyre::Result<Self> {
        let progress_num = Arc::new(AtomicU32::new(0));
        let progress_denom = Arc::new(AtomicU32::new(0));
        let p_num = Arc::clone(&progress_num);
        let p_denom = Arc::clone(&progress_denom);

        tokio::spawn(async move {
            /*/
                let mut file = tokio::fs::File::open("/var/log/emerge.log")
                    .await
                    .expect("failed to open log file");
                file.seek(SeekFrom::End(0))
                    .await
                    .expect("failed to seek to end of file");
                let mut reader = tokio::io::BufReader::new(file);

                Self::tail_follow(&mut reader, |line| {
                    Self::parse_progress(line, Arc::clone(&p_num), Arc::clone(&p_denom));
                })
                .await;
            */
        });

        Ok(Self {
            progress_num,
            progress_denom,
        })
    }

    fn parse_progress(line: &str, num: Arc<AtomicU32>, denom: Arc<AtomicU32>) {
        let fraction_progress =
            Regex::new(r"^\[(\d+)/(\d+)\]").expect("failed to compile regex for fraction");
        let percentage_progress =
            Regex::new(r"^\[\s*(\d+)%\]").expect("failed to compile regex for percentage");

        if let Some(caps) = fraction_progress.captures(line) {
            let num_val = caps[1].parse::<u32>().unwrap_or(0);
            let denom_val = caps[2].parse::<u32>().unwrap_or(0);
            num.store(num_val, Ordering::SeqCst);
            denom.store(denom_val, Ordering::SeqCst);
        } else if let Some(caps) = percentage_progress.captures(line) {
            let percentage = caps[1].parse::<u32>().unwrap_or(0);
            num.store(percentage, Ordering::SeqCst);
            denom.store(100, Ordering::SeqCst);
        } else {
            num.store(0, Ordering::SeqCst);
            denom.store(0, Ordering::SeqCst);
        }
    }

    async fn tail_follow<T: AsyncBufReadExt + Unpin>(
        reader: &mut T,
        mut on_line: impl FnMut(&str),
    ) {
        let mut line = String::new();

        loop {
            line.clear();
            match reader.read_line(&mut line).await {
                Ok(0) => {
                    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                }
                Ok(_) => {
                    on_line(line.trim_end());
                }
                Err(e) => {
                    eprintln!("Error reading file: {}", e);
                    break;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_parse_progress() {
        let num = Arc::new(AtomicU32::new(0));
        let denom = Arc::new(AtomicU32::new(0));

        // fraction format
        BuildWatcher::parse_progress(
            "[1/10] Building package A",
            Arc::clone(&num),
            Arc::clone(&denom),
        );
        assert_eq!(num.load(Ordering::SeqCst), 1);
        assert_eq!(denom.load(Ordering::SeqCst), 10);

        // fraction format (next step)
        BuildWatcher::parse_progress(
            "[2/10] Building package B",
            Arc::clone(&num),
            Arc::clone(&denom),
        );
        assert_eq!(num.load(Ordering::SeqCst), 2);
        assert_eq!(denom.load(Ordering::SeqCst), 10);

        // percentage format
        BuildWatcher::parse_progress(
            "[ 50%] Building package C",
            Arc::clone(&num),
            Arc::clone(&denom),
        );
        assert_eq!(num.load(Ordering::SeqCst), 50);
        assert_eq!(denom.load(Ordering::SeqCst), 100);

        // no match -> reset to 0
        BuildWatcher::parse_progress("some random line", Arc::clone(&num), Arc::clone(&denom));
        assert_eq!(num.load(Ordering::SeqCst), 0);
        assert_eq!(denom.load(Ordering::SeqCst), 0);
    }
}
