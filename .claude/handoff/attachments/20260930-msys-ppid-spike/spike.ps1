$ErrorActionPreference = 'Continue'
$bash = 'C:\Program Files\Git\usr\bin\bash.exe'
$out = Join-Path $PSScriptRoot 'result.txt'
"" | Set-Content $out

$cases = @(
  @{n='1 loop';        c='while true; do /usr/bin/sleep 5.011; done';                         m='5.011'},
  @{n='2 bg&wait';     c='/usr/bin/sleep 30.012 & wait';                                     m='30.012'},
  @{n='3 pipeline';    c='/usr/bin/sleep 30.013 | /usr/bin/cat';                             m='30.013'},
  @{n='4 subshell';    c='( /usr/bin/sleep 30.014 ); true';                                  m='30.014'},
  @{n='5 exec-last';   c='/usr/bin/sleep 30.015';                                            m='30.015'},
  @{n='6 native loop'; c='while true; do ping -n 5 -w 1016 127.0.0.1 >/dev/null; done';      m='1016'},
  @{n='7 cmdsubst';    c='x=$(/usr/bin/sleep 30.017; echo); true';                           m='30.017'}
)

function Snap { Get-CimInstance Win32_Process | Select-Object ProcessId,ParentProcessId,Name,CreationDate,CommandLine }

foreach ($k in $cases) {
  $t0 = Get-Date
  $p = Start-Process -FilePath $bash -ArgumentList @('-c', "`"$($k.c)`"") -WindowStyle Hidden -PassThru
  Start-Sleep -Seconds 2.5
  $s = Snap
  $byPid = @{}; foreach ($r in $s) { $byPid[[int]$r.ProcessId] = $r }
  $kids = $s | Where-Object { $_.CommandLine -and $_.CommandLine.Contains($k.m) -and $_.Name -in @('sleep.exe','PING.EXE','ping.exe') }
  "=== CASE $($k.n)  launchedBashPid=$($p.Id) alive=$(-not $p.HasExited) t0=$($t0.ToString('HH:mm:ss.fff'))" | Add-Content $out
  if (-not $kids) { "  NO CHILD FOUND" | Add-Content $out }
  foreach ($kd in $kids) {
    $pp = [int]$kd.ParentProcessId
    $line = "  child $($kd.Name) pid=$($kd.ProcessId) created=$($kd.CreationDate.ToString('HH:mm:ss.fff')) ppid=$pp"
    if ($byPid.ContainsKey($pp)) { $line += " ppidAlive=YES name=$($byPid[$pp].Name) created=$($byPid[$pp].CreationDate.ToString('HH:mm:ss.fff'))" }
    else { $line += " ppidAlive=NO" }
    $line | Add-Content $out
    # chain
    $chain = @(); $cur = [int]$kd.ProcessId; $guard=0
    while ($byPid.ContainsKey($cur) -and $guard -lt 10) { $r=$byPid[$cur]; $chain += "$($r.Name)($cur)"; if ($cur -eq $p.Id) {break}; $cur=[int]$r.ParentProcessId; $guard++ }
    if (-not $byPid.ContainsKey($cur)) { $chain += "<dead $cur>" }
    "  chain: " + ($chain -join ' <- ') | Add-Content $out
  }
  # other processes descended from launched bash, for context
  $desc = $s | Where-Object { [int]$_.ParentProcessId -eq $p.Id } | ForEach-Object { "$($_.Name)($($_.ProcessId))" }
  "  direct children of launched bash: " + ($desc -join ', ') | Add-Content $out
  # is there any process with pid = launched bash? and processes whose parent is that
  # second snapshot for loop cases
  if ($k.n -match 'loop') {
    Start-Sleep -Seconds 5.5
    $s2 = Snap; $b2=@{}; foreach ($r in $s2){$b2[[int]$r.ProcessId]=$r}
    foreach ($kd in ($s2 | Where-Object { $_.CommandLine -and $_.CommandLine.Contains($k.m) -and $_.Name -in @('sleep.exe','PING.EXE','ping.exe') })) {
      $pp=[int]$kd.ParentProcessId
      "  [2nd iter] child pid=$($kd.ProcessId) ppid=$pp alive=$($b2.ContainsKey($pp)) name=$(if($b2.ContainsKey($pp)){$b2[$pp].Name})" | Add-Content $out
    }
  }
  # kill: launched bash + all markered children + descendants of launched bash
  $s3 = Snap
  $toKill = @($p.Id)
  $toKill += ($s3 | Where-Object { $_.CommandLine -and $_.CommandLine.Contains($k.m) } | ForEach-Object { [int]$_.ProcessId })
  $frontier = @($toKill)
  for ($i=0;$i -lt 5;$i++){ $n = $s3 | Where-Object { $frontier -contains [int]$_.ParentProcessId } | ForEach-Object {[int]$_.ProcessId}; $toKill += $n; $frontier=$n }
  $toKill = $toKill | Sort-Object -Unique
  foreach ($id in $toKill) { Stop-Process -Id $id -Force -ErrorAction SilentlyContinue }
  Start-Sleep -Seconds 1
  $left = Snap | Where-Object { ($toKill -contains [int]$_.ProcessId) -or ($_.CommandLine -and $_.CommandLine.Contains($k.m) -and $_.Name -ne 'powershell.exe') }
  "  killed: $($toKill -join ','); leftover: " + (($left | ForEach-Object { "$($_.Name)($($_.ProcessId))" }) -join ',') | Add-Content $out
}
"DONE" | Add-Content $out
