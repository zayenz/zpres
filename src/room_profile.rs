use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use thiserror::Error;

pub const ROOM_PROFILE_SCHEMA_VERSION: u32 = 1;
const PROJECTED_ROOM_DEFAULT: &str = include_str!("../room-profiles/projected-room-default.toml");

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RoomProfile {
    pub schema_version: u32,
    pub name: String,
    pub status: RoomProfileStatus,
    pub logical_field: LogicalField,
    pub physical_assumptions: PhysicalAssumptions,
    pub font_metrics: FontMetrics,
    pub type_floors: TypeFloors,
    pub contrast_targets: ContrastTargets,
    pub occupancy: OccupancyTargets,
    pub autoscale: AutoscaleTargets,
    pub evidence: ProfileEvidence,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RoomProfileStatus {
    Provisional,
    Approved,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LogicalField {
    pub width: u32,
    pub height: u32,
    pub aspect: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PhysicalAssumptions {
    pub screen_geometry: String,
    pub farthest_viewing_position: String,
    pub ambient_conditions: String,
    pub display: String,
    pub machine: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FontMetrics {
    pub cap_height_proxy: f64,
    pub reference_families: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TypeFloors {
    pub display: f64,
    pub title: f64,
    pub heading: f64,
    pub body: f64,
    pub technical: f64,
    pub micro: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ContrastTargets {
    pub ordinary: f64,
    pub large: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OccupancyTargets {
    pub evidence_review_below: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AutoscaleTargets {
    pub warning_below: f64,
    pub hard_floor: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProfileEvidence {
    pub test_date: String,
    pub reviewers: Vec<String>,
    pub record: String,
    pub decision: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ResolvedRoomProfile {
    pub selection: String,
    pub decision_status: RoomProfileStatus,
    pub enforcement_status: RoomProfileEnforcementStatus,
    pub source: String,
    pub sha256: String,
    pub profile: RoomProfile,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RoomProfileEnforcementStatus {
    ReportOnly,
    BlockingV1,
}

#[derive(Debug, Error)]
pub enum RoomProfileError {
    #[error("unknown room profile '{0}'; available built-in profile: projected-room-default")]
    Unknown(String),
    #[error("cannot read room profile at {path}: {source}")]
    Read {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("invalid room profile TOML at {path}: {source}")]
    Toml {
        path: String,
        #[source]
        source: toml::de::Error,
    },
    #[error("invalid room profile '{name}': {reason}")]
    Invalid { name: String, reason: String },
}

pub fn resolve_builtin(name: &str) -> Result<ResolvedRoomProfile, RoomProfileError> {
    let source = match name {
        "projected-room-default" => PROJECTED_ROOM_DEFAULT,
        other => return Err(RoomProfileError::Unknown(other.to_string())),
    };
    resolve_source(
        source,
        "builtin:room-profiles/projected-room-default.toml".to_string(),
    )
}

pub fn resolve(selection: &str, base: &Path) -> Result<ResolvedRoomProfile, RoomProfileError> {
    if selection == "projected-room-default" {
        return resolve_builtin(selection);
    }
    let path = Path::new(selection);
    let path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        base.join(path)
    };
    let path = path.canonicalize().unwrap_or(path);
    let source = fs::read_to_string(&path).map_err(|source| RoomProfileError::Read {
        path: path.clone(),
        source,
    })?;
    resolve_source(&source, format!("file:{}", path.display()))
}

fn resolve_source(
    source: &str,
    source_name: String,
) -> Result<ResolvedRoomProfile, RoomProfileError> {
    let profile: RoomProfile = toml::from_str(source).map_err(|source| RoomProfileError::Toml {
        path: source_name.clone(),
        source,
    })?;
    validate(&profile)?;
    let sha256 = format!("{:x}", Sha256::digest(source.as_bytes()));
    Ok(ResolvedRoomProfile {
        selection: profile.name.clone(),
        decision_status: profile.status,
        enforcement_status: if profile.status == RoomProfileStatus::Approved {
            RoomProfileEnforcementStatus::BlockingV1
        } else {
            RoomProfileEnforcementStatus::ReportOnly
        },
        source: source_name,
        sha256,
        profile,
    })
}

pub fn validate(profile: &RoomProfile) -> Result<(), RoomProfileError> {
    let invalid = |reason: &str| RoomProfileError::Invalid {
        name: profile.name.clone(),
        reason: reason.to_string(),
    };
    if profile.schema_version != ROOM_PROFILE_SCHEMA_VERSION {
        return Err(invalid("schema_version must be 1"));
    }
    if profile.logical_field.width == 0 || profile.logical_field.height == 0 {
        return Err(invalid("logical field dimensions must be positive"));
    }
    let floors = &profile.type_floors;
    if ![
        floors.display,
        floors.title,
        floors.heading,
        floors.body,
        floors.technical,
        floors.micro,
    ]
    .iter()
    .all(|value| value.is_finite() && *value > 0.0)
    {
        return Err(invalid("type floors must be finite and positive"));
    }
    if !(floors.display >= floors.title
        && floors.title >= floors.heading
        && floors.heading >= floors.body
        && floors.body >= floors.technical
        && floors.technical >= floors.micro)
    {
        return Err(invalid(
            "type floors must preserve Display >= Title >= Heading >= Body >= Technical >= Micro",
        ));
    }
    if !(4.5..=21.0).contains(&profile.contrast_targets.ordinary)
        || !(3.0..=21.0).contains(&profile.contrast_targets.large)
    {
        return Err(invalid(
            "projection contrast targets must be finite, at least WCAG text minima, and at most 21:1",
        ));
    }
    if !(0.0..=1.0).contains(&profile.occupancy.evidence_review_below) {
        return Err(invalid(
            "evidence occupancy threshold must be finite and between 0 and 1",
        ));
    }
    if !(0.0 < profile.font_metrics.cap_height_proxy
        && profile.font_metrics.cap_height_proxy <= 1.0)
    {
        return Err(invalid(
            "cap-height proxy must be finite and greater than 0, up to 1",
        ));
    }
    if !(0.0 < profile.autoscale.hard_floor
        && profile.autoscale.hard_floor <= profile.autoscale.warning_below
        && profile.autoscale.warning_below <= 1.0)
    {
        return Err(invalid(
            "autoscale requires 0 < hard_floor <= warning_below <= 1",
        ));
    }
    if profile.status == RoomProfileStatus::Approved {
        let physical = &profile.physical_assumptions;
        let required = [
            physical.screen_geometry.as_str(),
            physical.farthest_viewing_position.as_str(),
            physical.ambient_conditions.as_str(),
            physical.display.as_str(),
            physical.machine.as_str(),
            profile.evidence.test_date.as_str(),
            profile.evidence.record.as_str(),
            profile.evidence.decision.as_str(),
        ];
        if required
            .iter()
            .any(|value| value.trim().is_empty() || value.contains("pending"))
            || profile.evidence.reviewers.is_empty()
        {
            return Err(invalid(
                "approved profiles require complete physical assumptions, dated evidence, reviewers, and a decision",
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn provisional_builtin_is_valid_and_report_only() {
        let resolved = resolve_builtin("projected-room-default").unwrap();
        assert_eq!(resolved.decision_status, RoomProfileStatus::Provisional);
        assert_eq!(
            resolved.enforcement_status,
            RoomProfileEnforcementStatus::ReportOnly
        );
        assert_eq!(resolved.profile.type_floors.body, 32.0);
        assert_eq!(resolved.profile.autoscale.hard_floor, 0.80);
        assert_eq!(resolved.sha256.len(), 64);
    }

    #[test]
    fn file_profile_is_resolved_relative_to_the_selected_base_and_hashed() {
        let temp = tempdir().unwrap();
        let path = temp.path().join("auditorium.toml");
        let source = PROJECTED_ROOM_DEFAULT
            .replace("name = \"projected-room-default\"", "name = \"auditorium\"");
        fs::write(&path, &source).unwrap();

        let resolved = resolve("auditorium.toml", temp.path()).unwrap();

        assert_eq!(resolved.selection, "auditorium");
        assert_eq!(
            resolved.source,
            format!("file:{}", path.canonicalize().unwrap().display())
        );
        assert_eq!(
            resolved.sha256,
            format!("{:x}", Sha256::digest(source.as_bytes()))
        );
        assert_eq!(
            resolved.enforcement_status,
            RoomProfileEnforcementStatus::ReportOnly
        );
    }

    #[test]
    fn approval_rejects_pending_physical_evidence() {
        let mut profile = resolve_builtin("projected-room-default").unwrap().profile;
        profile.status = RoomProfileStatus::Approved;
        let error = validate(&profile).unwrap_err();
        assert!(error.to_string().contains("complete physical assumptions"));
    }

    #[test]
    fn contrast_and_fraction_fields_enforce_finite_ranges() {
        for field in ["ordinary", "large", "occupancy", "cap-height"] {
            for value in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, -0.1, 21.1] {
                let mut profile = resolve_builtin("projected-room-default").unwrap().profile;
                match field {
                    "ordinary" => profile.contrast_targets.ordinary = value,
                    "large" => profile.contrast_targets.large = value,
                    "occupancy" => profile.occupancy.evidence_review_below = value,
                    "cap-height" => profile.font_metrics.cap_height_proxy = value,
                    _ => unreachable!(),
                }
                assert!(validate(&profile).is_err(), "{field}: {value}");
            }
        }
        let mut profile = resolve_builtin("projected-room-default").unwrap().profile;
        for (ordinary, large, occupancy, cap_height, valid) in [
            (4.5, 3.0, 0.0, 0.01, true),
            (21.0, 21.0, 1.0, 1.0, true),
            (4.4, 3.0, 0.5, 0.7, false),
            (4.5, 2.9, 0.5, 0.7, false),
            (4.5, 3.0, 1.01, 0.7, false),
            (4.5, 3.0, 0.5, 0.0, false),
            (4.5, 3.0, 0.5, 1.01, false),
        ] {
            profile.contrast_targets.ordinary = ordinary;
            profile.contrast_targets.large = large;
            profile.occupancy.evidence_review_below = occupancy;
            profile.font_metrics.cap_height_proxy = cap_height;
            assert_eq!(
                validate(&profile).is_ok(),
                valid,
                "{ordinary}, {large}, {occupancy}, {cap_height}"
            );
        }
    }

    #[test]
    fn ordering_and_autoscale_invariants_are_checked() {
        let mut profile = resolve_builtin("projected-room-default").unwrap().profile;
        profile.type_floors.technical = 40.0;
        assert!(
            validate(&profile)
                .unwrap_err()
                .to_string()
                .contains("Display >= Title")
        );
        profile.type_floors.technical = 24.0;
        profile.autoscale.hard_floor = 0.95;
        assert!(
            validate(&profile)
                .unwrap_err()
                .to_string()
                .contains("hard_floor")
        );
    }
}
