// Author: Jeff
// Date: 2026-09-18
// Description: Numbers people can read — bytes as KiB/MiB/GiB, rates per second, durations
// Notes: Binary units (1024) to match free(1), btop and the shell's memory card

const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
const STEP: f64 = 1024.0;

// 1536 → "1.5 KiB"; whole bytes stay whole
pub fn bytes(n: u64) -> String {
    let mut value = n as f64;
    let mut unit = 0;
    while value >= STEP && unit < UNITS.len() - 1 {
        value /= STEP;
        unit += 1;
    }
    if unit == 0 {
        format!("{n} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

// A rate, or "—" when the process would not tell us
pub fn rate(n: Option<f64>) -> String {
    match n {
        Some(v) => format!("{}/s", bytes(v.max(0.0) as u64)),
        None => "—".into(),
    }
}

// 93784 s → "1d 2h 3m"
pub fn duration(seconds: f64) -> String {
    let total = seconds.max(0.0) as u64;
    let (days, hours, minutes) = (total / 86_400, total % 86_400 / 3600, total % 3600 / 60);
    if days > 0 {
        format!("{days}d {hours}h {minutes}m")
    } else if hours > 0 {
        format!("{hours}h {minutes}m")
    } else {
        format!("{minutes}m")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_read_like_free_and_btop() {
        assert_eq!(bytes(512), "512 B");
        assert_eq!(bytes(1536), "1.5 KiB");
        assert_eq!(bytes(3 * 1024 * 1024 * 1024), "3.0 GiB");
        assert_eq!(rate(None), "—");
        assert_eq!(rate(Some(2048.0)), "2.0 KiB/s");
        assert_eq!(duration(93_784.0), "1d 2h 3m");
        assert_eq!(duration(59.0), "0m");
    }
}
