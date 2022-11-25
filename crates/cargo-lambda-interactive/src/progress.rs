use crate::is_stdout_tty;
use indicatif::{ProgressBar, ProgressStyle};
use std::time::Duration;

pub struct Progress {
    bar: Option<ProgressBar>,
}

impl Progress {
    pub fn start(msg: impl ToString) -> Progress {
        let bar = if is_stdout_tty() {
            Some(show_progress(msg))
        } else {
            println!("▹▹▹▹▹ {}", msg.to_string());
            None
        };
        Progress { bar }
    }

    pub fn finish(&self, msg: &str) {
        if let Some(bar) = &self.bar {
            bar.finish_with_message(msg.to_string());
        } else {
            println!("▪▪▪▪▪ {}", msg);
        }
    }

    pub fn set_message(&self, msg: &str) {
        if let Some(bar) = &self.bar {
            bar.set_message(msg.to_string());
        } else {
            println!("▹▹▹▹▹ {}", msg);
        }
    }

    pub fn finish_and_clear(&self) {
        if let Some(bar) = &self.bar {
            bar.finish_and_clear();
        }
    }
}

fn show_progress(msg: impl ToString) -> ProgressBar {
    let pb = ProgressBar::new_spinner();
    pb.enable_steady_tick(Duration::from_millis(120));
    pb.set_style(ProgressStyle::default_spinner().tick_strings(&[
        "▹▹▹▹▹",
        "▸▹▹▹▹",
        "▹▸▹▹▹",
        "▹▹▸▹▹",
        "▹▹▹▸▹",
        "▹▹▹▹▸",
        "▪▪▪▪▪",
    ]));
    pb.set_message(msg.to_string());
    pb
}
