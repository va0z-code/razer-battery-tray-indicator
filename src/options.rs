// Command line flags, mainly for testing notifications without waiting for
// a real battery to drain.

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Options {
    /// `--simulate-battery <N>`: report N% for every device (adds a fake
    /// device if no mouse is connected).
    pub simulate_battery: Option<i32>,
    /// `--simulate-game`: behave as if a fullscreen game is running.
    pub simulate_game: bool,
    /// `--fast-reminders`: reminder schedule minutes become seconds.
    pub fast_reminders: bool,
}

impl Options {
    pub fn parse(args: impl IntoIterator<Item = String>) -> Result<Self, String> {
        let mut opts = Options::default();
        let mut args = args.into_iter();
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--simulate-battery" => {
                    let value = args
                        .next()
                        .ok_or("--simulate-battery needs a value (0-100)")?;
                    let level: i32 = value
                        .parse()
                        .map_err(|_| format!("invalid battery level: {value}"))?;
                    if !(0..=100).contains(&level) {
                        return Err(format!("battery level out of range: {level}"));
                    }
                    opts.simulate_battery = Some(level);
                }
                "--simulate-game" => opts.simulate_game = true,
                "--fast-reminders" => opts.fast_reminders = true,
                other => return Err(format!("unknown argument: {other}")),
            }
        }
        Ok(opts)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<Options, String> {
        Options::parse(args.iter().map(|s| s.to_string()))
    }

    #[test]
    fn parses_all_flags() {
        let opts = parse(&[
            "--simulate-battery",
            "23",
            "--simulate-game",
            "--fast-reminders",
        ])
        .unwrap();
        assert_eq!(opts.simulate_battery, Some(23));
        assert!(opts.simulate_game);
        assert!(opts.fast_reminders);
    }

    #[test]
    fn rejects_bad_input() {
        assert!(parse(&["--simulate-battery"]).is_err());
        assert!(parse(&["--simulate-battery", "150"]).is_err());
        assert!(parse(&["--nope"]).is_err());
    }
}
