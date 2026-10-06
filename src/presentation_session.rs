use std::time::Duration;

use crate::presentation_plan::{PlannedScreenState, ScreenRouteInventory};

/// Elapsed monotonic time supplied by the host. The session never reads a
/// process clock, which keeps timer transitions deterministic in tests and in
/// future non-HTTP hosts.
pub(crate) type MonotonicNow = Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TimerStatus {
    Stopped,
    Running,
    Paused,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct TimerSnapshot {
    pub(crate) status: TimerStatus,
    pub(crate) elapsed: Duration,
}

#[derive(Debug, Default)]
struct PresentationTimer {
    status: TimerStatus,
    accumulated: Duration,
    running_since: Option<Duration>,
}

impl Default for TimerStatus {
    fn default() -> Self {
        Self::Stopped
    }
}

impl PresentationTimer {
    fn elapsed_at(&self, now: MonotonicNow) -> Duration {
        self.accumulated
            + self
                .running_since
                .map(|started| now.saturating_sub(started))
                .unwrap_or_default()
    }

    fn snapshot(&self, now: MonotonicNow) -> TimerSnapshot {
        TimerSnapshot {
            status: self.status,
            elapsed: self.elapsed_at(now),
        }
    }

    fn start_at(&mut self, now: MonotonicNow) {
        if self.status != TimerStatus::Running {
            self.running_since = Some(now);
            self.status = TimerStatus::Running;
        }
    }

    fn pause_at(&mut self, now: MonotonicNow) {
        if self.status == TimerStatus::Running {
            self.accumulated = self.elapsed_at(now);
            self.running_since = None;
            self.status = TimerStatus::Paused;
        }
    }

    fn reset(&mut self) {
        self.status = TimerStatus::Stopped;
        self.accumulated = Duration::ZERO;
        self.running_since = None;
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SessionCommand {
    Advance,
    Retreat,
    DetailNext,
    DetailPrevious,
    Navigate {
        generation: u64,
        section: usize,
        detail: usize,
        step: usize,
    },
    TimerStart,
    TimerPause,
    TimerReset,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SessionConflict {
    StaleGeneration,
    InvalidRoute,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RejectedCommand {
    pub(crate) reason: SessionConflict,
    pub(crate) snapshot: SessionSnapshot,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct AcceptedCommand {
    pub(crate) state_changed: bool,
    pub(crate) revision: u64,
}

pub(crate) type CommandOutcome = Result<AcceptedCommand, RejectedCommand>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SessionSnapshot {
    pub(crate) generation: u64,
    pub(crate) revision: u64,
    pub(crate) current: Option<PlannedScreenState>,
    pub(crate) next: Option<PlannedScreenState>,
    pub(crate) timer: TimerSnapshot,
}

#[derive(Debug, Default)]
pub(crate) struct PresentationSession {
    generation: u64,
    revision: u64,
    routes: ScreenRouteInventory,
    current: usize,
    timer: PresentationTimer,
}

impl PresentationSession {
    fn insertion_clamp(mut available: Vec<usize>, requested: usize) -> Option<usize> {
        available.sort_unstable();
        available.dedup();
        let insertion = available.partition_point(|value| *value < requested);
        available
            .get(insertion)
            .copied()
            .or_else(|| available.last().copied())
    }

    pub(crate) fn generation(&self) -> u64 {
        self.generation
    }

    #[cfg(test)]
    pub(crate) fn revision(&self) -> u64 {
        self.revision
    }

    #[cfg(test)]
    pub(crate) fn current_index(&self) -> usize {
        self.current
    }

    #[cfg(test)]
    pub(crate) fn timer_status(&self) -> TimerStatus {
        self.timer.status
    }

    pub(crate) fn routes(&self) -> &ScreenRouteInventory {
        &self.routes
    }

    pub(crate) fn current_route(&self) -> Option<&PlannedScreenState> {
        self.routes.get(self.current)
    }

    fn next_route(&self) -> Option<&PlannedScreenState> {
        self.advance_index()
            .and_then(|index| self.routes.get(index))
    }

    fn route_index(&self, section: usize, detail: usize, step: usize) -> Option<usize> {
        self.routes.route_index(section, detail, step)
    }

    fn advance_index(&self) -> Option<usize> {
        let route = self.current_route()?;
        if route.step < route.step_count {
            self.route_index(route.section, route.detail, route.step + 1)
        } else {
            self.route_index(route.section + 1, 0, 0)
        }
    }

    fn retreat_index(&self) -> Option<usize> {
        let route = self.current_route()?;
        if route.step > 0 {
            self.route_index(route.section, route.detail, route.step - 1)
        } else {
            route
                .section
                .checked_sub(1)
                .and_then(|section| self.route_index(section, 0, 0))
        }
    }

    fn detail_index(&self, delta: isize) -> Option<usize> {
        let route = self.current_route()?;
        let detail = route.detail.checked_add_signed(delta)?;
        self.route_index(route.section, detail, 0)
    }

    pub(crate) fn install_routes(&mut self, routes: impl Into<ScreenRouteInventory>) {
        let previous = self.current_route().cloned();
        self.routes = routes.into();
        self.generation = self.generation.saturating_add(1);
        if self.routes.is_empty() {
            self.current = 0;
        } else if let Some(previous) = previous {
            let same_slide_step = Self::insertion_clamp(
                self.routes
                    .iter()
                    .filter(|route| route.slide_id == previous.slide_id)
                    .map(|route| route.step)
                    .collect(),
                previous.step,
            );
            let same_slide = same_slide_step.and_then(|step| {
                self.routes.position(|candidate| {
                    candidate.slide_id == previous.slide_id && candidate.step == step
                })
            });
            self.current = same_slide.unwrap_or_else(|| {
                let section = Self::insertion_clamp(
                    self.routes.iter().map(|route| route.section).collect(),
                    previous.section,
                )
                .expect("nonempty routes have an available Section");
                let detail = Self::insertion_clamp(
                    self.routes
                        .iter()
                        .filter(|route| route.section == section)
                        .map(|route| route.detail)
                        .collect(),
                    previous.detail,
                )
                .expect("an available Section has an available Detail");
                let step = Self::insertion_clamp(
                    self.routes
                        .iter()
                        .filter(|route| route.section == section && route.detail == detail)
                        .map(|route| route.step)
                        .collect(),
                    previous.step,
                )
                .expect("an available slide has an available Step");
                self.route_index(section, detail, step)
                    .expect("clamped coordinates came from an available route")
            });
        }
        self.revision = self.revision.saturating_add(1);
    }

    pub(crate) fn apply(&mut self, command: SessionCommand, now: MonotonicNow) -> CommandOutcome {
        let before_current = self.current;
        let before_timer = self.timer.snapshot(now);
        let target = match command {
            SessionCommand::Advance => {
                let target = self.advance_index();
                if target.is_some() && self.timer.status == TimerStatus::Stopped {
                    self.timer.start_at(now);
                }
                target
            }
            SessionCommand::Retreat => self.retreat_index(),
            SessionCommand::DetailNext => self.detail_index(1),
            SessionCommand::DetailPrevious => self.detail_index(-1),
            SessionCommand::Navigate {
                generation,
                section,
                detail,
                step,
            } => {
                if generation != self.generation {
                    return Err(RejectedCommand {
                        reason: SessionConflict::StaleGeneration,
                        snapshot: self.snapshot(now),
                    });
                }
                let Some(target) = self.route_index(section, detail, step) else {
                    return Err(RejectedCommand {
                        reason: SessionConflict::InvalidRoute,
                        snapshot: self.snapshot(now),
                    });
                };
                Some(target)
            }
            SessionCommand::TimerStart => {
                self.timer.start_at(now);
                None
            }
            SessionCommand::TimerPause => {
                self.timer.pause_at(now);
                None
            }
            SessionCommand::TimerReset => {
                self.timer.reset();
                None
            }
        };
        if let Some(target) = target {
            self.current = target;
        }
        self.revision = self.revision.saturating_add(1);
        Ok(AcceptedCommand {
            state_changed: self.current != before_current
                || self.timer.snapshot(now) != before_timer,
            revision: self.revision,
        })
    }

    pub(crate) fn snapshot(&self, now: MonotonicNow) -> SessionSnapshot {
        SessionSnapshot {
            generation: self.generation,
            revision: self.revision,
            current: self.current_route().cloned(),
            next: self.next_route().cloned(),
            timer: self.timer.snapshot(now),
        }
    }

    #[cfg(test)]
    pub(crate) fn set_current_index_for_test(&mut self, current: usize) {
        self.current = current;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn route(
        section: usize,
        detail: usize,
        step: usize,
        count: usize,
        id: &str,
    ) -> PlannedScreenState {
        PlannedScreenState {
            section,
            detail,
            step,
            step_count: count,
            slide_id: id.into(),
            role: if detail == 0 { "main" } else { "detail" },
            title: Some(id.into()),
            generated: None,
        }
    }

    #[test]
    fn navigation_timer_and_accepted_no_ops_have_one_revision_policy() {
        let mut session = PresentationSession::default();
        session.install_routes(vec![
            route(0, 0, 0, 1, "one"),
            route(0, 0, 1, 1, "one"),
            route(1, 0, 0, 0, "two"),
        ]);
        let no_op = session
            .apply(SessionCommand::Retreat, Duration::ZERO)
            .unwrap();
        assert!(!no_op.state_changed);
        assert_eq!(no_op.revision, 2);
        session
            .apply(SessionCommand::Advance, Duration::from_secs(5))
            .unwrap();
        assert_eq!(
            session.snapshot(Duration::from_secs(17)).timer.elapsed,
            Duration::from_secs(12)
        );
        session
            .apply(SessionCommand::TimerPause, Duration::from_secs(17))
            .unwrap();
        session
            .apply(SessionCommand::TimerStart, Duration::from_secs(30))
            .unwrap();
        assert_eq!(
            session.snapshot(Duration::from_secs(35)).timer.elapsed,
            Duration::from_secs(17)
        );
        session
            .apply(SessionCommand::TimerReset, Duration::from_secs(40))
            .unwrap();
        assert_eq!(
            session.snapshot(Duration::from_secs(40)).timer.elapsed,
            Duration::ZERO
        );
    }

    #[test]
    fn rebuild_preserves_stable_identity_then_clamps_sparse_coordinates() {
        let mut session = PresentationSession::default();
        session.install_routes(
            (0..=150)
                .map(|step| route(0, 2, step, 150, "stable"))
                .collect::<Vec<_>>(),
        );
        session.set_current_index_for_test(150);
        session
            .apply(SessionCommand::TimerStart, Duration::from_secs(7))
            .unwrap();
        session.install_routes(
            (0..=2)
                .map(|step| route(4, 7, step, 2, "stable"))
                .collect::<Vec<_>>(),
        );
        let preserved = session.current_route().unwrap();
        assert_eq!(
            (preserved.section, preserved.detail, preserved.step),
            (4, 7, 2)
        );
        assert_eq!(
            session.snapshot(Duration::from_secs(10)).timer,
            TimerSnapshot {
                status: TimerStatus::Running,
                elapsed: Duration::from_secs(3),
            }
        );
        session.install_routes(vec![
            route(5, 7, 0, 9, "later"),
            route(5, 7, 4, 9, "later"),
            route(1, 0, 0, 0, "earlier"),
        ]);
        let clamped = session.current_route().unwrap();
        assert_eq!((clamped.section, clamped.detail, clamped.step), (5, 7, 4));
        session.install_routes(Vec::new());
        assert!(session.current_route().is_none());
        assert_eq!(session.timer_status(), TimerStatus::Running);
    }

    #[test]
    fn steps_main_groups_and_explicit_details_keep_navigation_semantics() {
        let mut session = PresentationSession::default();
        session.install_routes(vec![
            route(0, 0, 0, 2, "main-1"),
            route(0, 0, 1, 2, "main-1"),
            route(0, 0, 2, 2, "main-1"),
            route(0, 1, 0, 1, "detail-1"),
            route(0, 1, 1, 1, "detail-1"),
            route(1, 0, 0, 0, "main-2"),
        ]);
        session
            .apply(SessionCommand::Advance, Duration::ZERO)
            .unwrap();
        assert_eq!(session.snapshot(Duration::ZERO).current.unwrap().step, 1);
        session
            .apply(SessionCommand::Advance, Duration::ZERO)
            .unwrap();
        assert_eq!(
            session.snapshot(Duration::ZERO).next.unwrap().slide_id,
            "main-2"
        );
        session
            .apply(SessionCommand::DetailNext, Duration::ZERO)
            .unwrap();
        assert_eq!(session.current_route().unwrap().slide_id, "detail-1");
        session
            .apply(SessionCommand::Advance, Duration::ZERO)
            .unwrap();
        assert_eq!(session.current_route().unwrap().step, 1);
        session
            .apply(SessionCommand::Retreat, Duration::ZERO)
            .unwrap();
        assert_eq!(session.current_route().unwrap().step, 0);
    }

    #[test]
    fn absolute_navigation_reports_typed_conflicts_without_revision_changes() {
        let mut session = PresentationSession::default();
        session.install_routes(vec![route(0, 0, 0, 0, "one")]);
        assert_eq!(
            session
                .apply(
                    SessionCommand::Navigate {
                        generation: 0,
                        section: 0,
                        detail: 0,
                        step: 0
                    },
                    Duration::ZERO
                )
                .unwrap_err()
                .reason,
            SessionConflict::StaleGeneration
        );
        assert_eq!(
            session
                .apply(
                    SessionCommand::Navigate {
                        generation: 1,
                        section: 9,
                        detail: 0,
                        step: 0
                    },
                    Duration::ZERO
                )
                .unwrap_err()
                .reason,
            SessionConflict::InvalidRoute
        );
        assert_eq!(session.revision(), 1);
    }
}
