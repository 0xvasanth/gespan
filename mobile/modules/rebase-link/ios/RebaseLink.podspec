Pod::Spec.new do |s|
  s.name = 'RebaseLink'
  s.version = '0.1.0'
  s.summary = 'Rebase direct QUIC connection'
  s.description = s.summary
  s.license = { :type => 'MIT' }
  s.author = 'Rebase'
  s.homepage = 'https://github.com/0xvasanth/rebase'
  s.platforms = { :ios => '16.4' }
  s.source = { :git => 'https://github.com/0xvasanth/rebase.git' }
  s.static_framework = true
  s.dependency 'ExpoModulesCore'
  s.source_files = '**/*.swift'
  s.resource_bundles = { 'RebaseTranscriptAssets' => ['Resources/transcript-artwork.json'] }
  s.vendored_frameworks = 'RebaseLinkCore.xcframework'
  s.frameworks = 'Security', 'SystemConfiguration', 'UIKit', 'CoreText'
  s.pod_target_xcconfig = { 'DEFINES_MODULE' => 'YES' }
end