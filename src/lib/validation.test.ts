import { describe, expect, it } from "vitest";
import { buildRunRequest, commandPreview, validateRunRequest } from "./validation";

describe("run request validation", () => {
  it("requires image names for image-based commands", () => {
    const request = buildRunRequest({
      command: "split",
      optionState: {},
      driveMode: "auto",
      drive: "",
      driveSpeed: "",
      imagePath: "/tmp/out",
      imageName: "",
      workingDirectory: "",
      compressLogFiles: true,
      dumpTwiceCompareHashes: false,
      dangerConfirmed: false
    });

    expect(validateRunRequest(request)).toContain("Split requires an image name.");
  });

  it("quotes command preview paths with spaces", () => {
    const request = buildRunRequest({
      command: "disc",
      optionState: { "--verbose": { enabled: true } },
      driveMode: "manual",
      drive: "E:",
      driveSpeed: "8",
      imagePath: "/tmp/Redumper Dumps",
      imageName: "my_disc",
      workingDirectory: "",
      compressLogFiles: true,
      dumpTwiceCompareHashes: false,
      dangerConfirmed: false
    });

    expect(commandPreview(request)).toContain('"--image-path=/tmp/Redumper Dumps/my_disc"');
    expect(commandPreview(request)).toContain("--speed=8");
    expect(commandPreview(request)).toContain("--verbose");
  });

  it("keeps existing-image commands pointed at the selected folder", () => {
    const request = buildRunRequest({
      command: "refine",
      optionState: {},
      driveMode: "manual",
      drive: "disk4",
      driveSpeed: "",
      imagePath: "/tmp/Existing Dump",
      imageName: "my_disc",
      workingDirectory: "",
      compressLogFiles: true,
      dumpTwiceCompareHashes: false,
      dangerConfirmed: false
    });

    expect(commandPreview(request)).toContain('"--image-path=/tmp/Existing Dump"');
    expect(commandPreview(request)).not.toContain('"--image-path=/tmp/Existing Dump/my_disc"');
  });

  it("can keep dump commands pointed at the selected folder for existing image folders", () => {
    const request = buildRunRequest({
      command: "disc",
      optionState: {},
      driveMode: "manual",
      drive: "disk4",
      driveSpeed: "",
      imagePath: "/tmp/Existing Dump",
      imageName: "my_disc",
      workingDirectory: "",
      outputSubfolder: false,
      compressLogFiles: true,
      dumpTwiceCompareHashes: false,
      dangerConfirmed: false
    });

    expect(commandPreview(request)).toContain('"--image-path=/tmp/Existing Dump"');
    expect(commandPreview(request)).not.toContain('"--image-path=/tmp/Existing Dump/my_disc"');
  });

  it("previews the second dump when hash comparison is enabled", () => {
    const request = buildRunRequest({
      command: "dump",
      optionState: {},
      driveMode: "auto",
      drive: "",
      driveSpeed: "",
      imagePath: "/tmp/out",
      imageName: "movie",
      workingDirectory: "",
      compressLogFiles: true,
      dumpTwiceCompareHashes: true,
      dangerConfirmed: false
    });

    expect(commandPreview(request)).toContain("--image-name=movie");
    expect(commandPreview(request)).toContain("--image-name=movie_verify");
    expect(commandPreview(request)).toContain("check redump.info CRC32");
    expect(commandPreview(request)).toContain("compare SHA-256");
  });

  it("accepts an edited redumper command preview", () => {
    const request = buildRunRequest({
      command: "disc",
      optionState: {},
      driveMode: "manual",
      drive: "disk4",
      driveSpeed: "",
      imagePath: "/tmp/out",
      imageName: "movie",
      workingDirectory: "",
      manualCommand: 'redumper dump "--drive=disk4" "--image-path=/tmp/out" --drive-type=GENERIC',
      compressLogFiles: true,
      dumpTwiceCompareHashes: false,
      dangerConfirmed: false
    });

    expect(validateRunRequest(request)).toEqual([]);
    expect(commandPreview(request)).toContain("--drive-type=GENERIC");
  });

  it("rejects manual commands that are not redumper options", () => {
    const request = buildRunRequest({
      command: "disc",
      optionState: {},
      driveMode: "auto",
      drive: "",
      driveSpeed: "",
      imagePath: "/tmp/out",
      imageName: "movie",
      workingDirectory: "",
      manualCommand: "redumper dump && rm -rf /",
      compressLogFiles: true,
      dumpTwiceCompareHashes: false,
      dangerConfirmed: false
    });

    expect(validateRunRequest(request)).toContain("Manual command argument must be an option: &&");
  });
});
