package expo.modules.rebaselink

import expo.modules.kotlin.modules.Module
import expo.modules.kotlin.modules.ModuleDefinition

class RebaseLinkModule : Module() {
  companion object { init { System.loadLibrary("rebase_mobile_link") } }
  private external fun nativeCall(input: String): String
  override fun definition() = ModuleDefinition {
    Name("RebaseLink")
    AsyncFunction("execute") { input: String -> nativeCall(input) }
  }
}
