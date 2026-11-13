//! Original ASCII banner: a monolith carrying SEELE, red on black.
use std::io::IsTerminal;

const LETTERS: [[&str; 5]; 3] = [
    ["████", "█   ", "████", "   █", "████"], // S
    ["████", "█   ", "███ ", "█   ", "████"], // E
    ["█   ", "█   ", "█   ", "█   ", "████"], // L
];

pub fn hostname() -> String {
    std::fs::read_to_string("/proc/sys/kernel/hostname")
        .or_else(|_| std::fs::read_to_string("/etc/hostname"))
        .map(|s| s.trim().to_string())
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "localhost".into())
}

/// Colour is allowed when stdout is a terminal and NO_COLOR is unset.
pub fn color_ok() -> bool {
    std::io::stdout().is_terminal() && std::env::var_os("NO_COLOR").is_none()
}

pub fn render(color: bool) -> String {
    const W: usize = 30;
    let word = |row: usize| -> String { [0, 1, 1, 2, 1].iter().map(|&i| LETTERS[i][row]).collect::<Vec<_>>().join(" ") };
    let pad = |s: &str| -> String {
        let n = s.chars().count();
        let left = (W - n) / 2;
        format!("{}{}{}", " ".repeat(left), s, " ".repeat(W - n - left))
    };
    let mut lines = vec![format!("  ╔{}╗", "═".repeat(W))];
    lines.push(format!("  ║{}║", pad("")));
    for r in 0..5 {
        lines.push(format!("  ║{}║", pad(&word(r))));
    }
    lines.push(format!("  ║{}║", pad("")));
    lines.push(format!("  ║{}║", pad(&format!("rgb · {}", hostname()))));
    lines.push(format!("  ║{}║", pad("")));
    lines.push(format!("  ╚{}╝", "═".repeat(W)));
    let (on, off) = if color { ("\x1b[91;40m", "\x1b[0m") } else { ("", "") };
    lines.iter().map(|l| format!("{on}{l}{off}\n")).collect()
}

pub fn print(color: bool) {
    print!("{}", render(color));
}
