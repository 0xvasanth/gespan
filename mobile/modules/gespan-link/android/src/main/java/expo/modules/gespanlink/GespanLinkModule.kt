package expo.modules.gespanlink

import expo.modules.kotlin.modules.Module
import expo.modules.kotlin.modules.ModuleDefinition

class GespanLinkModule : Module() {
  companion object { init { System.loadLibrary("gespan_mobile_link") } }
  private external fun nativeCall(input: String): String
  override fun definition() = ModuleDefinition {
    Name("GespanLink")
    AsyncFunction("execute") { input: String -> nativeCall(input) }
  }
}
