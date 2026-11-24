//! Linear ADSR envelope segments.

/// Piecewise-linear ADSR envelope in seconds.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LinearAdsr {
    pub attack: f64,
    pub decay: f64,
    pub sustain: f32,
    pub release: f64,
}

impl Default for LinearAdsr {
    fn default() -> Self {
        Self {
            attack: 0.005,
            decay: 0.05,
            sustain: 0.8,
            release: 0.05,
        }
    }
}

impl LinearAdsr {
    /// Envelope amplitude at `elapsed` seconds from note onset over `note_duration` seconds.
    pub fn value_at(&self, elapsed: f64, note_duration: f64) -> f32 {
        if elapsed >= note_duration || note_duration <= 0.0 {
            return 0.0;
        }

        let release_start = (note_duration - self.release).max(0.0);

        if elapsed < self.attack {
            if self.attack <= 0.0 {
                return 1.0;
            }
            return (elapsed / self.attack) as f32;
        }

        if elapsed < self.attack + self.decay {
            if self.decay <= 0.0 {
                return self.sustain;
            }
            let progress = ((elapsed - self.attack) / self.decay) as f32;
            return 1.0 + progress * (self.sustain - 1.0);
        }

        if elapsed >= release_start {
            if self.release <= 0.0 {
                return 0.0;
            }
            let progress = ((elapsed - release_start) / self.release) as f32;
            let progress = progress.min(1.0);
            return self.sustain * (1.0 - progress);
        }

        self.sustain
    }

    /// Streaming-mode evaluation of the same envelope, gated by note-off instead of note length.
    ///
    /// While held (`released_after` is `None`) the envelope runs attack → decay → sustain and
    /// stays at sustain. Once released at `r` seconds after onset, it ramps linearly from its
    /// level at `r` to zero over `release` seconds, ringing past note-off. Deterministic mode
    /// ([`value_at`](Self::value_at)) instead places the release inside the known note length.
    pub fn gated_value_at(&self, elapsed: f64, released_after: Option<f64>) -> f32 {
        if elapsed < 0.0 {
            return 0.0;
        }
        let Some(released) = released_after else {
            return self.held_value_at(elapsed);
        };
        if elapsed < released {
            return self.held_value_at(elapsed);
        }
        if self.release <= 0.0 {
            return 0.0;
        }
        let progress = ((elapsed - released) / self.release).min(1.0) as f32;
        self.held_value_at(released) * (1.0 - progress)
    }

    /// Attack → decay → sustain with no release segment.
    fn held_value_at(&self, elapsed: f64) -> f32 {
        if elapsed < self.attack {
            return (elapsed / self.attack) as f32;
        }
        if elapsed < self.attack + self.decay {
            let progress = ((elapsed - self.attack) / self.decay) as f32;
            return 1.0 + progress * (self.sustain - 1.0);
        }
        self.sustain
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn envelope_starts_at_zero_and_rises() {
        let adsr = LinearAdsr::default();
        assert_eq!(adsr.value_at(0.0, 1.0), 0.0);
        assert!(adsr.value_at(0.002, 1.0) > 0.0);
    }

    #[test]
    fn gated_matches_deterministic_during_attack_and_decay() {
        let adsr = LinearAdsr::default();
        for elapsed in [0.0, 0.002, 0.004, 0.01, 0.03, 0.054] {
            let gated = adsr.gated_value_at(elapsed, None);
            assert!((gated - adsr.value_at(elapsed, 10.0)).abs() < 1e-6);
        }
    }

    #[test]
    fn gated_holds_sustain_while_held() {
        let adsr = LinearAdsr::default();
        assert_eq!(adsr.gated_value_at(100.0, None), adsr.sustain);
    }

    #[test]
    fn gated_release_rings_past_note_off_and_reaches_zero() {
        let adsr = LinearAdsr::default();
        let off = 0.5;
        assert_eq!(adsr.gated_value_at(off, Some(off)), adsr.sustain);
        let mid = adsr.gated_value_at(off + adsr.release / 2.0, Some(off));
        assert!((mid - adsr.sustain / 2.0).abs() < 1e-4);
        assert_eq!(adsr.gated_value_at(off + adsr.release, Some(off)), 0.0);
    }

    #[test]
    fn gated_release_mid_attack_is_continuous() {
        let adsr = LinearAdsr::default();
        let off = 0.002;
        let before = adsr.gated_value_at(off - 1e-7, None);
        let after = adsr.gated_value_at(off + 1e-7, Some(off));
        assert!((before - after).abs() < 1e-3);
    }

    #[test]
    fn deterministic_ends_with_the_note_while_gated_still_sounds() {
        let adsr = LinearAdsr::default();
        let length = 0.5;
        assert_eq!(adsr.value_at(length, length), 0.0);
        // At the same elapsed time in streaming, the note released at `length` is only
        // starting its release.
        assert!(adsr.gated_value_at(length, Some(length)) > 0.0);
        assert!(adsr.gated_value_at(length, None) > 0.0);
    }

    #[test]
    fn envelope_reaches_zero_at_note_end() {
        let adsr = LinearAdsr::default();
        assert_eq!(adsr.value_at(1.0, 1.0), 0.0);
    }
}
