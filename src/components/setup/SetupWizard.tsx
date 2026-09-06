import { useEffect, useMemo, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { useAppStore } from "../../store/app-store";
import { useProjectStore } from "../../store/project-store";
import {
  CODEBOOK_TEMPLATES,
  getCodebookTemplate,
} from "../../content/codebook-templates";
import { PROJECT_EXT } from "../../lib/open-project";
import { api } from "../../lib/api";
import { Icon } from "../ui/Icon";
import { labelFromFilename } from "../../lib/transcript-parser";
import { StepRail, type StepSpec } from "./StepRail";
import { StudyLabelField } from "../StudyLabelField";
import { OptionCard } from "./OptionCard";
import { setupJournal } from "../../lib/setup-journal";

const STEPS: StepSpec[] = [
  { id: "study", title: "Study", caption: "Name it and pick a home" },
  { id: "you", title: "You", caption: "The name you file under" },
  { id: "codebook", title: "Codebook", caption: "How codes start" },
  { id: "transcript", title: "Transcript", caption: "Your first interview" },
];

type StepId = (typeof STEPS)[number]["id"];

/** "Teacher Interviews 2026" -> "teacher-interviews-2026" */
function slugify(value: string): string {
  return value
    .toLowerCase()
    .trim()
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-+|-+$/g, "")
    .slice(0, 64);
}

function basename(path: string): string {
  const parts = path.split(/[/\\]/);
  return parts[parts.length - 1] || path;
}

export function SetupWizard() {
  const showSetup = useAppStore((s) => s.showSetup);
  const closeSetup = useAppStore((s) => s.closeSetup);
  const createProject = useProjectStore((s) => s.createProject);
  const seedCodes = useProjectStore((s) => s.seedCodes);
  const createInterview = useProjectStore((s) => s.createInterview);
  const importVtt = useProjectStore((s) => s.importVtt);

  const [stepIndex, setStepIndex] = useState(0);
  const [title, setTitle] = useState("");
  const [folderTouched, setFolderTouched] = useState(false);
  const [folderName, setFolderName] = useState("");
  const [parentDir, setParentDir] = useState<string | null>(null);
  /** The app's own library — offered as the default so nobody has to choose. */
  const [libraryDir, setLibraryDir] = useState<string | null>(null);
  /** Sync service owning `parentDir`, if any. Drives an advisory, not a block. */
  const [cloudProvider, setCloudProvider] = useState<string | null>(null);
  const [yourName, setYourName] = useState("");
  const [templateId, setTemplateId] = useState("empty");
  const [vttPath, setVttPath] = useState<string | null>(null);
  const [participantLabel, setParticipantLabel] = useState("");
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  // Reset every field when the wizard is reopened — a half-filled form from a
  // cancelled run is worse than a blank one.
  useEffect(() => {
    if (!showSetup) return;
    setStepIndex(0);
    setTitle("");
    setFolderTouched(false);
    setFolderName("");
    setParentDir(null);
    setYourName("");
    setTemplateId("empty");
    setVttPath(null);
    setParticipantLabel("");
    setBusy(null);
    setError(null);

    // Default the location to the app's own library rather than making the
    // user find one. Choosing a folder was the step that let a project end up
    // inside Box, where the sync client removed its database mid-session.
    // It is still changeable — it is just no longer the first thing asked.
    let cancelled = false;
    void (async () => {
      try {
        const [lib, libWarning] = await Promise.all([
          api.getProjectsLibraryDir(),
          api.librarySyncWarning(),
        ]);
        if (cancelled) return;
        setLibraryDir(lib);
        setParentDir(lib);
        // `document_dir()` is not guaranteed local — Windows Backup redirects
        // Documents into OneDrive, macOS moves it into iCloud with Desktop &
        // Documents syncing. If the default is itself synced, say so rather
        // than recommend it silently.
        setCloudProvider(libWarning);
      } catch {
        // No library path means the user picks one, as before.
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [showSetup]);

  // Re-check whenever the location changes, including back to the default.
  useEffect(() => {
    if (!parentDir) {
      setCloudProvider(null);
      return;
    }
    let cancelled = false;
    void (async () => {
      try {
        const provider = await api.cloudProviderForPath(parentDir);
        if (!cancelled) setCloudProvider(provider);
      } catch {
        if (!cancelled) setCloudProvider(null);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [parentDir]);

  const effectiveFolder = folderTouched
    ? folderName
    : slugify(title) || "untitled-study";

  const step = STEPS[stepIndex].id as StepId;

  const canAdvance = useMemo(() => {
    if (step === "study") return title.trim().length > 0 && !!parentDir;
    if (step === "you") return yourName.trim().length > 0;
    if (step === "codebook") return true;
    if (step === "transcript") return !vttPath || participantLabel.trim().length > 0;
    return false;
  }, [step, title, parentDir, yourName, vttPath, participantLabel]);

  if (!showSetup) return null;

  async function chooseLocation() {
    const picked = await open({
      directory: true,
      multiple: false,
      title: "Where should this project live?",
    });
    if (picked) setParentDir(picked as string);
  }

  async function chooseVtt() {
    const picked = await open({
      multiple: false,
      title: "Choose a transcript",
      filters: [
        {
          name: "Transcripts",
          extensions: ["vtt", "srt", "txt", "md", "csv", "tsv", "docx"],
        },
        { name: "All files", extensions: ["*"] },
      ],
    });
    if (picked && !Array.isArray(picked)) {
      setVttPath(picked);
      if (!participantLabel.trim()) {
        setParticipantLabel(labelFromFilename(picked));
      }
    }
  }

  async function finish() {
    setError(null);
    let journal = setupJournal.getActive();
    if (!journal || journal.title !== title.trim()) {
      journal = setupJournal.start("new_local", {
        title: title.trim(),
        coderName: yourName.trim(),
        canonicalFolder: effectiveFolder,
      });
    }

    try {
      if (!journal.completedStages.includes("created_folder")) {
        setBusy("Creating project folder…");
        const ok = await createProject(
          parentDir!,
          effectiveFolder,
          title.trim(),
          [yourName.trim()],
        );
        if (!ok) {
          setError(
            useProjectStore.getState().error ?? "Could not create the project.",
          );
          setBusy(null);
          return;
        }
        journal =
          setupJournal.markStageComplete(
            journal.operationId,
            "created_folder",
            "template_initialized",
          ) || journal;
      }

      const template = getCodebookTemplate(templateId);
      if (
        template.codes.length > 0 &&
        !journal.completedStages.includes("template_initialized")
      ) {
        setBusy("Adding starter codes…");
        try {
          await seedCodes(template.codes);
          journal =
            setupJournal.markStageComplete(
              journal.operationId,
              "template_initialized",
              "transcript_imported",
            ) || journal;
        } catch {
          setError("Your study was created. Starter codes could not be added.");
          setBusy(null);
          return;
        }
      }

      if (
        vttPath &&
        !journal.completedStages.includes("transcript_imported")
      ) {
        setBusy("Importing transcript…");
        try {
          await createInterview(
            participantLabel.trim(),
            new Date().toISOString().slice(0, 10),
          );
          await importVtt(vttPath);
          journal =
            setupJournal.markStageComplete(
              journal.operationId,
              "transcript_imported",
              "completed",
            ) || journal;
        } catch {
          setError("Your study was created. This transcript could not be imported.");
          setBusy(null);
          return;
        }
      }

      setupJournal.clear(journal.operationId);
      closeSetup();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
      setBusy(null);
    }
  }

  const isLast = stepIndex === STEPS.length - 1;

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center p-6">
      <button
        type="button"
        className="scrim"
        aria-label="Cancel setup"
        onClick={() => !busy && closeSetup()}
      />
      <div
        role="dialog"
        aria-modal="true"
        aria-label="New project setup"
        className="glass-sheet anim-sheet relative flex h-[560px] w-full max-w-3xl overflow-hidden"
      >
        <StepRail steps={STEPS} activeIndex={stepIndex} />

        <div className="flex min-w-0 flex-1 flex-col">
          <div className="scroll flex-1 px-9 pb-4 pt-9">
            {step === "study" && (
              <Section
                heading="What are you studying?"
                lede="This name shows in the window and on every export. You can change it later."
              >
                <div>
                  <label className="label" htmlFor="setup-title">
                    Study title
                  </label>
                  <input
                    id="setup-title"
                    className="field"
                    value={title}
                    onChange={(e) => setTitle(e.target.value)}
                    placeholder="Your study name"
                    autoFocus
                  />
                </div>

                <div>
                  <label className="label">Location</label>
                  <button
                    type="button"
                    onClick={chooseLocation}
                    className="btn btn-outline btn-block justify-start"
                  >
                    <Icon name="folder" size={15} />
                    <span className="truncate">
                      {parentDir ?? "Choose a folder…"}
                    </span>
                  </button>
                  {parentDir === libraryDir && libraryDir && (
                    <p className="hint mt-1.5">
                      Recommended — Fleuron keeps projects here.{" "}
                      <button
                        type="button"
                        onClick={chooseLocation}
                        className="underline"
                        style={{ color: "var(--accent)" }}
                      >
                        Choose a different folder
                      </button>
                    </p>
                  )}

                  <p className="hint mt-2">
                    {parentDir ? (
                      <>
                        Creates{" "}
                        <span className="font-mono" style={{ color: "var(--ink-2)" }}>
                          {effectiveFolder}
                          {PROJECT_EXT}
                        </span>{" "}
                        here. This local working folder stays on this computer.
                        If you collaborate, coding metadata syncs to the team —
                        transcript text never does.
                      </>
                    ) : (
                      "Your study's local working folder lives here."
                    )}
                  </p>

                  {/* Advisory, not a gate. Some people deliberately keep
                      projects in the cloud for backup — that is a legitimate
                      trade and theirs to make. What is not acceptable is
                      finding out only when a database goes missing. */}
                  {cloudProvider && (
                    <div className="notice notice-warn mt-2">
                      <p>
                        This folder is in <strong>{cloudProvider}</strong>. The
                        project database is written continuously while you code,
                        and sync clients have been seen removing or replacing it
                        mid-session — that is how a project loses its coding.
                      </p>
                      <p className="mt-1.5">
                        Safer: keep studies in the default local library{" "}
                        {libraryDir ? (
                          <strong>{libraryDir}</strong>
                        ) : (
                          <strong>under your home folder</strong>
                        )}
                        , and use exports/backups for anything you want copied
                        into {cloudProvider}. Exports are plain files — cloud
                        folders are a fine home for them.
                      </p>
                      {libraryDir && parentDir !== libraryDir && (
                        <button
                          type="button"
                          onClick={() => setParentDir(libraryDir)}
                          className="btn btn-outline btn-sm mt-2"
                        >
                          Use the recommended folder
                        </button>
                      )}
                    </div>
                  )}
                </div>

                {parentDir && (
                  <details>
                    <summary
                      className="hint cursor-pointer select-none"
                      style={{ color: "var(--accent)" }}
                    >
                      Rename the folder
                    </summary>
                    <input
                      className="field field-sm mt-2"
                      value={effectiveFolder}
                      onChange={(e) => {
                        setFolderTouched(true);
                        setFolderName(slugify(e.target.value));
                      }}
                    />
                  </details>
                )}
              </Section>
            )}

            {step === "you" && (
              <Section
                heading="What's your name?"
                lede="Codes you apply are stamped with this. Colleagues name themselves when they join a group — you do not list them here."
              >
                <div>
                  <label className="label" htmlFor="setup-coder">
                    Your name
                  </label>
                  <input
                    id="setup-coder"
                    className="field"
                    value={yourName}
                    onChange={(e) => setYourName(e.target.value)}
                    placeholder="Your name"
                    autoFocus
                  />
                </div>
              </Section>
            )}

            {step === "codebook" && (
              <Section
                heading="How should the codebook start?"
                lede="You'll build the real codebook while reading. This only decides what's there on day one."
              >
                <div className="flex flex-col gap-2.5">
                  {CODEBOOK_TEMPLATES.map((tpl) => (
                    <OptionCard
                      key={tpl.id}
                      selected={templateId === tpl.id}
                      onSelect={() => setTemplateId(tpl.id)}
                      icon={tpl.id === "empty" ? "book" : "sparkle"}
                      title={tpl.label}
                      blurb={tpl.blurb}
                    >
                      {tpl.codes.length > 0 && (
                        <div className="mt-2.5 flex flex-wrap gap-1.5">
                          {tpl.codes.map((c) => (
                            <span key={c.name} className="chip">
                              {c.name}
                            </span>
                          ))}
                        </div>
                      )}
                    </OptionCard>
                  ))}
                </div>
              </Section>
            )}

            {step === "transcript" && (
              <Section
                heading="Add your first transcript"
                lede="Zoom captions, SRT, a Word document, or plain text with speaker labels. You can skip this and import later — nothing depends on doing it now."
              >
                <button
                  type="button"
                  onClick={chooseVtt}
                  className="btn btn-outline btn-block justify-start"
                >
                  <Icon name="import" size={15} />
                  <span className="truncate">
                    {vttPath ? basename(vttPath) : "Choose a transcript file…"}
                  </span>
                </button>

                {vttPath && (
                  <>
                    <div>
                      <StudyLabelField
                        id="setup-participant"
                        value={participantLabel}
                        onChange={setParticipantLabel}
                        // A brand-new study has no roster to check against; the
                        // field still explains what the label is for, which is
                        // where the mistake gets made the first time.
                        knownLabels={[]}
                      />
                    </div>
                    <button
                      type="button"
                      onClick={() => {
                        setVttPath(null);
                        setParticipantLabel("");
                      }}
                      className="btn btn-ghost btn-sm self-start"
                    >
                      Remove file
                    </button>
                  </>
                )}

                {!vttPath && (
                  <p className="hint">
                    Accepts <span className="font-mono">.vtt</span>,{" "}
                    <span className="font-mono">.srt</span>,{" "}
                    <span className="font-mono">.docx</span>,{" "}
                    <span className="font-mono">.csv</span>, and plain text with
                    speaker labels.
                  </p>
                )}
              </Section>
            )}
          </div>

          {error && (
            <div
              className="mx-9 mb-3 flex flex-col gap-2 rounded-[12px] px-3 py-2.5 text-[12.5px]"
              style={{ background: "var(--danger-soft)", color: "var(--danger)" }}
              role="alert"
            >
              <div className="flex items-start gap-2">
                <Icon name="alert" size={15} />
                <span>{error}</span>
              </div>
              {error === "Your study was created. Starter codes could not be added." && (
                <div className="flex items-center gap-2 pl-5 pt-1">
                  <button
                    type="button"
                    onClick={() => void finish()}
                    className="btn btn-outline btn-xs"
                  >
                    Retry starter codes
                  </button>
                  <button
                    type="button"
                    onClick={() => {
                      setupJournal.clear();
                      closeSetup();
                    }}
                    className="btn btn-ghost btn-xs"
                  >
                    Open study without starter codes
                  </button>
                </div>
              )}
              {error === "Your study was created. This transcript could not be imported." && (
                <div className="flex items-center gap-2 pl-5 pt-1">
                  <button
                    type="button"
                    onClick={() => void finish()}
                    className="btn btn-outline btn-xs"
                  >
                    Retry import
                  </button>
                  <button
                    type="button"
                    onClick={() => {
                      setVttPath(null);
                      setError(null);
                    }}
                    className="btn btn-outline btn-xs"
                  >
                    Choose another file
                  </button>
                  <button
                    type="button"
                    onClick={() => {
                      setupJournal.clear();
                      closeSetup();
                    }}
                    className="btn btn-ghost btn-xs"
                  >
                    Finish later
                  </button>
                </div>
              )}
            </div>
          )}

          <footer className="flex items-center justify-between gap-3 px-9 pb-7 pt-2">
            <button
              type="button"
              onClick={() => closeSetup()}
              disabled={!!busy}
              className="btn btn-ghost"
            >
              Cancel
            </button>

            <div className="flex items-center gap-2">
              {busy && <span className="hint">{busy}</span>}
              {stepIndex > 0 && !busy && (
                <button
                  type="button"
                  onClick={() => setStepIndex(stepIndex - 1)}
                  className="btn btn-outline"
                >
                  <Icon name="arrowLeft" size={14} />
                  Back
                </button>
              )}
              <button
                type="button"
                disabled={!canAdvance || !!busy}
                onClick={() => (isLast ? finish() : setStepIndex(stepIndex + 1))}
                className="btn btn-primary btn-lg"
              >
                {isLast ? "Create project" : "Continue"}
                {!isLast && <Icon name="arrowRight" size={15} />}
              </button>
            </div>
          </footer>
        </div>
      </div>
    </div>
  );
}

function Section({
  heading,
  lede,
  children,
}: {
  heading: string;
  lede: string;
  children: React.ReactNode;
}) {
  return (
    <div className="anim-rise flex flex-col gap-5" key={heading}>
      <div>
        <h2 className="text-[21px] font-semibold tracking-[-0.02em]">
          {heading}
        </h2>
        <p className="hint mt-1.5 max-w-prose text-[13px]">{lede}</p>
      </div>
      {children}
    </div>
  );
}
