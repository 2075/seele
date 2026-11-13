//! Dry-run sink that draws each zone as truecolor blocks in the terminal.
use std::io::Write;

use super::{Frame, Sink};
use crate::config::Config;

pub struct TerminalSink {
    labels: Vec<String>,
    sizes: Vec<usize>,
    drawn: bool,
}

impl TerminalSink {
    pub fn new(cfg: &Config, sizes: Vec<usize>) -> Self {
        let labels = cfg
            .zones
            .iter()
            .map(|b| {
                let s: String = format!("{:>7} {}/{}", b.role, b.device, b.zone).chars().take(42).collect();
                format!("{s:<42}")
            })
            .collect();
        TerminalSink { labels, sizes, drawn: false }
    }
}

impl Sink for TerminalSink {
    fn sizes(&self) -> &[usize] {
        &self.sizes
    }

    fn show(&mut self, frame: &Frame, status: &str) -> anyhow::Result<()> {
        let mut out = String::new();
        if self.drawn {
            out.push_str(&format!("\x1b[{}F", self.labels.len() + 1));
        }
        for (i, label) in self.labels.iter().enumerate() {
            out.push_str(label);
            out.push(' ');
            for (r, g, b) in frame.get(&i).into_iter().flatten() {
                out.push_str(&format!("\x1b[38;2;{r};{g};{b}m██"));
            }
            out.push_str("\x1b[0m\x1b[K\n");
        }
        out.push_str(status);
        out.push_str("\x1b[K\n");
        let mut so = std::io::stdout().lock();
        so.write_all(out.as_bytes())?;
        so.flush()?;
        self.drawn = true;
        Ok(())
    }
}
