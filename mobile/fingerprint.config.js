/** @type {import('@expo/fingerprint').Config} */
module.exports = {
  // Native folders and compiled libraries are regenerated on EAS. Hash their
  // source inputs so local OTA publishing matches a clean cloud build.
  ignorePaths: [
    "ios",
    "ios/**/*",
    "android",
    "android/**/*",
    "modules/rebase-link/ios/*.xcframework",
    "modules/rebase-link/ios/*.xcframework/**/*",
    "modules/rebase-link/android/src/main/jniLibs",
    "modules/rebase-link/android/src/main/jniLibs/**/*",
  ],
  extraSources: [
    { type: "file", filePath: "../Cargo.toml", reasons: ["rebaseRust"] },
    { type: "file", filePath: "../Cargo.lock", reasons: ["rebaseRust"] },
    {
      type: "dir",
      filePath: "../crates/mobile-link",
      reasons: ["rebaseRust"],
    },
    {
      type: "file",
      filePath: "scripts/build-native-link.mjs",
      reasons: ["rebaseRust"],
    },
    {
      type: "file",
      filePath: "scripts/eas-build-pre-install.mjs",
      reasons: ["rebaseRust"],
    },
  ],
};
