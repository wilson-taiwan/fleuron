import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { api } from "../lib/api";
import { auditContrast } from "../lib/contrast-audit";
import type { SegmentInput } from "../lib/types";
import { useProjectStore } from "../store/project-store";

interface SuiteResult {
  suite: string;
  passed: boolean;
  status?: "passed" | "failed" | "skipped";
  error?: string;
  duration_ms?: number;
}

/** Every synthetic study files under this coder, mirroring selftest_seed. */
const SELFTEST_CODER = "Ada Lovelace";

/** Close the study, then remove its folder so no fixture data survives. */
async function disposeStudy(path: string): Promise<void> {
  try {
    await api.closeProject();
  } catch {
    // Already closed is fine — the deletion below is what matters.
  }
  await api.deleteProjectFolder(path);
}

/** Seed a pristine disposable study (temp dir, seeded codes/interview/passage). */
async function seedFreshStudy(suite: string): Promise<string> {
  const res = await invoke<{ project_path: string }>("selftest_seed", { suite });
  return res.project_path;
}

function assert(cond: unknown, message: string): asserts cond {
  if (!cond) throw new Error(message);
}

export function SelftestRunner() {
  const [activeSuite, setActiveSuite] = useState<string>("Initializing");
  const [results, setResults] = useState<SuiteResult[]>([]);
  const [done, setDone] = useState(false);

  useEffect(() => {
    let cancelled = false;

    async function runAllSuites() {
      const suiteResults: SuiteResult[] = [];

      async function runSuite(
        name: string,
        fn: () => Promise<void>,
        options?: { allowSkip?: boolean },
      ) {
        if (cancelled) return;
        setActiveSuite(name);
        const start = performance.now();
        try {
          await fn();
          const duration_ms = Math.round(performance.now() - start);
          suiteResults.push({
            suite: name,
            passed: true,
            status: "passed",
            duration_ms,
          });
        } catch (e) {
          const duration_ms = Math.round(performance.now() - start);
          const error = e instanceof Error ? e.message : String(e);
          if (options?.allowSkip && error.startsWith("SKIP:")) {
            suiteResults.push({
              suite: name,
              passed: true,
              status: "skipped",
              error: error.slice(5).trim(),
              duration_ms,
            });
          } else {
            suiteResults.push({
              suite: name,
              passed: false,
              status: "failed",
              error,
              duration_ms,
            });
          }
        }
        setResults([...suiteResults]);
      }

      // Suite 1: bundle-freshness
      await runSuite("bundle-freshness", async () => {
        const ver = await api.getAppVersion();
        if (!ver || !ver.name) {
          throw new Error("Failed to retrieve valid app version info");
        }
      });

      // Suite 2: window-drag
      await runSuite("window-drag", async () => {
        const dragRegions = document.querySelectorAll("[data-tauri-drag-region]");
        if (dragRegions.length === 0) {
          throw new Error("No data-tauri-drag-region elements found in DOM");
        }
        const topStrip = document.querySelector('[data-tauri-drag-region="deep"]');
        if (!topStrip) {
          throw new Error("Missing data-tauri-drag-region='deep' titlebar strip");
        }
      });

      let projectDir = "";

      // Suite 3: codebook-drag-nest
      await runSuite("codebook-drag-nest", async () => {
        // Seed test project
        const seedResult = await invoke<{ project_path: string }>("selftest_seed", {
          suite: "codebook-drag-nest",
        });
        if (seedResult?.project_path) {
          projectDir = seedResult.project_path;
        }
        const codesBefore = await api.listCodes();
        const c1 = codesBefore.find((c) => c.id === "c1");
        const c2 = codesBefore.find((c) => c.id === "c2");
        if (!c1 || !c2) throw new Error("Seed codes c1 and c2 not found");

        // Reparent c2 -> c1
        await api.updateCode({ ...c2, parent_id: c1.id });
        const codesAfterNest = await api.listCodes();
        const c2Nested = codesAfterNest.find((c) => c.id === "c2");
        if (c2Nested?.parent_id !== "c1") {
          throw new Error(`Expected c2.parent_id to be 'c1', got ${c2Nested?.parent_id}`);
        }

        // Promote c2 back to top level
        await api.updateCode({ ...c2, parent_id: null });
        const codesAfterPromote = await api.listCodes();
        const c2Promoted = codesAfterPromote.find((c) => c.id === "c2");
        if (c2Promoted?.parent_id !== null) {
          throw new Error(`Expected c2.parent_id to be null, got ${c2Promoted?.parent_id}`);
        }
      });

      // Suite 4: pdf-export
      await runSuite("pdf-export", async () => {
        const tempPath = projectDir
          ? `${projectDir}/selftest_export_${Date.now()}.pdf`
          : `selftest_export_${Date.now()}.pdf`;
        const sampleHtml = `<!DOCTYPE html><html><body><h1>Selftest Qualitative Report</h1><p>Verifying WKWebView / Edge PDF export in CI.</p></body></html>`;
        await api.renderReportPdf(sampleHtml, tempPath);
      });

      // Suite 5: dark-mode-contrast
      // The audit must own its input state. As shipped, the app inherits the
      // shared profile: `data-theme="system"` follows the OS (which flips at
      // dusk on macOS), and a crashed prior run can leave the onboarding
      // wizard mounted. Measuring unsettled theme tokens — dark --ink on a
      // light-painted ground still transitioning — reports 1.1:1 garbage that
      // clears itself. Force dark, let two paints plus the token transitions
      // finish, then measure; restore whatever was there afterwards.
      await runSuite("dark-mode-contrast", async () => {
        const root = document.documentElement;
        const priorTheme = root.getAttribute("data-theme");
        root.setAttribute("data-no-transitions", "true");
        void root.offsetHeight;
        root.setAttribute("data-theme", "dark");
        void root.offsetHeight;
        await new Promise<void>((resolve) => {
          const timer = setTimeout(() => resolve(), 300);
          requestAnimationFrame(() =>
            requestAnimationFrame(() => {
              clearTimeout(timer);
              setTimeout(() => resolve(), 150);
            }),
          );
        });
        try {
          const findings = auditContrast(document);
          if (findings.length > 0) {
            throw new Error(
              `Found ${findings.length} contrast violations (expected 0): ${JSON.stringify(findings)}`,
            );
          }
        } finally {
          void root.offsetHeight;
          if (priorTheme == null) root.removeAttribute("data-theme");
          else root.setAttribute("data-theme", priorTheme);
          void root.offsetHeight;
          root.removeAttribute("data-no-transitions");
        }
      });

      // Suite 7: ipc-surface
      await runSuite("ipc-surface", async () => {
        const prefs = await api.getAppPreferences();
        if (!prefs) throw new Error("getAppPreferences returned null");
        const activity = await api.listActivity();
        if (!Array.isArray(activity)) throw new Error("listActivity did not return an array");
        const info = await api.getProjectInfo();
        if (!info || !info.title) throw new Error("getProjectInfo returned invalid data");
      });

      // Suite 8: group-lifecycle-online (Task 9: 4-phase full lifecycle)
      await runSuite(
        "group-lifecycle-online",
        async () => {
          const isOnlineAvailable = await invoke<boolean>("selftest_online_status");
          if (!isOnlineAvailable) {
            throw new Error(
              "SKIP: Missing FLEURON_STAGING_* environment variables for online lifecycle suite",
            );
          }

          // Phase 1: Owner creates study, Joiner joins, baseline roster
          await invoke("selftest_sign_in_as", { role: "owner" });
          const ownerPath = await invoke<string>("selftest_seed_unbound", { coderName: "Owner Coder" });

          const testTitle = `Selftest Lifecycle ${Date.now()}`;
          const created = await api.syncCreateProject(testTitle);
          if (!created.groupKey || !created.projectId) {
            throw new Error("syncCreateProject failed to return groupKey or projectId");
          }

          const ownerInfo = await api.syncGroupInfo();
          if (ownerInfo.members.length !== 1 || !ownerInfo.members[0].isYou) {
            throw new Error("Owner not reflected in group info roster");
          }

          // Joiner signs in and joins
          await invoke("selftest_sign_in_as", { role: "joiner" });
          const joinerPath = await invoke<string>("selftest_seed_unbound", { coderName: "Joiner Coder" });
          const joined = await api.syncJoinGroup(created.groupKey, "Joiner Coder");
          if (joined.projectId !== created.projectId) {
            throw new Error("Joined project ID did not match created group ID");
          }

          // Phase 2: Joiner uses syncDetachLocal, creates 2 local memos, codes passages. Owner codes on server.
          await api.syncDetachLocal();

          await api.createCode({ name: "Detached Code 1", color: "#3b82f6" });
          await api.createCode({ name: "Detached Code 2", color: "#10b981" });
          const joinerIv = await api.createInterview({
            participant_label: "P-Joiner",
            interviewers: [],
          });
          await api.updateHubMemo(joinerIv.id, "Private local memo from joiner");

          const deletionSummaryBefore = await api.projectDeletionSummary(joinerPath);
          if (deletionSummaryBefore.memo_count < 1) {
            throw new Error("Local memo not detected in deletion summary");
          }

          // Owner signs back in and creates codes on server
          await invoke("selftest_sign_in_as", { role: "owner" });
          await api.openProject(ownerPath);
          await api.syncJoinProject(created.projectId);
          await api.createCode({ name: "Owner Code 1", color: "#ec4899" });
          await api.createCode({ name: "Owner Code 2", color: "#8b5cf6" });
          await api.syncNow();

          // Phase 3: Joiner re-joins study; both sync; verify convergence & memo privacy
          await invoke("selftest_sign_in_as", { role: "joiner" });
          await api.openProject(joinerPath);
          await api.syncJoinGroup(created.groupKey, "Joiner Coder");
          await api.syncNow();

          const joinerDeletionSummary = await api.projectDeletionSummary(joinerPath);
          if (joinerDeletionSummary.memo_count < 1) {
            throw new Error("Joiner lost local memos after rebind");
          }

          // Owner pulls and verifies joiner's codes arrived, but NOT local memos
          await invoke("selftest_sign_in_as", { role: "owner" });
          await api.openProject(ownerPath);
          await api.syncNow();
          const ownerCodes = await api.listCodes();
          const hasJoinerCode = ownerCodes.some((c) => c.name === "Detached Code 1");
          if (!hasJoinerCode) {
            throw new Error("Owner did not receive Joiner's re-bound codes");
          }
          const ownerDeletionSummary = await api.projectDeletionSummary(ownerPath);
          if (ownerDeletionSummary.memo_count !== 0) {
            throw new Error("Joiner's local memos leaked to Owner via sync");
          }

          // Phase 4: Owner deletes group on server; Joiner leaves locally (404 clean leave); both delete project folders
          await api.syncDeleteGroup(testTitle);

          await invoke("selftest_sign_in_as", { role: "joiner" });
          await api.openProject(joinerPath);
          await api.syncLeaveGroup(); // Should cleanly succeed on 404

          // Both delete folders with fallback
          await api.deleteProjectFolder(joinerPath);
          await api.deleteProjectFolder(ownerPath);
        },
        { allowSkip: true },
      );

      // ── Core coding workflow coverage (E2E plan Task 2) ──────────────────
      // Five suites drive the real backend through a complete coder journey
      // against disposable seeded studies. Every suite cleans up in `finally`
      // so a failure never leaves a "Selftest Study" folder behind.

      // Suite 9: study-lifecycle
      await runSuite("study-lifecycle", async () => {
        const seedPath = await seedFreshStudy("study-lifecycle");
        await disposeStudy(seedPath); // keep only this suite's own fixture
        const parentDir = seedPath.replace(/[\\/][^\\/]+$/, "");
        let projectPath = "";
        try {
          const created = await api.createProject({
            parent_dir: parentDir,
            project_name: `fleuron_selftest_lc_${Date.now()}`,
            title: "Selftest Study",
            coders: [SELFTEST_CODER],
          });
          projectPath = created.path;

          await api.openProject(projectPath);
          await api.createInterview({
            participant_label: "P01",
            interviewers: [],
          });
          await api.closeProject();

          // Persistence: what was written survives close/reopen.
          const reopened = await api.openProject(projectPath);
          assert(
            reopened.project.title === "Selftest Study",
            `reopen lost title: ${reopened.project.title}`,
          );
          assert(
            reopened.interviews.some((i) => i.participant_label === "P01"),
            "interview P01 did not survive close/reopen",
          );
        } finally {
          if (projectPath) {
            await disposeStudy(projectPath);
          }
          try {
            await api.readTextFile(`${projectPath}/project.db`);
            throw new Error("study folder still exists after delete");
          } catch (e) {
            if (e instanceof Error && e.message.includes("still exists")) throw e;
          }
        }
      });

      // Suite 10: transcript-import
      await runSuite("transcript-import", async () => {
        const path = await seedFreshStudy("transcript-import");
        try {
          await api.openProject(path);
          const iv = await api.createInterview({
            participant_label: "P02",
            interviewers: [],
          });
          const segs: SegmentInput[] = [
            {
              speaker: "Participant",
              timestamp_start: "00:00:01.000",
              timestamp_end: "00:00:09.000",
              text: "First synthetic passage about daily routines.",
              section_tag: null,
            },
            {
              speaker: "Interviewer",
              timestamp_start: "00:00:10.000",
              timestamp_end: null,
              text: "Can you say more about the routines?",
              section_tag: null,
            },
            {
              speaker: "Participant",
              timestamp_start: "00:00:12.000",
              timestamp_end: "00:00:20.000",
              text: "Second passage describing support networks.",
              section_tag: null,
            },
          ];
          await api.importSegments({ interview_id: iv.id, segments: segs });
          const first = await api.getSegments(iv.id);
          assert(first.length === segs.length, `expected ${segs.length} passages, got ${first.length}`);

          // Content-derived ids: re-importing identical content must upsert to
          // the SAME ids, not duplicate rows.
          await api.importSegments({ interview_id: iv.id, segments: segs });
          const second = await api.getSegments(iv.id);
          assert(second.length === first.length, `re-import duplicated rows (${first.length} → ${second.length})`);
          const idsMatch = first.map((s) => s.id).join(",") === second.map((s) => s.id).join(",");
          assert(idsMatch, "passage ids changed across an identical re-import");
          assert(second[0].speaker === segs[0].speaker, "speaker not persisted on import");
        } finally {
          await disposeStudy(path);
        }
      });

      // Suite 11: coding-roundtrip (drives the STORE apply path so undo is real)
      await runSuite("coding-roundtrip", async () => {
        const path = await seedFreshStudy("coding-roundtrip");
        try {
          const store = useProjectStore.getState();
          await store.openProject(path);

          const seededSeg = useProjectStore.getState().segments.find((s) => s.id === "seg1");
          assert(seededSeg, "seeded passage seg1 missing after open");

          const spanText = "sample qualitative transcript";
          const start = seededSeg.text.indexOf(spanText);
          assert(start >= 0, `span phrase not found in seeded passage: ${seededSeg.text}`);
          const end = start + spanText.length;

          // Drive the real store action (not raw API): hydrate opened snapshot,
          // point at the passage + span selection, then toggle the code on.
          useProjectStore.setState({
            activeInterviewId: "iv1",
            selectedSegmentId: "seg1",
            pendingSelection: { segmentId: "seg1", start, end, text: spanText },
          });

          const code = await api.createCode({ name: "Roundtrip Code", color: "#ef4444" });
          useProjectStore.setState({ codes: [...useProjectStore.getState().codes, code] });

          await useProjectStore.getState().toggleCodeOnTarget(code.id);

          const applied = await api.listCodedSegments();
          const row = applied.find(
            (c) => c.segment_id === "seg1" && c.code_ids.includes(code.id),
          );
          assert(row, "applied code row not persisted");
          assert(row.char_start === start && row.char_end === end, "span offsets not persisted verbatim");
          assert(row.quote_text === spanText, `quote_text should be the span, got "${row.quote_text}"`);

          await useProjectStore.getState().undoLastCoding();

          const afterUndo = (await api.listCodedSegments()).filter(
            (c) => c.segment_id === "seg1" && c.coder_name === SELFTEST_CODER,
          );
          assert(
            !afterUndo.some((c) => c.code_ids.includes(code.id)),
            "undo left the code applied",
          );
        } finally {
          await disposeStudy(path);
        }
      });

      // Suite 12: export-artifacts
      await runSuite("export-artifacts", async () => {
        const path = await seedFreshStudy("export-artifacts");
        try {
          await api.openProject(path);

          // A crafted interview whose ALPHA span is coded while OMEGA stays
          // uncoded — the export must carry the phrase, never the whole turn.
          const iv = await api.createInterview({
            participant_label: "P03",
            interviewers: [],
          });
          const omegaTail = "Omega trailing words that must never surface alone.";
          const alphaSpan = "Alpha probe phrase";
          await api.importSegments({
            interview_id: iv.id,
            segments: [
              {
                speaker: "Participant",
                timestamp_start: "00:00:01.000",
                timestamp_end: "00:00:05.000",
                text: `${alphaSpan} sits inside this turn. ${omegaTail}`,
                section_tag: null,
              },
            ],
          });
          const imported = await api.getSegments(iv.id);
          const start = imported[0].text.indexOf(alphaSpan);
          const end = start + alphaSpan.length;
          await api.ensureCodeAndApply({
            name: "Export Probe Code",
            color: "#0ea5e9",
            interview_id: iv.id,
            segment_id: imported[0].id,
            coder_name: SELFTEST_CODER,
            char_start: start,
            char_end: end,
          });

          const result = await api.exportWithConfig(
            path,
            {
              preset: "custom",
              items: ["report-html", "coded-segments"],
              includeParticipantScope: "all",
              includeCoderScope: "all",
            },
            "<!DOCTYPE html><html><body><h1>Selftest Export Report</h1></body></html>",
            null,
            SELFTEST_CODER,
          );

          // Match on the file name, not a "/" suffix: the backend returns
          // native paths, so on Windows these end in "\\report.html" and a
          // forward-slash endsWith never matches.
          const named = (name: string) =>
            result.files.find((f) => f.split(/[\\/]/).pop() === name);
          const html = named("report.html");
          const csv = named("coded-segments.csv");
          assert(html, `report.html missing from export files: ${result.files.join(", ")}`);
          assert(csv, `coded-segments.csv missing from export files: ${result.files.join(", ")}`);
          assert(result.coded_segment_count >= 1, "export recorded zero coded segments");

          const htmlBody = await api.readTextFile(html);
          assert(htmlBody.includes("Selftest Export Report"), "report.html is empty/wrong");
          const csvBody = await api.readTextFile(csv);
          assert(csvBody.includes(alphaSpan), "csv quote cell does not carry the coded phrase");
          assert(!csvBody.includes("Omega trailing"), "csv leaked whole-turn text instead of the span");
        } finally {
          await disposeStudy(path);
        }
      });

      // Suite 13: backup-restore
      await runSuite("backup-restore", async () => {
        const path = await seedFreshStudy("backup-restore");
        try {
          await api.openProject(path);
          const iv = await api.createInterview({
            participant_label: "P04",
            interviewers: [],
          });
          await api.importSegments({
            interview_id: iv.id,
            segments: [
              {
                speaker: "Participant",
                timestamp_start: "00:00:01.000",
                timestamp_end: null,
                text: "Baseline passage captured before any coding.",
                section_tag: null,
              },
            ],
          });
          const seg = (await api.getSegments(iv.id))[0];
          const baseline = await api.ensureCodeAndApply({
            name: "Backup Baseline Code",
            color: "#22c55e",
            interview_id: iv.id,
            segment_id: seg.id,
            coder_name: SELFTEST_CODER,
          });

          const info = await api.createBackup("selftest backup-restore");
          assert(info.reason === "manual", `backup reason: ${info.reason}`);

          // Mutate AFTER the snapshot. A distinct span (not whole-segment)
          // guarantees a second row rather than an upsert of the baseline.
          const text = seg.text;
          await api.ensureCodeAndApply({
            name: "Post-Backup Mutation Code",
            color: "#f97316",
            interview_id: iv.id,
            segment_id: seg.id,
            coder_name: SELFTEST_CODER,
            char_start: 0,
            char_end: Math.min(8, text.length),
          });
          const mutated = await api.listCodedSegments();
          assert(mutated.length >= 2, "mutation before restore did not add a row");

          const outcome = await api.restoreBackup(info.path);
          assert(outcome.restored.coded_segments >= 1, "restored snapshot reports no coded segments");
          assert(!!outcome.safety_backup_path, "pre-restore safety snapshot missing");

          // Everything the store held is stale after restore — reload cold.
          await api.openProject(path);
          const restoredRows = await api.listCodedSegments();
          assert(restoredRows.length === 1, `expected exactly the baseline row after restore, got ${restoredRows.length}`);
          assert(
            restoredRows[0].code_ids.includes(baseline.coded_segment.code_ids[0]),
            "baseline code id changed across restore",
          );

          await api.deleteBackup(outcome.safety_backup_path);
        } finally {
          // Backups live inside the project folder, so disposing the study
          // removes the snapshot, the pre-restore safety copy, and the export
          // directories together.
          await disposeStudy(path);
        }
      });

      // Suite 14: note-draft-lifecycle — checked writes over real IPC.
      //
      // Recovery is redirected at a disposable sibling directory FIRST, so no
      // suite ever reads, writes, or clears the operator's real recovery
      // records. The leftover directory lives in the OS temp area next to the
      // disposable study (both synthetic); the study itself is deleted below.
      await runSuite("note-draft-lifecycle", async () => {
        const path = await seedFreshStudy("note-draft-lifecycle");
        try {
          await api.setRecoveryRootForSelftest(`${path}-note-recovery`);
          const snap = await api.openProject(path);
          const projectKey = snap.project_key;
          const epoch = snap.workspace_epoch;
          assert(projectKey && epoch, "open snapshot carries no workspace identity");

          const status = await api.noteRecoveryStatus();
          assert(status.available, `recovery unavailable in selftest: ${status.error}`);

          const coding = await api.ensureCodeAndApply({
            name: "Note Probe Code",
            color: "#0ea5e9",
            interview_id: "iv1",
            segment_id: "seg1",
            coder_name: SELFTEST_CODER,
          });
          const targetId = coding.coded_segment.id;

          const begun = await api.beginNoteDraft({
            project_key: projectKey,
            epoch,
            kind: "coding",
            target_id: targetId,
            interview_id: "iv1",
            coder_name: SELFTEST_CODER,
            participant_label: "P01",
            segment_id: "seg1",
            segment_index: 0,
            char_start: null,
            char_end: null,
            quote_text: "This is a sample qualitative transcript passage for selftest.",
          });
          assert(begun.status === "active", `begin failed: ${begun.status}`);
          if (begun.status !== "active") throw new Error("unreachable");
          assert(begun.committed_text === "", "fresh coding should commit from empty");

          const put = await api.putNoteDraft({
            project_key: projectKey,
            epoch,
            draft_id: begun.record.draft_id,
            expected_revision: 0,
            draft_text: "probe draft",
          });
          assert(put.status === "stored", `put failed: ${put.status}`);

          const saved = await api.saveNoteDraft({
            project_key: projectKey,
            epoch,
            draft_id: begun.record.draft_id,
            revision: 1,
            kind: "coding",
            target_id: targetId,
            expected_saved_text: "",
            draft_text: "probe draft",
          });
          assert(saved.status === "saved", `save failed: ${JSON.stringify(saved)}`);
          if (saved.status !== "saved") throw new Error("unreachable");
          assert(saved.recovery_cleared, "acknowledged revision should clear recovery");

          const rows = await api.listCodedSegments("iv1");
          assert(
            rows.some((r) => r.id === targetId && r.memo === "probe draft"),
            "committed note missing after checked save",
          );

          // A late put against the cleaned-up generation fails, never revives.
          const late = await api.putNoteDraft({
            project_key: projectKey,
            epoch,
            draft_id: begun.record.draft_id,
            expected_revision: 1,
            draft_text: "late",
          });
          assert(late.status === "stale-generation", `late put should fail closed, got ${late.status}`);

          // Conflict: a newer revision commits behind the draft's back.
          const begun2 = await api.beginNoteDraft({
            project_key: projectKey,
            epoch,
            kind: "coding",
            target_id: targetId,
            interview_id: "iv1",
            coder_name: SELFTEST_CODER,
            participant_label: "P01",
            segment_id: "seg1",
            segment_index: 0,
            char_start: null,
            char_end: null,
            quote_text: null,
          });
          assert(begun2.status === "active", "re-begin failed");
          if (begun2.status !== "active") throw new Error("unreachable");
          await api.putNoteDraft({
            project_key: projectKey,
            epoch,
            draft_id: begun2.record.draft_id,
            expected_revision: 0,
            draft_text: "stale base edit",
          });
          await api.patchCodingMemo({ coded_segment_id: targetId, memo: "external edit" });
          const conflict = await api.saveNoteDraft({
            project_key: projectKey,
            epoch,
            draft_id: begun2.record.draft_id,
            revision: 1,
            kind: "coding",
            target_id: targetId,
            expected_saved_text: "probe draft",
            draft_text: "stale base edit",
          });
          assert(conflict.status === "conflict", `expected conflict, got ${conflict.status}`);
          if (conflict.status !== "conflict") throw new Error("unreachable");
          assert(conflict.current_text === "external edit", "conflict must carry the live text");

          // Missing target: deleting the coding turns the next save into a
          // report, and the draft survives in recovery for Copy/Discard.
          await api.deleteCodedSegment(targetId);
          const missing = await api.saveNoteDraft({
            project_key: projectKey,
            epoch,
            draft_id: begun2.record.draft_id,
            revision: 1,
            kind: "coding",
            target_id: targetId,
            expected_saved_text: "external edit",
            draft_text: "orphaned edit",
          });
          assert(missing.status === "missing-target", `expected missing-target, got ${missing.status}`);
          const drafts = await api.listNoteDrafts();
          assert(
            drafts.some((d) => d.draft_id === begun2.record.draft_id),
            "orphaned draft must survive its coding's deletion",
          );
          const resolved = await api.resolveNoteDraftTarget(begun2.record.draft_id);
          assert(resolved.status === "missing", `resolve should report missing, got ${resolved.status}`);

          // Interview notes, including the deleted-interview kind.
          const extra = await api.createInterview({ participant_label: "P99", interviewers: [] });
          await api.updateHubMemo(extra.id, "hub-a");
          const begunIv = await api.beginNoteDraft({
            project_key: projectKey,
            epoch,
            kind: "interview",
            target_id: extra.id,
            interview_id: extra.id,
            coder_name: SELFTEST_CODER,
            participant_label: "P99",
          });
          assert(begunIv.status === "active", "interview begin failed");
          if (begunIv.status !== "active") throw new Error("unreachable");
          await api.putNoteDraft({
            project_key: projectKey,
            epoch,
            draft_id: begunIv.record.draft_id,
            expected_revision: 0,
            draft_text: "hub-b",
          });
          const savedIv = await api.saveNoteDraft({
            project_key: projectKey,
            epoch,
            draft_id: begunIv.record.draft_id,
            revision: 1,
            kind: "interview",
            target_id: extra.id,
            expected_saved_text: "hub-a",
            draft_text: "hub-b",
          });
          assert(savedIv.status === "saved", `interview save failed: ${savedIv.status}`);
          await api.deleteInterview(extra.id, SELFTEST_CODER);
          const begunGone = await api.beginNoteDraft({
            project_key: projectKey,
            epoch,
            kind: "interview",
            target_id: extra.id,
            interview_id: extra.id,
            coder_name: SELFTEST_CODER,
            participant_label: "P99",
          });
          assert(begunGone.status === "missing-target", "begin on a deleted interview must report missing");

          // Explicit discard, then proof the generation stays dead.
          const extra2 = await api.createInterview({ participant_label: "P98", interviewers: [] });
          const begunDisc = await api.beginNoteDraft({
            project_key: projectKey,
            epoch,
            kind: "interview",
            target_id: extra2.id,
            interview_id: extra2.id,
            coder_name: SELFTEST_CODER,
            participant_label: "P98",
          });
          assert(begunDisc.status === "active", "discard-fixture begin failed");
          if (begunDisc.status !== "active") throw new Error("unreachable");
          await api.discardNoteDraft({ draft_id: begunDisc.record.draft_id });
          const afterDiscard = await api.putNoteDraft({
            project_key: projectKey,
            epoch,
            draft_id: begunDisc.record.draft_id,
            expected_revision: 0,
            draft_text: "resurrected",
          });
          assert(afterDiscard.status === "stale-generation", "discarded generation must stay dead");
        } finally {
          await disposeStudy(path);
        }
      });

      // Suite 15: note-inactive-interview — commits address captured identity,
      // never the currently selected interview.
      await runSuite("note-inactive-interview", async () => {
        const path = await seedFreshStudy("note-inactive-interview");
        try {
          await api.setRecoveryRootForSelftest(`${path}-note-recovery`);
          const snap = await api.openProject(path);
          const projectKey = snap.project_key;
          const epoch = snap.workspace_epoch;
          assert(projectKey && epoch, "open snapshot carries no workspace identity");

          const iv = await api.createInterview({ participant_label: "P02", interviewers: [] });
          await api.importSegments({
            interview_id: iv.id,
            segments: [
              {
                speaker: "Participant",
                timestamp_start: "00:00:01.000",
                timestamp_end: null,
                text: "Inactive interview passage for cross-interview note tests.",
                section_tag: null,
              },
            ],
          });
          const seg = (await api.getSegments(iv.id))[0];
          const coding = await api.ensureCodeAndApply({
            name: "Inactive Probe",
            color: "#22c55e",
            interview_id: iv.id,
            segment_id: seg.id,
            coder_name: SELFTEST_CODER,
          });

          // The store never selects iv2 here: the commit below must still
          // land on the captured coding, not on the active interview.
          const begun = await api.beginNoteDraft({
            project_key: projectKey,
            epoch,
            kind: "coding",
            target_id: coding.coded_segment.id,
            interview_id: iv.id,
            coder_name: SELFTEST_CODER,
            participant_label: "P02",
            segment_id: seg.id,
            segment_index: 0,
            char_start: null,
            char_end: null,
            quote_text: seg.text,
          });
          assert(begun.status === "active", "inactive-interview begin failed");
          if (begun.status !== "active") throw new Error("unreachable");
          await api.putNoteDraft({
            project_key: projectKey,
            epoch,
            draft_id: begun.record.draft_id,
            expected_revision: 0,
            draft_text: "note on the inactive interview",
          });
          const saved = await api.saveNoteDraft({
            project_key: projectKey,
            epoch,
            draft_id: begun.record.draft_id,
            revision: 1,
            kind: "coding",
            target_id: coding.coded_segment.id,
            expected_saved_text: "",
            draft_text: "note on the inactive interview",
          });
          assert(saved.status === "saved", `inactive save failed: ${saved.status}`);
          const rows = await api.listCodedSegments(iv.id);
          assert(
            rows.some((r) => r.memo === "note on the inactive interview"),
            "inactive interview note missing after save",
          );

          // Same for the interview memo of the inactive interview.
          await api.updateHubMemo(iv.id, "inactive hub");
          const begunIv = await api.beginNoteDraft({
            project_key: projectKey,
            epoch,
            kind: "interview",
            target_id: iv.id,
            interview_id: iv.id,
            coder_name: SELFTEST_CODER,
            participant_label: "P02",
          });
          assert(begunIv.status === "active", "inactive hub begin failed");
          if (begunIv.status !== "active") throw new Error("unreachable");
          await api.putNoteDraft({
            project_key: projectKey,
            epoch,
            draft_id: begunIv.record.draft_id,
            expected_revision: 0,
            draft_text: "inactive hub edited",
          });
          const savedIv = await api.saveNoteDraft({
            project_key: projectKey,
            epoch,
            draft_id: begunIv.record.draft_id,
            revision: 1,
            kind: "interview",
            target_id: iv.id,
            expected_saved_text: "inactive hub",
            draft_text: "inactive hub edited",
          });
          assert(savedIv.status === "saved", `inactive hub save failed: ${savedIv.status}`);
        } finally {
          await disposeStudy(path);
        }
      });

      // Suite 16: note-privacy-contract — recovery markers reach no committed
      // pipeline: the marker lives only in recovery, the export must carry
      // committed notes and nothing unfinished.
      await runSuite("note-privacy-contract", async () => {
        const path = await seedFreshStudy("note-privacy-contract");
        try {
          await api.setRecoveryRootForSelftest(`${path}-note-recovery`);
          const snap = await api.openProject(path);
          const projectKey = snap.project_key;
          const epoch = snap.workspace_epoch;
          assert(projectKey && epoch, "open snapshot carries no workspace identity");

          const coding = await api.ensureCodeAndApply({
            name: "Privacy Probe",
            color: "#a855f7",
            interview_id: "iv1",
            segment_id: "seg1",
            coder_name: SELFTEST_CODER,
          });
          await api.patchCodingMemo({
            coded_segment_id: coding.coded_segment.id,
            memo: "zz-selftest-committed-marker-7h2k",
          });
          const begun = await api.beginNoteDraft({
            project_key: projectKey,
            epoch,
            kind: "coding",
            target_id: coding.coded_segment.id,
            interview_id: "iv1",
            coder_name: SELFTEST_CODER,
            participant_label: "P01",
            segment_id: "seg1",
            segment_index: 0,
            char_start: null,
            char_end: null,
            quote_text: null,
          });
          assert(begun.status === "active", "privacy-fixture begin failed");
          if (begun.status !== "active") throw new Error("unreachable");
          await api.putNoteDraft({
            project_key: projectKey,
            epoch,
            draft_id: begun.record.draft_id,
            expected_revision: 0,
            draft_text: "zz-selftest-draft-marker-3m8p unfinished",
          });

          const result = await api.exportWithConfig(
            path,
            {
              preset: "custom",
              items: ["coded-segments", "memos"],
              includeParticipantScope: "all",
              includeCoderScope: "all",
            },
            null,
            null,
            SELFTEST_CODER,
          );
          let sawCommitted = false;
          for (const file of result.files) {
            const body = await api.readTextFile(
              `${result.exports_dir}/${file.split(/[\\/]/).pop()}`,
            ).catch(() => "");
            assert(
              !body.includes("zz-selftest-draft-marker-3m8p"),
              `export file carries unfinished draft text: ${file}`,
            );
            if (body.includes("zz-selftest-committed-marker-7h2k")) sawCommitted = true;
          }
          assert(sawCommitted, "export does not carry committed notes");
        } finally {
          await disposeStudy(path);
        }
      });

      // Suite 17: note-single-editor — at most one editable note surface, with
      // keyboard focus landing in it. Opening a second note focuses the same
      // single editor on the new target instead of mounting another.
      await runSuite("note-single-editor", async () => {
        const path = await seedFreshStudy("note-single-editor");
        try {
          await api.setRecoveryRootForSelftest(`${path}-note-recovery`);
          const store = useProjectStore.getState();
          await store.openProject(path);

          const first = await api.ensureCodeAndApply({
            name: "Single Editor A",
            color: "#0ea5e9",
            interview_id: "iv1",
            segment_id: "seg1",
            coder_name: SELFTEST_CODER,
            char_start: 0,
            char_end: 6,
          });
          const second = await api.ensureCodeAndApply({
            name: "Single Editor B",
            color: "#22c55e",
            interview_id: "iv1",
            segment_id: "seg1",
            coder_name: SELFTEST_CODER,
            char_start: 7,
            char_end: 18,
          });

          await store.loadCodes();
          await store.loadCodedSegments();

          store.openNoteForCoding(first.coded_segment.id);
          store.openNoteForCoding(second.coded_segment.id);
          let box: HTMLTextAreaElement | null = null;
          for (let i = 0; i < 40; i++) {
            const boxes = document.querySelectorAll<HTMLTextAreaElement>(
              'textarea[aria-label="Note content"]',
            );
            assert(boxes.length <= 1, `expected at most one editor, found ${boxes.length}`);
            if (boxes.length === 1 && !boxes[0].disabled) {
              box = boxes[0];
              break;
            }
            await new Promise((resolve) => setTimeout(resolve, 50));
          }
          assert(box instanceof HTMLElement, "expected enabled note editor textarea in the DOM");
          box.focus();
          assert(
            document.activeElement?.getAttribute("aria-label") === "Note content",
            "keyboard focus must land in the single editor",
          );
          // Test-approved clean departure: no dirty drafts exist (both notes
          // were begun, never typed into), so the store close passes its own
          // guard without a UI answer and never hangs the runner.
          await store.closeProject();
        } finally {
          await disposeStudy(path);
        }
      });

      if (cancelled) return;
      setDone(true);

      // Report back to backend
      try {
        await invoke("selftest_report", { results: suiteResults });
      } catch (err) {
        console.error("Failed to report selftest results:", err);
      }
    }

    void runAllSuites();

    return () => {
      cancelled = true;
    };
  }, []);

  return (
    <div
      className="fixed inset-0 z-[999999] flex flex-col p-8 font-mono text-xs"
      style={{ backgroundColor: "rgb(10, 10, 10)", color: "rgb(245, 245, 245)" }}
    >
      <div
        className="mb-6 flex items-center justify-between border-b pb-4"
        style={{ borderColor: "rgb(38, 38, 38)" }}
      >
        <div>
          <h1
            className="text-base font-bold"
            style={{ color: "rgb(255, 255, 255)", backgroundColor: "rgb(10, 10, 10)" }}
          >
            FLEURON SELFTEST RUNNER
          </h1>
          <p
            className="mt-1"
            style={{ color: "rgb(163, 163, 163)", backgroundColor: "rgb(10, 10, 10)" }}
          >
            Status: {done ? "Completed" : `Running: ${activeSuite}`}
          </p>
        </div>
        <div className="text-right">
          <span
            className="rounded px-2.5 py-1"
            style={{ backgroundColor: "rgb(38, 38, 38)", color: "rgb(212, 212, 212)" }}
          >
            {results.filter((r) => r.passed).length}/{results.length} Passed
          </span>
        </div>
      </div>

      <div className="flex-1 space-y-2 overflow-y-auto">
        {results.map((r) => (
          <div
            key={r.suite}
            className="flex items-center justify-between rounded p-2.5"
            style={{
              backgroundColor: r.passed
                ? "rgb(6, 44, 28)"
                : r.status === "skipped"
                  ? "rgb(40, 35, 10)"
                  : "rgb(60, 15, 20)",
              color: r.passed
                ? "rgb(167, 243, 208)"
                : r.status === "skipped"
                  ? "rgb(253, 224, 71)"
                  : "rgb(254, 205, 211)",
            }}
          >
            <div className="flex items-center gap-2">
              <span>{r.passed ? (r.status === "skipped" ? "↷" : "✓") : "❌"}</span>
              <span className="font-semibold">{r.suite}</span>
              {r.error && (
                <span
                  className="ml-2"
                  style={{
                    color: r.status === "skipped" ? "rgb(250, 204, 21)" : "rgb(251, 113, 133)",
                  }}
                >
                  ({r.error})
                </span>
              )}
            </div>
            {r.duration_ms !== undefined && (
              <span style={{ color: "rgb(115, 115, 115)" }}>{r.duration_ms}ms</span>
            )}
          </div>
        ))}
      </div>
    </div>
  );
}
