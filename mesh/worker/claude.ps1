# Point Claude Code at a shared gateway (cx docs/claude.md): irm __MESH_URL__/claude/<code>|iex
# Claims the lend once, merges ANTHROPIC_BASE_URL, ANTHROPIC_AUTH_TOKEN and (off the mesh) ANTHROPIC_CUSTOM_HEADERS
# into the env of Claude Code's settings.json (backed up first), then checks the gateway with one cheap request.
& {
  $ErrorActionPreference = 'Stop'
  $Url = '__MESH_URL__'; $Code = '__CLAUDE_CODE__'; $MeshUrl = '__CLAUDE_MESH_URL__'; $MeshOnly = '__CLAUDE_MESH_ONLY__'
  $Dir = if ($env:CLAUDE_CONFIG_DIR) { $env:CLAUDE_CONFIG_DIR } else { Join-Path $HOME '.claude' }
  $Settings = Join-Path $Dir 'settings.json'
  function Say($m) { Write-Host "[claude] $m" }

  $Route = 'access'
  if ($MeshUrl -and $env:CLAUDE_LEND_ROUTE -ne 'access') {
    try { if ((Invoke-WebRequest -UseBasicParsing -TimeoutSec 4 "$MeshUrl/health").StatusCode -eq 200) { $Route = 'mesh' } } catch {}
  }
  if ($env:CLAUDE_LEND_ROUTE -eq 'mesh') { $Route = 'mesh' }
  if ($Route -eq 'access' -and $MeshOnly) { Write-Error "this lend works only on the mesh, and $MeshUrl does not answer: join the mesh, then run this again (the code is still unused)"; return }
  Say "route: $Route"

  try { $L = Invoke-RestMethod -Method Post -ContentType 'application/json' -Body (@{ code = $Code } | ConvertTo-Json) "$Url/v1/claude/claim" }
  catch { $m = try { ($_.ErrorDetails.Message | ConvertFrom-Json).error } catch { "$_" }; Write-Error $m; return }
  $Base = if ($Route -eq 'mesh') { $MeshUrl } else { $L.base_url }
  $Headers = if ($Route -eq 'mesh') { @() } else { @($L.headers) }

  New-Item -ItemType Directory -Force $Dir | Out-Null
  $d = [pscustomobject]@{}
  if ((Test-Path $Settings) -and (Get-Item $Settings).Length) {
    $b = "$Settings.bak-$(Get-Date -Format yyyyMMdd-HHmmss)"; Copy-Item $Settings $b; Say "backed up $Settings to $b"
    $d = Get-Content -Raw $Settings | ConvertFrom-Json
  }
  if (-not $d.PSObject.Properties['env']) { $d | Add-Member env ([pscustomobject]@{}) }
  $e = $d.env
  $set = @{ ANTHROPIC_BASE_URL = $Base; ANTHROPIC_AUTH_TOKEN = $L.token }
  if ($Headers.Count) { $set.ANTHROPIC_CUSTOM_HEADERS = ($Headers -join "`n") } else { $e.PSObject.Properties.Remove('ANTHROPIC_CUSTOM_HEADERS') }
  foreach ($k in $set.Keys) { $e | Add-Member -Force $k $set[$k] }
  [IO.File]::WriteAllText($Settings, ($d | ConvertTo-Json -Depth 32), (New-Object Text.UTF8Encoding $false)) # no BOM
  Say "wrote ANTHROPIC_BASE_URL ($Base), ANTHROPIC_AUTH_TOKEN$(if ($Headers.Count) { ' and ANTHROPIC_CUSTOM_HEADERS' }) to $Settings"

  $h = @{ authorization = "Bearer $($L.token)"; 'anthropic-version' = '2023-06-01' }
  foreach ($x in $Headers) { $k, $v = $x -split ':\s*', 2; $h[$k] = $v }
  $body = '{"model":"claude-haiku-4-5","messages":[{"role":"user","content":"ok"}]}'
  try {
    Invoke-RestMethod -Method Post -ContentType 'application/json' -Headers $h -Body $body "$Base/v1/messages/count_tokens" | Out-Null
    Say "$($L.name): ready (the gateway answered; start claude)"
  } catch { Write-Error "settings are written, but the gateway did not answer: $($_.Exception.Message); tell whoever sent the code" }
}
