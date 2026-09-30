$m = @('5.011','30.012','30.013','30.014','30.015','1016','30.017')
Get-CimInstance Win32_Process | Where-Object { $c=$_.CommandLine; $c -and $_.Name -ne 'powershell.exe' -and ($m | Where-Object { $c.Contains($_) }) } | Select-Object ProcessId,ParentProcessId,Name,CreationDate,CommandLine | Format-List
"--- sleep/ping/cat now:"
Get-CimInstance Win32_Process | Where-Object { $_.Name -in 'sleep.exe','PING.EXE','cat.exe' } | Select-Object ProcessId,ParentProcessId,Name,CreationDate,CommandLine | Format-List
