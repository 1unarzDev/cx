# Join the __MESH_NAME__ mesh (cx docs/mesh.md), in an administrator PowerShell: irm __MESH_URL__/<code>|iex
# Internet over WARP (MASQUE), local networks untouched, the mesh reachable. Leave: irm __MESH_URL__/leave|iex
$ErrorActionPreference = 'Stop'
$Url = '__MESH_URL__'; $Code = '__MESH_CODE__'
$State = Join-Path $env:ProgramData 'cx-mesh'; $Mdm = Join-Path $env:ProgramData 'Cloudflare\mdm.xml'
$Cli = Join-Path $env:ProgramFiles 'Cloudflare\Cloudflare WARP\warp-cli.exe'
function Say($m) { Write-Host "[mesh] $m" }
if (-not $Code) { throw "this needs an invite code: ask the mesh owner for their command (irm $Url/<code>|iex)" }
$admin = ([Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
if (-not $admin) { throw 'run this in an administrator PowerShell (right-click PowerShell, Run as administrator)' }
if (Test-Path (Join-Path $State 'device.json')) { throw "this machine is already on a mesh; leave first: irm $Url/leave|iex" }
New-Item -ItemType Directory -Force $State | Out-Null
& icacls $State /inheritance:r /grant:r 'Administrators:(OI)(CI)F' 'SYSTEM:(OI)(CI)F' | Out-Null
$installed = $false
if (-not (Test-Path $Cli)) {
  Say 'installing Cloudflare WARP'
  if (Get-Command winget -ErrorAction SilentlyContinue) {
    winget install --id Cloudflare.Warp -e --silent --accept-package-agreements --accept-source-agreements | Out-Null
  } else {
    $msi = Join-Path $env:TEMP 'Cloudflare_WARP.msi'
    Invoke-WebRequest -UseBasicParsing -Uri 'https://downloads.cloudflareclient.com/v1/download/windows/ga' -OutFile $msi
    Start-Process msiexec.exe -Wait -ArgumentList '/i', "`"$msi`"", '/qn', '/norestart'
  }
  for ($i = 0; $i -lt 30 -and -not (Test-Path $Cli); $i++) { Start-Sleep 2 }
  $installed = $true
}
if (-not (Test-Path (Join-Path $State 'prior.json'))) {
  $hadMdm = Test-Path $Mdm
  if ($hadMdm) { Copy-Item $Mdm (Join-Path $State 'mdm.prior') }
  @{ installed = $installed; mdm_prior = $hadMdm } | ConvertTo-Json | Set-Content (Join-Path $State 'prior.json')
}
$name = if ($env:MESH_NAME) { $env:MESH_NAME } else { $env:COMPUTERNAME.ToLower() }
$j = Invoke-RestMethod -Method Post -Uri "$Url/v1/enroll" -ContentType 'application/json' -Body (@{ code = $Code; name = $name; host = $env:COMPUTERNAME; os = 'Windows' } | ConvertTo-Json)
@{ name = $j.name; url = $Url; token = $j.device_token } | ConvertTo-Json | Set-Content (Join-Path $State 'device.json')
& $Cli --accept-tos disconnect 2>$null | Out-Null
& $Cli --accept-tos registration delete 2>$null | Out-Null
New-Item -ItemType Directory -Force (Split-Path $Mdm) | Out-Null
"<dict>`n  <key>organization</key><string>$($j.org)</string>`n  <key>auth_client_id</key><string>$($j.client_id)</string>`n  <key>auth_client_secret</key><string>$($j.client_secret)</string>`n  <key>onboarding</key><false/>`n</dict>" | Set-Content $Mdm
Restart-Service CloudflareWARP
Say 'registering'
for ($i = 0; $i -lt 45; $i++) { & $Cli --accept-tos registration show 2>$null | Out-Null; if ($LASTEXITCODE -eq 0) { break }; Start-Sleep 2 }
& $Cli --accept-tos connect | Out-Null
$r = $null
for ($i = 0; $i -lt 20; $i++) {
  try { $r = Invoke-RestMethod -Method Post -Uri "$Url/v1/registered" -Headers @{ authorization = "Bearer $($j.device_token)" } -ContentType 'application/json' -Body (@{ host = $env:COMPUTERNAME } | ConvertTo-Json); break } catch { Start-Sleep 3 }
}
Say "on the mesh as $($j.name) ($($r.ipv4) $($r.ipv6))"
if (-not (Get-Service sshd -ErrorAction SilentlyContinue)) { Say 'no OpenSSH server; add it (Settings > Optional features > OpenSSH Server) to be reachable over ssh' }
$u = if ($j.ssh_user) { $j.ssh_user } else { $env:USERNAME }
Write-Host "ssh $u@$($j.fqdn)"
Say "leave: irm $Url/leave|iex"
