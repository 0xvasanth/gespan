/** @type {import('@expo/fingerprint').Config} */
module.exports = {
  // Native folders and compiled libraries are regenerated on EAS. Hash their
  // source inputs so local OTA publishing matches a clean cloud build.
  ignorePaths: [
    "ios",
    "ios/**/*",
    "android",
    "android/**/*",
    "modules/gespan-link/ios/*.xcframework",
    "modules/gespan-link/ios/*.xcframework/**/*",
    "modules/gespan-link/android/src/main/jniLibs",
    "modules/gespan-link/android/src/main/jniLibs/**/*",
  ],
  extraSources: [
    { type: "file", filePath: "../Cargo.toml", reasons: ["gespanRust"] },
    { type: "file", filePath: "../Cargo.lock", reasons: ["gespanRust"] },
    {
      type: "dir",
      filePath: "../crates/mobile-link",
      reasons: ["gespanRust"],
    },
    {
      type: "file",
      filePath: "scripts/build-native-link.mjs",
      reasons: ["gespanRust"],
    },
    {
      type: "file",
      filePath: "scripts/eas-build-pre-install.mjs",
      reasons: ["gespanRust"],
    },
  ],
};
