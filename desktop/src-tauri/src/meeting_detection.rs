//! Cross-platform meeting-observation policy.
//!
//! Platform adapters report raw evidence (for example Core Audio on macOS or WASAPI on Windows).
//! This module owns the reusable policy that turns those observations into stable call edges. It has no
//! Apple, Windows, Tauri, process, network, or capture dependencies.

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MeetingSourceFamily {
    Desktop,
    Browser,
}

impl MeetingSourceFamily {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "desktop" => Some(Self::Desktop),
            "browser" => Some(Self::Browser),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MeetingObservation {
    pub identity: String,
    pub app: String,
    pub title: String,
    pub bundle: String,
    pub family: MeetingSourceFamily,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MeetingDetectionPolicy {
    pub enabled: bool,
    pub desktop: bool,
    pub browser: bool,
}

impl MeetingDetectionPolicy {
    fn allows(self, observation: &MeetingObservation) -> bool {
        self.enabled
            && match observation.family {
                MeetingSourceFamily::Desktop => self.desktop,
                MeetingSourceFamily::Browser => self.browser,
            }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MeetingTransition {
    None,
    Started,
    Switched,
    Ended,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MeetingDetection {
    pub active: Option<MeetingObservation>,
    pub transition: MeetingTransition,
}

pub struct MeetingDetector {
    start_hits: u8,
    end_misses: u8,
    active: Option<MeetingObservation>,
    candidate: Option<MeetingObservation>,
    candidate_hits: u8,
    missing_hits: u8,
}

impl Default for MeetingDetector {
    fn default() -> Self {
        Self::new(2, 3)
    }
}

impl MeetingDetector {
    pub fn new(start_hits: u8, end_misses: u8) -> Self {
        Self {
            start_hits: start_hits.max(1),
            end_misses: end_misses.max(1),
            active: None,
            candidate: None,
            candidate_hits: 0,
            missing_hits: 0,
        }
    }

    pub fn reset(&mut self) -> MeetingDetection {
        let ended = self.active.take().is_some();
        self.candidate = None;
        self.candidate_hits = 0;
        self.missing_hits = 0;
        MeetingDetection {
            active: None,
            transition: if ended {
                MeetingTransition::Ended
            } else {
                MeetingTransition::None
            },
        }
    }

    pub fn update(
        &mut self,
        observed: Option<MeetingObservation>,
        policy: MeetingDetectionPolicy,
    ) -> MeetingDetection {
        if !policy.enabled {
            return self.reset();
        }
        let observed = observed.filter(|value| policy.allows(value));

        if let Some(active) = self.active.as_mut() {
            if let Some(observed) = observed {
                if observed.identity == active.identity {
                    *active = observed;
                    self.candidate = None;
                    self.candidate_hits = 0;
                    self.missing_hits = 0;
                    return self.decision(MeetingTransition::None);
                }

                self.missing_hits = 0;
                if self
                    .candidate
                    .as_ref()
                    .is_some_and(|candidate| candidate.identity == observed.identity)
                {
                    self.candidate_hits = self.candidate_hits.saturating_add(1);
                    self.candidate = Some(observed);
                } else {
                    self.candidate = Some(observed);
                    self.candidate_hits = 1;
                }
                if self.candidate_hits >= self.start_hits {
                    self.active = self.candidate.take();
                    self.candidate_hits = 0;
                    return self.decision(MeetingTransition::Switched);
                }
                return self.decision(MeetingTransition::None);
            }

            self.candidate = None;
            self.candidate_hits = 0;
            self.missing_hits = self.missing_hits.saturating_add(1);
            if self.missing_hits >= self.end_misses {
                return self.reset();
            }
            return self.decision(MeetingTransition::None);
        }

        self.missing_hits = 0;
        let Some(observed) = observed else {
            self.candidate = None;
            self.candidate_hits = 0;
            return self.decision(MeetingTransition::None);
        };
        if self
            .candidate
            .as_ref()
            .is_some_and(|candidate| candidate.identity == observed.identity)
        {
            self.candidate_hits = self.candidate_hits.saturating_add(1);
            self.candidate = Some(observed);
        } else {
            self.candidate = Some(observed);
            self.candidate_hits = 1;
        }
        if self.candidate_hits >= self.start_hits {
            self.active = self.candidate.take();
            self.candidate_hits = 0;
            return self.decision(MeetingTransition::Started);
        }
        self.decision(MeetingTransition::None)
    }

    fn decision(&self, transition: MeetingTransition) -> MeetingDetection {
        MeetingDetection {
            active: self.active.clone(),
            transition,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn observation(identity: &str, family: MeetingSourceFamily) -> MeetingObservation {
        MeetingObservation {
            identity: identity.into(),
            app: identity.split('|').next().unwrap_or(identity).into(),
            title: String::new(),
            bundle: identity.split('|').nth(1).unwrap_or_default().into(),
            family,
        }
    }

    fn all_sources() -> MeetingDetectionPolicy {
        MeetingDetectionPolicy {
            enabled: true,
            desktop: true,
            browser: true,
        }
    }

    #[test]
    fn stable_edges_ignore_brief_process_loss() {
        let mut detector = MeetingDetector::default();
        let zoom = observation("zoom|us.zoom.xos", MeetingSourceFamily::Desktop);
        assert_eq!(
            detector.update(Some(zoom.clone()), all_sources()).active,
            None
        );
        assert_eq!(
            detector
                .update(Some(zoom.clone()), all_sources())
                .transition,
            MeetingTransition::Started
        );
        assert_eq!(
            detector.update(None, all_sources()).active,
            Some(zoom.clone())
        );
        assert_eq!(
            detector.update(None, all_sources()).active,
            Some(zoom.clone())
        );
        assert_eq!(
            detector.update(None, all_sources()).transition,
            MeetingTransition::Ended
        );
    }

    #[test]
    fn platform_switch_requires_a_stable_replacement() {
        let mut detector = MeetingDetector::default();
        let meet = observation("meet|com.google.Chrome", MeetingSourceFamily::Browser);
        let teams = observation("teams|com.microsoft.teams2", MeetingSourceFamily::Desktop);
        detector.update(Some(meet.clone()), all_sources());
        detector.update(Some(meet.clone()), all_sources());
        assert_eq!(
            detector.update(Some(teams.clone()), all_sources()).active,
            Some(meet)
        );
        let switched = detector.update(Some(teams.clone()), all_sources());
        assert_eq!(switched.active, Some(teams));
        assert_eq!(switched.transition, MeetingTransition::Switched);
    }

    #[test]
    fn source_controls_and_disabled_detection_fail_closed() {
        let mut detector = MeetingDetector::default();
        let meet = observation("meet|com.google.Chrome", MeetingSourceFamily::Browser);
        let desktop_only = MeetingDetectionPolicy {
            enabled: true,
            desktop: true,
            browser: false,
        };
        assert_eq!(
            detector.update(Some(meet.clone()), desktop_only).active,
            None
        );
        assert_eq!(detector.update(Some(meet), desktop_only).active, None);

        let zoom = observation("zoom|us.zoom.xos", MeetingSourceFamily::Desktop);
        detector.update(Some(zoom.clone()), all_sources());
        detector.update(Some(zoom), all_sources());
        assert_eq!(
            detector
                .update(
                    None,
                    MeetingDetectionPolicy {
                        enabled: false,
                        desktop: true,
                        browser: true,
                    }
                )
                .transition,
            MeetingTransition::Ended
        );
    }

    #[test]
    fn current_observation_refreshes_metadata_without_a_new_edge() {
        let mut detector = MeetingDetector::default();
        let mut zoom = observation("zoom|us.zoom.xos", MeetingSourceFamily::Desktop);
        detector.update(Some(zoom.clone()), all_sources());
        detector.update(Some(zoom.clone()), all_sources());
        zoom.title = "Customer review".into();
        let refreshed = detector.update(Some(zoom.clone()), all_sources());
        assert_eq!(refreshed.transition, MeetingTransition::None);
        assert_eq!(refreshed.active, Some(zoom));
    }
}
