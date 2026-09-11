# Publishes entmoot after crates.io post-delete name cooldown.
# Intended as a one-shot Task Scheduler job; unregister on success.

$ErrorActionPreference = "Stop"
$Repo = "G:\GitHub\entmoot"
$EnvFile = "G:\GitHub\logic-addressable-identifier\.env"
$LogDir = Join-Path $Repo "scripts"
$Log = Join-Path $LogDir "publish-after-cooldown.log"
$TaskName = "entmoot-publish-0.0.1"

function Write-Log([string]$Message) {
    $line = "{0:o} {1}" -f (Get-Date), $Message
    Add-Content -Path $Log -Value $line
    Write-Output $line
}

try {
    Write-Log "start"
    Set-Location $Repo

    $cargoHome = Join-Path $env:USERPROFILE ".cargo\bin"
    $cargo = Join-Path $cargoHome "cargo.exe"
    if (-not (Test-Path $cargo)) {
        throw "cargo not found at $cargo"
    }
    $env:Path = "$cargoHome;$env:Path"

    if (-not (Test-Path $EnvFile)) {
        throw "env file missing: $EnvFile"
    }

    Get-Content $EnvFile | ForEach-Object {
        if ($_ -match '^\s*CARGO_REGISTRY_TOKEN\s*=\s*(.+)\s*$') {
            $env:CARGO_REGISTRY_TOKEN = $Matches[1].Trim().Trim('"').Trim("'")
        }
    }
    if (-not $env:CARGO_REGISTRY_TOKEN) {
        throw "CARGO_REGISTRY_TOKEN not found in $EnvFile"
    }
    Write-Log "token loaded (len=$($env:CARGO_REGISTRY_TOKEN.Length))"

    $toml = Get-Content (Join-Path $Repo "Cargo.toml") -Raw
    if ($toml -notmatch 'version\s*=\s*"0\.0\.1"') {
        throw "Cargo.toml is not at version 0.0.1; aborting"
    }

    $status = & curl.exe -sS -o NUL -w "%{http_code}" -H "User-Agent: entmoot-publish-job" "https://crates.io/api/v1/crates/entmoot"
    Write-Log "crates.io status for entmoot: $status"
    if ($status -eq "200") {
        Write-Log "crate already exists; skipping publish"
        schtasks /Delete /TN $TaskName /F 2>$null | Out-Null
        exit 0
    }

    # Native cargo stderr must not trip $ErrorActionPreference Stop.
    $publishOut = & cmd /c "`"$cargo`" publish 2>&1"
    $publishOut | ForEach-Object { Write-Log $_ }
    if ($LASTEXITCODE -ne 0) {
        throw "cargo publish failed with exit $LASTEXITCODE"
    }

    Write-Log "publish succeeded"
    schtasks /Delete /TN $TaskName /F 2>$null | Out-Null
    Write-Log "scheduled task removed"
    exit 0
}
catch {
    Write-Log "ERROR: $($_.Exception.Message)"
    exit 1
}
finally {
    Remove-Item Env:CARGO_REGISTRY_TOKEN -ErrorAction SilentlyContinue
}
