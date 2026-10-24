// https://docs.expo.dev/guides/using-eslint/
const { defineConfig } = require('eslint/config');
const expoConfig = require("eslint-config-expo/flat");

module.exports = defineConfig([
  expoConfig,
  {
    ignores: ["dist/*"],
  },
  {
    // TypeScript already checks namespace members. The plugin's quick module
    // test misses SDK 58 builds that put `export` after a comment on the same
    // line (expo-notifications), and then reports their enums as missing.
    files: ["**/*.ts", "**/*.tsx"],
    rules: { "import/namespace": "off" },
  },
]);
