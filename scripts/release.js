import { execFileSync } from "node:child_process";
import { readFileSync, writeFileSync } from "node:fs";
import { createInterface } from "node:readline/promises";
import { stdin as input, stdout as output } from "node:process";

const tauriConfigPath = "src-tauri/tauri.conf.json";
const cargoTomlPath = "src-tauri/Cargo.toml";
const packageJsonPath = "package.json";

const readJson = (path) => JSON.parse(readFileSync(path, "utf8"));
const writeJson = (path, value) => {
  writeFileSync(path, `${JSON.stringify(value, null, 2)}\n`);
};

const bumpVersion = (version, bump) => {
  const parts = version.split(".").map(Number);
  if (parts.length !== 3 || parts.some((part) => !Number.isInteger(part))) {
    throw new Error(`Current version is not semver: ${version}`);
  }

  const [major, minor, patch] = parts;
  if (bump === "major") return `${major + 1}.0.0`;
  if (bump === "minor") return `${major}.${minor + 1}.0`;
  if (bump === "patch") return `${major}.${minor}.${patch + 1}`;
  if (/^\d+\.\d+\.\d+$/.test(bump)) return bump;

  throw new Error("Use patch, minor, major, or a custom X.Y.Z version.");
};

const updateCargoPackageVersion = (path, version) => {
  const inputText = readFileSync(path, "utf8");
  const lines = inputText.split("\n");
  let inPackage = false;
  let updated = false;

  const outputLines = lines.map((line) => {
    if (/^\s*\[.+\]\s*$/.test(line)) {
      inPackage = line.trim() === "[package]";
    }

    if (inPackage && !updated && /^\s*version\s*=/.test(line)) {
      updated = true;
      return `version = "${version}"`;
    }

    return line;
  });

  if (!updated) {
    throw new Error("Could not find [package] version in src-tauri/Cargo.toml");
  }

  writeFileSync(path, outputLines.join("\n"));
};

const main = async () => {
  const tauriConfig = readJson(tauriConfigPath);
  const currentVersion = tauriConfig.version;
  const rl = createInterface({ input, output });
  const answer = await rl.question(
    `New version from ${currentVersion} (patch/minor/major or X.Y.Z) [patch]: `,
  );
  rl.close();

  const nextVersion = bumpVersion(currentVersion, answer.trim() || "patch");
  tauriConfig.version = nextVersion;
  writeJson(tauriConfigPath, tauriConfig);

  const packageJson = readJson(packageJsonPath);
  packageJson.version = nextVersion;
  writeJson(packageJsonPath, packageJson);
  updateCargoPackageVersion(cargoTomlPath, nextVersion);

  execFileSync("git", ["add", tauriConfigPath, packageJsonPath, cargoTomlPath], { stdio: "inherit" });
  execFileSync("git", ["commit", "-m", `chore: release v${nextVersion}`], { stdio: "inherit" });
  execFileSync("git", ["tag", `v${nextVersion}`], { stdio: "inherit" });

  console.log("Release commit and tag created.");
  console.log("Run: git push && git push --tags");
};

main().catch((error) => {
  console.error(error instanceof Error ? error.message : error);
  process.exit(1);
});
