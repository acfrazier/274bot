param([Parameter(Mandatory=$true)][ValidatePattern('^[A-Za-z0-9_-]+$')][string]$CellId,[switch]$Final)
$ErrorActionPreference='Stop'; $run=Join-Path 'C:\Users\BotTest' ('274bot-runs\managed-tile-boxed-'+$CellId); $name='274bot-BotTest-tile-boxed-'+$CellId; $task=Get-ScheduledTask -TaskName $name -ErrorAction SilentlyContinue; $info=if($task){Get-ScheduledTaskInfo -TaskName $name}else{$null}; $completion=Join-Path $run 'completion.json'; $startup=@()
function Read-StartupLine([string]$line,[string]$path){
    if($line -notmatch '"event"\s*:\s*"startup"' -or $line -notmatch '"adapter_name"\s*:'){return}
    try {
        $first=$line.IndexOf('{'); $last=$line.LastIndexOf('}')
        if($first -lt 0 -or $last -le $first){return}
        $obj=$line.Substring($first,$last-$first+1)|ConvertFrom-Json
        if($obj.event -eq 'startup'){$script:startup+=[pscustomobject]@{path=$path;adapter_name=$obj.adapter_name;requested_adapter_name=$obj.requested_adapter_name;adapter_backend=$obj.adapter_backend}}
    } catch {}
}
if(Test-Path $run){
    foreach($receiptPath in @(Get-ChildItem $run -Recurse -File -Filter receipt.json -ErrorAction SilentlyContinue)){
        try{$receipt=Get-Content $receiptPath.FullName -Raw|ConvertFrom-Json;$raw=[string]$receipt.run_dir}catch{continue}
        if($raw -and (Test-Path $raw)){
            foreach($f in @(Get-ChildItem $raw -Recurse -File -Include *.json,*.log -ErrorAction SilentlyContinue)){
                foreach($line in @(Get-Content $f.FullName -ErrorAction SilentlyContinue)){Read-StartupLine $line $f.FullName}
            }
        }
    }
}
$result=[ordered]@{utc=[DateTime]::UtcNow.ToString('o');task=$name;cell_id=$CellId;taskState=if($task){[string]$task.State}else{'Missing'};lastTaskResult=if($info){$info.LastTaskResult}else{$null};completionPresent=(Test-Path $completion);startup=$startup;performanceAcceptance=$false};$result|ConvertTo-Json -Depth 6
if($Final){
    if(-not (Test-Path $completion)){throw 'Completion receipt missing'}
    $c=Get-Content $completion -Raw|ConvertFrom-Json
    if($c.exit_code -ne 0){throw ('Diagnostic exit code '+$c.exit_code)}
    $requested='Intel(R) Graphics'
    $matching=@($startup|Where-Object {$_.adapter_name -eq $requested -and $_.requested_adapter_name -eq $requested})
    if($matching.Count -lt 1){throw 'No startup record proves requested and selected adapter both equal Intel(R) Graphics'}
}
