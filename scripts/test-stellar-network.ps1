# Run from Windows PowerShell: .\scripts\test-stellar-network.ps1
# Prompts for the SAME key embedded in the failing build. Never prints the key,
# writes it to disk, or passes it as a process command-line argument.
[CmdletBinding()]
param(
    [switch]$WithoutKey,
    [ValidateRange(1, 5)][int]$Rounds = 2
)
$ErrorActionPreference = 'Stop'
$curl = (Get-Command curl.exe -ErrorAction Stop).Source
$key = ''
$config = ''
try {
    if (-not $WithoutKey) {
        $secure = Read-Host 'Stellar API key (hidden)' -AsSecureString
        $ptr = [Runtime.InteropServices.Marshal]::SecureStringToBSTR($secure)
        try { $key = [Runtime.InteropServices.Marshal]::PtrToStringBSTR($ptr) }
        finally { [Runtime.InteropServices.Marshal]::ZeroFreeBSTR($ptr); $secure.Dispose() }
        if (-not $key -or $key -match '[^\x20-\x7e]') {
            throw 'Enter a nonempty printable ASCII API key.'
        }
        $escaped = $key.Replace('\', '\\').Replace('"', '\"')
        $config = 'header = "X-API-Key: ' + $escaped + '"'
    }
    Write-Output 'Direct HTTPS test: HTTP/1.1, Apogee/0.6.1, no Origin, no proxy, certificate validation enabled.'
    Write-Output ('API key present: ' + [bool]$key)
    for ($round = 1; $round -le $Rounds; $round++) {
        # Reverse the order on alternate rounds to reduce timing bias.
        $families = @(4, 6)
        if ($round % 2 -eq 0) { $families = @(6, 4) }
        foreach ($family in $families) {
            Write-Output ("Round {0}, IPv{1}, UTC {2}" -f $round, $family, [DateTime]::UtcNow.ToString('o'))
            $curlArgs = @('--disable', '--config', '-', "-$family", '--http1.1',
                '--noproxy', '*', '--silent', '--connect-timeout', '10', '--max-time', '25',
                '--user-agent', 'Apogee/0.6.1', '--dump-header', '-', '--output', 'NUL',
                '--write-out', '\nRESULT status=%{http_code} remote=%{remote_ip} protocol=%{http_version} seconds=%{time_total}\n',
                'https://api.stellartunerlog.com/v1/nowplaying')
            $output = $config | & $curl @curlArgs
            $curlExit = $LASTEXITCODE
            foreach ($line in $output) {
                # Allowlist output: do not dump cookies or arbitrary response headers.
                if ($line -match '^(HTTP/|RESULT |cf-ray:|cf-error-type:|cf-error-origin:|server:|date:|cf-cache-status:)') {
                    $safe = $line
                    if ($key) { $safe = $safe.Replace($key, '[redacted]') }
                    Write-Output $safe
                }
            }
            Write-Output "curl_exit=$curlExit (0=completed, 6=DNS failure, 7=connect failure, 28=timeout)"
            Start-Sleep -Seconds 2
        }
    }
} finally {
    $key = $null
    $escaped = $null
    $config = $null
}
