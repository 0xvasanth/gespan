import assert from "node:assert/strict";
import test from "node:test";
import { rpmVersion } from "./build-rpm.mjs";

test("stable versions keep their version and the default release", () => {
  assert.deepEqual(rpmVersion("0.16.0"), { version: "0.16.0", release: "1" });
});

test("prereleases move the label into the release, which rpm allows", () => {
  assert.deepEqual(rpmVersion("0.0.1-beta"), { version: "0.0.1", release: "0.beta" });
  assert.deepEqual(rpmVersion("0.0.1-beta.2"), { version: "0.0.1", release: "0.beta.2" });
  assert.deepEqual(rpmVersion("1.2.3-rc-1"), { version: "1.2.3", release: "0.rc_1" });
  for (const { version, release } of [rpmVersion("0.0.1-beta.2"), rpmVersion("1.2.3-rc-1")]) {
    assert.match(`${version}-${release}`, /^[0-9.]+-[0-9A-Za-z._]+$/);
  }
});

test("anything else is refused before a build starts", () => {
  for (const bad of ["v0.0.1", "0.0.1-", "0.0", "0.0.1-beta+build"]) {
    assert.throws(() => rpmVersion(bad), bad);
  }
});
