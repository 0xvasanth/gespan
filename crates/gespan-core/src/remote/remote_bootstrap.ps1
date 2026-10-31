$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
$base = Join-Path ([Environment]::GetFolderPath('UserProfile')) '.gespan-host'
$version = @@VERSION@@
$release = @@RELEASE@@
$forceUpgrade = $env:GESPAN_HOST_FORCE_UPGRADE -eq '1'
$hostPort = if ($env:GESPAN_HOST_PORT) { [int] $env:GESPAN_HOST_PORT } else { 3774 }
@@ACL@@

function Download-Gespan([string] $Url, [string] $Destination) {
  Add-Type -AssemblyName System.Net.Http
  [Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12
  $handler = New-Object Net.Http.HttpClientHandler
  $handler.AllowAutoRedirect = $false
  $client = New-Object Net.Http.HttpClient($handler)
  $client.Timeout = [TimeSpan]::FromSeconds(180)
  try {
    $uri = [Uri] $Url
    for ($i = 0; $i -lt 10; $i++) {
      if ($uri.Scheme -ne 'https') { throw 'Host downloads require HTTPS.' }
      $response = $client.GetAsync($uri).GetAwaiter().GetResult()
      try {
        if ([int]$response.StatusCode -ge 300 -and [int]$response.StatusCode -lt 400) {
          if ($null -eq $response.Headers.Location) { throw 'Missing download redirect.' }
          $uri = New-Object Uri($uri, $response.Headers.Location)
          continue
        }
        $null = $response.EnsureSuccessStatusCode()
        [IO.File]::WriteAllBytes($Destination, $response.Content.ReadAsByteArrayAsync().GetAwaiter().GetResult())
        return
      } finally { $response.Dispose() }
    }
    throw 'Too many download redirects.'
  } finally { $client.Dispose(); $handler.Dispose() }
}

New-Item -ItemType Directory -Force -Path $base | Out-Null
Protect-GespanDirectory $base
$lock = $null
$temporary = $null
try {
  # File sharing locks serialize separate SSH logons, too.
  for ($attempt = 0; $attempt -lt 120; $attempt++) {
    try {
      $lock = [IO.File]::Open((Join-Path $base 'install.lock'), 'OpenOrCreate', 'ReadWrite', 'None')
      break
    } catch [IO.IOException] { Start-Sleep -Milliseconds 250 }
  }
  if ($null -eq $lock) { throw 'Another host installation is running. Try again shortly.' }
  $pointer = Join-Path $base 'runtime-path'
  $existed = Test-Path -LiteralPath $pointer
  if (-not $existed -or $forceUpgrade) {
    $arch = $env:PROCESSOR_ARCHITEW6432
    if (-not $arch) { $arch = $env:PROCESSOR_ARCHITECTURE }
    switch ($arch.ToUpperInvariant()) {
      'AMD64' { $target = 'win32-x64' }
      'ARM64' { $target = 'win32-arm64' }
      default { throw 'Gespan Host requires x64 or ARM64 Windows.' }
    }
    $filename = "gespan-host-$target.zip"
    $runtimeRoot = Join-Path $base 'runtime'
    New-Item -ItemType Directory -Force -Path $runtimeRoot | Out-Null
    $temporary = Join-Path $runtimeRoot ('.install-' + [Guid]::NewGuid().ToString('N'))
    New-Item -ItemType Directory -Path $temporary | Out-Null
    $archive = Join-Path $temporary $filename
    $checksum = Join-Path $temporary 'checksum'
    try {
      Download-Gespan "$release/$filename" $archive
      Download-Gespan "$release/$filename.sha256" $checksum
    } catch { throw "The Windows host package for version $version could not be downloaded. Install a release with host packages. $($_.Exception.Message)" }
    $expected = ((Get-Content -LiteralPath $checksum -Raw).Trim() -split '\s+')[0]
    if ($expected -notmatch '^[a-fA-F0-9]{64}$') { throw 'Invalid host package checksum.' }
    if ((Get-FileHash -LiteralPath $archive -Algorithm SHA256).Hash -ne $expected) { throw 'Gespan Host package checksum mismatch.' }
    $unpacked = Join-Path $temporary 'unpacked'
    Expand-Archive -LiteralPath $archive -DestinationPath $unpacked
    $actual = & (Join-Path $unpacked 'node.exe') (Join-Path $unpacked 'host.mjs') --version
    if ($LASTEXITCODE -ne 0 -or $actual -ne $version) { throw 'Gespan Host version mismatch.' }
    $runtime = Join-Path $runtimeRoot ("$version-$target-" + [Guid]::NewGuid().ToString('N'))
    # The version check above just ran node.exe from here. Windows can hold the
    # file for a moment after a process exits, and antivirus scans new binaries.
    # Rename the folder in one step. Move-Item isn't safe to retry here: it can
    # leave $runtime half filled, and the next try nests unpacked inside it.
    for ($attempt = 1; ; $attempt++) {
      try {
        [IO.Directory]::Move($unpacked, $runtime)
        break
      } catch [IO.IOException], [UnauthorizedAccessException] {
        if ($attempt -ge 40) { throw }
        Start-Sleep -Milliseconds 250
      }
    }
    $bin = Join-Path $base 'bin'
    New-Item -ItemType Directory -Force -Path $bin | Out-Null
    $runtimeName = Split-Path -Leaf $runtime
    # Keep the batch file ASCII; cmd's set /p would misread a UTF-8 profile path.
    $launcher = "@echo off`r`nsetlocal DisableDelayedExpansion`r`n`"%~dp0..\runtime\$runtimeName\node.exe`" `"%~dp0..\runtime\$runtimeName\host.mjs`" %*`r`nexit /b %errorlevel%`r`n"
    [IO.File]::WriteAllText((Join-Path $bin 'gespan-host.cmd'), $launcher, [Text.Encoding]::ASCII)
    $nextPointer = Join-Path $temporary 'runtime-path'
    [IO.File]::WriteAllText($nextPointer, $runtime, (New-Object Text.UTF8Encoding($false)))
    if (Test-Path -LiteralPath $pointer) {
      # Windows PowerShell coerces $null to an empty backup path here, which
      # .NET Framework rejects. Keep the previous pointer in this temporary
      # directory; the install's finally block removes it after replacement.
      [IO.File]::Replace($nextPointer, $pointer, (Join-Path $temporary 'previous-runtime-path'))
    } else {
      [IO.File]::Move($nextPointer, $pointer)
    }
  }
  $runtime = [IO.File]::ReadAllText($pointer).Trim()
  $node = Join-Path $runtime 'node.exe'
  $entry = Join-Path $runtime 'host.mjs'
  if ($existed -and $forceUpgrade) {
    & $node $entry service uninstall | Out-Null
    if ($LASTEXITCODE -ne 0) { throw 'Could not stop the old host service.' }
  }
  & $node $entry service install --port $hostPort | Out-Null
  if ($LASTEXITCODE -ne 0) { throw 'Host service setup failed. Check the error above and sign in to the Windows desktop as the SSH user.' }
  & $node $entry connection-info
  if ($LASTEXITCODE -ne 0) { throw 'The host did not report a connection.' }
} finally {
  if ($null -ne $lock) { $lock.Dispose() }
  if ($temporary -and (Test-Path -LiteralPath $temporary)) { Remove-Item -LiteralPath $temporary -Recurse -Force }
}
