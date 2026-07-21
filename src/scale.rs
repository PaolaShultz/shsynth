//! Scale membership used by the beginner-friendly live-input filter.

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ScaleKind {
    Major,
    NaturalMinor,
}

impl ScaleKind {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Major => "MAJOR",
            Self::NaturalMinor => "MINOR",
        }
    }

    const fn intervals(self) -> &'static [u8] {
        match self {
            Self::Major => &[0, 2, 4, 5, 7, 9, 11],
            Self::NaturalMinor => &[0, 2, 3, 5, 7, 8, 10],
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Scale {
    pub root: u8,
    pub kind: ScaleKind,
}

impl Default for Scale {
    fn default() -> Self {
        Self {
            root: 0,
            kind: ScaleKind::Major,
        }
    }
}

impl Scale {
    /// N00B is a gate, not a quantizer: an out-of-scale key stays silent.
    pub fn contains(self, note: u8) -> bool {
        let pitch = (12 + i16::from(note % 12) - i16::from(self.root % 12)) % 12;
        self.kind.intervals().contains(&(pitch as u8))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn major_and_natural_minor_membership_suppress_chromatic_outsiders() {
        let c_major = Scale::default();
        assert!(c_major.contains(60));
        assert!(!c_major.contains(61));
        assert!(c_major.contains(71));

        let c_sharp_minor = Scale {
            root: 1,
            kind: ScaleKind::NaturalMinor,
        };
        for note in [61, 63, 64, 66, 68, 69, 71] {
            assert!(c_sharp_minor.contains(note));
        }
        for note in [60, 62, 65, 67, 70] {
            assert!(!c_sharp_minor.contains(note));
        }
    }
}
