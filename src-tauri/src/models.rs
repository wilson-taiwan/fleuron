use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectInfo {
    pub path: String,
    pub title: String,
    pub methodology: String,
    pub coders: Vec<String>,
    pub last_saved_by: Option<String>,
    pub last_saved_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Code {
    pub id: String,
    pub name: String,
    pub definition: Option<String>,
    pub inclusion_criteria: Option<String>,
    pub exclusion_criteria: Option<String>,
    pub example: Option<String>,
    pub parent_id: Option<String>,
    pub color: String,
    pub sort_order: i32,
    pub is_retired: bool,
    /// Distinct passages carrying this code, across every interview and coder.
    /// Counted per segment, not per row, matching `handoff_digest`.
    pub usage_count: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateCodeInput {
    pub name: String,
    pub definition: Option<String>,
    pub inclusion_criteria: Option<String>,
    pub exclusion_criteria: Option<String>,
    pub example: Option<String>,
    pub parent_id: Option<String>,
    pub color: Option<String>,
}

/// Every field is applied. The UI sends the whole code back, so an omitted
/// optional means "clear it" rather than "leave it alone" — a partial-update
/// shape would make clearing a definition impossible to express.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateCodeInput {
    pub id: String,
    pub name: String,
    pub definition: Option<String>,
    pub inclusion_criteria: Option<String>,
    pub exclusion_criteria: Option<String>,
    pub example: Option<String>,
    pub parent_id: Option<String>,
    pub color: String,
}

/// What "delete this code" should mean when passages already carry it.
///
/// Retiring is the reflexive-TA-safe default: the codebook evolves constantly,
/// and a code you stop using is part of the audit trail rather than a mistake.
/// Purging exists because genuinely mistaken codes also happen, but it is the
/// only operation here that destroys analysis, so the UI states the passage
/// count before offering it.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum DeleteCodeMode {
    /// Hide from the codebook; existing coding keeps the label.
    Retire,
    /// Strip the code from every passage, then delete it outright.
    Purge,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeleteCodeResult {
    pub mode: DeleteCodeMode,
    pub code_name: String,
    /// Passages the code was stripped from (purge only).
    pub segments_updated: i32,
    /// Coded-segment rows removed because the code was their last one.
    pub coded_segments_removed: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Interview {
    pub id: String,
    pub participant_label: String,
    pub interview_date: Option<String>,
    pub modality: Option<String>,
    pub diagnosis_notes: Option<String>,
    pub interviewers: Vec<String>,
    pub hub_memo: Option<String>,
    pub audio_path: Option<String>,
    pub segment_count: i32,
    /// Passages the *other* coder reports for this interview, when sync has
    /// seen it. `Some(n)` with a local `segment_count` of 0 is the joining
    /// coder's normal state: the interview arrived through the roster and its
    /// transcript is still sitting in the shared folder.
    pub remote_segment_count: Option<i32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateInterviewInput {
    pub participant_label: String,
    pub interview_date: Option<String>,
    pub modality: Option<String>,
    pub diagnosis_notes: Option<String>,
    pub interviewers: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateInterviewInput {
    pub id: String,
    pub participant_label: String,
    pub interview_date: Option<String>,
    pub modality: Option<String>,
    pub diagnosis_notes: Option<String>,
}

/// What deleting an interview would destroy. Fetched before the confirm so the
/// dialog can name real numbers instead of warning in the abstract.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InterviewDeleteImpact {
    pub participant_label: String,
    pub segment_count: i32,
    pub coded_segment_count: i32,
    pub has_hub_memo: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TranscriptSegment {
    pub id: String,
    pub interview_id: String,
    pub segment_index: i32,
    pub speaker: String,
    pub timestamp_start: String,
    pub timestamp_end: Option<String>,
    pub text: String,
    pub block_id: Option<String>,
    pub section_tag: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SegmentInput {
    pub speaker: String,
    pub timestamp_start: String,
    pub timestamp_end: Option<String>,
    pub text: String,
    pub section_tag: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportSegmentsInput {
    pub interview_id: String,
    pub segments: Vec<SegmentInput>,
    pub raw_vtt_path: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SegmentSpeakerChange {
    pub segment_id: String,
    pub old_speaker: String,
    pub new_speaker: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CodedSegment {
    pub id: String,
    pub interview_id: String,
    pub segment_id: String,
    pub code_ids: Vec<String>,
    pub coder_name: String,
    pub memo: Option<String>,
    pub char_start: Option<i32>,
    pub char_end: Option<i32>,
    pub quote_text: String,
    pub block_id: Option<String>,
    pub timestamp_start: String,
    pub participant_label: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApplyCodesInput {
    pub interview_id: String,
    pub segment_id: String,
    pub code_ids: Vec<String>,
    pub coder_name: String,
    pub memo: Option<String>,
    pub char_start: Option<i32>,
    pub char_end: Option<i32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnsureCodeAndApplyInput {
    pub name: String,
    pub color: Option<String>,
    pub interview_id: String,
    pub segment_id: String,
    pub coder_name: String,
    pub char_start: Option<i32>,
    pub char_end: Option<i32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChangedCodingEdge {
    pub interview_id: String,
    pub segment_id: String,
    pub code_id: String,
    pub char_start: Option<i32>,
    pub char_end: Option<i32>,
    pub present: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnsureCodeAndApplyResult {
    pub code: Code,
    pub coded_segment: CodedSegment,
    pub created: bool,
    pub changed_edges: Vec<ChangedCodingEdge>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MutateCodingEdgeInput {
    pub interview_id: String,
    pub segment_id: String,
    pub code_id: String,
    pub coder_name: String,
    pub char_start: Option<i32>,
    pub char_end: Option<i32>,
    pub present: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MutateCodingEdgeResult {
    pub coded_segment: Option<CodedSegment>,
    pub changed_edge: ChangedCodingEdge,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PatchCodingMemoInput {
    pub coded_segment_id: String,
    pub memo: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActivityLogEntry {
    pub id: String,
    pub coder_name: String,
    pub action: String,
    pub detail: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateProjectInput {
    pub parent_dir: String,
    pub project_name: String,
    pub title: String,
    pub coders: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportConfigInput {
    pub preset: String,
    pub items: Vec<String>,
    #[serde(rename = "includeParticipantScope")]
    pub include_participant_scope: String,
    #[serde(rename = "selectedParticipantIds")]
    pub selected_participant_ids: Option<Vec<String>>,
    #[serde(rename = "includeCoderScope")]
    pub include_coder_scope: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportResult {
    pub exports_dir: String,
    pub coded_segment_count: usize,
    pub interview_file_count: usize,
    pub unresolved_conflict_count: usize,
    pub files: Vec<String>,
    pub exported_at: String,
    pub exported_by: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct WorkspaceState {
    pub active_interview_id: Option<String>,
    pub selected_segment_id: Option<String>,
    pub active_coder: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum StudyReadiness {
    #[serde(rename_all = "camelCase")]
    MissingTranscripts {
        missing_count: usize,
    },
    Unlinked,
    #[serde(rename_all = "camelCase")]
    Behind {
        behind_count: usize,
    },
    Diverged,
    Ready,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "verdict", rename_all = "snake_case")]
pub enum JoinTargetVerdict {
    AlreadySetUpHere {
        path: String,
    },
    AdoptableUnbound {
        path: String,
    },
    BoundElsewhere {
        path: String,
        suggested_name: String,
        suggested_path: String,
    },
    Occupied {
        path: String,
        suggested_name: String,
        suggested_path: String,
    },
    Available {
        suggested_name: String,
        suggested_path: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum StudyLocation {
    Reachable,
    VolumeNotMounted { volume_name: String },
    CloudNotDownloaded { provider: String },
    CloudProviderAbsent { provider: String },
    PermissionDenied,
    Gone,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum RelinkOutcome {
    ExactMatch {
        new_path: String,
        message: String,
    },
    NameMatchOnly {
        candidate_path: String,
        message: String,
    },
    NotFound,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct LeftStudy {
    pub project_id: String,
    pub title: String,
    pub group_key: String,
    pub coder_name: String,
    pub left_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct RecentProject {
    pub path: String,
    pub title: String,
    pub last_opened_at: String,
    /// The shared project id this folder is bound to, when it is in a group.
    #[serde(default)]
    pub group_id: Option<String>,
    /// The group's title, cached for offline rendering.
    #[serde(default)]
    pub group_title: Option<String>,
    /// The name this machine files coding under in that group.
    #[serde(default)]
    pub coder_name: Option<String>,
    #[serde(default)]
    pub former_group_id: Option<String>,
    #[serde(default)]
    pub former_group_title: Option<String>,
    #[serde(default)]
    pub readiness: Option<StudyReadiness>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RecordRecentProjectInput {
    pub path: String,
    pub title: String,
    #[serde(default)]
    pub group_id: Option<String>,
    #[serde(default)]
    pub group_title: Option<String>,
    #[serde(default)]
    pub coder_name: Option<String>,
    #[serde(default)]
    pub former_group_id: Option<String>,
    #[serde(default)]
    pub former_group_title: Option<String>,
    #[serde(default)]
    pub readiness: Option<StudyReadiness>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PanelWidths {
    pub codebook: u32,
    pub memos: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppPreferences {
    #[serde(default)]
    pub reopen_last_project: bool,
    #[serde(default)]
    pub signin_prompt_seen: bool,
    /// v1.2 intent-first onboarding: the user answered the local-vs-
    /// collaboration choice. `signin_prompt_seen` is kept only as a migration
    /// input — anyone who saw the old credential gate never gets re-onboarded.
    #[serde(default)]
    pub onboarding_choice_seen: bool,
    /// Quiet update checks. Default ON for fresh installs and for preference
    /// files written before this field existed; explicitly opt-out only.
    #[serde(default = "default_automatic_update_checks")]
    pub automatic_update_checks: bool,
    pub last_guide_section_id: Option<String>,
    pub panel_widths: Option<PanelWidths>,
    /// Whether the workspace next-step coach has been dismissed for good.
    #[serde(default)]
    pub coach_dismissed: bool,
    /// Merge consecutive same-speaker turns into one passage on import.
    ///
    /// Import-time only — the value is read when a transcript is parsed, and
    /// re-importing with a different one does not re-split what is already
    /// imported. Defaulted `true` in both directions (serde for a prefs file
    /// written before the field existed, `Default` for no prefs file at
    /// all): the two paths disagreeing would mean fresh installs and upgrades
    /// quietly behaved differently.
    #[serde(default = "default_merge_same_speaker")]
    pub merge_same_speaker: bool,
    /// "light", "dark", or "system".
    ///
    /// Absent means light. The app deliberately does **not** default to the
    /// operating system's appearance: transcripts are long-form reading, and
    /// dark mode is a preference for that rather than an obviously better
    /// default. Anyone who wants it can say so, and "system" is there for
    /// people who want the machine to decide.
    #[serde(default)]
    pub theme: Option<String>,
    /// Which coder this machine is, per project path.
    ///
    /// Identity belongs to the person at the keyboard, not to the project, so
    /// it lives in app data and never travels in a handoff bundle. Keyed by
    /// path because one person can be "Ada Lovelace" in one study and "A.L." in
    /// another, and because a shared Mac genuinely has two different answers.
    #[serde(default)]
    pub coder_identities: std::collections::HashMap<String, String>,
    /// Sync server for this installation.
    ///
    /// Per machine rather than per project, and deliberately not inside
    /// `project.db`: where this laptop syncs is a property of the laptop, not
    /// of the study. The anon key is
    /// safe here — it is designed to ship inside clients, and row-level
    /// security, not the key's secrecy, is what protects the data.
    #[serde(default)]
    pub sync_url: Option<String>,
    #[serde(default)]
    pub sync_anon_key: Option<String>,
    /// Passage-text zoom as a scale factor (1.0 = 100%). Null means default.
    /// Local-only reading preference: never synced, never exported.
    #[serde(default)]
    pub transcript_zoom: Option<f64>,
    /// Whether the left codebook rail is collapsed. Local-only, like widths.
    #[serde(default)]
    pub codebook_collapsed: bool,
    /// Per-interview speaker-redaction toggle, keyed by interview id.
    /// Local-only view/export layer: the real names stay stored, and this map
    /// never enters a sync payload (it lives in app preferences, which have
    /// no sync path at all).
    #[serde(default)]
    pub speaker_redaction: std::collections::HashMap<String, bool>,
}

fn default_merge_same_speaker() -> bool {
    true
}

fn default_automatic_update_checks() -> bool {
    true
}

/// Hand-written rather than derived: the derive would give
/// `merge_same_speaker: false`, contradicting the serde default above for the
/// no-preferences-file case. Every other field's falsey default is intended.
impl Default for AppPreferences {
    fn default() -> Self {
        Self {
            reopen_last_project: false,
            signin_prompt_seen: false,
            onboarding_choice_seen: false,
            automatic_update_checks: true,
            last_guide_section_id: None,
            panel_widths: None,
            coach_dismissed: false,
            theme: None,
            merge_same_speaker: true,
            coder_identities: Default::default(),
            sync_url: None,
            sync_anon_key: None,
            transcript_zoom: None,
            codebook_collapsed: false,
            speaker_redaction: Default::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppVersionInfo {
    pub name: String,
    pub version: String,
    pub copyright: Option<String>,
    /// Full commit this binary was built from. "development" is the explicit
    /// fallback in non-CI builds — never a fake SHA.
    #[serde(default)]
    pub build_commit: Option<String>,
    #[serde(default)]
    pub source_url: Option<String>,
    #[serde(default)]
    pub release_url: Option<String>,
    #[serde(default)]
    pub install_guide_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectFileEntry {
    pub path: String,
    pub relative_path: String,
    pub name: String,
    pub size: u64,
    pub modified_at: Option<String>,
    pub kind: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClearWorkspaceInput {
    pub interview_id: Option<String>,
    pub clear_hub_memos: bool,
    pub clear_activity_log: bool,
    pub coder_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClearWorkspaceResult {
    pub cleared_coded_segments: i32,
    pub cleared_block_ids: i32,
    pub cleared_hub_memos: i32,
    pub cleared_activity_log: bool,
    pub scope: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiagnosticCounts {
    pub codes: usize,
    pub interviews: usize,
    pub segments: usize,
    pub coded_segments: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiagnosticTimings {
    pub connection: u64,
    pub schema: u64,
    pub snapshot_queries: u64,
    pub total: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectOpenDiagnostics {
    pub schema_version: i32,
    pub counts: DiagnosticCounts,
    pub timings_ms: DiagnosticTimings,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectOpenSnapshot {
    pub project: ProjectInfo,
    pub codes: Vec<Code>,
    pub interviews: Vec<Interview>,
    pub workspace: WorkspaceState,
    pub active_interview_id: Option<String>,
    pub selected_segment_id: Option<String>,
    pub segments: Vec<TranscriptSegment>,
    pub coded_segments: Vec<CodedSegment>,
    pub total_coded_count: usize,
    pub recent_code_ids: Vec<String>,
    pub reviewed_segment_ids: Vec<String>,
    pub diagnostics: ProjectOpenDiagnostics,
    /// Local-study UUID this open is bound to (recovery scope). Additive:
    /// absent in snapshots built before the recovery release.
    #[serde(default)]
    pub project_key: Option<String>,
    /// Backend workspace epoch for checked note writes. Rotated on every
    /// open/replacement; writes carrying an older epoch fail closed.
    #[serde(default)]
    pub workspace_epoch: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncConflictSummary {
    pub id: String,
    pub entity_type: String,
    pub entity_id: String,
    pub field_name: String,
    pub status: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncConflictDetail {
    pub id: String,
    pub entity_type: String,
    pub entity_label: String,
    pub field_name: String,
    pub current_value: serde_json::Value,
    pub proposed_value: serde_json::Value,
    pub proposer_label: Option<String>,
    pub status: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LiveWorkspaceSyncStatus {
    pub protocol: i64,
    pub generation: Option<String>,
    pub local_sequence: i64,
    pub observed_head: i64,
    pub outbox_count: i64,
    pub blocked_count: i64,
    pub unresolved_conflict_count: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LiveWorkspaceSnapshot {
    pub project: ProjectInfo,
    pub interviews: Vec<Interview>,
    pub codes: Vec<Code>,
    pub retired_codes: Vec<Code>,
    pub active_interview_id: Option<String>,
    pub selected_segment_id: Option<String>,
    pub segments: Vec<TranscriptSegment>,
    pub coded_segments: Vec<CodedSegment>,
    pub pending_coded_count: i64,
    pub coded_count: usize,
    pub conflicts: Vec<SyncConflictSummary>,
    pub sync_status: LiveWorkspaceSyncStatus,
    pub local_revision: i64,
    pub reviewed_segment_ids: Vec<String>,
    /// Local-study UUID this open is bound to (recovery scope). Additive.
    #[serde(default)]
    pub project_key: Option<String>,
    /// Current backend workspace epoch for checked note writes. Additive.
    #[serde(default)]
    pub workspace_epoch: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CandidateSegment {
    pub index: i64,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectDeletionSummary {
    pub interview_count: usize,
    pub coded_segment_count: usize,
    pub memo_count: usize,
}

// ── Note drafts: local crash recovery + checked writes ──────────────────────
//
// Recovery drafts stay on this computer. They are not committed notes, never
// sync, and never enter exports or study backups. Field names are snake_case
// to match the existing IPC convention (see PatchCodingMemoInput).

/// One unfinished note generation, as stored in the app-local recovery DB.
/// A new `draft_id` starts a new generation: late writes carrying an old id
/// fail instead of resurrecting discarded text.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NoteDraftRecord {
    pub draft_id: String,
    pub project_key: String,
    pub kind: String,
    pub target_id: String,
    pub interview_id: String,
    pub coder_name: Option<String>,
    pub base_text: String,
    pub draft_text: String,
    pub revision: i64,
    pub participant_label: String,
    pub segment_id: Option<String>,
    pub segment_index: Option<i64>,
    pub char_start: Option<i64>,
    pub char_end: Option<i64>,
    pub quote_text: Option<String>,
    pub updated_at: String,
    pub discarded: bool,
    /// Study title at list time (JOINed from recovery_projects). Absent on
    /// single-record reads; the recovery list uses it for grouping.
    #[serde(default)]
    pub project_title: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BeginNoteDraftInput {
    pub project_key: String,
    pub epoch: String,
    pub kind: String,
    pub target_id: String,
    pub interview_id: String,
    pub coder_name: Option<String>,
    pub participant_label: String,
    pub segment_id: Option<String>,
    pub segment_index: Option<i64>,
    pub char_start: Option<i64>,
    pub char_end: Option<i64>,
    pub quote_text: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "kebab-case")]
pub enum BeginNoteDraftResult {
    Active {
        // Boxed: the enum would otherwise carry a 370-byte variant next to
        // fieldless ones. Serde is transparent over Box, so the IPC JSON is
        // unchanged.
        record: Box<NoteDraftRecord>,
        committed_text: String,
    },
    MissingTarget,
    StaleWorkspace,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PutNoteDraftInput {
    pub project_key: String,
    pub epoch: String,
    pub draft_id: String,
    pub expected_revision: i64,
    pub draft_text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "kebab-case")]
pub enum PutNoteDraftResult {
    Stored { record: NoteDraftRecord },
    RevisionMismatch { record: NoteDraftRecord },
    StaleGeneration,
    StaleWorkspace,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscardNoteDraftInput {
    pub draft_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscardNoteDraftResult {
    pub draft_id: String,
    pub revision: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SaveNoteDraftInput {
    pub project_key: String,
    pub epoch: String,
    pub draft_id: String,
    pub revision: i64,
    pub kind: String,
    pub target_id: String,
    pub expected_saved_text: String,
    pub draft_text: String,
}

/// Typed commit outcome. IO failures reject the command with friendly text;
/// every content/workspace outcome below resolves normally so the UI can
/// show the truthful state (a conflict is not a crash).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "kebab-case")]
pub enum SaveNoteDraftResult {
    Saved {
        draft_id: String,
        revision: i64,
        committed_text: String,
        recovery_cleared: bool,
    },
    Conflict {
        current_text: String,
    },
    MissingTarget,
    StaleWorkspace,
    StaleGeneration,
}

impl SaveNoteDraftResult {
    pub fn ready_to_save(committed_text: String) -> Self {
        // Filled in by the caller with identity/revision once the commit and
        // the conditional recovery cleanup have both run.
        SaveNoteDraftResult::Saved {
            draft_id: String::new(),
            revision: -1,
            committed_text,
            recovery_cleared: false,
        }
    }

    pub fn conflict(current_text: String) -> Self {
        SaveNoteDraftResult::Conflict { current_text }
    }

    pub fn missing_target() -> Self {
        SaveNoteDraftResult::MissingTarget
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "kebab-case")]
pub enum ResolveNoteDraftTargetResult {
    LiveCoding {
        draft_id: String,
        interview_id: String,
        participant_label: String,
        committed_text: String,
    },
    LiveInterview {
        draft_id: String,
        participant_label: String,
        committed_text: String,
    },
    Missing {
        draft_id: String,
        reason: String,
    },
}

impl ResolveNoteDraftTargetResult {
    pub fn live_coding(
        draft_id: String,
        interview_id: String,
        participant_label: String,
        committed_text: String,
    ) -> Self {
        ResolveNoteDraftTargetResult::LiveCoding {
            draft_id,
            interview_id,
            participant_label,
            committed_text,
        }
    }

    pub fn live_interview(
        draft_id: String,
        participant_label: String,
        committed_text: String,
    ) -> Self {
        ResolveNoteDraftTargetResult::LiveInterview {
            draft_id,
            participant_label,
            committed_text,
        }
    }

    pub fn missing(draft_id: String, reason: &str) -> Self {
        ResolveNoteDraftTargetResult::Missing {
            draft_id,
            reason: reason.to_string(),
        }
    }
}

/// One-use approval minted by the frontend after its draft preflight, so the
/// native updater entry can prove it did not bypass unsaved work.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateDepartureApproval {
    pub token: String,
    pub project_key: Option<String>,
    pub epoch: Option<String>,
    pub draft_write_seq: u64,
}

/// Outcome of completing a native close/quit intent from the frontend.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DepartureCompletion {
    pub intent_id: String,
    pub replayed: String,
}

/// App-local recovery health for the draft UI surface.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecoveryStatus {
    pub available: bool,
    pub error: Option<String>,
}
