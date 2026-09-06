import { open } from "@tauri-apps/plugin-dialog";
import { isWindows } from "./platform";

/** Extension for projects created from now on. */
export const PROJECT_EXT = ".fleuron";

/**
 * Extensions the app will open. `.codemap` and `.qcproj` are pre-rename names
 * and stay openable indefinitely — projects already on Drive must not stop
 * working because the app got renamed.
 */
export const PROJECT_EXTENSIONS = ["fleuron", "codemap", "qcproj"] as const;

/**
 * Pick a project folder.
 *
 * This used to open two dialogs in sequence, because a `.fleuron` could be
 * either a handoff bundle (a single file) or a working project (a folder), and
 * no native picker selects both. Handoff bundles no longer exist — coding
 * travels through sync and transcripts through Box — so a `.fleuron` is now
 * only ever a folder, and one dialog is enough.
 *
 * macOS still needs the file-style picker: a `.fleuron` folder is presented as
 * a package there, and a directory picker descends *into* it rather than
 * selecting it. Windows has no such notion and needs the directory picker.
 */
export async function pickProjectPath(): Promise<string | null> {
  const picked = isWindows
    ? await open({
        directory: true,
        multiple: false,
        title: "Choose a .fleuron project folder",
      })
    : await open({
        multiple: false,
        title: "Open a Fleuron project",
        filters: [
          { name: "Fleuron Project", extensions: [...PROJECT_EXTENSIONS] },
        ],
      });
  return picked ? (picked as string) : null;
}

export const BACKUP_EXTENSIONS = ["fleuronbak", "codemapbak"] as const;

export async function pickBackupPath(): Promise<string | null> {
  const picked = await open({
    multiple: false,
    title: "Restore a study backup",
    filters: [
      { name: "Fleuron Backup", extensions: [...BACKUP_EXTENSIONS] },
    ],
  });
  return picked ? (picked as string) : null;
}

