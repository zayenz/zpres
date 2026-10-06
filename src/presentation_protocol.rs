use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::presentation_plan::PlannedScreenState;
use crate::presentation_session::{SessionCommand, SessionSnapshot, TimerStatus};
use crate::speaker_notes::SpeakerNotesCatalog;

pub(crate) const PROTOCOL_VERSION_V1: u8 = 1;
pub(crate) const MAX_CONTROL_BODY: usize = 64 * 1024;

#[derive(Debug, Deserialize)]
#[serde(tag = "command", rename_all = "kebab-case")]
enum ControlRequestV1 {
    Advance,
    Retreat,
    DetailNext,
    DetailPrevious,
    Navigate {
        generation: u64,
        section: usize,
        detail: usize,
        #[serde(default)]
        step: usize,
    },
    TimerStart,
    TimerPause,
    TimerReset,
}

pub(crate) fn decode_control_v1(body: &[u8]) -> Result<SessionCommand, serde_json::Error> {
    serde_json::from_slice::<ControlRequestV1>(body).map(|request| match request {
        ControlRequestV1::Advance => SessionCommand::Advance,
        ControlRequestV1::Retreat => SessionCommand::Retreat,
        ControlRequestV1::DetailNext => SessionCommand::DetailNext,
        ControlRequestV1::DetailPrevious => SessionCommand::DetailPrevious,
        ControlRequestV1::Navigate {
            generation,
            section,
            detail,
            step,
        } => SessionCommand::Navigate {
            generation,
            section,
            detail,
            step,
        },
        ControlRequestV1::TimerStart => SessionCommand::TimerStart,
        ControlRequestV1::TimerPause => SessionCommand::TimerPause,
        ControlRequestV1::TimerReset => SessionCommand::TimerReset,
    })
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
struct RouteV1 {
    section: usize,
    detail: usize,
    step: usize,
    step_count: usize,
    slide_id: String,
    role: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    title: Option<String>,
    notes_html: String,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
enum TimerStatusV1 {
    Stopped,
    Running,
    Paused,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
struct TimerV1 {
    status: TimerStatusV1,
    elapsed_ms: u64,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub(crate) struct SessionSnapshotV1 {
    protocol_version: u8,
    pub(crate) generation: u64,
    pub(crate) revision: u64,
    current: Option<RouteV1>,
    next: Option<RouteV1>,
    timer: TimerV1,
}

fn route_v1(route: PlannedScreenState, notes: &SpeakerNotesCatalog) -> RouteV1 {
    RouteV1 {
        notes_html: notes.html_for(&route.slide_id).to_string(),
        section: route.section,
        detail: route.detail,
        step: route.step,
        step_count: route.step_count,
        slide_id: route.slide_id,
        role: route.role,
        title: route.title,
    }
}

pub(crate) fn project_snapshot_v1(
    snapshot: SessionSnapshot,
    notes: &SpeakerNotesCatalog,
) -> SessionSnapshotV1 {
    SessionSnapshotV1 {
        protocol_version: PROTOCOL_VERSION_V1,
        generation: snapshot.generation,
        revision: snapshot.revision,
        current: snapshot.current.map(|route| route_v1(route, notes)),
        next: snapshot.next.map(|route| route_v1(route, notes)),
        timer: TimerV1 {
            status: match snapshot.timer.status {
                TimerStatus::Stopped => TimerStatusV1::Stopped,
                TimerStatus::Running => TimerStatusV1::Running,
                TimerStatus::Paused => TimerStatusV1::Paused,
            },
            elapsed_ms: snapshot.timer.elapsed.as_millis().min(u64::MAX as u128) as u64,
        },
    }
}

pub(crate) fn snapshot_json_v1(snapshot: &SessionSnapshotV1) -> Value {
    serde_json::to_value(snapshot).expect("protocol v1 session snapshots are serializable")
}

pub(crate) fn snapshot_string_v1(snapshot: &SessionSnapshotV1) -> String {
    // v1 was historically constructed as a serde_json::Value. Serializing the
    // explicit DTO through Value retains that exact stable map ordering.
    snapshot_json_v1(snapshot).to_string()
}

pub(crate) fn error_payload_v1(message: &str, snapshot: Option<&SessionSnapshotV1>) -> Value {
    let mut value = json!({ "error": message });
    if let Some(snapshot) = snapshot {
        value["session"] = snapshot_json_v1(snapshot);
    }
    value
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SseEventV1 {
    Diagnostics {
        html: String,
    },
    Session {
        snapshot: String,
        generation: u64,
        revision: u64,
    },
    Reload {
        generation: u64,
    },
    ThemeCss {
        generation: u64,
        href: String,
    },
    Assets {
        generation: u64,
        version: u64,
        theme_css_href: String,
    },
    RenderState {
        generation: u64,
    },
}

impl SseEventV1 {
    pub(crate) fn wire_name(&self) -> &'static str {
        match self {
            Self::Diagnostics { .. } => "diagnostics",
            Self::Session { .. } => "session",
            Self::Reload { .. } => "reload",
            Self::ThemeCss { .. } => "theme-css",
            Self::Assets { .. } => "assets",
            Self::RenderState { .. } => "render-state",
        }
    }

    pub(crate) fn wire_data(&self) -> String {
        match self {
            Self::Diagnostics { html } => json!({ "html": html }).to_string(),
            Self::Session { snapshot, .. } => snapshot.clone(),
            Self::Reload { generation } => json!({ "generation": generation }).to_string(),
            Self::ThemeCss { generation, href } => json!({
                "generation": generation,
                "href": href,
            })
            .to_string(),
            Self::Assets {
                generation,
                version,
                theme_css_href,
            } => json!({
                "generation": generation,
                "version": version,
                "themeCssHref": theme_css_href,
            })
            .to_string(),
            Self::RenderState { generation } => json!({ "generation": generation }).to_string(),
        }
    }
}

pub(crate) fn session_event_v1(snapshot: &SessionSnapshotV1) -> SseEventV1 {
    SseEventV1::Session {
        snapshot: snapshot_string_v1(snapshot),
        generation: snapshot.generation,
        revision: snapshot.revision,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    use crate::presentation_session::{SessionSnapshot, TimerSnapshot};

    #[test]
    fn v1_control_decode_accepts_extra_fields_and_preserves_golden_errors() {
        assert_eq!(
            decode_control_v1(br#"{"command":"advance","future":true}"#).unwrap(),
            SessionCommand::Advance
        );
        assert_eq!(
            decode_control_v1(br#"{"command":"navigate","generation":3,"section":2,"detail":1}"#)
                .unwrap(),
            SessionCommand::Navigate {
                generation: 3,
                section: 2,
                detail: 1,
                step: 0
            }
        );
        assert_eq!(
            decode_control_v1(br#"{"command":"unknown"}"#)
                .unwrap_err()
                .to_string(),
            "unknown variant `unknown`, expected one of `advance`, `retreat`, `detail-next`, `detail-previous`, `navigate`, `timer-start`, `timer-pause`, `timer-reset` at line 1 column 20"
        );
    }

    #[test]
    fn v1_projection_enriches_notes_without_putting_html_in_session_state() {
        let route = PlannedScreenState {
            section: 0,
            detail: 0,
            step: 0,
            step_count: 0,
            slide_id: "slide".into(),
            role: "main",
            title: Some("Title".into()),
            generated: None,
        };
        let snapshot = SessionSnapshot {
            generation: 1,
            revision: 2,
            current: Some(route),
            next: None,
            timer: TimerSnapshot {
                status: TimerStatus::Stopped,
                elapsed: Duration::ZERO,
            },
        };
        let deck = crate::deck::parse_source_text(
            "::: slide\nid: slide\n:::\n# Title\n\n::: notes\nPrivate\n:::\n",
            None,
        )
        .unwrap();
        let notes = SpeakerNotesCatalog::from_deck(&deck);
        let projected = project_snapshot_v1(snapshot, &notes);
        assert_eq!(
            snapshot_json_v1(&projected),
            json!({
                "protocol_version": 1, "generation": 1, "revision": 2,
                "current": { "section":0, "detail":0, "step":0, "step_count":0, "slide_id":"slide", "role":"main", "title":"Title", "notes_html":"<p>Private</p>" },
                "next": null, "timer": { "status":"stopped", "elapsed_ms":0 }
            })
        );
        assert_eq!(
            snapshot_string_v1(&projected),
            r#"{"current":{"detail":0,"notes_html":"<p>Private</p>","role":"main","section":0,"slide_id":"slide","step":0,"step_count":0,"title":"Title"},"generation":1,"next":null,"protocol_version":1,"revision":2,"timer":{"elapsed_ms":0,"status":"stopped"}}"#
        );
    }

    #[test]
    fn v1_sse_event_names_and_wire_bytes_are_exhaustive_and_stable() {
        let events = [
            (
                SseEventV1::Diagnostics {
                    html: "<p>bad</p>".into(),
                },
                "diagnostics",
                r#"{"html":"<p>bad</p>"}"#,
            ),
            (
                SseEventV1::Session {
                    snapshot: r#"{"generation":2,"revision":4}"#.into(),
                    generation: 2,
                    revision: 4,
                },
                "session",
                r#"{"generation":2,"revision":4}"#,
            ),
            (
                SseEventV1::Reload { generation: 3 },
                "reload",
                r#"{"generation":3}"#,
            ),
            (
                SseEventV1::ThemeCss {
                    generation: 3,
                    href: "assets/theme.css?zpres_live=3".into(),
                },
                "theme-css",
                r#"{"generation":3,"href":"assets/theme.css?zpres_live=3"}"#,
            ),
            (
                SseEventV1::Assets {
                    generation: 3,
                    version: 3,
                    theme_css_href: "assets/theme.css?zpres_live=3".into(),
                },
                "assets",
                r#"{"generation":3,"themeCssHref":"assets/theme.css?zpres_live=3","version":3}"#,
            ),
            (
                SseEventV1::RenderState { generation: 3 },
                "render-state",
                r#"{"generation":3}"#,
            ),
        ];

        for (event, name, data) in events {
            assert_eq!(event.wire_name(), name);
            assert_eq!(event.wire_data(), data);
        }
    }
}
