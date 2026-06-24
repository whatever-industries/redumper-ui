export type DangerLevel = "normal" | "advanced" | "dangerous";
export type OptionType = "boolean" | "string" | "number" | "enum" | "path";
export type ArchiveFormat = "sevenZip" | "zip";

export interface CommandSpec {
  id: string;
  label: string;
  category: "Dumping" | "Analysis" | "Drive" | "Firmware" | "Debug";
  danger: DangerLevel;
  requiresDrive: boolean;
  requiresImageName: boolean;
  writesFiles: boolean;
}

export interface OptionSpec {
  flag: string;
  label: string;
  type: OptionType;
  group: "General" | "Drive" | "CD Dump" | "DVD/BD" | "Split" | "Offset" | "Drive Test" | "Firmware" | "Advanced";
  values?: string[];
  defaultValue?: string;
  defaultEnabled?: boolean;
  placeholder?: string;
  danger?: DangerLevel;
}

export interface SelectedOption {
  enabled: boolean;
  value?: string;
}

export type OptionState = Record<string, SelectedOption>;

export interface RunRequest {
  command: string;
  options: Array<{
    flag: string;
    value?: string;
    enabled: boolean;
  }>;
  driveMode: "auto" | "manual";
  drive?: string;
  driveSpeed?: string;
  imagePath?: string;
  imageName?: string;
  workingDirectory?: string;
  manualCommand?: string;
  outputSubfolder: boolean;
  archiveToolPath?: string;
  compressLogFiles: boolean;
  archiveFormat: ArchiveFormat;
  dumpTwiceCompareHashes: boolean;
  dangerConfirmed: boolean;
}

export interface ProgressEvent {
  percentage?: number;
  lbaCurrent?: number;
  lbaTotal?: number;
  scsiErrors?: number;
  c2Errors?: number;
  qErrors?: number;
  edcErrors?: number;
}

export interface RunEvent {
  runId: string;
  kind: "started" | "stdout" | "stderr" | "stage" | "progress" | "warning" | "error" | "exit";
  stream?: string;
  line?: string;
  stage?: string;
  progress?: ProgressEvent;
  exitCode?: number;
  message?: string;
  duplicateIsoPath?: string;
}

export interface AppInfo {
  appVersion: string;
  upstreamTag: string;
  upstreamAppVersion: string;
  platform: string;
  arch: string;
  defaultOutputDir: string;
  redumperPath: string;
  redumperAvailable: boolean;
  resourceDir: string;
  diagnostics: Array<{ level: "info" | "warning" | "error"; message: string }>;
}

export interface DriveCandidate {
  path: string;
  label: string;
  source: string;
  volumeName?: string | null;
  mediaKind?: "cd" | "dvd" | "bd" | "unknown";
  redumpCompliant: boolean;
  genericModeRequired: boolean;
}

export interface ExistingImageCandidate {
  directory: string;
  imageName: string;
  files: string[];
  supportsRefine: boolean;
  supportsSplit: boolean;
  supportsHash: boolean;
}

export interface ExistingOutputConflict {
  exists: boolean;
  directory: string;
  matches: string[];
}

export interface UpdateCheckResult {
  available: boolean;
  currentVersion: string;
  latestVersion?: string;
  body?: string;
  date?: string;
  downloadUrl?: string;
  message: string;
}
