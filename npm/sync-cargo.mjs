// Gives the crate the npm package's version, so `prolix --version` matches what npm installs.
import fs from "node:fs";

const { version } = JSON.parse(fs.readFileSync("npm/cli/package.json", "utf8"));
for (const [file, re] of [
  ["Cargo.toml", /^(version = )".*"/m],
  ["Cargo.lock", /(name = "prolix"\nversion = )".*"/],
]) {
  fs.writeFileSync(file, fs.readFileSync(file, "utf8").replace(re, `$1"${version}"`));
}
