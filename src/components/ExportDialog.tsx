import { useEffect, useMemo, useState } from "react";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { useAppStore } from "../store/app-store";
import { useProjectStore } from "../store/project-store";
import { useNoteDraftStore } from "../store/note-draft-store";
import { useNoteDepartureStore } from "../store/note-departure-store";
import { api } from "../lib/api";
import {
  type ExportConfig,
  type ExportItem,
  type PresetId,
  getDefaultConfig,
  isCustom,
  setPreset,
  toggleItem,
} from "../lib/export-config";
import { Modal } from "./ui/Surfaces";
import { OptionCard } from "./setup/OptionCard";
import { Icon, type IconName } from "./ui/Icon";

function toPresetId(methodology?: string): PresetId {
  if (methodology === "content-analysis" || methodology === "framework-analysis") {
    return methodology;
  }
  return "reflexive-ta";
}

export function ExportDialog({
  open,
  onClose,
}: {
  open: boolean;
  onClose: () => void;
}) {
  const project = useProjectStore((s) => s.project);
  const codes = useProjectStore((s) => s.codes);
  const interviews = useProjectStore((s) => s.interviews);
  const codedSegments = useProjectStore((s) => s.codedSegments);
  const activeCoder = useProjectStore((s) => s.activeCoder);
  const unresolvedConflictCount = useProjectStore(
    (s) => s.liveSyncStatus?.unresolved_conflict_count ?? s.syncConflicts.length,
  );
  const exportWithConfig = useProjectStore((s) => s.exportWithConfig);
  const exporting = useProjectStore((s) => s.exporting);
  // Speaker redaction notice (T06): count of interviews with the toggle on.
  const redactedCount = useAppStore((s) =>
    interviews.filter((iv) => s.preferences.speaker_redaction?.[iv.id] ?? false)
      .length,
  );

  const [config, setConfig] = useState<ExportConfig>(() =>
    getDefaultConfig(toPresetId(project?.methodology)),
  );

  useEffect(() => {
    if (open) {
      setConfig(getDefaultConfig(toPresetId(project?.methodology)));
    }
  }, [open, project?.methodology]);

  const handleSelectPreset = (preset: PresetId) => {
    setConfig((prev) => setPreset(prev, preset));
  };

  const handleToggleItem = (item: ExportItem) => {
    setConfig((prev) => toggleItem(prev, item));
  };

  const handleScopeChange = (
    type: "participant" | "coder",
    value: string,
  ) => {
    setConfig((prev) => {
      if (type === "participant") {
        return {
          ...prev,
          includeParticipantScope: value as "all" | "selected",
          selectedParticipantIds:
            value === "selected"
              ? prev.selectedParticipantIds ?? interviews.map((iv) => iv.id)
              : undefined,
        };
      }
      return {
        ...prev,
        includeCoderScope: value as "all" | "active-coder",
      };
    });
  };

  const handleToggleParticipant = (id: string) => {
    setConfig((prev) => {
      const current = prev.selectedParticipantIds ?? interviews.map((iv) => iv.id);
      const next = current.includes(id)
        ? current.filter((x) => x !== id)
        : [...current, id];
      return {
        ...prev,
        selectedParticipantIds: next,
      };
    });
  };

  const activeInterviews = useMemo(() => {
    if (
      config.includeParticipantScope === "selected" &&
      config.selectedParticipantIds
    ) {
      return interviews.filter((iv) =>
        config.selectedParticipantIds?.includes(iv.id),
      );
    }
    return interviews;
  }, [interviews, config.includeParticipantScope, config.selectedParticipantIds]);

  const activeSegments = useMemo(() => {
    let list = codedSegments.filter((cs) =>
      activeInterviews.some((iv) => iv.id === cs.interview_id),
    );
    if (config.includeCoderScope === "active-coder" && activeCoder) {
      list = list.filter((cs) => cs.coder_name === activeCoder);
    }
    return list;
  }, [codedSegments, activeInterviews, config.includeCoderScope, activeCoder]);

  const previewFiles = useMemo(() => {
    const files: { name: string; desc: string; icon: IconName }[] = [];
    if (config.items.includes("report-html")) {
      files.push({
        name: "report.html",
        desc: "Readable in a browser and ready to print",
        icon: "note",
      });
    }
    if (config.items.includes("report-pdf")) {
      files.push({
        name: "report.pdf",
        desc: "Formatted PDF ready to share or print",
        icon: "note",
      });
    }
    if (config.items.includes("coded-segments")) {
      files.push({
        name: "coded-segments.csv",
        desc: "Spreadsheet of coded passages",
        icon: "code",
      });
    }
    if (config.items.includes("framework-matrix")) {
      files.push({
        name: "framework-matrix.csv",
        desc: "Case-by-code grid with illustrative snippets",
        icon: "layers",
      });
    }
    files.push({
      name: "export-manifest.json",
      desc: "Audit manifest with export metadata & checksums",
      icon: "note",
    });
    return files;
  }, [config.items]);

  // Final preflight: recheck at Export to Folder when draft revisions changed
  // while the options were open. The opening preflight already settled
  // drafts; a quiet dialog must not re-prompt when nothing changed.
  const [openedDirtySignature, setOpenedDirtySignature] = useState<string>("");
  const [orphanExcludedCount, setOrphanExcludedCount] = useState(0);
  useEffect(() => {
    if (!open) return;
    const drafts = useNoteDraftStore.getState();
    const signature = drafts
      .dirtyEntries()
      .map((e) => `${e.key}:${e.revision}`)
      .sort()
      .join("|");
    setOpenedDirtySignature(signature);
    void api
      .listNoteDrafts()
      .then((listed) => {
        setOrphanExcludedCount(listed.filter((d) => d.draft_text !== d.base_text).length);
      })
      .catch(() => setOrphanExcludedCount(0));
  }, [open ]);

  const handleExport = async () => {
    try {
      const drafts = useNoteDraftStore.getState();
      const currentSignature = drafts
        .dirtyEntries()
        .map((e) => `${e.key}:${e.revision}`)
        .sort()
        .join("|");
      if (currentSignature !== openedDirtySignature) {
        // New edits landed while the options were open: settle them first
        // instead of exporting a mix of saved and half-typed notes.
        const gate = await useNoteDepartureStore.getState().requestDeparture("export");
        if (!gate.proceed) return;
        setOpenedDirtySignature(
          useNoteDraftStore
            .getState()
            .dirtyEntries()
            .map((e) => `${e.key}:${e.revision}`)
            .sort()
            .join("|"),
        );
      }
      const selected = await openDialog({
        directory: true,
        multiple: false,
        title: "Choose Export Destination",
      });
      // Cancelling the folder picker keeps drafts and editing state intact.
      if (!selected || typeof selected !== "string") return;

      const result = await exportWithConfig(selected, config);
      if (result) {
        onClose();
      }
    } catch (e) {
      console.error("Export destination picker failed:", e);
    }
  };

  const custom = isCustom(config);

  return (
    <Modal
      open={open}
      onClose={onClose}
      title="Export Study Data & Reports"
      subtitle={project?.title ? `Study: ${project.title}` : "Select format and destination"}
      width="max-w-4xl"
      footer={
        <>
          <button
            type="button"
            onClick={onClose}
            disabled={exporting}
            className="btn btn-ghost"
          >
            Cancel
          </button>
          <button
            type="button"
            onClick={handleExport}
            disabled={exporting || config.items.length === 0}
            className="btn btn-primary flex items-center gap-1.5"
          >
            <Icon name="export" size={15} />
            {exporting ? "Exporting…" : "Export to Folder…"}
          </button>
        </>
      }
    >
      {unresolvedConflictCount > 0 && (
        <div
          className="mb-5 rounded-lg border px-3 py-2 text-[13px]"
          style={{ borderColor: "var(--warning)", background: "var(--warn-soft)" }}
        >
          <strong>{unresolvedConflictCount} unresolved sync conflict{unresolvedConflictCount === 1 ? "" : "s"}.</strong>{" "}
          This export uses the current canonical values; pending proposals are not merged into analysis.
        </div>
      )}
      {orphanExcludedCount > 0 && (
        <p className="hint mb-5 text-[12.5px]">
          {orphanExcludedCount === 1
            ? "1 recovered draft has no coding and will remain in Unfinished notes; it is excluded from this export."
            : `${orphanExcludedCount} recovered drafts have no coding and will remain in Unfinished notes; they are excluded from this export.`}
        </p>
      )}
      {redactedCount > 0 && (
        <p className="hint mb-5 text-[12.5px]">
          Speaker names are redacted (Speaker 1, Speaker 2) for{" "}
          {redactedCount} interview{redactedCount === 1 ? "" : "s"} — see
          Interview settings to change this.
        </p>
      )}
      <div className="grid grid-cols-1 gap-6 md:grid-cols-12">
        {/* Left Column: Preset & Checkbox Options */}
        <div className="space-y-5 md:col-span-7">
          <div>
            <div className="text-[13px] font-medium text-[var(--ink-2)] mb-2 uppercase tracking-wider">
              Methodology Presets
            </div>
            <div className="space-y-2">
              <OptionCard
                selected={!custom && config.preset === "reflexive-ta"}
                onSelect={() => handleSelectPreset("reflexive-ta")}
                icon="book"
                title="Reflexive Thematic Analysis"
                blurb="Qualitative synthesis emphasizing patterns of meaning. Omits counts by default to avoid quantitative misinterpretation."
              />
              <OptionCard
                selected={!custom && config.preset === "content-analysis"}
                onSelect={() => handleSelectPreset("content-analysis")}
                icon="layers"
                title="Qualitative Content Analysis"
                blurb="Systematic categorization including coding frequencies and corpus breadth tables."
              />
              <OptionCard
                selected={!custom && config.preset === "framework-analysis"}
                onSelect={() => handleSelectPreset("framework-analysis")}
                icon="folder"
                title="Framework Analysis"
                blurb="Structured case-by-code thematic grid matrix with sub-code rollup and key quote extracts."
              />
            </div>
          </div>

          <div>
            <div className="text-[13px] font-medium text-[var(--ink-2)] mb-2 uppercase tracking-wider flex items-center justify-between">
              <span>Included Content</span>
              {custom && (
                <span className="text-[11px] font-normal text-[var(--accent-ink)] bg-[var(--accent-soft)] px-2 py-0.5 rounded-full">
                  Custom Configuration
                </span>
              )}
            </div>

            <div className="rounded-xl border border-[var(--g-rim)] bg-[var(--fill)] p-3.5 space-y-3">
              <div className="text-[12px] font-semibold text-[var(--ink-3)] uppercase tracking-wide">
                Report Sections (HTML / PDF)
              </div>
              <label className="flex items-center gap-2.5 text-[13.5px] cursor-pointer">
                <input
                  type="checkbox"
                  checked={config.items.includes("report-html")}
                  onChange={() => handleToggleItem("report-html")}
                  className="rounded text-[var(--accent)]"
                />
                <span className="font-medium">Self-contained HTML Report</span>
              </label>

              {config.items.includes("report-html") && (
                <div className="ml-6 space-y-2 border-l-2 border-[var(--g-rim)] pl-3">
                  <label className="flex items-center gap-2 text-[13px] cursor-pointer">
                    <input
                      type="checkbox"
                      checked={config.items.includes("codebook")}
                      onChange={() => handleToggleItem("codebook")}
                      className="rounded text-[var(--accent)]"
                    />
                    <span>Codebook hierarchy &amp; definitions</span>
                  </label>
                  <label className="flex items-center gap-2 text-[13px] cursor-pointer">
                    <input
                      type="checkbox"
                      checked={config.items.includes("memos")}
                      onChange={() => handleToggleItem("memos")}
                      className="rounded text-[var(--accent)]"
                    />
                    <span>Interview and passage notes</span>
                  </label>
                  <label className="flex items-center gap-2 text-[13px] cursor-pointer">
                    <input
                      type="checkbox"
                      checked={config.items.includes("counts")}
                      onChange={() => handleToggleItem("counts")}
                      className="rounded text-[var(--accent)]"
                    />
                    <span>Coding frequencies &amp; corpus breadth</span>
                  </label>
                </div>
              )}

              <div className="text-[12px] font-semibold text-[var(--ink-3)] uppercase tracking-wide pt-2">
                Data Tables (CSV)
              </div>
              <label className="flex items-center gap-2.5 text-[13.5px] cursor-pointer">
                <input
                  type="checkbox"
                  checked={config.items.includes("coded-segments")}
                  onChange={() => handleToggleItem("coded-segments")}
                  className="rounded text-[var(--accent)]"
                />
                <span className="font-medium">Coded passages spreadsheet (CSV)</span>
              </label>
              <label className="flex items-center gap-2.5 text-[13.5px] cursor-pointer">
                <input
                  type="checkbox"
                  checked={config.items.includes("framework-matrix")}
                  onChange={() => handleToggleItem("framework-matrix")}
                  className="rounded text-[var(--accent)]"
                />
                <span className="font-medium">Framework Analysis Matrix CSV (Case × Code)</span>
              </label>
            </div>

            {config.preset === "reflexive-ta" && !config.items.includes("counts") && (
              <p className="mt-2 text-[12px] text-[var(--ink-3)] italic leading-relaxed">
                Note (Braun &amp; Clarke): Code-frequency tables are omitted by default in this Reflexive Thematic Analysis preset to avoid implying quantitative significance. Overview and extract totals still appear.
              </p>
            )}
          </div>

          {/* Scope selection */}
          <div className="space-y-3 pt-1">
            <div className="text-[13px] font-medium text-[var(--ink-2)] uppercase tracking-wider">
              Participant Scope
            </div>
            <div className="flex gap-4 text-[13.5px]">
              <label className="flex items-center gap-2 cursor-pointer">
                <input
                  type="radio"
                  name="participantScope"
                  checked={config.includeParticipantScope === "all"}
                  onChange={() => handleScopeChange("participant", "all")}
                />
                <span>All Participants ({interviews.length})</span>
              </label>
              <label className="flex items-center gap-2 cursor-pointer">
                <input
                  type="radio"
                  name="participantScope"
                  checked={config.includeParticipantScope === "selected"}
                  onChange={() => handleScopeChange("participant", "selected")}
                />
                <span>Selected Participants…</span>
              </label>
            </div>

            {config.includeParticipantScope === "selected" && (
              <div className="max-h-32 overflow-y-auto rounded-lg border border-[var(--g-rim)] bg-[var(--fill)] p-2 space-y-1.5 text-[12.5px]">
                {interviews.map((iv) => (
                  <label key={iv.id} className="flex items-center gap-2 cursor-pointer">
                    <input
                      type="checkbox"
                      checked={config.selectedParticipantIds?.includes(iv.id) ?? true}
                      onChange={() => handleToggleParticipant(iv.id)}
                    />
                    <span>
                      {iv.participant_label} {iv.interview_date ? `(${iv.interview_date})` : ""}
                    </span>
                  </label>
                ))}
              </div>
            )}
          </div>
        </div>

        {/* Right Column: Live Preview & File Output */}
        <div className="space-y-5 md:col-span-5">
          <div className="text-[13px] font-medium text-[var(--ink-2)] mb-2 uppercase tracking-wider">
            Output Preview
          </div>

          <div className="rounded-2xl border border-[var(--g-rim)] bg-[var(--fill)] p-4 space-y-4">
            <div className="text-[12px] font-semibold text-[var(--ink-3)] uppercase tracking-wide">
              Files to be Generated
            </div>
            <div className="space-y-2.5">
              {previewFiles.map((file) => (
                <div
                  key={file.name}
                  className="rounded-xl border border-[var(--g-rim)] bg-[var(--surface-card)] p-2.5 flex items-start gap-2.5"
                >
                  <span className="mt-0.5 text-[var(--ink-2)]">
                    <Icon name={file.icon} size={15} />
                  </span>
                  <div className="min-w-0 flex-1">
                    <div className="font-mono text-[12.5px] font-semibold text-[var(--ink-1)]">
                      {file.name}
                    </div>
                    <div className="text-[11.5px] text-[var(--ink-3)] leading-tight mt-0.5">
                      {file.desc}
                    </div>
                  </div>
                </div>
              ))}
            </div>

            <div className="divider" />

            <div className="text-[12px] font-semibold text-[var(--ink-3)] uppercase tracking-wide">
              Scope Summary
            </div>
            <div className="grid grid-cols-2 gap-2 text-[12px]">
              <div className="rounded-lg bg-[var(--surface)] p-2">
                <div className="text-[var(--ink-3)]">Participants</div>
                <div className="text-[14px] font-semibold text-[var(--ink-1)]">
                  {activeInterviews.length} / {interviews.length}
                </div>
              </div>
              <div className="rounded-lg bg-[var(--surface)] p-2">
                <div className="text-[var(--ink-3)]">Coded Extracts</div>
                <div className="text-[14px] font-semibold text-[var(--ink-1)]">
                  {activeSegments.length}
                </div>
              </div>
              <div className="rounded-lg bg-[var(--surface)] p-2">
                <div className="text-[var(--ink-3)]">Active Codes</div>
                <div className="text-[14px] font-semibold text-[var(--ink-1)]">
                  {codes.filter((c) => !c.is_retired).length}
                </div>
              </div>
              <div className="rounded-lg bg-[var(--surface)] p-2">
                <div className="text-[var(--ink-3)]">Exported By</div>
                <div className="text-[13px] font-semibold text-[var(--ink-1)] truncate">
                  {activeCoder || "Anonymous"}
                </div>
              </div>
            </div>
          </div>
        </div>
      </div>
    </Modal>
  );
}
