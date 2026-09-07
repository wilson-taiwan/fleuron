import { useState } from "react";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { Modal } from "./ui/Surfaces";
import { Icon } from "./ui/Icon";
import { api } from "../lib/api";
import {
  fileAccessCopy,
  parseFileError,
  type FileAccessUi,
} from "../lib/file-access";
import { matchFolderToRoster, type MatchMap, type CandidateFile } from "../lib/transcript-match";
import { resolveInterviewForImport } from "./JoinStudyModal";
import { useProjectStore } from "../store/project-store";
import { useSyncStore } from "../store/sync-store";
import type { InterviewRosterEntry } from "../lib/types";

interface TranscriptLinkPanelProps {
  open: boolean;
  onClose: () => void;
  /** Optional pre-filtered list of missing transcripts. */
  rosterOverride?: InterviewRosterEntry[];
}

export function TranscriptLinkPanel({
  open,
  onClose,
  rosterOverride,
}: TranscriptLinkPanelProps) {
  const interviews = useProjectStore((s) => s.interviews);
  const importVtt = useProjectStore((s) => s.importVtt);
  const selectInterview = useProjectStore((s) => s.selectInterview);
  const createInterview = useProjectStore((s) => s.createInterview);
  const loadInterviews = useProjectStore((s) => s.loadInterviews);
  const mergeSameSpeaker = useProjectStore((s) => s.project?.merge_same_speaker ?? true);

  const [folderPath, setFolderPath] = useState<string | null>(null);
  const [scanning, setScanning] = useState(false);
  const [matchMap, setMatchMap] = useState<MatchMap>(new Map());
  const [importing, setImporting] = useState<string | null>(null);
  const [importingAll, setImportingAll] = useState(false);
  const [imported, setImported] = useState<Set<string>>(new Set());
  const [error, setError] = useState<string | null>(null);
  const [scanFailure, setScanFailure] = useState<FileAccessUi | null>(null);

  // Derive the target roster
  const unlinkedInterviews = interviews.filter((i) => i.segment_count === 0);
  const targetRoster: InterviewRosterEntry[] = rosterOverride || unlinkedInterviews.map((iv) => ({
    id: iv.id,
    project_id: "",
    study_label: iv.participant_label,
    segment_count: iv.remote_segment_count ?? 0,
    content_hash: null,
    revision: 0,
    deleted: false,
    updated_at: null,
  }));

  async function handlePickFolder() {
    setScanFailure(null);
    try {
      const picked = await openDialog({
        directory: true,
        multiple: false,
        title: "Choose folder containing transcripts",
      });
      if (!picked || typeof picked !== "string") return;

      setFolderPath(picked);
      setScanning(true);

      const files = await api.scanTranscriptFolder(picked);
      const candidates: CandidateFile[] = files.map((f) => ({
        path: f.path,
        name: f.name,
        rawText: f.raw_text,
      }));

      const matches = await matchFolderToRoster(candidates, targetRoster, {
        mergeSameSpeaker,
      });
      setMatchMap(matches);
    } catch (e) {
      // Classified failure → neutral recovery card; raw strings stay out of
      // the panel and never look like the app is blaming the user's setup.
      const ui =
        parseFileError(e) ??
        ({
          category: "permission_denied",
          message: "Fleuron could not read that folder.",
          detail: e instanceof Error ? e.message : String(e),
        } satisfies FileAccessUi);
      setScanFailure(ui);
    } finally {
      setScanning(false);
    }
  }

  async function importSingle(label: string, filePath: string) {
    setError(null);
    setImporting(label);
    try {
      const iv = await resolveInterviewForImport(label, createInterview);
      await selectInterview(iv.id);
      await importVtt(filePath);
      await loadInterviews();
      setImported((prev) => new Set([...prev, label]));
      void useSyncStore.getState().refreshStatus();
    } catch (e) {
      setError(`Failed to import ${label}: ${e instanceof Error ? e.message : String(e)}`);
    } finally {
      setImporting(null);
    }
  }

  async function pickSingleFile(label: string) {
    setError(null);
    try {
      const file = await openDialog({
        multiple: false,
        filters: [
          {
            name: "Transcripts",
            extensions: ["vtt", "srt", "txt", "md", "csv", "tsv", "docx"],
          },
          { name: "All files", extensions: ["*"] },
        ],
        title: `Choose transcript file for ${label}`,
      });
      if (!file || typeof file !== "string") return;
      await importSingle(label, file);
    } catch (e) {
      setError(`Failed to pick file for ${label}: ${e instanceof Error ? e.message : String(e)}`);
    }
  }

  async function handleImportAllMatched() {
    if (importingAll) return;
    setError(null);
    setImportingAll(true);
    try {
      for (const r of targetRoster) {
        if (imported.has(r.study_label)) continue;
        const match = matchMap.get(r.study_label);
        if (match && match.file && match.confidence !== "unmatched") {
          const iv = await resolveInterviewForImport(r.study_label, createInterview);
          await selectInterview(iv.id);
          await importVtt(match.file.path);
          setImported((prev) => new Set([...prev, r.study_label]));
        }
      }
      await loadInterviews();
      void useSyncStore.getState().refreshStatus();
    } catch (e) {
      setError(`Error importing matched transcripts: ${e instanceof Error ? e.message : String(e)}`);
    } finally {
      setImportingAll(false);
    }
  }

  const matchedCount = Array.from(matchMap.values()).filter(
    (m) => m.file && m.confidence !== "unmatched" && !imported.has(m.label),
  ).length;

  const remainingNeeded = targetRoster.filter((r) => !imported.has(r.study_label)).length;

  return (
    <Modal
      open={open}
      onClose={onClose}
      title="Link transcripts"
      subtitle={
        remainingNeeded > 0
          ? `${remainingNeeded} participant${remainingNeeded === 1 ? "" : "s"} waiting for transcripts on this computer.`
          : "All transcripts are linked on this computer."
      }
      footer={
        <>
          <button type="button" onClick={onClose} className="btn btn-ghost">
            {remainingNeeded === 0 ? "Done" : "Close"}
          </button>
          {matchedCount > 0 && (
            <button
              type="button"
              onClick={handleImportAllMatched}
              disabled={importingAll || !!importing}
              className="btn btn-primary"
            >
              <Icon name="import" size={14} />
              {importingAll ? "Importing…" : `Import all matched (${matchedCount})`}
            </button>
          )}
        </>
      }
    >
      <div className="flex flex-col gap-4">
        <p className="text-[12.5px] leading-relaxed" style={{ color: "var(--ink-2)" }}>
          Linking simply means pointing Fleuron at the <code>.docx</code> or <code>.vtt</code> transcript file on this computer that corresponds to each Participant ID.
        </p>

        {/* Persistent disclosure — visible before the picker opens, no modal,
            no extra confirmation. The picker itself is the consent. */}
        <p className="text-[12px] leading-relaxed" style={{ color: "var(--ink-2)" }}>
          Fleuron reads supported transcript files directly inside the folder
          you choose (not subfolders) to match labels and content hashes.
          Files are processed on this computer; transcript text is not
          uploaded.
        </p>

        {/* Top Folder Scan Bar */}
        <div className="flex items-center justify-between rounded-lg border border-[var(--border)] bg-[var(--surface)] p-3">
          <div className="min-w-0 flex-1 pr-2">
            <div className="text-[13px] font-medium text-[var(--ink-1)]">
              Scan a folder of transcripts
            </div>
            <div className="truncate text-[11.5px] text-[var(--ink-3)]">
              {folderPath || "Match files automatically by content hash and Participant ID"}
            </div>
          </div>
          <button
            type="button"
            onClick={handlePickFolder}
            disabled={scanning || importingAll}
            className="btn btn-secondary shrink-0 gap-1.5"
          >
            <Icon name="folder" size={14} />
            {scanning ? "Scanning…" : "Choose folder…"}
          </button>
        </div>

        {scanFailure && (
          <div role="alert" className="rounded-lg border border-[var(--border)] bg-[var(--surface)] p-3">
            <p className="text-[12.5px] font-medium">{fileAccessCopy(scanFailure).title}</p>
            <p className="mt-1 text-[12px]" style={{ color: "var(--ink-2)" }}>
              {fileAccessCopy(scanFailure).recovery}
            </p>
            <div className="mt-2 flex items-center gap-2">
              <button
                type="button"
                className="btn btn-outline btn-sm"
                onClick={handlePickFolder}
              >
                Choose another folder
              </button>
              <a
                href="https://github.com/wilsonhyeh/fleuron/blob/main/docs/INSTALLING.md#troubleshooting-file-access"
                target="_blank"
                rel="noreferrer"
                className="btn btn-ghost btn-sm underline"
              >
                How to fix access
              </a>
            </div>
          </div>
        )}

        {error && (
          <div className="rounded-lg border border-[var(--danger)]/30 bg-[var(--danger)]/10 p-2.5 text-[12.5px] text-[var(--danger)]">
            {error}
          </div>
        )}

        {/* Missing Transcripts List */}
        <div className="flex flex-col gap-2">
          <label className="label">Participants in this study</label>
          <div className="max-h-[320px] overflow-y-auto flex flex-col gap-1.5 pr-1">
            {targetRoster.map((r) => {
              const isImported = imported.has(r.study_label);
              const match = matchMap.get(r.study_label);
              const isCurrentImporting = importing === r.study_label;

              return (
                <div
                  key={r.study_label}
                  className="flex items-center justify-between gap-3 rounded-lg border border-[var(--border)] bg-[var(--surface)] p-2.5"
                >
                  <div className="min-w-0 flex-1">
                    <div className="flex items-center gap-2">
                      <span className="font-semibold text-[13.5px] text-[var(--ink-1)]">
                        {r.study_label}
                      </span>
                      {isImported ? (
                        <span className="flex items-center gap-1 text-[11.5px] text-[var(--ok)]">
                          <Icon name="check" size={12} /> Linked
                        </span>
                      ) : match?.confidence === "exact-hash" ? (
                        <span className="flex items-center gap-1 text-[11.5px] text-[var(--ok)]">
                          <Icon name="check" size={12} /> Matched: {match.file?.name}
                        </span>
                      ) : match?.confidence === "near-miss-filename" ? (
                        <span className="flex items-center gap-1 text-[11.5px] text-[var(--warn)]">
                          <Icon name="alert" size={12} /> Near-miss: {match.file?.name}
                        </span>
                      ) : (
                        <span className="text-[11.5px] text-[var(--ink-4)]">
                          Not linked
                        </span>
                      )}
                    </div>
                    {match?.why && !isImported && (
                      <p className="text-[11px] text-[var(--ink-3)] truncate mt-0.5">
                        {match.why}
                      </p>
                    )}
                  </div>

                  <div className="shrink-0 flex items-center gap-2">
                    {!isImported && match?.file && (
                      <button
                        type="button"
                        onClick={() => importSingle(r.study_label, match.file!.path)}
                        disabled={isCurrentImporting || importingAll}
                        className="btn btn-sm btn-primary"
                      >
                        {isCurrentImporting ? "Importing…" : "Import"}
                      </button>
                    )}
                    {!isImported && (
                      <button
                        type="button"
                        onClick={() => pickSingleFile(r.study_label)}
                        disabled={isCurrentImporting || importingAll}
                        className="btn btn-sm btn-ghost"
                        title="Pick file individually"
                      >
                        Choose file…
                      </button>
                    )}
                  </div>
                </div>
              );
            })}
          </div>
        </div>
      </div>
    </Modal>
  );
}
