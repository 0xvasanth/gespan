Pod::Spec.new do |s|
  s.name = 'GespanLink'
  s.version = '0.1.0'
  s.summary = 'Gespan direct QUIC connection'
  s.description = s.summary
  s.license = { :type => 'MIT' }
  s.author = 'Gespan'
  s.homepage = 'https://gespan.dev'
  s.platforms = { :ios => '16.4' }
  s.source = { :git => 'https://github.com/hardbeat920/gespan.git' }
  s.static_framework = true
  s.dependency 'ExpoModulesCore'
  s.source_files = '**/*.swift'
  s.resource_bundles = { 'GespanTranscriptAssets' => ['Resources/transcript-artwork.json'] }
  s.vendored_frameworks = 'GespanLinkCore.xcframework'
  s.frameworks = 'Security', 'SystemConfiguration', 'UIKit', 'CoreText'
  s.pod_target_xcconfig = { 'DEFINES_MODULE' => 'YES' }
end
