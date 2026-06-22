#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    Ok,
    Nearing,
    Over,
}

impl Verdict {
    pub fn as_str(self) -> &'static str {
        match self {
            Verdict::Ok => "ok",
            Verdict::Nearing => "nearing",
            Verdict::Over => "over -> recycle",
        }
    }

    /// Stable enum string for JSON output (REQ-005).
    pub fn as_json_str(self) -> &'static str {
        match self {
            Verdict::Ok => "ok",
            Verdict::Nearing => "nearing",
            Verdict::Over => "over_recycle",
        }
    }
}

/// Default thresholds: nearing = 70%, ceiling = 90%.
#[derive(Debug, Clone, Copy)]
pub struct Thresholds {
    pub nearing: u8,
    pub ceiling: u8,
}

impl Default for Thresholds {
    fn default() -> Self {
        Self {
            nearing: 70,
            ceiling: 90,
        }
    }
}

impl Thresholds {
    pub fn verdict(self, fill_percent: u8) -> Verdict {
        if fill_percent >= self.ceiling {
            Verdict::Over
        } else if fill_percent >= self.nearing {
            Verdict::Nearing
        } else {
            Verdict::Ok
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verdict_ok() {
        let t = Thresholds::default();
        assert_eq!(t.verdict(0), Verdict::Ok);
        assert_eq!(t.verdict(69), Verdict::Ok);
    }

    #[test]
    fn verdict_nearing() {
        let t = Thresholds::default();
        assert_eq!(t.verdict(70), Verdict::Nearing);
        assert_eq!(t.verdict(89), Verdict::Nearing);
    }

    #[test]
    fn verdict_over() {
        let t = Thresholds::default();
        assert_eq!(t.verdict(90), Verdict::Over);
        assert_eq!(t.verdict(100), Verdict::Over);
    }
}
