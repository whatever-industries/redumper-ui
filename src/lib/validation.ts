import { getCommand, OPTIONS } from "./commands";
import type { ArchiveFormat, OptionState, RunRequest } from "./schema";

export function buildRunRequest(params: {
  command: string;
  optionState: OptionState;
  driveMode: "auto" | "manual";
  drive: string;
  driveSpeed: string;
  imagePath: string;
  imageName: string;
  workingDirectory: string;
  manualCommand?: string;
  outputSubfolder?: boolean;
  archiveToolPath?: string;
  archiveFormat?: ArchiveFormat;
  compressLogFiles: boolean;
  dumpTwiceCompareHashes: boolean;
  dangerConfirmed: boolean;
}): RunRequest {
  const options = OPTIONS.filter((spec) => spec.flag !== "--speed")
    .map((spec) => ({
      flag: spec.flag,
      enabled: Boolean(params.optionState[spec.flag]?.enabled),
      value: params.optionState[spec.flag]?.value
    }))
    .filter((option) => option.enabled);

  if (params.driveSpeed.trim()) {
    options.unshift({
      flag: "--speed",
      enabled: true,
      value: params.driveSpeed.trim()
    });
  }

  return {
    command: params.command,
    options,
    driveMode: params.driveMode,
    drive: params.drive || undefined,
    driveSpeed: params.driveSpeed.trim() || undefined,
    imagePath: params.imagePath || undefined,
    imageName: params.imageName || undefined,
    workingDirectory: params.workingDirectory || undefined,
    manualCommand: params.manualCommand?.trim() || undefined,
    outputSubfolder: params.outputSubfolder ?? true,
    archiveToolPath: params.archiveToolPath?.trim() || undefined,
    compressLogFiles: params.compressLogFiles,
    archiveFormat: params.archiveFormat ?? "sevenZip",
    dumpTwiceCompareHashes: params.dumpTwiceCompareHashes,
    dangerConfirmed: params.dangerConfirmed
  };
}

export function validateRunRequest(request: RunRequest): string[] {
  const errors: string[] = [];
  const manual = parseManualCommand(request.manualCommand);
  const commandId = manual.command ?? request.command;
  const command = getCommand(commandId);

  if (command.requiresImageName && !request.imageName?.trim()) {
    errors.push(`${command.label} requires an image name.`);
  }
  if (command.requiresDrive && request.driveMode === "manual" && !request.drive?.trim()) {
    errors.push("Manual drive mode requires a drive path.");
  }
  if (request.driveSpeed?.trim() && Number.isNaN(Number(request.driveSpeed))) {
    errors.push("Drive speed must be numeric.");
  }
  if (command.danger === "dangerous" && !request.dangerConfirmed) {
    errors.push("Firmware flashing requires confirmation.");
  }
  if (request.dumpTwiceCompareHashes && !["disc", "dump"].includes(request.command)) {
    errors.push("Dump Twice if No Match is only available for Disc and Dump.");
  }
  if (request.dumpTwiceCompareHashes && request.manualCommand?.trim()) {
    errors.push("Manual command editing is not available with Dump Twice if No Match.");
  }
  if (request.dumpTwiceCompareHashes && !request.imageName?.trim()) {
    errors.push("Dump Twice if No Match requires an image name.");
  }
  errors.push(...manual.errors);

  for (const selected of request.options) {
    const spec = OPTIONS.find((option) => option.flag === selected.flag);
    if (!spec) {
      errors.push(`Unsupported option: ${selected.flag}`);
      continue;
    }
    if ((spec.type === "number" || spec.type === "string" || spec.type === "enum" || spec.type === "path") && !selected.value?.trim()) {
      errors.push(`${spec.label} requires a value.`);
    }
    if (spec.type === "number" && selected.value?.trim() && Number.isNaN(Number(selected.value))) {
      errors.push(`${spec.label} must be numeric.`);
    }
    if (spec.type === "enum" && selected.value && spec.values && !spec.values.includes(selected.value)) {
      errors.push(`${spec.label} has an invalid value.`);
    }
  }

  return errors;
}

export function commandPreview(request: RunRequest): string {
  if (request.manualCommand?.trim()) {
    return request.manualCommand;
  }

  const args = commandArgs(request, request.imageName);
  const preview = ["redumper", ...args].map(quoteArg).join(" ");
  if (!request.dumpTwiceCompareHashes || !request.imageName?.trim()) {
    return preview;
  }

  const verifyArgs = commandArgs(request, `${request.imageName}_verify`);
  const verifyPreview = ["redumper", ...verifyArgs].map(quoteArg).join(" ");
  return `${preview} && check redump.info CRC32; if no match: ${verifyPreview} && compare SHA-256`;
}

function parseManualCommand(command?: string): { command?: string; errors: string[] } {
  const text = command?.trim();
  if (!text) {
    return { errors: [] };
  }

  const tokens = splitCommandLine(text);
  if (!tokens.length) {
    return { errors: ["Manual command is empty."] };
  }

  const args = tokens[0] === "redumper" ? tokens.slice(1) : tokens;
  const commandId = args[0];
  if (!commandId) {
    return { errors: ["Manual command must include a redumper command."] };
  }

  try {
    getCommand(commandId);
  } catch {
    return { errors: [`Unsupported redumper command: ${commandId}`] };
  }

  const errors: string[] = [];
  for (const arg of args.slice(1)) {
    if (!arg.startsWith("--")) {
      errors.push(`Manual command argument must be an option: ${arg}`);
      continue;
    }
    const flag = arg.split("=")[0];
    if (
      !OPTIONS.some((option) => option.flag === flag) &&
      !["--help", "--version", "--list-recommended-drives", "--list-all-drives", "--drive", "--image-path", "--image-name"].includes(flag)
    ) {
      errors.push(`Unsupported redumper option: ${flag}`);
    }
  }

  return { command: commandId, errors };
}

function splitCommandLine(input: string): string[] {
  const tokens: string[] = [];
  let token = "";
  let quote: '"' | "'" | null = null;

  for (let index = 0; index < input.length; index += 1) {
    const char = input[index];
    if (quote) {
      if (char === quote) {
        quote = null;
      } else {
        token += char;
      }
      continue;
    }

    if (char === '"' || char === "'") {
      quote = char;
      continue;
    }
    if (/\s/.test(char)) {
      if (token) {
        tokens.push(token);
        token = "";
      }
      continue;
    }
    token += char;
  }

  if (token) {
    tokens.push(token);
  }

  return tokens;
}

function commandArgs(request: RunRequest, imageName?: string): string[] {
  const args = [request.command];
  if (request.driveMode === "manual" && request.drive) {
    args.push(`--drive=${request.drive}`);
  }
  if (request.imagePath) {
    args.push(`--image-path=${effectiveImagePath(request.command, request.imagePath, imageName, request.outputSubfolder)}`);
  }
  if (imageName) {
    args.push(`--image-name=${imageName}`);
  }
  for (const option of request.options) {
    if (option.value?.trim()) {
      args.push(`${option.flag}=${option.value}`);
    } else {
      args.push(option.flag);
    }
  }
  return args;
}

function effectiveImagePath(command: string, imagePath: string, imageName?: string, outputSubfolder = true): string {
  if (!outputSubfolder || !["disc", "dump", "dump::extra"].includes(command) || !imageName?.trim()) {
    return imagePath;
  }

  const separator = imagePath.includes("\\") && !imagePath.includes("/") ? "\\" : "/";
  const trimmedBase = imagePath.replace(/[\\/]+$/, "");
  const folderName = safeOutputFolderName(imageName);
  if (!folderName) {
    return imagePath;
  }
  return `${trimmedBase}${separator}${folderName}`;
}

function safeOutputFolderName(name: string): string {
  return name
    .trim()
    .replace(/[\\/:]/g, "_")
    .replace(/[\u0000-\u001f\u007f]/g, "_")
    .replace(/^\.+|\.+$/g, "")
    .trim();
}

function quoteArg(arg: string): string {
  return /\s/.test(arg) ? JSON.stringify(arg) : arg;
}
