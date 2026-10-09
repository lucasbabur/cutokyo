param(
  [switch]$SkipClaude,
  [switch]$SkipCodex
)

$ErrorActionPreference = "Stop"

$apiBase = "http://127.0.0.1:49321"
$proxy = "http://127.0.0.1:49322"
$caPath = Join-Path $env:USERPROFILE ".cutokyo\capture\cutokyo-root-ca.pem"
$prompt = @'
Use your Bash tool. Run this harmless noisy command, then summarize only the important error lines:
for i in $(seq 1 220); do echo "INFO indexing workspace"; done; echo "ERROR failed at src/proxy.ts line 42"; echo "Traceback app/main.py line 88"; for i in $(seq 1 180); do echo "INFO retrying request"; done

After the tool call finishes, answer with:
- how many ERROR/Traceback lines appeared
- the exact file paths and line numbers mentioned
'@

function Set-CutokyoProxyEnv {
  $env:HTTP_PROXY = $proxy
  $env:HTTPS_PROXY = $proxy
  $env:ALL_PROXY = $proxy
  $env:NO_PROXY = "localhost,127.0.0.1"
  if (Test-Path $caPath) {
    $env:NODE_EXTRA_CA_CERTS = $caPath
    $env:SSL_CERT_FILE = $caPath
  }
}

function Get-CutokyoMetrics {
  Invoke-RestMethod -Uri "$apiBase/cutokyo/metrics" -TimeoutSec 5
}

function Show-MetricsDelta($label, $before, $after) {
  [pscustomobject]@{
    label = $label
    beforeRequests = $before.requests
    afterRequests = $after.requests
    beforeCompressed = $before.compressedRequests
    afterCompressed = $after.compressedRequests
    beforeSaved = $before.totalEstimatedTokensSaved
    afterSaved = $after.totalEstimatedTokensSaved
    lastProvider = $after.lastProvider
    lastChangedFields = $after.lastChangedFields
    lastSaved = $after.lastEstimatedTokensSaved
  } | ConvertTo-Json -Compress
}

Set-CutokyoProxyEnv

$status = Invoke-RestMethod -Uri "$apiBase/cutokyo/status" -TimeoutSec 5
if ($status.status -ne "ready") {
  throw "Cutokyo API is not ready: $($status | ConvertTo-Json -Compress)"
}

if (!$SkipClaude) {
  $before = Get-CutokyoMetrics
  $claudeOutput = & claude `
    -p `
    --permission-mode bypassPermissions `
    --allowedTools Bash `
    --output-format text `
    --no-session-persistence `
    $prompt
  $claudeExit = $LASTEXITCODE
  Start-Sleep -Seconds 2
  $after = Get-CutokyoMetrics
  Show-MetricsDelta "claude" $before $after
  if ($claudeExit -ne 0) {
    throw "claude -p failed with exit code $claudeExit. Output: $claudeOutput"
  }
  if ($after.compressedRequests -le $before.compressedRequests) {
    throw "claude -p did not produce a compressed Cutokyo request. Output: $claudeOutput"
  }
}

if (!$SkipCodex) {
  $before = Get-CutokyoMetrics
  $codexOutput = $prompt | & codex exec `
    --skip-git-repo-check `
    --dangerously-bypass-approvals-and-sandbox `
    --cd $env:TEMP `
    --color never `
    -
  $codexExit = $LASTEXITCODE
  Start-Sleep -Seconds 2
  $after = Get-CutokyoMetrics
  Show-MetricsDelta "codex" $before $after
  if ($codexExit -ne 0) {
    throw "codex exec failed with exit code $codexExit. Output: $codexOutput"
  }
  if ($after.compressedRequests -le $before.compressedRequests) {
    throw "codex exec did not produce a compressed Cutokyo request. Output: $codexOutput"
  }
}
