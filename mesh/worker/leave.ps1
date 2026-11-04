# Leave the __MESH_NAME__ mesh (cx docs/mesh.md), in an administrator PowerShell: irm __MESH_URL__/leave|iex
# Deregisters this machine and restores the MDM file, and removes WARP when the join installed it.
$ErrorActionPreference = 'Continue'
$State = Join-Path $env:ProgramData 'cx-mesh'; $Mdm = Join-Path $env:ProgramData 'Cloudflare\mdm.xml'
$Cli = Join-Path $env:ProgramFiles 'Cloudflare\Cloudflare WARP\warp-cli.exe'
if (-not (Test-Path $State)) { Write-Host '[mesh] this machine is not on a mesh'; return }
$d = Get-Content (Join-Path $State 'device.json') -ErrorAction SilentlyContinue | ConvertFrom-Json
$p = Get-Content (Join-Path $State 'prior.json') -ErrorAction SilentlyContinue | ConvertFrom-Json
if ($d.token) {
  try { Invoke-RestMethod -Method Post -Uri "$($d.url)/v1/leave" -Headers @{ authorization = "Bearer $($d.token)" } | Out-Null; Write-Host "[mesh] deregistered $($d.name)" }
  catch { Write-Host "[mesh] the server did not confirm; ask the mesh owner to run: cx mesh revoke $($d.name)" }
}
if (Test-Path $Cli) { & $Cli --accept-tos disconnect 2>$null | Out-Null; & $Cli --accept-tos registration delete 2>$null | Out-Null }
Remove-Item $Mdm -ErrorAction SilentlyContinue
if ($p.mdm_prior) { Copy-Item (Join-Path $State 'mdm.prior') $Mdm }
if ($p.installed) { Write-Host '[mesh] removing Cloudflare WARP (this join installed it)'; if (Get-Command winget -ErrorAction SilentlyContinue) { winget uninstall --id Cloudflare.Warp -e --silent | Out-Null } else { $app = Get-CimInstance Win32_Product -Filter "Name LIKE 'Cloudflare WARP%'"; if ($app) { Start-Process msiexec.exe -Wait -ArgumentList '/x', $app.IdentifyingNumber, '/qn' } } }
Remove-Item -Recurse -Force $State
Write-Host '[mesh] left the mesh; the previous network state is back'
