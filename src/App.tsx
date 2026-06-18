import { invoke } from "@tauri-apps/api/core";
import { emit, listen, type UnlistenFn } from "@tauri-apps/api/event";
import { getCurrentWindow, LogicalSize } from "@tauri-apps/api/window";
import { open, save } from "@tauri-apps/plugin-dialog";
import * as Tooltip from "@radix-ui/react-tooltip";
import clsx from "clsx";
import {
  AlertTriangle,
  ChevronDown,
  ChevronUp,
  FileSearch,
  FolderOpen,
  Play,
  RefreshCw,
  Save,
  SlidersHorizontal,
  Square,
  Zap
} from "lucide-react";
import { type CSSProperties, useEffect, useMemo, useRef, useState } from "react";
import { OPTIONS, getCommand } from "./lib/commands";
import type { AppInfo, DriveCandidate, ExistingImageCandidate, OptionSpec, OptionState, RunEvent, RunRequest, UpdateCheckResult } from "./lib/schema";
import { buildRunRequest, commandPreview, validateRunRequest } from "./lib/validation";

interface LogLine {
  id: string;
  level: "info" | "stdout" | "stderr" | "warning" | "error" | "exit";
  kind?: "progress";
  text: string;
}

interface DuplicateIsoMatch {
  message: string;
  path: string;
}

type ThemeMode = "system" | "light" | "dark";
type SettingsTab = "general" | "cd" | "dvd" | "offset" | "drive" | "image" | "firmware";
type CommandMode = "generic" | "redump";
type ExistingImageActionCommand = "refine" | "split" | "hash" | "info" | "protection" | "dvdisokey";
type FirmwareCommandId = "flash::mt1339" | "flash::mt1959" | "flash::sd616" | "flash::plextor";

const isTauri = "__TAURI_INTERNALS__" in window;
const THEME_STORAGE_KEY = "redumper-ui-theme";
const THEME_CHANGED_EVENT = "redumper-ui://theme-changed";
const SETTINGS_STORAGE_KEYS = {
  optionState: "redumper-ui-option-state",
  commandMode: "redumper-ui-command-mode",
  createOutputSubfolder: "redumper-ui-create-output-subfolder",
  compressLogFiles: "redumper-ui-compress-log-files",
  archiveToolPath: "redumper-ui-archive-tool-path",
  dumpTwiceCompareHashes: "redumper-ui-dump-twice-compare-hashes",
  firmwareCommandId: "redumper-ui-firmware-command-id",
  firmwarePath: "redumper-ui-firmware-path",
  firmwareForceFlash: "redumper-ui-firmware-force-flash",
  firmwareConfirmed: "redumper-ui-firmware-confirmed"
} as const;
const COMPACT_WINDOW_WIDTH = 700;
const LOG_BODY_HEIGHT = 220;
const MAIN_MIN_WINDOW_HEIGHT = 400;
const MAIN_MAX_WINDOW_HEIGHT = 900;
const MAIN_WINDOW_RESIZE_BUFFER = 4;
const SETTINGS_TABS: Array<{ id: SettingsTab; label: string }> = [
  { id: "general", label: "General" },
  { id: "cd", label: "CD Dump" },
  { id: "dvd", label: "DVD/BD" },
  { id: "offset", label: "Offset" },
  { id: "drive", label: "Drive" },
  { id: "image", label: "Image Tools" },
  { id: "firmware", label: "Firmware" }
];
const IMAGE_ACTION_ALLOWED_FLAGS: Record<ExistingImageActionCommand, Set<string>> = {
  refine: new Set([
    "--verbose",
    "--overwrite",
    "--debug",
    "--refine-subchannel",
    "--refine-sector-mode",
    "--force-refine",
    "--skip",
    "--lba-start",
    "--lba-end",
    "--lba-end-by-subcode"
  ]),
  split: new Set(["--verbose", "--overwrite", "--debug", "--force-split", "--leave-unchanged", "--force-qtoc", "--legacy-subs", "--skip-fill", "--filesystem-trim"]),
  hash: new Set(["--verbose", "--overwrite", "--debug"]),
  info: new Set(["--verbose", "--overwrite", "--debug"]),
  protection: new Set(["--verbose", "--overwrite", "--debug"]),
  dvdisokey: new Set(["--verbose", "--overwrite", "--debug"])
};
const DRIVE_TEST_ALLOWED_FLAGS = new Set(["--verbose", "--debug", "--speed", "--drive-type", "--scsi-timeout", "--drive-test-skip-plextor-leadin", "--drive-test-skip-cache-read"]);
const RINGS_ALLOWED_FLAGS = new Set(["--verbose", "--debug", "--speed", "--disc-type", "--drive-type", "--retries", "--scsi-timeout"]);
const DUMP_EXTRA_ALLOWED_FLAGS = new Set([
  "--verbose",
  "--overwrite",
  "--debug",
  "--speed",
  "--disc-type",
  "--drive-type",
  "--retries",
  "--scsi-timeout",
  "--lba-start",
  "--lba-end",
  "--lba-end-by-subcode",
  "--skip"
]);
const DVD_KEY_ALLOWED_FLAGS = new Set(["--verbose", "--debug", "--speed", "--drive-type", "--scsi-timeout"]);
const FIRMWARE_ALLOWED_FLAGS = new Set(["--verbose", "--debug", "--speed", "--drive-type", "--scsi-timeout"]);
const FIRMWARE_COMMANDS: Array<{ id: FirmwareCommandId; label: string }> = [
  { id: "flash::mt1339", label: "Flash MT1339" },
  { id: "flash::mt1959", label: "Flash MT1959" },
  { id: "flash::sd616", label: "Flash SD-616" },
  { id: "flash::plextor", label: "Flash Plextor" }
];

const FALLBACK_INFO: AppInfo = {
  appVersion: "0.1.722",
  upstreamTag: "b722",
  upstreamAppVersion: "0.1.722",
  platform: "browser",
  arch: "dev",
  defaultOutputDir: "",
  redumperPath: "src-tauri/resources/redumper/bin/redumper",
  redumperAvailable: false,
  resourceDir: "src-tauri/resources/redumper",
  diagnostics: [
    {
      level: "info",
      message: "Tauri runtime unavailable in browser preview."
    }
  ]
};

export default function App() {
  const isSettingsWindow = new URLSearchParams(window.location.search).get("window") === "settings";
  const [themeMode, setThemeMode] = useState<ThemeMode>(() => initialThemeMode());
  const [systemPrefersDark, setSystemPrefersDark] = useState(() => prefersDarkTheme());
  const [appInfo, setAppInfo] = useState<AppInfo>(FALLBACK_INFO);
  const [drives, setDrives] = useState<DriveCandidate[]>([]);
  const [optionState, setOptionState] = useSyncedState<OptionState>(SETTINGS_STORAGE_KEYS.optionState, defaultOptionState);
  const [drive, setDrive] = useState("");
  const [driveSpeed, setDriveSpeed] = useState("");
  const [imagePath, setImagePath] = useState("");
  const [imageName, setImageName] = useState("");
  const [imageNameSeed] = useState(() => formatDateStamp(new Date()));
  const [existingImageCandidate, setExistingImageCandidate] = useState<ExistingImageCandidate | null>(null);
  const [existingImageChecking, setExistingImageChecking] = useState(false);
  const [commandMode, setCommandMode] = useSyncedState<CommandMode>(SETTINGS_STORAGE_KEYS.commandMode, "redump");
  const [manualCommand, setManualCommand] = useState("");
  const [manualCommandDirty, setManualCommandDirty] = useState(false);
  const [createOutputSubfolder, setCreateOutputSubfolder] = useSyncedState(SETTINGS_STORAGE_KEYS.createOutputSubfolder, true);
  const [compressLogFiles, setCompressLogFiles] = useSyncedState(SETTINGS_STORAGE_KEYS.compressLogFiles, true);
  const [archiveToolPath, setArchiveToolPath] = useSyncedState(SETTINGS_STORAGE_KEYS.archiveToolPath, "");
  const [dumpTwiceCompareHashes, setDumpTwiceCompareHashes] = useSyncedState(SETTINGS_STORAGE_KEYS.dumpTwiceCompareHashes, false);
  const [firmwareCommandId, setFirmwareCommandId] = useSyncedState<FirmwareCommandId>(SETTINGS_STORAGE_KEYS.firmwareCommandId, "flash::mt1339");
  const [firmwarePath, setFirmwarePath] = useSyncedState(SETTINGS_STORAGE_KEYS.firmwarePath, "");
  const [firmwareForceFlash, setFirmwareForceFlash] = useSyncedState(SETTINGS_STORAGE_KEYS.firmwareForceFlash, false);
  const [firmwareConfirmed, setFirmwareConfirmed] = useSyncedState(SETTINGS_STORAGE_KEYS.firmwareConfirmed, false);
  const [settingsTab, setSettingsTab] = useState<SettingsTab>("general");
  const [running, setRunning] = useState(false);
  const [cancelRequested, setCancelRequested] = useState(false);
  const [drivesRefreshing, setDrivesRefreshing] = useState(false);
  const [drivesReady, setDrivesReady] = useState(!isTauri);
  const [appInfoLoading, setAppInfoLoading] = useState(isTauri);
  const [updateChecking, setUpdateChecking] = useState(false);
  const [updateInstalling, setUpdateInstalling] = useState(false);
  const [availableUpdate, setAvailableUpdate] = useState<UpdateCheckResult | null>(null);
  const [stage, setStage] = useState("Idle");
  const [progress, setProgress] = useState<RunEvent["progress"] | null>(null);
  const [visualProgressPercent, setVisualProgressPercent] = useState(0);
  const [runFailed, setRunFailed] = useState(false);
  const [activeDriveLabel, setActiveDriveLabel] = useState("");
  const [duplicateIsoMatch, setDuplicateIsoMatch] = useState<DuplicateIsoMatch | null>(null);
  const [deletingDuplicateIso, setDeletingDuplicateIso] = useState(false);
  const [logs, setLogs] = useState<LogLine[]>([]);
  const [logExpanded, setLogExpanded] = useState(false);
  const appMainRef = useRef<HTMLElement | null>(null);
  const settingsWindowRef = useRef<HTMLElement | null>(null);
  const logEndRef = useRef<HTMLDivElement | null>(null);
  const commandTextareaRef = useRef<HTMLTextAreaElement | null>(null);
  const cancelRequestedRef = useRef(false);
  const activeTheme = themeMode === "system" ? (systemPrefersDark ? "dark" : "light") : themeMode;

  const selectedDrive = useMemo(() => drives.find((candidate) => candidate.path === drive), [drive, drives]);
  const driveFallback = running ? activeDriveLabel || "Auto-selected drive" : driveFallbackLabel(appInfo.platform);
  const driveFieldLoading = !running && !drivesReady;
  const outputFieldLoading = !running && (appInfoLoading || !drivesReady);
  const missingSelectedDriveLabel = running && drive && !selectedDrive ? activeDriveLabel || drive : "";
  const genericUserModeActive = commandMode === "generic";
  const commandId = genericUserModeActive ? "dump" : "disc";
  const command = getCommand(commandId);
  const suggestedImageName = useMemo(
    () => suggestImageName(selectedDrive?.volumeName || selectedDrive?.label || drive, imageNameSeed),
    [drive, imageNameSeed, selectedDrive?.label, selectedDrive?.volumeName]
  );
  const effectiveImageName = imageName.trim() || suggestedImageName;
  const outputSubfolder = createOutputSubfolder;
  const visibleOptions = useMemo(
    () => OPTIONS.filter((option) => option.flag !== "--speed" && option.group !== "Firmware"),
    []
  );
  const generatedRunRequest = useMemo(
    () =>
      buildRunRequest({
        command: commandId,
        optionState,
        driveMode: drive ? "manual" : "auto",
        drive,
        driveSpeed,
        imagePath,
        imageName: effectiveImageName,
        workingDirectory: "",
        outputSubfolder,
        archiveToolPath,
        compressLogFiles,
        dumpTwiceCompareHashes,
        dangerConfirmed: false
      }),
    [archiveToolPath, commandId, compressLogFiles, drive, driveSpeed, dumpTwiceCompareHashes, effectiveImageName, imagePath, optionState, outputSubfolder]
  );
  const generatedPreview = useMemo(() => commandPreview(generatedRunRequest), [generatedRunRequest]);
  const commandText = manualCommandDirty ? manualCommand : generatedPreview;
  const runRequest = useMemo(
    () => ({
      ...generatedRunRequest,
      manualCommand: manualCommandDirty ? commandText.trim() || undefined : undefined
    }),
    [commandText, generatedRunRequest, manualCommandDirty]
  );
  const validationErrors = useMemo(() => validateRunRequest(runRequest), [runRequest]);
  const progressPercent = Math.min(Math.max(progress?.percentage ?? 0, 0), 100);
  const raceComplete = !runFailed && !cancelRequested && visualProgressPercent >= 99.5;
  const isCdProgress = progress?.c2Errors != null || progress?.qErrors != null;
  const refineRunRequest = useMemo(
    () => buildExistingImageRunRequest("refine", existingImageCandidate, runRequest, drive),
    [drive, existingImageCandidate, runRequest]
  );
  const splitRunRequest = useMemo(
    () => buildExistingImageRunRequest("split", existingImageCandidate, runRequest, drive),
    [drive, existingImageCandidate, runRequest]
  );
  const hashRunRequest = useMemo(
    () => buildExistingImageRunRequest("hash", existingImageCandidate, runRequest, drive),
    [drive, existingImageCandidate, runRequest]
  );
  const infoRunRequest = useMemo(
    () => buildExistingImageRunRequest("info", existingImageCandidate, runRequest, drive),
    [drive, existingImageCandidate, runRequest]
  );
  const protectionRunRequest = useMemo(
    () => buildExistingImageRunRequest("protection", existingImageCandidate, runRequest, drive),
    [drive, existingImageCandidate, runRequest]
  );
  const dvdIsoKeyRunRequest = useMemo(
    () => buildExistingImageRunRequest("dvdisokey", existingImageCandidate, runRequest, drive),
    [drive, existingImageCandidate, runRequest]
  );
  const driveTestRunRequest = useMemo(
    () => buildDriveTestRunRequest(runRequest, drive),
    [drive, runRequest]
  );
  const ringsRunRequest = useMemo(
    () => buildRingsRunRequest(runRequest, drive),
    [drive, runRequest]
  );
  const dumpExtraRunRequest = useMemo(
    () => buildDiscActionRunRequest("dump::extra", runRequest, drive, DUMP_EXTRA_ALLOWED_FLAGS, {
      imageName: effectiveImageName
    }),
    [drive, effectiveImageName, runRequest]
  );
  const dvdKeyRunRequest = useMemo(
    () => buildDiscActionRunRequest("dvdkey", runRequest, drive, DVD_KEY_ALLOWED_FLAGS, {
      compressLogFiles: false
    }),
    [drive, runRequest]
  );
  const firmwareRunRequest = useMemo(
    () => buildFirmwareRunRequest(runRequest, drive, firmwareCommandId, firmwarePath, firmwareForceFlash, firmwareConfirmed),
    [drive, firmwareCommandId, firmwareConfirmed, firmwareForceFlash, firmwarePath, runRequest]
  );

  function updateThemeMode(nextThemeMode: ThemeMode) {
    setThemeMode(nextThemeMode);
    localStorage.setItem(THEME_STORAGE_KEY, nextThemeMode);
    if (isTauri) {
      void emit(THEME_CHANGED_EVENT, nextThemeMode).catch(() => undefined);
    }
  }

  async function refreshDrives(silent = false) {
    if (!isTauri) {
      if (!silent) {
        pushLog("warning", "Drive refresh is available inside the Tauri app.");
      }
      setDrivesReady(true);
      return;
    }

    setDrivesRefreshing(true);
    try {
      const candidates = await invoke<DriveCandidate[]>("list_drives");
      setDrives(candidates);
      setDrive((current) => (candidates.some((candidate) => candidate.path === current) ? current : candidates[0]?.path ?? ""));
      if (!silent) {
        pushLog("info", candidates.length ? `Found ${candidates.length} drive(s) with media.` : "No drives with inserted discs were found.");
      }
    } catch (error) {
      pushLog("warning", String(error));
    } finally {
      setDrivesReady(true);
      setDrivesRefreshing(false);
    }
  }

  useEffect(() => {
    const media = window.matchMedia?.("(prefers-color-scheme: dark)");
    if (!media) {
      return;
    }

    const handleChange = () => setSystemPrefersDark(media.matches);
    handleChange();
    media.addEventListener("change", handleChange);
    return () => media.removeEventListener("change", handleChange);
  }, []);

  useEffect(() => {
    let unlisten: UnlistenFn | undefined;

    function syncTheme(nextThemeMode: unknown) {
      if (isThemeMode(nextThemeMode)) {
        setThemeMode(nextThemeMode);
      }
    }

    function handleStorage(event: StorageEvent) {
      if (event.key === THEME_STORAGE_KEY) {
        syncTheme(event.newValue);
      }
    }

    window.addEventListener("storage", handleStorage);

    if (isTauri) {
      let disposed = false;
      void listen<ThemeMode>(THEME_CHANGED_EVENT, (event) => {
        syncTheme(event.payload);
      }).then((dispose) => {
        if (disposed) {
          dispose();
          return;
        }
        unlisten = dispose;
      });

      return () => {
        disposed = true;
        window.removeEventListener("storage", handleStorage);
        unlisten?.();
      };
    }

    return () => {
      window.removeEventListener("storage", handleStorage);
    };
  }, []);

  useEffect(() => {
    let cancelled = false;

    async function loadAppInfo() {
      if (!isTauri) {
        setAppInfoLoading(false);
        return;
      }
      try {
        const info = await invoke<AppInfo>("get_app_info");
        if (!cancelled) {
          setAppInfo(info);
          setImagePath((current) => current || info.defaultOutputDir);
        }
      } catch (error) {
        pushLog("error", String(error));
      } finally {
        if (!cancelled) {
          setAppInfoLoading(false);
        }
      }
    }

    void loadAppInfo();
    void refreshDrives(true);
    return () => {
      cancelled = true;
    };
  }, []);

  useEffect(() => {
    if (!isTauri || isSettingsWindow) {
      return;
    }

    let unlisten: UnlistenFn | undefined;
    let disposed = false;
    void listen<RunEvent>("redumper://event", (event) => {
      handleRunEvent(event.payload);
    }).then((dispose) => {
      if (disposed) {
        dispose();
        return;
      }
      unlisten = dispose;
    });

    return () => {
      disposed = true;
      unlisten?.();
    };
  }, []);

  useEffect(() => {
    logEndRef.current?.scrollIntoView({ block: "end" });
  }, [logs]);

  useEffect(() => {
    if (cancelRequested) {
      setVisualProgressPercent(0);
      return;
    }

    if (!running) {
      setVisualProgressPercent((current) => Math.max(current, progressPercent));
      return;
    }

    let frame = 0;
    let lastTime = performance.now();

    function tick(now: number) {
      const elapsedSeconds = Math.min((now - lastTime) / 1000, 0.1);
      lastTime = now;
      setVisualProgressPercent((current) => {
        const target = progressPercent;
        const catchUpRate = 32;
        const guessRate = 0.9;
        const guessedCeiling = Math.min(99, Math.max(target + 4, current));
        const desired = current < target ? target : guessedCeiling;
        const rate = current < target ? catchUpRate : guessRate;
        return Math.min(desired, current + rate * elapsedSeconds);
      });
      frame = window.requestAnimationFrame(tick);
    }

    frame = window.requestAnimationFrame(tick);
    return () => window.cancelAnimationFrame(frame);
  }, [cancelRequested, progressPercent, running]);

  useEffect(() => {
    if (!isTauri || isSettingsWindow) {
      return;
    }

    let cancelled = false;
    const frame = window.requestAnimationFrame(() => {
      void resizeMainWindowToContent().catch(() => undefined);
    });

    async function resizeMainWindowToContent() {
      if (cancelled) {
        return;
      }
      await resizeMainWindowForLog();
    }

    return () => {
      cancelled = true;
      window.cancelAnimationFrame(frame);
    };
  }, [commandText, isSettingsWindow, logExpanded, logs.length]);

  useEffect(() => {
    if (!commandTextareaRef.current) {
      return;
    }
    commandTextareaRef.current.style.height = "auto";
    commandTextareaRef.current.style.height = `${commandTextareaRef.current.scrollHeight}px`;
  }, [commandText]);

  useEffect(() => {
    setManualCommand("");
    setManualCommandDirty(false);
  }, [commandMode]);

  useEffect(() => {
    if (!isTauri || !isSettingsWindow) {
      return;
    }

    let cancelled = false;
    const frame = window.requestAnimationFrame(() => {
      void resizeSettingsWindowToContent().catch(() => undefined);
    });

    async function resizeSettingsWindowToContent() {
      const windowRef = getCurrentWindow();
      const scaleFactor = await windowRef.scaleFactor();
      const currentSize = await windowRef.innerSize();
      if (cancelled) {
        return;
      }

      const contentHeight = settingsWindowRef.current?.scrollHeight ?? document.documentElement.scrollHeight;
      const nextHeight = Math.min(Math.max(contentHeight, 360), 1120);
      const nextWidth = Math.max(currentSize.width / scaleFactor, 820);
      await windowRef.setSize(new LogicalSize(nextWidth, nextHeight));
    }

    return () => {
      cancelled = true;
      window.cancelAnimationFrame(frame);
    };
  }, [
    isSettingsWindow,
    settingsTab,
    optionState,
    compressLogFiles,
    dumpTwiceCompareHashes,
    firmwareCommandId,
    firmwarePath,
    firmwareForceFlash,
    firmwareConfirmed
  ]);

  useEffect(() => {
    setProgress(null);
    setStage("Idle");
  }, [commandId]);

  useEffect(() => {
    setOptionState((current) => (genericUserModeActive ? applyCompatibleAllDrivesPreset(current) : clearCompatibleAllDrivesPreset(current)));
  }, [genericUserModeActive, setOptionState]);

  useEffect(() => {
    let cancelled = false;

    async function scanExistingImageCandidate() {
      if (!isTauri || createOutputSubfolder || !imagePath.trim()) {
        setExistingImageCandidate(null);
        setExistingImageChecking(false);
        return;
      }

      setExistingImageChecking(true);
      try {
        const candidate = await invoke<ExistingImageCandidate | null>("find_existing_image_candidate", { directory: imagePath });
        if (!cancelled) {
          setExistingImageCandidate(candidate);
        }
      } catch (error) {
        if (!cancelled) {
          setExistingImageCandidate(null);
          pushLog("warning", String(error));
        }
      } finally {
        if (!cancelled) {
          setExistingImageChecking(false);
        }
      }
    }

    void scanExistingImageCandidate();
    return () => {
      cancelled = true;
    };
  }, [createOutputSubfolder, imagePath]);

  useEffect(() => {
    if (!isTauri) {
      return;
    }
    void checkForUpdates(true);
  }, []);

  function pushLog(level: LogLine["level"], text: string, options: { replaceProgress?: boolean } = {}) {
    setLogs((current) => {
      if (options.replaceProgress) {
        const nextLine = {
          id: `${Date.now()}-${Math.random()}`,
          level,
          kind: "progress" as const,
          text
        };
        const progressIndex = current.findIndex((line) => line.kind === "progress");
        if (progressIndex >= 0) {
          if (current[progressIndex].level === level && current[progressIndex].text === text) {
            return current;
          }

          const next = [...current];
          next[progressIndex] = {
            ...nextLine,
            id: current[progressIndex].id
          };
          return next;
        }

        return [...current.slice(-399), nextLine];
      }

      const last = current.at(-1);
      if (last?.level === level && last.text === text) {
        return current;
      }

      return [
        ...current.slice(-399),
        {
          id: `${Date.now()}-${Math.random()}`,
          level,
          text
        }
      ];
    });
  }

  function handleRunEvent(event: RunEvent) {
    if (event.kind === "started") {
      cancelRequestedRef.current = false;
      setRunning(true);
      setCancelRequested(false);
      setRunFailed(false);
      setStage("STARTED");
      setProgress(null);
      setVisualProgressPercent(0);
      pushLog("info", event.message ?? "redumper started");
      return;
    }

    if (event.stage) {
      setStage(event.stage);
    }
    if (event.progress) {
      setProgress(event.progress);
    }
    if (event.duplicateIsoPath) {
      setDuplicateIsoMatch({
        path: event.duplicateIsoPath,
        message: event.message ?? "The duplicate ISO matched the first dump."
      });
    }
    if (event.line) {
      const level = event.kind === "warning" || event.kind === "error" || event.kind === "stderr" ? event.kind : "stdout";
      pushLog(level as LogLine["level"], event.line, { replaceProgress: event.kind === "progress" });
    }
    if (event.kind === "exit") {
      const wasCancelled = cancelRequestedRef.current;
      setRunning(false);
      setCancelRequested(false);
      cancelRequestedRef.current = false;
      setActiveDriveLabel("");
      setRunFailed(!wasCancelled && typeof event.exitCode === "number" && event.exitCode !== 0);
      setStage(wasCancelled ? "Idle" : "END");
      if (wasCancelled) {
        setProgress(null);
        setVisualProgressPercent(0);
      } else {
        setVisualProgressPercent((current) => (event.exitCode === 0 ? 100 : current));
      }
      pushLog("exit", `redumper exited${typeof event.exitCode === "number" ? ` with code ${event.exitCode}` : ""}`);
    }
    if (event.message) {
      pushLog(event.kind === "error" ? "error" : "info", event.message);
    }
  }

  async function chooseDirectory(setter: (value: string) => void) {
    if (!isTauri) {
      return;
    }
    const selected = await open({ directory: true, multiple: false });
    if (typeof selected === "string") {
      setter(selected);
    }
  }

  async function chooseFile(setter: (value: string) => void) {
    if (!isTauri) {
      return;
    }
    const selected = await open({ directory: false, multiple: false });
    if (typeof selected === "string") {
      setter(selected);
    }
  }

  async function checkForUpdates(silent = false) {
    if (!isTauri) {
      if (!silent) {
        pushLog("warning", "Update checks are available inside the packaged Tauri app.");
      }
      return;
    }

    setUpdateChecking(true);
    try {
      const result = await invoke<UpdateCheckResult>("check_for_updates");
      setAvailableUpdate(result.available ? result : null);
      if (result.available || !silent) {
        pushLog(result.available ? "warning" : "info", result.message);
      }
    } catch (error) {
      setAvailableUpdate(null);
      if (!silent) {
        pushLog("error", String(error));
      }
    } finally {
      setUpdateChecking(false);
    }
  }

  async function installAvailableUpdate() {
    if (!availableUpdate?.available) {
      await checkForUpdates(false);
      return;
    }
    if (!isTauri) {
      pushLog("warning", "Update installation is available inside the packaged Tauri app.");
      return;
    }

    setUpdateInstalling(true);
    try {
      const message = await invoke<string>("install_update");
      pushLog("info", message);
    } catch (error) {
      pushLog("error", String(error));
    } finally {
      setUpdateInstalling(false);
    }
  }

  async function startRedumperRequest(request: RunRequest) {
    const errors = validateRunRequest(request);
    if (errors.length) {
      errors.forEach((error) => pushLog("error", error));
      return;
    }

    if (!isTauri) {
      pushLog("warning", "Run is available inside the Tauri app.");
      return;
    }

    setLogs([]);
    setProgress(null);
    setVisualProgressPercent(0);
    setRunFailed(false);
    setActiveDriveLabel(selectedDrive?.label ?? request.drive ?? "Auto-selected drive");
    setCancelRequested(false);
    cancelRequestedRef.current = false;
    setDuplicateIsoMatch(null);
    setStage("QUEUED");
    try {
      await invoke<string>("run_redumper", { request });
    } catch (error) {
      setRunning(false);
      setActiveDriveLabel("");
      pushLog("error", String(error));
    }
  }

  async function run() {
    await startRedumperRequest(runRequest);
  }

  async function ejectDrive() {
    await startRedumperRequest({
      command: "eject",
      options: [],
      driveMode: drive ? "manual" : "auto",
      drive: drive || undefined,
      imagePath: undefined,
      imageName: undefined,
      workingDirectory: undefined,
      outputSubfolder: false,
      compressLogFiles: false,
      dumpTwiceCompareHashes: false,
      dangerConfirmed: false
    });
  }

  async function refineExistingImage() {
    if (!refineRunRequest) {
      return;
    }
    await startRedumperRequest(refineRunRequest);
  }

  async function splitExistingImage() {
    if (!splitRunRequest) {
      return;
    }
    await startRedumperRequest(splitRunRequest);
  }

  async function hashExistingImage() {
    if (!hashRunRequest) {
      return;
    }
    await startRedumperRequest(hashRunRequest);
  }

  async function runImageInfo() {
    if (!infoRunRequest) {
      return;
    }
    await startRedumperRequest(infoRunRequest);
  }

  async function runProtectionScan() {
    if (!protectionRunRequest) {
      return;
    }
    await startRedumperRequest(protectionRunRequest);
  }

  async function runDvdIsoKey() {
    if (!dvdIsoKeyRunRequest) {
      return;
    }
    await startRedumperRequest(dvdIsoKeyRunRequest);
  }

  async function runDriveTest() {
    await startRedumperRequest(driveTestRunRequest);
  }

  async function runRings() {
    await startRedumperRequest(ringsRunRequest);
  }

  async function runDumpExtra() {
    await startRedumperRequest(dumpExtraRunRequest);
  }

  async function runDvdKey() {
    await startRedumperRequest(dvdKeyRunRequest);
  }

  async function runFirmwareFlash() {
    await startRedumperRequest(firmwareRunRequest);
  }

  async function deleteDuplicateIso() {
    if (!duplicateIsoMatch) {
      return;
    }
    if (!isTauri) {
      setDuplicateIsoMatch(null);
      return;
    }

    setDeletingDuplicateIso(true);
    try {
      const message = await invoke<string>("delete_duplicate_iso", { path: duplicateIsoMatch.path });
      pushLog("info", message);
      setDuplicateIsoMatch(null);
    } catch (error) {
      pushLog("error", String(error));
    } finally {
      setDeletingDuplicateIso(false);
    }
  }

  async function cancel() {
    if (!isTauri) {
      return;
    }
    try {
      cancelRequestedRef.current = true;
      setCancelRequested(true);
      setProgress(null);
      setVisualProgressPercent(0);
      await invoke("cancel_redumper");
      pushLog("warning", "Cancel requested.");
    } catch (error) {
      pushLog("error", String(error));
    }
  }

  async function toggleLogExpanded() {
    const nextExpanded = !logExpanded;
    setLogExpanded(nextExpanded);
    if (isTauri && !isSettingsWindow) {
      window.requestAnimationFrame(() => {
        if (nextExpanded) {
          logEndRef.current?.scrollIntoView({ block: "end" });
        }
        void resizeMainWindowForLog().catch(() => undefined);
      });
    }
  }

  async function saveLog() {
    const contents = logs.map((line) => line.text).join("\n");
    const path = await save({
      defaultPath: `redumper-ui-${formatDateStamp(new Date())}.log`,
      filters: [{ name: "Log files", extensions: ["log", "txt"] }]
    });
    if (!path) {
      return;
    }

    try {
      if (isTauri) {
        await invoke("save_log_file", { path, contents });
      } else {
        const blob = new Blob([contents], { type: "text/plain;charset=utf-8" });
        const url = URL.createObjectURL(blob);
        const link = document.createElement("a");
        link.href = url;
        link.download = path;
        link.click();
        URL.revokeObjectURL(url);
      }
      pushLog("info", `Saved log to ${path}`);
    } catch (error) {
      pushLog("error", String(error));
    }
  }

  async function resizeMainWindowForLog() {
    const windowRef = getCurrentWindow();
    const scaleFactor = await windowRef.scaleFactor();
    const innerSize = await windowRef.innerSize();
    const outerSize = await windowRef.outerSize();
    const chromeHeight = Math.max(0, (outerSize.height - innerSize.height) / scaleFactor);
    const measuredHeight = measuredChildrenHeight(appMainRef.current) ?? document.documentElement.scrollHeight;
    const nextHeight = clampWindowHeight(measuredHeight + chromeHeight + MAIN_WINDOW_RESIZE_BUFFER);
    const nextWidth = COMPACT_WINDOW_WIDTH;
    await windowRef.setSize(new LogicalSize(nextWidth, nextHeight));

    await new Promise((resolve) => window.requestAnimationFrame(resolve));
    const overflow = document.documentElement.scrollHeight - window.innerHeight;
    if (overflow > 1) {
      await windowRef.setSize(new LogicalSize(nextWidth, clampWindowHeight(nextHeight + overflow + 2)));
    }
  }

  function renderSettingsGroup(group: string) {
    const groupOptions = visibleOptions.filter((option) => option.group === group);
    const hasCustomRows = group === "General" || group === "Drive Test" || group === "CD Dump" || group === "DVD/BD";

    if (!groupOptions.length && !hasCustomRows) {
      return null;
    }

    return (
      <section key={group} className="advanced-card rounded-md border">
        <h3 className="advanced-card-title px-2.5 py-1.5 text-sm font-semibold">{group}</h3>
        <div className="option-section-grid">
          {group === "General" ? (
            <>
              <ThemeOptionRow value={themeMode} onChange={updateThemeMode} />
              <CompressLogOptionRow checked={compressLogFiles} disabled={!command.writesFiles} onChange={setCompressLogFiles} />
              <ArchiveToolOptionRow value={archiveToolPath} disabled={!compressLogFiles} onChange={setArchiveToolPath} onChoose={() => void chooseFile(setArchiveToolPath)} />
            </>
          ) : null}
          {group === "Drive Test" ? <DriveTestActionRow running={running} onRun={() => void runDriveTest()} /> : null}
          {group === "CD Dump" || group === "DVD/BD" ? (
            <DumpTwiceCompareOptionRow checked={dumpTwiceCompareHashes} disabled={false} onChange={setDumpTwiceCompareHashes} />
          ) : null}
          {groupOptions.map((option) => (
            <OptionRow
              key={option.flag}
              option={option}
              state={optionState[option.flag] ?? defaultSelectedOption(option)}
              onChange={(next) =>
                setOptionState((current) => ({
                  ...current,
                  [option.flag]: next
                }))
              }
              onChooseFile={chooseFile}
            />
          ))}
        </div>
      </section>
    );
  }

  function renderSettingsTab() {
    if (settingsTab === "general") {
      return (
        <>
          {renderSettingsGroup("General")}
          {renderSettingsGroup("Advanced")}
        </>
      );
    }

    if (settingsTab === "cd") {
      return renderSettingsGroup("CD Dump");
    }

    if (settingsTab === "dvd") {
      return renderSettingsGroup("DVD/BD");
    }

    if (settingsTab === "offset") {
      return renderSettingsGroup("Offset");
    }

    if (settingsTab === "drive") {
      return (
        <>
          <section className="advanced-card rounded-md border">
            <h3 className="advanced-card-title px-2.5 py-1.5 text-sm font-semibold">Disc Actions</h3>
            <div className="option-section-grid">
              <RingsActionRow running={running} onRun={() => void runRings()} />
              <AdvancedActionRow label="Dump Extra Areas" code="dump::extra" running={running} onRun={() => void runDumpExtra()} />
              <AdvancedActionRow label="Extract DVD Key" code="dvdkey" running={running} onRun={() => void runDvdKey()} />
            </div>
          </section>
          {renderSettingsGroup("Drive")}
          {renderSettingsGroup("Drive Test")}
        </>
      );
    }

    if (settingsTab === "image") {
      return (
        <>
          <section className="advanced-card rounded-md border">
            <h3 className="advanced-card-title px-2.5 py-1.5 text-sm font-semibold">Image Tools</h3>
            <div className="option-section-grid">
              <AdvancedActionRow label="Image Info" code="info" running={running} disabled={!infoRunRequest} onRun={() => void runImageInfo()} />
              <AdvancedActionRow label="Scan Protection" code="protection" running={running} disabled={!protectionRunRequest} onRun={() => void runProtectionScan()} />
              <AdvancedActionRow label="DVD ISO Key" code="dvdisokey" running={running} disabled={!dvdIsoKeyRunRequest} onRun={() => void runDvdIsoKey()} />
            </div>
          </section>
          {renderSettingsGroup("Split")}
        </>
      );
    }

    return (
      <FirmwareFlashSection
        commandId={firmwareCommandId}
        firmwarePath={firmwarePath}
        forceFlash={firmwareForceFlash}
        confirmed={firmwareConfirmed}
        running={running}
        onCommandChange={setFirmwareCommandId}
        onFirmwarePathChange={setFirmwarePath}
        onForceFlashChange={setFirmwareForceFlash}
        onConfirmedChange={setFirmwareConfirmed}
        onChooseFirmware={() => void chooseFile(setFirmwarePath)}
        onRun={() => void runFirmwareFlash()}
      />
    );
  }

  if (isSettingsWindow) {
    return (
      <Tooltip.Provider delayDuration={180}>
        <div className="app-shell settings-window-shell" data-theme={activeTheme}>
          <main ref={settingsWindowRef} className="settings-window-main">
            <div className="settings-window-header">
              <div className="flex min-w-0 items-center gap-2">
                <SlidersHorizontal size={18} />
                <h1>Settings</h1>
              </div>
            </div>

            <div className="settings-tabs" role="tablist" aria-label="Settings sections">
              {SETTINGS_TABS.map((tab) => (
                <button
                  key={tab.id}
                  className={clsx("settings-tab", settingsTab === tab.id && "active")}
                  type="button"
                  role="tab"
                  aria-selected={settingsTab === tab.id}
                  onClick={() => setSettingsTab(tab.id)}
                >
                  {tab.label}
                </button>
              ))}
            </div>

            <div className="settings-window-body">
              <div className="settings-panel-grid">{renderSettingsTab()}</div>
            </div>
          </main>
        </div>
      </Tooltip.Provider>
    );
  }

  return (
    <Tooltip.Provider delayDuration={180}>
      <div className="app-shell" data-theme={activeTheme}>
        <main ref={appMainRef} className="app-main">
          <section className="app-section app-section-panel border-b px-5 py-4">
            <div className="settings-form app-content grid gap-2">
              <SettingsRow label="Drive">
                <div className="drive-row">
                  <select
                    value={drive}
                    onChange={(event) => setDrive(event.target.value)}
                    className={clsx("control drive-control", driveFieldLoading && "control-loading")}
                    title={driveFieldLoading ? "Checking drives" : selectedDrive?.label ?? driveFallback}
                    disabled={driveFieldLoading}
                  >
                    <option value="">{driveFieldLoading ? "Checking drives..." : driveFallback}</option>
                    {missingSelectedDriveLabel ? <option value={drive}>{missingSelectedDriveLabel}</option> : null}
                    {drives.map((candidate) => (
                      <option key={candidate.path} value={candidate.path}>
                        {candidate.label}
                      </option>
                    ))}
                  </select>
                  <select
                    value={driveSpeed}
                    onChange={(event) => setDriveSpeed(event.target.value)}
                    className="control speed-control"
                    aria-label="Speed"
                    title="Speed"
                  >
                    <option value="">Auto</option>
                    {["1", "2", "4", "6", "8", "12", "16", "24", "32", "48"].map((speed) => (
                      <option key={speed} value={speed}>
                        {speed}x
                      </option>
                    ))}
                  </select>
                  <IconButton title="Refresh drives" disabled={running || drivesRefreshing} onClick={() => void refreshDrives()}>
                    <RefreshCw size={18} className={clsx(drivesRefreshing && "animate-spin")} />
                  </IconButton>
                </div>
              </SettingsRow>

              <SettingsRow label="Output">
                <div className="output-row">
                  <input
                    value={imageName}
                    onChange={(event) => setImageName(event.target.value)}
                    placeholder={outputFieldLoading ? "Loading output..." : suggestedImageName}
                    className={clsx("control", outputFieldLoading && "control-loading")}
                    disabled={outputFieldLoading}
                  />
                  <IconButton
                    title={imagePath || "Select output folder"}
                    aria-label="Select output folder"
                    disabled={outputFieldLoading}
                    onClick={() => void chooseDirectory(setImagePath)}
                  >
                    <FolderOpen size={18} />
                  </IconButton>
                  <label className="create-subfolder-toggle" title="Create a folder named after Output inside the selected directory">
                    <input
                      type="checkbox"
                      checked={createOutputSubfolder}
                      disabled={outputFieldLoading}
                      onChange={(event) => setCreateOutputSubfolder(event.target.checked)}
                      className="accent-checkbox h-3.5 w-3.5 shrink-0"
                    />
                    <span>Create Subfolder</span>
                  </label>
                </div>
              </SettingsRow>

              <div className="command-settings-row command-settings-row-open">
                <div className="command-row-heading">
                  <div className="settings-label">Command</div>
                  <label className="command-mode-toggle" title="Use generic drive mode flags">
                    <span>Generic Mode</span>
                    <input
                      type="checkbox"
                      checked={commandMode === "generic"}
                      onChange={(event) => {
                        if (event.target.checked) {
                          setCommandMode("generic");
                        }
                      }}
                      aria-label="Use Generic Mode"
                      className="accent-checkbox command-checkbox shrink-0"
                    />
                  </label>
                  <label className="command-mode-toggle" title="Use redump.info-compatible command">
                    <span>Redump Compatible</span>
                    <input
                      type="checkbox"
                      checked={commandMode === "redump"}
                      onChange={(event) => {
                        if (event.target.checked) {
                          setCommandMode("redump");
                        }
                      }}
                      aria-label="Use Redump Compatible mode"
                      className="accent-checkbox command-checkbox shrink-0"
                    />
                  </label>
                </div>
                <div className="command-row">
                  <textarea
                    ref={commandTextareaRef}
                    value={commandText}
                    onChange={(event) => {
                      setManualCommand(event.target.value);
                      setManualCommandDirty(true);
                    }}
                    className="control command-textarea"
                    rows={3}
                    spellCheck={false}
                    aria-label="Redumper command"
                  />
                </div>
              </div>

              {validationErrors.length ? (
                <div className="preview-validation-row">
                  {validationErrors.map((error) => (
                    <span key={error} className="inline-flex items-center gap-1 rounded bg-copper/15 px-2 py-1 text-xs text-copper">
                      <AlertTriangle size={13} />
                      {error}
                    </span>
                  ))}
                </div>
              ) : null}
            </div>
          </section>

          <section className="app-section border-b px-5 py-4">
            <div className="app-content grid gap-4">
              <div className="workflow-column space-y-3">
                <div className="progress-card rounded-md border p-3">
                  <div className="progress-track">
                    <div
                      className={clsx("progress-runner", running && "is-running", runFailed && "is-failed")}
                      style={{ left: `clamp(16px, ${visualProgressPercent}%, calc(100% - 34px))` }}
                      aria-hidden="true"
                    >
                      {running && !runFailed && !cancelRequested ? <span className="progress-dust">💨</span> : null}
                      <span className="progress-car">🏎️</span>
                      {runFailed ? <span className="progress-fire">🔥</span> : null}
                    </div>
                    <div className="progress-finish" aria-hidden="true">{raceComplete ? "🏆" : "🏁"}</div>
                    <div className="progress-fill" style={{ width: `${visualProgressPercent}%` }} />
                  </div>
                  <div className="metric-grid mt-2 grid grid-cols-5 gap-2 text-xs">
                    <SpeedometerMetric speed={driveSpeed} stage={stage} running={running} progressPercent={progressPercent} />
                    <Metric label="LBA" value={<LbaValue progress={progress} />} />
                    <Metric label="SCSI" value={progress?.scsiErrors ?? 0} />
                    <Metric label="EDC" value={progress?.edcErrors ?? 0} />
                    <Metric label="C2/Q" value={`${progress?.c2Errors ?? 0}/${progress?.qErrors ?? 0}`} inactive={!isCdProgress} />
                  </div>
                </div>

                <div className="action-row">
                  <button
                    className={clsx("primary-button", running && "stop-button")}
                    disabled={!running && validationErrors.length > 0}
                    onClick={() => void (running ? cancel() : run())}
                  >
                    {running ? (
                      <Square size={18} fill="currentColor" strokeWidth={0} />
                    ) : (
                      <Play size={18} fill="currentColor" strokeWidth={0} />
                    )}
                    {running ? "Stop" : "Dump Disc"}
                  </button>
                  {refineRunRequest ? (
                    <button
                      className="secondary-button workflow-action-button"
                      disabled={running}
                      title={`Refine ${refineRunRequest.imageName}`}
                      onClick={() => void refineExistingImage()}
                    >
                      <FileSearch size={18} />
                      Refine Existing Image
                    </button>
                  ) : null}
                  {splitRunRequest ? (
                    <button
                      className="secondary-button workflow-action-button"
                      disabled={running}
                      title={`Split ${splitRunRequest.imageName}`}
                      onClick={() => void splitExistingImage()}
                    >
                      Split
                    </button>
                  ) : null}
                  {hashRunRequest ? (
                    <button
                      className="secondary-button workflow-action-button"
                      disabled={running}
                      title={`Hash ${hashRunRequest.imageName}`}
                      onClick={() => void hashExistingImage()}
                    >
                      Hash
                    </button>
                  ) : null}
                  <IconButton title="Eject" className="eject-button" disabled={running} onClick={() => void ejectDrive()}>
                    <EjectIcon />
                  </IconButton>
                </div>
                {existingImageChecking ? <div className="subtle-text text-xs">Checking output folder for an existing image...</div> : null}
                {availableUpdate?.available ? (
                  <button className="update-button w-full" disabled={running || updateChecking || updateInstalling} onClick={() => void installAvailableUpdate()}>
                    Update App
                  </button>
                ) : null}
              </div>

              {appInfo.diagnostics.length ? (
                <div className="flex flex-wrap gap-2 lg:col-span-2">
                  {appInfo.diagnostics.map((diagnostic) => (
                    <span key={diagnostic.message} className={clsx("diagnostic", diagnostic.level)}>
                      {diagnostic.message}
                    </span>
                  ))}
                </div>
              ) : null}
            </div>
          </section>

          <section className="log-section border-t border-black/10 bg-[#17191e] text-white">
            <div className="log-header flex h-12 items-center justify-between gap-2 px-4">
              <button
                className="log-toggle flex min-w-0 flex-1 items-center gap-2 border-0 bg-transparent p-0 text-left text-sm font-semibold text-white"
                type="button"
                data-testid="log-toggle"
                onClick={() => void toggleLogExpanded()}
                aria-expanded={logExpanded}
              >
                {logExpanded ? <ChevronUp size={18} /> : <ChevronDown size={18} />}
                <span className="log-title">Log</span>
              </button>
              <div className="log-actions flex shrink-0 items-center gap-2">
                <button className="icon-dark" onClick={() => setLogs([])} title="Clear log" aria-label="Clear log">
                  <RefreshCw size={16} />
                </button>
                <button className="icon-dark" onClick={() => void saveLog()} title="Save log" aria-label="Save log">
                  <Save size={16} />
                </button>
              </div>
            </div>

            {logExpanded ? (
              <div className="log-body overflow-auto border-t border-white/10 p-3 font-mono text-xs leading-5">
                {logs.length === 0 ? <div className="text-white/35">Waiting for output</div> : null}
                {logs.map((line) => (
                  <div key={line.id} className={clsx("log-line", line.level, line.kind)}>
                    {line.text}
                  </div>
                ))}
                <div ref={logEndRef} />
              </div>
            ) : null}
          </section>

          {duplicateIsoMatch ? (
            <DuplicateIsoMatchModal
              match={duplicateIsoMatch}
              deleting={deletingDuplicateIso}
              onClose={() => setDuplicateIsoMatch(null)}
              onDelete={() => void deleteDuplicateIso()}
            />
          ) : null}
        </main>
      </div>
    </Tooltip.Provider>
  );
}

function defaultOptionState(): OptionState {
  return OPTIONS.reduce<OptionState>((state, option) => {
    state[option.flag] = defaultSelectedOption(option);
    return state;
  }, {});
}

function initialThemeMode(): ThemeMode {
  const stored = localStorage.getItem(THEME_STORAGE_KEY);
  return isThemeMode(stored) ? stored : "system";
}

function isThemeMode(value: unknown): value is ThemeMode {
  return value === "light" || value === "dark" || value === "system";
}

function prefersDarkTheme() {
  return Boolean(window.matchMedia?.("(prefers-color-scheme: dark)").matches);
}

function measuredChildrenHeight(element: HTMLElement | null) {
  if (!element) {
    return null;
  }

  return Array.from(element.children).reduce((height, child) => height + child.getBoundingClientRect().height, 0);
}

function clampWindowHeight(height: number) {
  return Math.min(Math.max(height, MAIN_MIN_WINDOW_HEIGHT), MAIN_MAX_WINDOW_HEIGHT);
}

function useSyncedState<T>(key: string, fallback: T | (() => T)) {
  const getFallback = () => (typeof fallback === "function" ? (fallback as () => T)() : fallback);
  const [value, setValue] = useState<T>(() => readSyncedState(key, getFallback()));

  useEffect(() => {
    localStorage.setItem(key, JSON.stringify(value));
  }, [key, value]);

  useEffect(() => {
    function handleStorage(event: StorageEvent) {
      if (event.key !== key || event.newValue === null) {
        return;
      }
      try {
        setValue(JSON.parse(event.newValue) as T);
      } catch {
        setValue(getFallback());
      }
    }

    window.addEventListener("storage", handleStorage);
    return () => window.removeEventListener("storage", handleStorage);
  }, [key]);

  return [value, setValue] as const;
}

function readSyncedState<T>(key: string, fallback: T) {
  const stored = localStorage.getItem(key);
  if (!stored) {
    return fallback;
  }
  try {
    return JSON.parse(stored) as T;
  } catch {
    return fallback;
  }
}

function defaultSelectedOption(option: OptionSpec) {
  return {
    enabled: Boolean(option.defaultEnabled),
    value: option.defaultValue ?? ""
  };
}

function applyCompatibleAllDrivesPreset(state: OptionState): OptionState {
  return {
    ...state,
    "--drive-type": { enabled: true, value: "GENERIC" },
    "--force-split": { enabled: true, value: state["--force-split"]?.value ?? "" },
    "--leave-unchanged": { enabled: true, value: state["--leave-unchanged"]?.value ?? "" },
    "--skeleton": { enabled: false, value: state["--skeleton"]?.value ?? "" },
    "--retries": { enabled: false, value: state["--retries"]?.value ?? "100" }
  };
}

function clearCompatibleAllDrivesPreset(state: OptionState): OptionState {
  return {
    ...state,
    "--drive-type": { enabled: false, value: state["--drive-type"]?.value ?? "GENERIC" },
    "--force-split": { enabled: false, value: state["--force-split"]?.value ?? "" },
    "--leave-unchanged": { enabled: false, value: state["--leave-unchanged"]?.value ?? "" },
    "--skeleton": { enabled: true, value: state["--skeleton"]?.value ?? "" },
    "--retries": { enabled: true, value: state["--retries"]?.value ?? "100" }
  };
}

function buildExistingImageRunRequest(
  command: ExistingImageActionCommand,
  candidate: ExistingImageCandidate | null,
  baseRequest: RunRequest,
  drive: string
): RunRequest | null {
  if (!candidateSupportsImageAction(command, candidate)) {
    return null;
  }

  const usesDrive = command === "refine";
  return {
    ...baseRequest,
    command,
    driveMode: usesDrive && drive ? "manual" : "auto",
    drive: usesDrive ? drive || undefined : undefined,
    imagePath: candidate.directory,
    imageName: candidate.imageName,
    options: baseRequest.options.filter((option) => IMAGE_ACTION_ALLOWED_FLAGS[command].has(option.flag)),
    outputSubfolder: false,
    compressLogFiles: false,
    dumpTwiceCompareHashes: false,
    dangerConfirmed: false
  };
}

function candidateSupportsImageAction(
  command: ExistingImageActionCommand,
  candidate: ExistingImageCandidate | null
): candidate is ExistingImageCandidate {
  if (!candidate) {
    return false;
  }
  if (command === "refine") {
    return candidate.supportsRefine;
  }
  if (command === "split") {
    return candidate.supportsSplit;
  }
  if (command === "dvdisokey") {
    return candidate.supportsHash && candidateHasIso(candidate);
  }
  return candidate.supportsHash;
}

function candidateHasIso(candidate: ExistingImageCandidate) {
  return candidate.files.some((file) => file.toLowerCase().endsWith(".iso"));
}

function buildDiscActionRunRequest(
  command: "dump::extra" | "dvdkey",
  baseRequest: RunRequest,
  drive: string,
  allowedFlags: Set<string>,
  overrides: Partial<Pick<RunRequest, "imageName" | "compressLogFiles">> = {}
): RunRequest {
  return {
    ...baseRequest,
    command,
    driveMode: drive ? "manual" : "auto",
    drive: drive || undefined,
    imageName: overrides.imageName,
    options: baseRequest.options.filter((option) => allowedFlags.has(option.flag)),
    outputSubfolder: command === "dump::extra" ? baseRequest.outputSubfolder : false,
    compressLogFiles: overrides.compressLogFiles ?? baseRequest.compressLogFiles,
    dumpTwiceCompareHashes: false,
    dangerConfirmed: false
  };
}

function buildFirmwareRunRequest(
  baseRequest: RunRequest,
  drive: string,
  command: FirmwareCommandId,
  firmwarePath: string,
  forceFlash: boolean,
  confirmed: boolean
): RunRequest {
  return {
    ...baseRequest,
    command,
    driveMode: drive ? "manual" : "auto",
    drive: drive || undefined,
    imagePath: undefined,
    imageName: undefined,
    options: [
      ...baseRequest.options.filter((option) => FIRMWARE_ALLOWED_FLAGS.has(option.flag)),
      {
        flag: "--firmware",
        enabled: true,
        value: firmwarePath
      },
      {
        flag: "--force-flash",
        enabled: forceFlash
      }
    ],
    workingDirectory: undefined,
    outputSubfolder: false,
    compressLogFiles: false,
    dumpTwiceCompareHashes: false,
    dangerConfirmed: confirmed
  };
}

function buildDriveTestRunRequest(baseRequest: RunRequest, drive: string): RunRequest {
  return {
    ...baseRequest,
    command: "drive::test",
    driveMode: drive ? "manual" : "auto",
    drive: drive || undefined,
    imagePath: undefined,
    imageName: undefined,
    options: baseRequest.options.filter((option) => DRIVE_TEST_ALLOWED_FLAGS.has(option.flag)),
    workingDirectory: undefined,
    outputSubfolder: false,
    compressLogFiles: false,
    dumpTwiceCompareHashes: false,
    dangerConfirmed: false
  };
}

function buildRingsRunRequest(baseRequest: RunRequest, drive: string): RunRequest {
  return {
    ...baseRequest,
    command: "rings",
    driveMode: drive ? "manual" : "auto",
    drive: drive || undefined,
    options: baseRequest.options.filter((option) => RINGS_ALLOWED_FLAGS.has(option.flag)),
    dumpTwiceCompareHashes: false,
    dangerConfirmed: false
  };
}

function ThemeOptionRow({ value, onChange }: { value: ThemeMode; onChange: (value: ThemeMode) => void }) {
  return (
    <div className="option-row">
      <label className="option-toggle" htmlFor="theme-mode">
        <span className="option-label">Theme</span>
      </label>
      <select
        id="theme-mode"
        value={value}
        onChange={(event) => onChange(event.target.value as ThemeMode)}
        className="control compact-control"
      >
        <option value="system">System</option>
        <option value="light">Light</option>
        <option value="dark">Dark</option>
      </select>
    </div>
  );
}

function CompressLogOptionRow({
  checked,
  disabled,
  onChange
}: {
  checked: boolean;
  disabled: boolean;
  onChange: (checked: boolean) => void;
}) {
  return (
    <div className={clsx("option-row", checked && "enabled", disabled && "opacity-55")}>
      <label className="option-toggle">
        <input
          type="checkbox"
          checked={checked}
          disabled={disabled}
          onChange={(event) => onChange(event.target.checked)}
          className="accent-checkbox h-3.5 w-3.5 shrink-0"
        />
        <span className="option-label">Archive auxiliary files</span>
      </label>
      <code className="option-flag">7z / zip fallback</code>
    </div>
  );
}

function ArchiveToolOptionRow({
  value,
  disabled,
  onChange,
  onChoose
}: {
  value: string;
  disabled: boolean;
  onChange: (value: string) => void;
  onChoose: () => void;
}) {
  return (
    <div className={clsx("option-row archive-tool-row", value && "enabled", disabled && "opacity-55")}>
      <label className="option-label-group">
        <span className="option-label">7z Binary</span>
        <span className="option-description">Optional. Auto-detects 7z/7zz/7za; falls back to .zip when unavailable.</span>
      </label>
      <div className="archive-tool-control">
        <input
          className="control"
          value={value}
          disabled={disabled}
          onChange={(event) => onChange(event.target.value)}
          placeholder="Auto-detect"
          spellCheck={false}
        />
        <IconButton title="Select 7z binary" disabled={disabled} onClick={onChoose}>
          <FolderOpen size={16} />
        </IconButton>
      </div>
    </div>
  );
}

function DumpTwiceCompareOptionRow({
  checked,
  disabled,
  onChange
}: {
  checked: boolean;
  disabled: boolean;
  onChange: (checked: boolean) => void;
}) {
  return (
    <div className={clsx("option-row", checked && !disabled && "enabled", disabled && "opacity-55")}>
      <label className="option-toggle">
        <input
          type="checkbox"
          checked={checked && !disabled}
          disabled={disabled}
          onChange={(event) => onChange(event.target.checked)}
          className="accent-checkbox h-3.5 w-3.5 shrink-0"
        />
        <span className="option-label">Dump Twice, Compare Hashes</span>
      </label>
      <code className="option-flag">second dump verify</code>
    </div>
  );
}

function DriveTestActionRow({ running, onRun }: { running: boolean; onRun: () => void }) {
  return <AdvancedActionRow label="Run Drive Test" code="drive::test" running={running} onRun={onRun} />;
}

function RingsActionRow({ running, onRun }: { running: boolean; onRun: () => void }) {
  return <AdvancedActionRow label="Run Rings" code="rings" running={running} onRun={onRun} />;
}

function AdvancedActionRow({
  label,
  code,
  running,
  disabled,
  onRun
}: {
  label: string;
  code: string;
  running: boolean;
  disabled?: boolean;
  onRun: () => void;
}) {
  return (
    <div className="option-action-row">
      <button className="secondary-button workflow-action-button" disabled={running || disabled} onClick={onRun}>
        {label}
      </button>
      <code className="option-flag">{code}</code>
    </div>
  );
}

function FirmwareFlashSection({
  commandId,
  firmwarePath,
  forceFlash,
  confirmed,
  running,
  onCommandChange,
  onFirmwarePathChange,
  onForceFlashChange,
  onConfirmedChange,
  onChooseFirmware,
  onRun
}: {
  commandId: FirmwareCommandId;
  firmwarePath: string;
  forceFlash: boolean;
  confirmed: boolean;
  running: boolean;
  onCommandChange: (command: FirmwareCommandId) => void;
  onFirmwarePathChange: (path: string) => void;
  onForceFlashChange: (checked: boolean) => void;
  onConfirmedChange: (checked: boolean) => void;
  onChooseFirmware: () => void;
  onRun: () => void;
}) {
  const flashDisabled = running || !confirmed || !firmwarePath.trim();

  return (
    <section className="advanced-card firmware-card rounded-md border">
      <h3 className="advanced-card-title px-2.5 py-1.5 text-sm font-semibold">Firmware</h3>
      <div className="option-section-grid">
        <div className="option-row">
          <label className="option-toggle" htmlFor="firmware-command">
            <span className="option-label">Flash Type</span>
          </label>
          <select
            id="firmware-command"
            value={commandId}
            onChange={(event) => onCommandChange(event.target.value as FirmwareCommandId)}
            className="control compact-control"
          >
            {FIRMWARE_COMMANDS.map((command) => (
              <option key={command.id} value={command.id}>
                {command.label}
              </option>
            ))}
          </select>
        </div>

        <div className="option-row">
          <label className="option-toggle">
            <span className="option-label">Firmware File</span>
          </label>
          <button className="path-button compact-path-button" type="button" title={firmwarePath || "Choose firmware file"} onClick={onChooseFirmware}>
            <FolderOpen size={15} />
            <span>{firmwarePath || "Choose file"}</span>
          </button>
        </div>

        <div className={clsx("option-row", forceFlash && "enabled")}>
          <label className="option-toggle">
            <input
              type="checkbox"
              checked={forceFlash}
              onChange={(event) => onForceFlashChange(event.target.checked)}
              className="h-3.5 w-3.5 shrink-0 accent-copper"
            />
            <span className="option-label">Force Flash</span>
            <Zap size={14} className="shrink-0 text-copper" />
          </label>
          <code className="option-flag">--force-flash</code>
        </div>

        <div className={clsx("option-row", confirmed && "enabled")}>
          <label className="option-toggle">
            <input
              type="checkbox"
              checked={confirmed}
              onChange={(event) => onConfirmedChange(event.target.checked)}
              className="h-3.5 w-3.5 shrink-0 accent-copper"
            />
            <span className="option-label">Confirm Firmware Flash</span>
          </label>
          <code className="option-flag">required</code>
        </div>

        <AdvancedActionRow label="Flash Firmware" code={commandId} running={running} disabled={flashDisabled} onRun={onRun} />
      </div>
    </section>
  );
}

function DuplicateIsoMatchModal({
  match,
  deleting,
  onClose,
  onDelete
}: {
  match: DuplicateIsoMatch;
  deleting: boolean;
  onClose: () => void;
  onDelete: () => void;
}) {
  return (
    <div className="modal-backdrop" role="presentation">
      <div className="match-modal" role="dialog" aria-modal="true" aria-labelledby="duplicate-iso-match-title">
        <h2 id="duplicate-iso-match-title">Both Dumps Match</h2>
        <p>{match.message}</p>
        <code title={match.path}>{match.path}</code>
        <div className="modal-actions">
          <button className="secondary-button modal-button" type="button" disabled={deleting} onClick={onClose}>
            Close
          </button>
          <button className="primary-button modal-button" type="button" disabled={deleting} onClick={onDelete}>
            Delete Duplicate .iso
          </button>
        </div>
      </div>
    </div>
  );
}

function EjectIcon() {
  return (
    <svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" width="24" height="24" fill="currentColor" aria-hidden="true">
      <path d="M10.9 4.8 C11.5 4 12.5 4 13.1 4.8 L19.6 13 C20.3 13.9 19.6 15 18.5 15 H5.5 C4.4 15 3.7 13.9 4.4 13 Z" />
      <rect x="4.5" y="17" width="15" height="3.5" rx="1.75" />
    </svg>
  );
}

function SettingsRow({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div className="settings-row">
      <div className="settings-label">{label}</div>
      <div className="settings-control">{children}</div>
    </div>
  );
}

function OptionRow({
  option,
  state,
  onChange,
  onChooseFile
}: {
  option: OptionSpec;
  state: { enabled: boolean; value?: string };
  onChange: (state: { enabled: boolean; value?: string }) => void;
  onChooseFile: (setter: (value: string) => void) => Promise<void>;
}) {
  const enabled = state.enabled;
  const value = state.value ?? option.defaultValue ?? "";

  return (
    <div className={clsx("option-row", enabled && "enabled")}>
      <label className="option-toggle">
        <input
          type="checkbox"
          checked={enabled}
          onChange={(event) => onChange({ ...state, enabled: event.target.checked })}
          className="accent-checkbox h-3.5 w-3.5 shrink-0"
        />
        <span className="option-label">{option.label}</span>
        {option.danger === "dangerous" ? <Zap size={14} className="shrink-0 text-copper" /> : null}
      </label>

      {option.type === "boolean" ? (
        <code className="option-flag">{option.flag}</code>
      ) : option.type === "enum" ? (
        <select
          disabled={!enabled}
          value={value}
          onChange={(event) => onChange({ ...state, value: event.target.value })}
          className="control compact-control"
        >
          <option value="">Select</option>
          {option.values?.map((item) => (
            <option key={item} value={item}>
              {item}
            </option>
          ))}
        </select>
      ) : option.type === "path" ? (
        <div className="grid grid-cols-[1fr_auto] gap-2">
          <input
            disabled={!enabled}
            value={value}
            onChange={(event) => onChange({ ...state, value: event.target.value })}
            className="control compact-control"
          />
          <IconButton title="Choose file" disabled={!enabled} onClick={() => void onChooseFile((next) => onChange({ ...state, value: next }))}>
            <FolderOpen size={16} />
          </IconButton>
        </div>
      ) : (
        <input
          disabled={!enabled}
          value={value}
          type={option.type === "number" ? "number" : "text"}
          placeholder={option.placeholder}
          onChange={(event) => onChange({ ...state, value: event.target.value })}
          className="control compact-control"
        />
      )}
    </div>
  );
}

function IconButton({
  title,
  children,
  disabled,
  className,
  onClick
}: {
  title: string;
  children: React.ReactNode;
  disabled?: boolean;
  className?: string;
  onClick: () => void;
}) {
  return (
    <Tooltip.Root>
      <Tooltip.Trigger asChild>
        <button className={clsx("icon-button", className)} type="button" disabled={disabled} onClick={onClick} aria-label={title}>
          {children}
        </button>
      </Tooltip.Trigger>
      <Tooltip.Portal>
        <Tooltip.Content className="tooltip" sideOffset={6}>
          {title}
          <Tooltip.Arrow className="fill-ink" />
        </Tooltip.Content>
      </Tooltip.Portal>
    </Tooltip.Root>
  );
}

function Metric({ label, value, inactive = false }: { label: string; value: React.ReactNode; inactive?: boolean }) {
  return (
    <div className={clsx("metric min-w-0 rounded px-2 py-1", inactive && "is-inactive")}>
      <div className="metric-label truncate uppercase">{label}</div>
      <div className="metric-value truncate">{value}</div>
    </div>
  );
}

function LbaValue({ progress }: { progress: RunEvent["progress"] | null }) {
  const current = progress?.lbaCurrent ?? 0;
  const total = progress?.lbaTotal ?? 0;

  return (
    <div className="lba-value" aria-label={`LBA ${current} of ${total}`}>
      <span>{current}</span>
      <span>{total}</span>
    </div>
  );
}

function SpeedometerMetric({ speed, stage, running, progressPercent }: { speed: string; stage: string; running: boolean; progressPercent: number }) {
  const activePercent = running ? 80 : Math.min(100, Math.max(0, progressPercent));
  const needleAngle = running ? -58 + activePercent * 1.16 : -58;
  const speedLabel = speed ? `${speed}x` : "48x";
  const statusText = running ? "Dumping in Progress..." : stage;

  return (
    <div
      className={clsx("metric speedometer-metric min-w-0 rounded px-2 py-1", running && "is-running")}
      style={{ "--needle-angle": `${needleAngle}deg` } as CSSProperties}
    >
      <div className="speedometer-main">
        <div className="speedometer-dial" aria-hidden="true">
          <span className="speedometer-tick tick-left" />
          <span className="speedometer-tick tick-mid" />
          <span className="speedometer-tick tick-right" />
          <span className="speedometer-needle" />
          <span className="speedometer-hub" />
        </div>
        <div className="speedometer-readout">
          <div className="metric-value truncate font-semibold">{speedLabel}</div>
        </div>
      </div>
      <div className="speedometer-status">{statusText}</div>
    </div>
  );
}

function suggestImageName(label: string, stamp: string) {
  const normalized = label
    .replace(/^\/dev\//, "")
    .replace(/^[A-Z]:\\?$/i, (drive) => drive.replace(":", ""))
    .replace(/[^a-z0-9]+/gi, "_")
    .replace(/^_+|_+$/g, "")
    .toLowerCase();
  return `${normalized || "disc"}_${stamp}`;
}

function formatDateStamp(date: Date) {
  const pad = (value: number) => String(value).padStart(2, "0");
  return `${date.getFullYear()}${pad(date.getMonth() + 1)}${pad(date.getDate())}-${pad(date.getHours())}${pad(date.getMinutes())}`;
}

function driveFallbackLabel(_platform: string) {
  return "None";
}
