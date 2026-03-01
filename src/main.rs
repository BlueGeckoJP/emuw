use std::{
    io::{BufRead, BufReader},
    process::exit,
    sync::{
        Arc,
        atomic::{AtomicU32, Ordering},
    },
};

use eyre::eyre;
use portable_pty::{CommandBuilder, PtySize, native_pty_system};
use regex::Regex;

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
            let mut reader = Self::spawn_emerge_process()
                .await
                .expect("failed to spawn emerge process");
            Self::tail_follow(&mut reader, |line| {
                Self::parse_progress(line, Arc::clone(&p_num), Arc::clone(&p_denom));
            })
            .await;
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

    async fn tail_follow(
        tx: &mut tokio::sync::broadcast::Sender<String>,
        mut on_line: impl FnMut(&str),
    ) {
        let mut rx = tx.subscribe();
        let mut line = String::new();

        loop {
            line.clear();
            match rx.recv().await {
                Ok(msg) => {
                    on_line(&msg);
                }
                Err(_) => {
                    break;
                }
            }
        }
    }

    async fn spawn_emerge_process() -> eyre::Result<tokio::sync::broadcast::Sender<String>> {
        let pty_system = native_pty_system();

        let pair = pty_system
            .openpty(PtySize {
                rows: 24,
                cols: 200,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|e| eyre!("failed to open pty: {e}"))?;

        let mut cmd = CommandBuilder::new("emerge");
        cmd.args(["-avuDN", "@world"]);

        pair.slave
            .spawn_command(cmd)
            .map_err(|e| eyre!("failed to spawn command in pty: {e}"))?;

        let reader = pair
            .master
            .try_clone_reader()
            .map_err(|e| eyre!("failed to clone pty reader: {e}"))?;

        let (tx, _) = tokio::sync::broadcast::channel(4);
        let tx_clone = tx.clone();

        tokio::task::spawn_blocking(move || {
            let reader = BufReader::new(reader);
            let mut lines = reader.lines();
            while let Some(Ok(line)) = lines.next() {
                println!("{}", line);
                if tx_clone.send(line).is_err() {
                    println!("Receiver dropped, exiting...");
                    exit(0);
                }
            }
        });

        Ok(tx)
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
