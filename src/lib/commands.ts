import type { CommandSpec, OptionSpec } from "./schema";

export const COMMANDS: CommandSpec[] = [
  { id: "disc", label: "Disc", category: "Dumping", danger: "normal", requiresDrive: true, requiresImageName: false, writesFiles: true },
  { id: "dump", label: "Dump", category: "Dumping", danger: "normal", requiresDrive: true, requiresImageName: false, writesFiles: true },
  { id: "dump::extra", label: "Dump Extra", category: "Dumping", danger: "advanced", requiresDrive: true, requiresImageName: true, writesFiles: true },
  { id: "refine", label: "Refine", category: "Dumping", danger: "normal", requiresDrive: true, requiresImageName: true, writesFiles: true },
  { id: "rings", label: "Rings", category: "Dumping", danger: "advanced", requiresDrive: true, requiresImageName: false, writesFiles: true },
  { id: "split", label: "Split", category: "Analysis", danger: "normal", requiresDrive: false, requiresImageName: true, writesFiles: true },
  { id: "hash", label: "Hash", category: "Analysis", danger: "normal", requiresDrive: false, requiresImageName: true, writesFiles: true },
  { id: "info", label: "Info", category: "Analysis", danger: "normal", requiresDrive: false, requiresImageName: true, writesFiles: true },
  { id: "protection", label: "Protection", category: "Analysis", danger: "normal", requiresDrive: false, requiresImageName: true, writesFiles: true },
  { id: "skeleton", label: "Skeleton", category: "Analysis", danger: "advanced", requiresDrive: false, requiresImageName: true, writesFiles: true },
  { id: "dvdkey", label: "DVD Key", category: "Drive", danger: "advanced", requiresDrive: true, requiresImageName: false, writesFiles: false },
  { id: "dvdisokey", label: "DVD ISO Key", category: "Analysis", danger: "advanced", requiresDrive: false, requiresImageName: true, writesFiles: true },
  { id: "eject", label: "Eject", category: "Drive", danger: "normal", requiresDrive: true, requiresImageName: false, writesFiles: false },
  { id: "drive::test", label: "Drive Test", category: "Drive", danger: "advanced", requiresDrive: true, requiresImageName: false, writesFiles: false },
  { id: "flash::mt1339", label: "Flash MT1339", category: "Firmware", danger: "dangerous", requiresDrive: true, requiresImageName: false, writesFiles: false },
  { id: "flash::mt1959", label: "Flash MT1959", category: "Firmware", danger: "dangerous", requiresDrive: true, requiresImageName: false, writesFiles: false },
  { id: "flash::sd616", label: "Flash SD-616", category: "Firmware", danger: "dangerous", requiresDrive: true, requiresImageName: false, writesFiles: false },
  { id: "flash::plextor", label: "Flash Plextor", category: "Firmware", danger: "dangerous", requiresDrive: true, requiresImageName: false, writesFiles: false },
  { id: "subchannel", label: "Subchannel", category: "Debug", danger: "advanced", requiresDrive: false, requiresImageName: true, writesFiles: true },
  { id: "fixmsf", label: "Fix MSF", category: "Debug", danger: "advanced", requiresDrive: false, requiresImageName: true, writesFiles: true },
  { id: "debug", label: "Debug", category: "Debug", danger: "advanced", requiresDrive: false, requiresImageName: false, writesFiles: false },
  { id: "debug::flip", label: "Debug Flip", category: "Debug", danger: "advanced", requiresDrive: false, requiresImageName: true, writesFiles: true }
];

export const OPTIONS: OptionSpec[] = [
  { flag: "--verbose", label: "Verbose", type: "boolean", group: "General" },
  { flag: "--overwrite", label: "Overwrite", type: "boolean", group: "General", danger: "advanced" },
  { flag: "--auto-eject", label: "Auto Eject", type: "boolean", group: "General" },
  { flag: "--skeleton", label: "Output Skeleton", type: "boolean", group: "General", defaultEnabled: true },
  { flag: "--debug", label: "Debug Mode", type: "boolean", group: "General", danger: "advanced" },
  { flag: "--disc-type", label: "Disc Type", type: "enum", group: "General", values: ["CD", "DVD", "BLURAY", "BLURAY-R", "HD-DVD"] },
  { flag: "--speed", label: "Read Speed", type: "number", group: "Drive", placeholder: "8" },
  { flag: "--retries", label: "Retries", type: "number", group: "Drive", defaultValue: "100", defaultEnabled: true },
  { flag: "--scsi-timeout", label: "SCSI Timeout", type: "number", group: "Drive", defaultValue: "50000" },
  { flag: "--drive-type", label: "Drive Type", type: "enum", group: "Drive", values: ["GENERIC", "PLEXTOR", "MTK2", "MTK2B", "MTK3", "MTK8A", "MTK8B", "MTK8C"] },
  { flag: "--drive-read-offset", label: "Drive Read Offset", type: "number", group: "Drive" },
  { flag: "--drive-c2-shift", label: "Drive C2 Shift", type: "number", group: "Drive" },
  { flag: "--drive-pregap-start", label: "Drive Pregap Start", type: "number", group: "Drive", placeholder: "-135" },
  { flag: "--drive-read-method", label: "Drive Read Method", type: "enum", group: "Drive", values: ["BE", "D8", "BE_CDDA"] },
  { flag: "--drive-sector-order", label: "Drive Sector Order", type: "enum", group: "Drive", values: ["DATA_C2_SUB", "DATA_SUB_C2", "DATA_SUB", "DATA_C2"] },
  { flag: "--refine-subchannel", label: "Refine Subchannel", type: "boolean", group: "CD Dump" },
  { flag: "--refine-sector-mode", label: "Refine Sector Mode", type: "boolean", group: "CD Dump" },
  { flag: "--continue", label: "Continue From", type: "enum", group: "CD Dump", values: ["dump", "dump::extra", "protection", "refine", "dvdkey", "split", "hash", "info", "skeleton"] },
  { flag: "--lba-start", label: "LBA Start", type: "number", group: "CD Dump" },
  { flag: "--lba-end", label: "LBA End", type: "number", group: "CD Dump" },
  { flag: "--lba-end-by-subcode", label: "LBA End By Subcode", type: "boolean", group: "CD Dump" },
  { flag: "--skip", label: "Skip Ranges", type: "string", group: "CD Dump", placeholder: "100-150,200" },
  { flag: "--skip-subcode-desync", label: "Skip Subcode Desync", type: "boolean", group: "CD Dump", danger: "advanced" },
  { flag: "--plextor-skip-leadin", label: "Plextor Skip Lead-In", type: "boolean", group: "CD Dump" },
  { flag: "--plextor-leadin-retries", label: "Plextor Lead-In Retries", type: "number", group: "CD Dump", defaultValue: "4" },
  { flag: "--plextor-leadin-force-store", label: "Plextor Force Store Lead-In", type: "boolean", group: "CD Dump", danger: "advanced" },
  { flag: "--mediatek-skip-leadout", label: "MediaTek Skip Lead-Out", type: "boolean", group: "CD Dump" },
  { flag: "--mediatek-leadout-retries", label: "MediaTek Lead-Out Retries", type: "number", group: "CD Dump", defaultValue: "32" },
  { flag: "--disable-cdtext", label: "Disable CD-TEXT", type: "boolean", group: "CD Dump" },
  { flag: "--overread-leadout", label: "Overread Lead-Out", type: "boolean", group: "CD Dump", danger: "advanced" },
  { flag: "--force-unscrambled", label: "Force Unscrambled", type: "boolean", group: "CD Dump", danger: "advanced" },
  { flag: "--force-refine", label: "Force Refine", type: "boolean", group: "CD Dump", danger: "advanced" },
  { flag: "--cdr-error-threshold", label: "CD-R Error Threshold", type: "number", group: "CD Dump", defaultValue: "16" },
  { flag: "--kreon-partial-ss", label: "Kreon Partial Security Sector", type: "boolean", group: "DVD/BD" },
  { flag: "--dvd-raw", label: "DVD Raw", type: "boolean", group: "DVD/BD", danger: "advanced" },
  { flag: "--bd-raw", label: "BD Raw", type: "boolean", group: "DVD/BD", danger: "advanced" },
  { flag: "--dump-read-size", label: "Dump Read Size", type: "number", group: "DVD/BD" },
  { flag: "--filesystem-trim", label: "Filesystem Trim", type: "boolean", group: "DVD/BD" },
  { flag: "--force-split", label: "Force Split", type: "boolean", group: "Split", danger: "advanced" },
  { flag: "--leave-unchanged", label: "Leave Unchanged", type: "boolean", group: "Split" },
  { flag: "--force-qtoc", label: "Force QTOC", type: "boolean", group: "Split" },
  { flag: "--legacy-subs", label: "Legacy Subs", type: "boolean", group: "Split" },
  { flag: "--skip-fill", label: "Skip Fill", type: "number", group: "Split", defaultValue: "85" },
  { flag: "--force-offset", label: "Force Offset", type: "number", group: "Offset" },
  { flag: "--audio-silence-threshold", label: "Audio Silence Threshold", type: "number", group: "Offset", defaultValue: "32" },
  { flag: "--dump-write-offset", label: "Dump Write Offset", type: "number", group: "Offset" },
  { flag: "--correct-offset-shift", label: "Offset Shift Correction & Uncorrected", type: "boolean", group: "Offset", defaultEnabled: true },
  { flag: "--offset-shift-relocate", label: "Offset Shift Relocate", type: "boolean", group: "Offset" },
  { flag: "--drive-test-skip-plextor-leadin", label: "Skip Plextor Lead-In Test", type: "boolean", group: "Drive Test" },
  { flag: "--drive-test-skip-cache-read", label: "Skip Cache Read Test", type: "boolean", group: "Drive Test" },
  { flag: "--firmware", label: "Firmware", type: "path", group: "Firmware", danger: "dangerous" },
  { flag: "--force-flash", label: "Force Flash", type: "boolean", group: "Firmware", danger: "dangerous" },
  { flag: "--rings", label: "Rings Detection", type: "boolean", group: "Advanced" }
];

export const OPTION_GROUPS = Array.from(new Set(OPTIONS.map((option) => option.group)));

export function getCommand(id: string): CommandSpec {
  return COMMANDS.find((command) => command.id === id) ?? COMMANDS[0];
}

export function groupedCommands() {
  return COMMANDS.reduce<Record<string, CommandSpec[]>>((groups, command) => {
    groups[command.category] ??= [];
    groups[command.category].push(command);
    return groups;
  }, {});
}
